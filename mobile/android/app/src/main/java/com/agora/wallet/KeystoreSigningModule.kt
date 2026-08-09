package com.agora.wallet

import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyInfo
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import android.util.Base64
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import java.security.KeyStore
import java.security.KeyStoreException
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.SecretKeyFactory
import javax.crypto.spec.GCMParameterSpec

/**
 * Wraps (encrypts) and unwraps (decrypts) an arbitrary secret — in practice
 * this app's sr25519 wallet seed, see `mobile/src/chain/keystoreWallet.ts` —
 * using a 256-bit AES-GCM key that lives in the Android Keystore
 * (`KeyStore.getInstance("AndroidKeyStore")`), generated non-exportable
 * (`KeyProperties.PURPOSE_ENCRYPT or PURPOSE_DECRYPT`, no
 * `PURPOSE_SIGN`/export path exists for it) and, on devices with a StrongBox
 * secure element (API 28+, best-effort — see `getOrCreateWrappingKey`),
 * backed by that dedicated hardware rather than just the TEE.
 *
 * ## Why this wraps a seed instead of holding the signing key itself
 *
 * This is genuinely hardware-backed encryption **at rest**, but it is
 * deliberately *not* the same guarantee as "the private key material never
 * leaves secure hardware, even during use" that CLAUDE.md's Identity System
 * section describes as the target. That stronger guarantee is not achievable
 * here: Agora's chain (`pallets/pallet-identity`, `runtime/src/lib.rs` via
 * `sp_runtime::MultiSignature`) verifies Sr25519/Ed25519/Ecdsa(secp256k1)
 * signatures, and Sr25519 signing itself is done in JS by `@polkadot/keyring`
 * (Schnorrkel/Ristretto over Wasm). Android Keystore's own key-generation and
 * signing operations only cover NIST curves (P-256/P-384/P-521) and RSA —
 * confirmed against the `KeyGenParameterSpec`/`KeyProperties` API surface —
 * so it cannot generate or sign with an Sr25519 key directly, and a Keystore
 * key's whole non-exportability guarantee only holds for operations Keystore
 * itself performs.
 *
 * Given that, the realistic improvement over the previous
 * `DEV_ONLY_MNEMONIC` (a bare hardcoded 32-byte seed, identical on every
 * install, held in JS-land in plaintext) is: generate a real random seed
 * on-device, and never let it touch disk unencrypted. The AES wrapping key
 * here is non-exportable and (best-effort) hardware-isolated; the wrapped
 * seed file on disk is useless without it. The seed is still decrypted into
 * JS memory for the lifetime of a signing operation (there is no way around
 * this without either a runtime change to accept a NIST-curve signature or a
 * from-scratch Sr25519-in-secure-hardware implementation, neither of which
 * this pass attempts) — this module's job is solely to keep the seed off
 * disk in plaintext and stop a single file-read from recovering it.
 *
 * See `mobile/src/native/keystoreSigner.ts` for the JS bridge and
 * `mobile/src/chain/keystoreWallet.ts` for how the wrapped seed is persisted
 * (via `react-native-fs`) and turned into a `KeyringPair`.
 *
 * Structurally this follows `../nfc/NfcPassportModule.kt` /
 * `NfcPassportPackage.kt`: a classic (pre-Codegen) `ReactContextBaseJavaModule`
 * + `ReactPackage` pair, registered manually in `MainApplication.kt` — this
 * app has `newArchEnabled=false` (`android/gradle.properties`), so a
 * TurboModule/Codegen spec would silently fail to load under the old bridge.
 */
class KeystoreSigningModule(reactContext: ReactApplicationContext) : ReactContextBaseJavaModule(reactContext) {

  companion object {
    private const val ANDROID_KEYSTORE = "AndroidKeyStore"
    private const val KEY_ALIAS = "agora_wallet_wrapping_key"
    private const val TRANSFORMATION = "AES/GCM/NoPadding"

    /** GCM's standard, non-negotiable authentication tag length. */
    private const val GCM_TAG_BITS = 128
  }

  override fun getName(): String = "KeystoreSigningModule"

  /**
   * Returns the AES-GCM wrapping key from the Android Keystore, generating
   * it on first use. Idempotent — a second call on a device that already has
   * the alias just loads the existing key; Keystore itself is the source of
   * truth, nothing is cached here.
   *
   * Attempts StrongBox (a dedicated secure-element chip, where present) on
   * API 28+ first, falling back to the TEE-backed (non-StrongBox) key if the
   * device has no StrongBox — `StrongBoxUnavailableException` is the
   * documented signal for exactly that, not a real error.
   */
  @Throws(Exception::class)
  private fun getOrCreateWrappingKey(): SecretKey {
    val keyStore = KeyStore.getInstance(ANDROID_KEYSTORE)
    keyStore.load(null)

    (keyStore.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }

    val purposes = KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT

    fun buildSpec(strongBox: Boolean): KeyGenParameterSpec {
      val builder = KeyGenParameterSpec.Builder(KEY_ALIAS, purposes)
        .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
        .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
        .setKeySize(256)
        // No biometric/lock-screen gate on the wrapping key itself in this
        // pass — signing extrinsics/auth challenges happens from screens
        // that don't currently have a re-auth prompt flow. Requiring
        // authentication here without one would just make every signature
        // fail. A follow-up could add setUserAuthenticationRequired(true)
        // once AuthScreen/VoteScreen have a matching re-auth step.
        .setUserAuthenticationRequired(false)
      if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
        builder.setIsStrongBoxBacked(true)
      }
      return builder.build()
    }

