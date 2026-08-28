package com.agora.integrity

import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.google.android.play.core.integrity.IntegrityManagerFactory
import com.google.android.play.core.integrity.IntegrityTokenRequest

/**
 * Requests a Google Play Integrity API token bound to a caller-supplied
 * nonce — the client-side half of the device/app-integrity attestation
 * captured alongside registration (see `mobile/src/chain/deviceIntegrity.ts`
 * for the full design note: why this exists, why it can only ever be a
 * defense-in-depth *signal* from this app's side, and — the important part —
 * exactly why verifying the returned token requires a server-side call to
 * Google that **does not exist anywhere in this codebase**. Read that file
 * before touching this one.
 *
 * Uses the Play Integrity **classic** request shape
 * (`IntegrityManagerFactory` / `IntegrityTokenRequest.builder().setNonce(...)`)
 * rather than the newer Standard API (`StandardIntegrityManager`, request-hash
 * based, requires a one-time `prepareIntegrityToken` warm-up call). The
 * classic nonce-based shape was chosen because it maps directly onto what
 * this app already has — a fresh per-attempt nonce
 * (`deviceIntegrity.ts`'s `generateDeviceIntegrityNonce`) — with no extra
 * warm-up step, and its stated 5-requests-per-minute-per-app-instance budget
 * is comfortably enough for "once per registration attempt." If usage ever
 * needs to scale beyond that, the Standard API is the documented upgrade
 * path — not evaluated further here since nothing in this app currently
 * needs it.
 *
 * Gradle dependency: `com.google.android.play:integrity:1.6.0`
 * (`android/app/build.gradle`) — supersedes the older, now-deprecated
 * `com.google.android.play:core` artifact that used to bundle this API
 * alongside unrelated Play Core features (in-app review, app updates, asset
 * delivery); the standalone `integrity` artifact is Google's current
 * recommended dependency for this API specifically.
 *
 * Not source-verified beyond Android's published Play Integrity
 * documentation (developer.android.com/google/play/integrity/classic) — same
 * standing caveat as this app's other native modules (CameraX/ML Kit in
 * `FaceCaptureModule.kt`): not compiled or run, no Android SDK in this
 * environment. Requesting a *real* verdict additionally needs this app to
 * actually be registered with a Google Play Console listing (a real package
 * name + signing cert Google recognizes) and a device with real Play
 * Services — neither of which exist in this development environment either,
 * so this has never been exercised even at the "does it compile and return
 * *something*" level.
 *
 * `newArchEnabled=false` applies here too, same as every other native module
 * in this app — classic `ReactContextBaseJavaModule`.
 */
class PlayIntegrityModule(private val reactContext: ReactApplicationContext) :
  ReactContextBaseJavaModule(reactContext) {

  override fun getName(): String = "PlayIntegrityModule"

  /**
   * Requests a fresh integrity token bound to [nonceBase64] (must already be
   * base64, URL-safe, no padding — see `deviceIntegrity.ts`'s
   * `nonceToBase64Url`, the only producer of this argument). Resolves with
   * the raw, opaque, encrypted token Google's client library returns — this
   * module makes no attempt to interpret it, since it structurally can't
   * (see this file's doc comment). Rejects if Play Services/Play Integrity
   * is unavailable or the request otherwise fails; callers
   * (`../chain/deviceIntegrity.ts`) treat any rejection as "no signal this
   * attempt" and proceed without blocking registration.
   */
  @ReactMethod
  fun requestIntegrityToken(nonceBase64: String, promise: Promise) {
    val integrityManager = IntegrityManagerFactory.create(reactContext.applicationContext)
    integrityManager.requestIntegrityToken(
      IntegrityTokenRequest.builder().setNonce(nonceBase64).build(),
    ).addOnSuccessListener { response ->
      promise.resolve(response.token())
    }.addOnFailureListener { exception ->
      promise.reject("PLAY_INTEGRITY_ERROR", exception.message ?: "Play Integrity token request failed", exception)
    }
  }
}
