package com.agora.nfc

import android.nfc.Tag
import android.nfc.tech.IsoDep
import android.util.Base64
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import net.sf.scuba.smartcards.CardFileInputStream
import net.sf.scuba.smartcards.CardServiceException
import net.sf.scuba.smartcards.IsoDepCardService
import org.jmrtd.BACKey
import org.jmrtd.CardServiceProtocolException
import org.jmrtd.PassportService
import java.io.IOException

/**
 * Reads the RAW EF.DG1 / EF.DG15 / EF.SOD data-group bytes off a biometric
 * passport's NFC chip via Basic Access Control (BAC) — not parsed/summarized
 * fields, since these bytes are direct witness inputs to the Rarimo ZK
 * circuit (see `mobile/src/chain/zkProving.ts` / `proofEncoding.ts` for the
 * proving half this feeds; this module is strictly upstream and does not
 * touch those files).
 *
 * Built on **JMRTD** (`org.jmrtd:jmrtd`, LGPL) — the real, current, standard
 * open-source library for ICAO 9303 chip access, and confirmed (HANDOFF.md
 * log #57) to be exactly what Rarimo's own production Android app pins,
 * alongside **scuba-sc-android** (`net.sf.scuba:scuba-sc-android`), the
 * bridge from Android's `android.nfc.tech.IsoDep` to JMRTD's `CardService`
 * abstraction. Every JMRTD/scuba API call below was cross-checked against
 * the libraries' actual source (jmrtd `master` branch mirror +
 * `ugochirico/SCUBA` mirror of the SourceForge `scuba` project — see
 * HANDOFF.md log #57), not written from memory/pattern-matching.
 *
 * This app has `newArchEnabled=false` (mobile/android/gradle.properties), so
 * this is deliberately a classic `ReactContextBaseJavaModule` +
 * `ReactPackage` (see `NfcPassportPackage.kt`), not a TurboModule/Codegen
 * spec — a Codegen spec would silently fail to load under the old bridge.
 *
 * ## Tag delivery
 * This module does NOT talk to `NfcAdapter` directly — Android's NFC
 * foreground-dispatch pattern requires `Activity`-level `onResume`/`onPause`/
 * `onNewIntent` hooks, which live in `MainActivity.kt`. That activity calls
 * the static `onTagDiscovered` below whenever a tag intent arrives while the
 * app is foregrounded; this module matches it up with whatever `readPassport`
 * call is currently waiting (if any) and does the actual chip read on a
 * background thread (BAC + APDU exchange are blocking I/O and must never run
 * on the main/UI thread that `onNewIntent` runs on).
 *
 * ## Scope: BAC only, not PACE
 * `PassportService.doPACE(...)` exists and was read from source, but using it
 * correctly requires first reading and parsing `EF.CardAccess` (only
 * accessible in the *root* file system, before `sendSelectApplet`) to pick
 * the right `PACEInfo` (OID + domain params) for a given chip — real
 * additional protocol logic this pass didn't have enough source-verified
 * confidence in to implement correctly. BAC alone (this module) covers every
 * passport that doesn't mandate PACE, which is the large majority of
 * currently-issued ICAO passports as of this writing.
 */
class NfcPassportModule(reactContext: ReactApplicationContext) : ReactContextBaseJavaModule(reactContext) {

  companion object {
    /** Milliseconds to wait for a single APDU round-trip before the `IsoDep` connection gives up. */
    private const val TAG_TIMEOUT_MS = 20_000

    /** The module instance currently attached to the bridge, if any — see `onTagDiscovered`. */
    @Volatile
    private var activeInstance: NfcPassportModule? = null

    /**
     * Called by `MainActivity.onNewIntent` (main/UI thread) whenever Android's
     * NFC foreground dispatch delivers a discovered tag. If a `readPassport`
     * call is currently pending on the active module instance, the tag is
     * handed off to it; otherwise it is silently ignored (e.g. a stray tap
     * with no pending read, or the module hasn't been constructed yet).
     */
    @JvmStatic
    fun onTagDiscovered(tag: Tag) {
      activeInstance?.handleTagDiscovered(tag)
    }
  }

  /** Guards `pendingPromise`/`pendingBacKey` against concurrent access from the JS thread (`readPassport`) and the UI thread (`onTagDiscovered`). */
  private val lock = Any()
  private var pendingPromise: Promise? = null
  private var pendingBacKey: BACKey? = null

  init {
    activeInstance = this
  }

  override fun getName(): String = "NfcPassportModule"

  /**
   * Arms this module to read the next NFC tag presented to the phone using
   * the given MRZ fields as the BAC key. Resolves once a tag is presented
   * *and* the full BAC + DG1/DG15/SOD read completes; rejects on any
   * failure (BAC denied — usually wrong MRZ data, connection lost mid-read,
   * unsupported/non-ICAO tag, etc).
   *
   * `dateOfBirth`/`dateOfExpiry` must be `YYMMDD` (confirmed against
   * `BACKey`'s source: `SDF = "yyMMdd"`, and its constructor throws
   * `IllegalArgumentException` if either string isn't exactly 6 characters).
   *
   * Only one read may be pending at a time.
   */
  @ReactMethod
  fun readPassport(documentNumber: String, dateOfBirth: String, dateOfExpiry: String, promise: Promise) {
    synchronized(lock) {
      if (pendingPromise != null) {
        promise.reject("NFC_BUSY", "A passport read is already waiting for a tag or in progress")
        return
      }

      val bacKey: BACKey
      try {
        bacKey = BACKey(documentNumber, dateOfBirth, dateOfExpiry)
      } catch (e: IllegalArgumentException) {
        promise.reject("NFC_INVALID_MRZ", "Invalid MRZ data (expected YYMMDD dates): ${e.message}", e)
        return
      }

      pendingBacKey = bacKey
      pendingPromise = promise
    }
  }

