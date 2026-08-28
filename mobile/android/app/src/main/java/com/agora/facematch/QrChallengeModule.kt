package com.agora.facematch

import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.common.InputImage
import android.graphics.BitmapFactory
import java.io.File
import java.util.concurrent.TimeUnit

/**
 * The decode half of the QR-code alternate liveness challenge (see
 * `mobile/src/screens/qrLivenessChallenge.ts` for the session/nonce design,
 * `RegisterScreen.tsx` for where this is offered as an accessible
 * alternative to the default blink/turn challenge that `FaceCaptureModule`
 * in this same package drives).
 *
 * Captures one still frame from the same shared `<FaceCameraView>` preview
 * `FaceCaptureModule` already uses ([FaceCaptureModule.currentImageCapture],
 * bound by `FaceCameraViewManager` — already mounted for the liveness step
 * regardless of which challenge the user picks) and decodes any QR code in
 * it via ML Kit's *Barcode Scanning* API — a different ML Kit module from
 * `com.google.mlkit:face-detection` (already a dependency, used for the
 * blink/turn signals in `FaceCaptureModule`), but the same bundled/on-device
 * model family: no Play Services download, no network call, consistent with
 * "nothing leaves your phone." Checked first per this app's existing
 * precedent of reusing what's already there — ML Kit ships barcode scanning
 * as a natural sibling dependency (`com.google.mlkit:barcode-scanning`) —
 * before reaching for a third-party RN camera/barcode library. See
 * `android/app/build.gradle`'s dependency comment for the version.
 *
 * Deliberately does NOT reuse [FaceCaptureModule.capturePhoto]: that method
 * runs ML Kit *face* detection on the frame and rejects with
 * `NO_FACE_DETECTED` if it finds no face — exactly wrong here, since a QR
 * challenge frame shows a QR code, not the citizen's face. This module
 * captures and decodes independently instead, with no face-detection gate at
 * all, and never persists the captured frame: it's deleted immediately after
 * decoding (success or failure) since — unlike the face-match captures
 * `FaceCaptureModule` tracks — there's no reason to ever hand this one back
 * to JS or keep it around.
 *
 * **No longer used by the QR-liveness-challenge flow itself.** A code-only
 * decode with no face check at all was a real design flaw: an attacker
 * holding only a static photo of the citizen (enough to pass the separate
 * baseline face capture once) could complete the whole liveness+face-match
 * pipeline by just showing this QR code to the camera afterward, with no
 * live person required for the QR substep at all. `RegisterScreen.tsx`'s QR
 * substeps now call `FaceCaptureModule.captureFaceAndQr` instead — a single
 * capture that runs face detection *and* barcode decode against the same
 * frame, done twice with a freshly-regenerated nonce each time — see that
 * method's doc comment for the full redesign. `captureAndDecodeQrCode` below
 * is kept working (including the threading fix below) as a standalone
 * code-only decode utility and as the subject of `qrChallenge.test.ts`, but
 * nothing in the production liveness flow calls it anymore.
 *
 * Written against CameraX's and ML Kit's documented public APIs, same
 * caveats as `FaceCaptureModule.kt`'s doc comment: not source-verified this
 * session, and not compiled/run (no Android SDK in this environment).
 *
 * `newArchEnabled=false` applies here too — classic `ReactContextBaseJavaModule`.
 */
class QrChallengeModule(private val reactContext: ReactApplicationContext) :
  ReactContextBaseJavaModule(reactContext) {

  companion object {
    private const val CAPTURE_FILE_PREFIX = "qrchallenge-"
    private const val CAPTURE_FILE_SUFFIX = ".jpg"

    private val barcodeScannerOptions = BarcodeScannerOptions.Builder()
      .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
      .build()
  }

  override fun getName(): String = "QrChallengeModule"

  /**
   * Captures one frame from the live preview and resolves with the raw text
   * of the first QR code found, or `null` if none was found — never rejects
   * for "no code visible," only for a genuine capture/decode failure or the
   * camera not being bound yet (`CAMERA_NOT_READY`, mirroring
   * `FaceCaptureModule.capturePhoto`'s own error code).
   *
   * Runs on `FaceCaptureModule`'s shared [FaceCaptureModule.captureExecutor],
   * not the main thread — this method used to pass
   * `ContextCompat.getMainExecutor(reactContext)` to `takePicture` instead,
   * which put the blocking `Tasks.await` call below on the main/UI thread.
   * Play Services' `Tasks.await` throws unconditionally when called from the
   * main thread, so every call to this method rejected before this fix,
   * regardless of whether a QR code was actually in frame — see
   * `FaceCaptureModule.captureExecutor`'s doc comment for the full story.
   */
  @ReactMethod
  fun captureAndDecodeQrCode(promise: Promise) {
    val imageCapture = FaceCaptureModule.currentImageCapture()
    if (imageCapture == null) {
      promise.reject("CAMERA_NOT_READY", "No live camera preview is bound yet — is <FaceCameraView> mounted?")
      return
    }
    val outputFile = File(reactContext.cacheDir, "$CAPTURE_FILE_PREFIX${System.currentTimeMillis()}$CAPTURE_FILE_SUFFIX")
    val outputOptions = ImageCapture.OutputFileOptions.Builder(outputFile).build()
    imageCapture.takePicture(
      outputOptions,
      FaceCaptureModule.captureExecutor(),
      object : ImageCapture.OnImageSavedCallback {
        override fun onImageSaved(output: ImageCapture.OutputFileResults) {
          try {
            val bitmap = BitmapFactory.decodeFile(outputFile.absolutePath)
              ?: throw IllegalStateException("Captured frame could not be decoded")
            val barcodes = Tasks.await(
              BarcodeScanning.getClient(barcodeScannerOptions).process(InputImage.fromBitmap(bitmap, 0)),
              10, TimeUnit.SECONDS,
            )
            val text = barcodes.firstNotNullOfOrNull { it.rawValue }
            promise.resolve(text)
          } catch (e: Exception) {
            promise.reject("QR_DECODE_ERROR", e.message ?: "QR decode failed", e)
          } finally {
            // Never needs to persist — not biometric content, but cleaned up
            // immediately either way, same hygiene discipline as
            // FaceCaptureModule's own capture files.
            runCatching { outputFile.delete() }
          }
        }

        override fun onError(exception: ImageCaptureException) {
          runCatching { outputFile.delete() }
          promise.reject("CAMERA_CAPTURE_ERROR", exception.message ?: "Photo capture failed", exception)
        }
      },
    )
  }
}
