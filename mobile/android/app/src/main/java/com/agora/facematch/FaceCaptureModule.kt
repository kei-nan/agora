package com.agora.facematch

import android.content.pm.PackageManager
import android.net.Uri
import androidx.camera.core.ImageCapture
import androidx.camera.core.ImageCaptureException
import androidx.core.content.ContextCompat
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.facebook.react.modules.core.PermissionAwareActivity
import com.facebook.react.modules.core.PermissionListener
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.face.FaceDetection
import com.google.mlkit.vision.face.FaceDetectorOptions
import android.graphics.BitmapFactory
import java.io.File
import java.util.concurrent.Executor
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * Captures a single still frame from the live front-camera preview
 * (`FaceCameraViewManager.kt` owns the actual CameraX `Preview`/`ImageCapture`
 * binding — this module only reads the `ImageCapture` use case it publishes
 * via `bindImageCapture`, the same "companion object holds shared live
 * state" pattern `NfcPassportModule.kt` uses for `activeInstance`) and runs
 * Google ML Kit Face Detection (`com.google.mlkit:face-detection`, the
 * *bundled* model variant — no Play Services download, no network call, see
 * `docs/project/changelog/087.md` for why that variant was chosen) against
 * it, returning the per-shot liveness signals `RegisterScreen.tsx`'s
 * challenge-response flow needs (`leftEyeOpenProbability`/
 * `rightEyeOpenProbability`/`headEulerAngleY`). Face-*match* against the
 * passport's DG2 photo is a separate call (`FaceMatchModule.matchAgainstPassport`)
 * — this module only ever sees the live captured frame.
 *
 * Written against CameraX's and ML Kit's documented public APIs. RN's
 * permission plumbing (`PermissionAwareActivity`/`PermissionListener`) *was*
 * cross-checked against `react-android-0.74.0`'s real source this session —
 * `ReactActivity implements PermissionAwareActivity` and already forwards
 * `onRequestPermissionsResult` to its delegate, so no `MainActivity.kt`
 * change is needed for `requestCameraPermission` below to work. CameraX/ML
 * Kit themselves were not source-verified this session — mainstream, stable
 * APIs, but treat this file as unverified until it's actually compiled (no
 * Android SDK in this environment — see CLAUDE.md's Current State).
 *
 * `newArchEnabled=false` applies here too — classic `ReactContextBaseJavaModule`,
 * same reasoning as `NfcPassportModule.kt`.
 */
class FaceCaptureModule(private val reactContext: ReactApplicationContext) :
  ReactContextBaseJavaModule(reactContext), PermissionListener {

  companion object {
    private const val CAMERA_PERMISSION_REQUEST_CODE = 5824

    /** Filename pattern written by `capturePhoto` below — shared with `sweepCaptureFiles`. */
    private const val CAPTURE_FILE_PREFIX = "facematch-"
    private const val CAPTURE_FILE_SUFFIX = ".jpg"

    private val faceDetectorOptions = FaceDetectorOptions.Builder()
      .setPerformanceMode(FaceDetectorOptions.PERFORMANCE_MODE_ACCURATE)
      .setClassificationMode(FaceDetectorOptions.CLASSIFICATION_MODE_ALL)
      .build()

    /** QR-decode half of [captureFaceAndQr] below — same options [QrChallengeModule] uses for its own standalone decode. */
    private val barcodeScannerOptions = BarcodeScannerOptions.Builder()
      .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
      .build()

    /**
     * Shared background executor for every CameraX `takePicture` callback in
     * this package (this module's own [capturePhoto]/[captureFaceAndQr], and
     * [QrChallengeModule]'s `captureAndDecodeQrCode` via [captureExecutor]).
     *
     * **Fixes a real bug, not a hardening measure**: `takePicture`'s second
     * argument is the `Executor` its `OnImageSavedCallback` runs on. This
     * code previously passed `ContextCompat.getMainExecutor(reactContext)`,
     * so `onImageSaved` ran on the main/UI thread — and then called the
     * blocking `Tasks.await(...)` from inside it. Play Services' `Tasks.await`
     * explicitly forbids being called from the main thread and throws
     * unconditionally when it is (this is documented and enforced, not just
     * discouraged) — so every one of these captures was guaranteed to reject
     * before this fix, regardless of what was actually in frame. (The
     * `capturePhoto` doc comment used to justify the `Tasks.await` call by
     * saying "this native-module method already runs off the JS thread" —
     * true of the `@ReactMethod` entry point itself, but irrelevant: the
     * `Tasks.await` call happens later, inside the `takePicture` callback,
     * on whatever thread *that* executor provides, which was main.) A
     * single-thread executor is enough — captures are inherently sequential
     * (one `<FaceCameraView>` preview, one capture in flight at a time from
     * the JS side) — this just needs to not be the main thread.
     */
    private val backgroundExecutor: ExecutorService = Executors.newSingleThreadExecutor()

    /** Exposes [backgroundExecutor] to [QrChallengeModule] so its own `takePicture` callback runs off the main thread too — see that property's doc comment. */
    @JvmStatic
    fun captureExecutor(): Executor = backgroundExecutor

    /** The live `ImageCapture` use case `FaceCameraViewManager` currently has bound, if any. */
    @Volatile
    private var activeImageCapture: ImageCapture? = null

    /** Called by `FaceCameraViewManager` once CameraX has bound (or torn down) a live `ImageCapture` use case. */
    @JvmStatic
    fun bindImageCapture(imageCapture: ImageCapture?) {
      activeImageCapture = imageCapture
    }

    /**
     * Read-only access to the currently bound `ImageCapture` use case, for
     * [QrChallengeModule] (same package) to capture a frame from the same
     * live preview this module drives, without duplicating the CameraX
     * binding logic `FaceCameraViewManager` already owns.
     */
    @JvmStatic
    fun currentImageCapture(): ImageCapture? = activeImageCapture

    /**
     * Deletes every registration-selfie JPEG this module has ever written into
     * [cacheDir] (the `facematch-<challenge>-<timestamp>.jpg` pattern `capturePhoto`
     * writes below) — these are biometric capture photos, not data that should ever
     * accumulate on disk. Called from two places: `FaceMatchModule` right after it
     * finishes comparing a captured selfie against the passport's DG2 photo (success
     * *or* failure — see that module), and this module's own `init` block below as a
     * startup sweep for anything orphaned by a previous crashed/interrupted session
     * (e.g. the app was killed mid-registration, after a shot was captured but before
     * the match call that would otherwise trigger cleanup). Best-effort: an individual
     * file failing to delete doesn't stop the sweep or throw.
     */
    @JvmStatic
    fun sweepCaptureFiles(cacheDir: File) {
      cacheDir.listFiles { file ->
        file.isFile && file.name.startsWith(CAPTURE_FILE_PREFIX) && file.name.endsWith(CAPTURE_FILE_SUFFIX)
      }?.forEach { file ->
        runCatching { file.delete() }
      }
    }

    /**
     * `true` iff [file] is one of this module's own `facematch-*.jpg` cache
     * captures — the same prefix/suffix check [sweepCaptureFiles] uses, plus
     * a parent-directory check, so [deleteCaptureFile] below can never be
     * used (even via a malformed/foreign `uri` argument from the JS side) to
     * delete a file outside this module's own capture directory.
     */
    private fun isOwnCaptureFile(file: File, cacheDir: File): Boolean {
      return file.parentFile == cacheDir &&
        file.name.startsWith(CAPTURE_FILE_PREFIX) &&
        file.name.endsWith(CAPTURE_FILE_SUFFIX)
    }
  }

  /** Set while a `requestCameraPermission` call is waiting on the user's response; see `onRequestPermissionsResult`. */
  private var pendingPermissionPromise: Promise? = null

  init {
    // Startup cleanup sweep — see `sweepCaptureFiles` doc comment. Runs once per
    // process, when RN's module registry instantiates this module (FaceMatchPackage
    // constructs it eagerly alongside FaceMatchModule).
    sweepCaptureFiles(reactContext.cacheDir)
  }

  override fun getName(): String = "FaceCaptureModule"

  @ReactMethod
  fun hasCameraPermission(promise: Promise) {
    promise.resolve(
      ContextCompat.checkSelfPermission(reactContext, android.Manifest.permission.CAMERA) ==
        PackageManager.PERMISSION_GRANTED,
    )
  }

  @ReactMethod
  fun requestCameraPermission(promise: Promise) {
    val activity = currentActivity
    if (activity !is PermissionAwareActivity) {
      promise.reject("CAMERA_NO_ACTIVITY", "No foreground PermissionAwareActivity available to request CAMERA permission")
      return
    }
    pendingPermissionPromise = promise
    activity.requestPermissions(arrayOf(android.Manifest.permission.CAMERA), CAMERA_PERMISSION_REQUEST_CODE, this)
  }

  override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray): Boolean {
    if (requestCode != CAMERA_PERMISSION_REQUEST_CODE) return false
    val granted = grantResults.isNotEmpty() && grantResults[0] == PackageManager.PERMISSION_GRANTED
    pendingPermissionPromise?.resolve(granted)
    pendingPermissionPromise = null
    return true
  }

  /**
   * Deletes one specific still-outstanding capture file by its `file://`
   * URI — the pre-compare-step counterpart to `FaceMatchModule`'s
   * `matchAgainstPassport` finally-block sweep (see `sweepCaptureFiles`'s
   * doc comment). `RegisterScreen.tsx` calls this right after a baseline or
   * challenge shot fails its liveness check (eyes-closed/angle threshold)
   * and is about to be retried — that shot will never reach
   * `matchAgainstPassport`, so nothing else will ever clean it up — and
   * again from a screen-unmount cleanup for a passed-baseline shot left
   * over if the user abandons the flow before finishing the challenge
   * substep. A full [sweepCaptureFiles] sweep isn't right for either call
   * site: at both points there can be a *different* still-needed capture
   * file on disk (e.g. a just-passed baseline while the challenge substep
   * is retried) that a blanket sweep would wrongly delete too. Restricted
   * to this module's own capture files ([isOwnCaptureFile]) so a bad/foreign
   * `uri` can't be used to delete something else. Best-effort and always
   * resolves — a cleanup helper must never surface an error that blocks
   * the registration flow.
   */
  @ReactMethod
  fun deleteCaptureFile(uri: String, promise: Promise) {
    runCatching {
      val path = Uri.parse(uri).path ?: return@runCatching
      val file = File(path)
      if (isOwnCaptureFile(file, reactContext.cacheDir)) {
        file.delete()
      }
    }
    promise.resolve(null)
  }

  /**
   * Takes one still frame from the already-bound live preview and runs ML
   * Kit face detection against it. Runs on [backgroundExecutor], not the
   * main thread — see that property's doc comment for why the blocking
   * `Tasks.await` call below requires this (this method used to pass
   * `ContextCompat.getMainExecutor(reactContext)` here instead, which made
   * every capture reject unconditionally).
   * `challenge` is opaque to this method — purely an identifier folded into
   * the saved filename so a caller juggling multiple shots (a frontal
   * baseline plus a randomized liveness challenge, see `RegisterScreen.tsx`)
   * can tell them apart on disk; no on-device challenge verification happens
   * here — the JS side compares the returned probabilities/angle against
   * whichever challenge it asked for.
   */
  @ReactMethod
  fun capturePhoto(challenge: String, promise: Promise) {
    val imageCapture = activeImageCapture
    if (imageCapture == null) {
      promise.reject("CAMERA_NOT_READY", "No live camera preview is bound yet — is <FaceCameraView> mounted?")
      return
    }
    val outputFile = File(reactContext.cacheDir, "$CAPTURE_FILE_PREFIX$challenge-${System.currentTimeMillis()}$CAPTURE_FILE_SUFFIX")
    val outputOptions = ImageCapture.OutputFileOptions.Builder(outputFile).build()
    imageCapture.takePicture(
      outputOptions,
      backgroundExecutor,
      object : ImageCapture.OnImageSavedCallback {
        override fun onImageSaved(output: ImageCapture.OutputFileResults) {
          try {
            val bitmap = BitmapFactory.decodeFile(outputFile.absolutePath)
              ?: throw IllegalStateException("Captured frame could not be decoded")
            val faces = Tasks.await(
              FaceDetection.getClient(faceDetectorOptions).process(InputImage.fromBitmap(bitmap, 0)),
              10, TimeUnit.SECONDS,
            )
            val face = faces.maxByOrNull { it.boundingBox.width() * it.boundingBox.height() }
            if (face == null) {
              // The frame was still written to outputFile even though it has no usable
              // face in it — this rejection is a dead end for the caller (RegisterScreen
              // retries with a brand-new capturePhoto call), so nothing else will ever
              // clean this file up. Delete it now rather than leaving it for the next
              // sweep (matchAgainstPassport won't run, and the app may not restart for
              // a long time). Best-effort: a delete failure must not mask the real error.
              runCatching { outputFile.delete() }
              promise.reject("NO_FACE_DETECTED", "No face was detected in the captured frame")
              return
            }
            val result = Arguments.createMap().apply {
              putString("uri", Uri.fromFile(outputFile).toString())
              putDouble("leftEyeOpenProbability", (face.leftEyeOpenProbability ?: -1f).toDouble())
              putDouble("rightEyeOpenProbability", (face.rightEyeOpenProbability ?: -1f).toDouble())
              putDouble("headEulerAngleY", face.headEulerAngleY.toDouble())
            }
            promise.resolve(result)
          } catch (e: Exception) {
            // Same reasoning as the NO_FACE_DETECTED branch above — outputFile was
            // written before decode/detection failed, and this rejection is likewise
            // a dead end for the caller.
            runCatching { outputFile.delete() }
            promise.reject("FACE_DETECT_ERROR", e.message ?: "Face detection failed", e)
          }
        }

        override fun onError(exception: ImageCaptureException) {
          // CameraX may or may not have written a partial file before failing —
          // best-effort delete either way, same reasoning as above.
          runCatching { outputFile.delete() }
          promise.reject("CAMERA_CAPTURE_ERROR", exception.message ?: "Photo capture failed", exception)
        }
      },
    )
  }

  /**
   * Combined capture backing the QR-liveness-challenge's two-shot redesign
   * (see `mobile/src/screens/qrLivenessChallenge.ts` and `RegisterScreen.tsx`'s
   * `qrCapture1`/`qrCapture2` substeps): takes one still frame and runs BOTH
   * ML Kit face detection and ML Kit barcode scanning against the very same
   * [InputImage], so a single captured frame is the only thing that can ever
   * satisfy both checks together — not two independently-satisfiable steps.
   *
   * This replaces the old flaw directly: the QR challenge substep used to
   * call [QrChallengeModule.captureAndDecodeQrCode], which decodes a barcode
   * and does *no* face check at all, while the *separate*, earlier baseline
   * shot (a plain [capturePhoto] call, satisfiable by a static photo held up
   * once) was what got reused for the eventual passport face-match. Nothing
   * ever required a face and a fresh QR nonce to be present in front of the
   * camera at the same moment. Calling this method twice, with a freshly
   * regenerated QR session between calls (`RegisterScreen.tsx`'s
   * `toggleLivenessMethod`/retry logic), is what the JS side now does
   * instead — see that file for the residual-risk note on what this still
   * doesn't defend against (an attacker who can hold up the same static
   * photo *and* a validly-refreshed QR code twice in a row).
   *
   * Deliberately never rejects for "no face"/"no code" the way
   * [capturePhoto]/[QrChallengeModule.captureAndDecodeQrCode] do
   * individually — absence comes back as `-1` eye-open probabilities
   * (mirroring [capturePhoto]'s existing "-1 = uncomputed" convention) and
   * `qrText: null`. The caller evaluates both signals itself (the same
   * thresholds `RegisterScreen.tsx` already applies to [capturePhoto]'s
   * output, plus `isQrChallengeValid` from `qrLivenessChallenge.ts`) and
   * decides whether to advance, retry, or clean up the file — this method
   * only reports what it saw. Runs on [backgroundExecutor] for the same
   * `Tasks.await`-off-the-main-thread reason as [capturePhoto].
   */
  @ReactMethod
  fun captureFaceAndQr(promise: Promise) {
    val imageCapture = activeImageCapture
    if (imageCapture == null) {
      promise.reject("CAMERA_NOT_READY", "No live camera preview is bound yet — is <FaceCameraView> mounted?")
      return
    }
    val outputFile = File(reactContext.cacheDir, "${CAPTURE_FILE_PREFIX}qrcombined-${System.currentTimeMillis()}$CAPTURE_FILE_SUFFIX")
    val outputOptions = ImageCapture.OutputFileOptions.Builder(outputFile).build()
    imageCapture.takePicture(
      outputOptions,
      backgroundExecutor,
      object : ImageCapture.OnImageSavedCallback {
        override fun onImageSaved(output: ImageCapture.OutputFileResults) {
          try {
            val bitmap = BitmapFactory.decodeFile(outputFile.absolutePath)
              ?: throw IllegalStateException("Captured frame could not be decoded")
            val inputImage = InputImage.fromBitmap(bitmap, 0)
            val faces = Tasks.await(
              FaceDetection.getClient(faceDetectorOptions).process(inputImage),
              10, TimeUnit.SECONDS,
            )
            val barcodes = Tasks.await(
              BarcodeScanning.getClient(barcodeScannerOptions).process(inputImage),
              10, TimeUnit.SECONDS,
            )
            val face = faces.maxByOrNull { it.boundingBox.width() * it.boundingBox.height() }
            val qrText = barcodes.firstNotNullOfOrNull { it.rawValue }
            val result = Arguments.createMap().apply {
              putString("uri", Uri.fromFile(outputFile).toString())
              putDouble("leftEyeOpenProbability", (face?.leftEyeOpenProbability ?: -1f).toDouble())
              putDouble("rightEyeOpenProbability", (face?.rightEyeOpenProbability ?: -1f).toDouble())
              putDouble("headEulerAngleY", (face?.headEulerAngleY ?: 0f).toDouble())
              putString("qrText", qrText)
            }
            // Deliberately not deleted here even when face/qrText came back empty — unlike
            // capturePhoto's NO_FACE_DETECTED path, this isn't a dead end the caller can't
            // act on: the JS side (RegisterScreen.tsx) decides pass/fail from the returned
            // signals and, on failure, calls deleteCaptureFile itself (same pattern already
            // used for a failed baseline/challenge shot). On success this file is passed to
            // matchAgainstPassport, whose own finally-block sweep cleans it up either way.
            promise.resolve(result)
          } catch (e: Exception) {
            runCatching { outputFile.delete() }
            promise.reject("FACE_QR_CAPTURE_ERROR", e.message ?: "Combined face/QR capture failed", e)
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