  private fun handleTagDiscovered(tag: Tag) {
    val promise: Promise
    val bacKey: BACKey
    synchronized(lock) {
      val p = pendingPromise ?: return
      val k = pendingBacKey ?: return
      promise = p
      bacKey = k
      pendingPromise = null
      pendingBacKey = null
    }

    // BAC + APDU exchange is blocking I/O — must not run on the calling
    // thread, which for a tag delivered via onNewIntent is the main/UI thread.
    Thread {
      var cardService: IsoDepCardService? = null
      try {
        val isoDep = IsoDep.get(tag)
          ?: throw IOException("Tag does not support ISO-DEP — not an ICAO-compliant passport chip")
        isoDep.timeout = TAG_TIMEOUT_MS

        cardService = IsoDepCardService(isoDep)

        // PassportService(CardService, maxTranceiveLengthForSecureMessaging,
        // maxBlockSize, isSFIEnabled, shouldCheckMAC) — confirmed real 5-arg
        // constructor from PassportService.java source; it delegates to the
        // 6-arg overload using NORMAL_MAX_TRANCEIVE_LENGTH as the PACE
        // tranceive length, which is irrelevant here since PACE isn't used.
        val passportService = PassportService(
          cardService,
          PassportService.NORMAL_MAX_TRANCEIVE_LENGTH,
          PassportService.DEFAULT_MAX_BLOCKSIZE,
          /* isSFIEnabled = */ false,
          /* shouldCheckMAC = */ false,
        )
        passportService.open()
        // As of JMRTD 0.4.10+, open() no longer auto-selects the passport
        // applet (confirmed from PassportService.open()'s doc comment) —
        // sendSelectApplet must be called explicitly. hasPACESucceeded=false
        // since this module only performs BAC.
        passportService.sendSelectApplet(false)

        passportService.doBAC(bacKey)

        // getInputStream(fid, maxBlockSize) — the non-deprecated overload
        // (the 1-arg version is `@Deprecated` in the real source, delegating
        // to this one with the constructor's maxBlockSize).
        val dg1 = drain(passportService.getInputStream(PassportService.EF_DG1, PassportService.DEFAULT_MAX_BLOCKSIZE))
        val dg15 = drain(passportService.getInputStream(PassportService.EF_DG15, PassportService.DEFAULT_MAX_BLOCKSIZE))
        // EF_SOD and EF_CARD_SECURITY share the raw FID 0x011D in JMRTD's
        // constants (confirmed from source) — not a bug, they live in
        // different file-system contexts (applet DF vs root MF) per ICAO
        // 9303 and are never ambiguous within a single selected context.
        // sendSelectApplet above means we're reading EF.SOD here.
        val sod = drain(passportService.getInputStream(PassportService.EF_SOD, PassportService.DEFAULT_MAX_BLOCKSIZE))

        val result = Arguments.createMap().apply {
          putString("dg1", Base64.encodeToString(dg1, Base64.NO_WRAP))
          putString("dg15", Base64.encodeToString(dg15, Base64.NO_WRAP))
          putString("sod", Base64.encodeToString(sod, Base64.NO_WRAP))
        }
        promise.resolve(result)
      } catch (e: CardServiceProtocolException) {
        // The real exception BACProtocol.doBAC throws on failure (confirmed
        // from protocol/BACProtocol.java source: "BAC failed in GET
        // CHALLENGE" / "BAC failed in MUTUAL AUTH") — NOT the older
        // `BACDeniedException`, which is `@Deprecated` in current JMRTD.
        // Must be caught before the broader CardServiceException below
        // since it's a subclass.
        promise.reject(
          "NFC_BAC_FAILED",
          "BAC failed — check that the document number, date of birth, and date of expiry exactly match the passport's MRZ: ${e.message}",
          e,
        )
      } catch (e: CardServiceException) {
        promise.reject("NFC_CARD_ERROR", "Passport chip communication error: ${e.message}", e)
      } catch (e: IOException) {
        promise.reject("NFC_IO_ERROR", "Lost contact with the passport chip — hold the phone steady against the passport cover: ${e.message}", e)
      } catch (e: Exception) {
        promise.reject("NFC_READ_ERROR", e.message ?: "Unknown error reading passport", e)
      } finally {
        cardService?.close()
      }
    }.start()
  }

  /**
   * Drains a `CardFileInputStream` (a plain `InputStream` — confirmed from
   * `net.sf.scuba.smartcards.CardFileInputStream` source: it overrides only
   * `read()`, so the standard `InputStream` buffered-read loop that Kotlin's
   * `readBytes()` extension performs is the correct, general way to drain it
   * to EOF) to a `ByteArray`, closing it afterward.
   */
  private fun drain(stream: CardFileInputStream): ByteArray = stream.use { it.readBytes() }
}