    val keyGenerator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
    try {
      keyGenerator.init(buildSpec(strongBox = true))
      return keyGenerator.generateKey()
    } catch (e: StrongBoxUnavailableException) {
      // Expected on the majority of devices (no StrongBox secure element) —
      // fall back to the standard TEE-backed key, still non-exportable.
      val fallbackGenerator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
      fallbackGenerator.init(buildSpec(strongBox = false))
      return fallbackGenerator.generateKey()
    }
  }

  /**
   * Encrypts `plaintextB64` (base64-encoded arbitrary bytes — the caller,
   * `keystoreSigner.ts`, handles the binary<->base64 conversion since the RN
   * bridge cannot carry raw binary) under the Keystore wrapping key.
   * Resolves `{ ciphertext, iv }`, both base64. A fresh random IV is
   * generated by `Cipher` itself for every call — GCM requires a unique IV
   * per encryption under the same key, never reused.
   */
  @ReactMethod
  fun encryptSecret(plaintextB64: String, promise: Promise) {
    try {
      val plaintext = Base64.decode(plaintextB64, Base64.NO_WRAP)
      val key = getOrCreateWrappingKey()
      val cipher = Cipher.getInstance(TRANSFORMATION)
      cipher.init(Cipher.ENCRYPT_MODE, key)
      val ciphertext = cipher.doFinal(plaintext)

      val result = com.facebook.react.bridge.Arguments.createMap().apply {
        putString("ciphertext", Base64.encodeToString(ciphertext, Base64.NO_WRAP))
        putString("iv", Base64.encodeToString(cipher.iv, Base64.NO_WRAP))
      }
      promise.resolve(result)
    } catch (e: KeyStoreException) {
      promise.reject("KEYSTORE_UNAVAILABLE", "Android Keystore is not available on this device: ${e.message}", e)
    } catch (e: Exception) {
      promise.reject("KEYSTORE_ENCRYPT_FAILED", e.message ?: "Failed to encrypt secret", e)
    }
  }

  /**
   * Decrypts a `{ ciphertextB64, ivB64 }` pair previously produced by
   * [encryptSecret]. Resolves the base64-encoded plaintext. Fails (rejects,
   * does not silently return garbage — GCM's authentication tag guarantees
   * this) if the ciphertext was tampered with, the IV doesn't match, or the
   * wrapping key alias no longer exists (e.g. the user cleared app data
   * partially, or Keystore itself was reset — `keystoreWallet.ts` treats
   * this as "no wallet on file" and generates a fresh one, per its own doc
   * comment).
   */
  @ReactMethod
  fun decryptSecret(ciphertextB64: String, ivB64: String, promise: Promise) {
    try {
      val ciphertext = Base64.decode(ciphertextB64, Base64.NO_WRAP)
      val iv = Base64.decode(ivB64, Base64.NO_WRAP)
      val key = getOrCreateWrappingKey()
      val cipher = Cipher.getInstance(TRANSFORMATION)
      cipher.init(Cipher.DECRYPT_MODE, key, GCMParameterSpec(GCM_TAG_BITS, iv))
      val plaintext = cipher.doFinal(ciphertext)
      promise.resolve(Base64.encodeToString(plaintext, Base64.NO_WRAP))
    } catch (e: KeyStoreException) {
      promise.reject("KEYSTORE_UNAVAILABLE", "Android Keystore is not available on this device: ${e.message}", e)
    } catch (e: Exception) {
      promise.reject("KEYSTORE_DECRYPT_FAILED", e.message ?: "Failed to decrypt secret", e)
    }
  }

  /**
   * Resolves `true` if the wrapping key currently reports
   * `KeyInfo.isInsideSecureHardware` (TEE or, where available, StrongBox),
   * `false` otherwise. Purely informational — exposed so the JS side can
   * surface it in diagnostics/support flows; nothing in this module's own
   * encrypt/decrypt behavior depends on the answer.
   */
  @ReactMethod
  fun isHardwareBacked(promise: Promise) {
    try {
      val key = getOrCreateWrappingKey()
      val factory = SecretKeyFactory.getInstance(key.algorithm, ANDROID_KEYSTORE)
      val keyInfo = factory.getKeySpec(key, KeyInfo::class.java) as KeyInfo
      promise.resolve(keyInfo.isInsideSecureHardware)
    } catch (e: Exception) {
      promise.reject("KEYSTORE_INFO_FAILED", e.message ?: "Failed to read wrapping key info", e)
    }
  }
}
