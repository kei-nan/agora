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
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.face.FaceDetection
import com.google.mlkit.vision.face.FaceDetectorOptions
import android.graphics.BitmapFactory
import java.io.File
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

    /** The live `ImageCapture` use case `FaceCameraViewManager` currently has bound, if any. */
    @Volatile
    private var activeImageCapture: ImageCapture? = null

    /** Called by `FaceCameraViewManager` once CameraX has bound (or torn down) a live `ImageCapture` use case. */
    @JvmStatic
    fun bindImageCapture(imageCapture: ImageCapture?) {
      activeImageCapture = imageCapture
    }

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
   * Takes one still frame from the already-bound live preview and runs ML
   * Kit face detection against it synchronously — this native-module method
   * already runs off the JS thread (RN dispatches `@ReactMethod` calls to
   * their own handler thread), so blocking on `Tasks.await` here is safe.
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
      ContextCompat.getMainExecutor(reactContext),
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
            promise.reject("FACE_DETECT_ERROR", e.message ?: "Face detection failed", e)
          }
        }

        override fun onError(exception: ImageCaptureException) {
          promise.reject("CAMERA_CAPTURE_ERROR", exception.message ?: "Photo capture failed", exception)
        }
      },
    )
  }
}
