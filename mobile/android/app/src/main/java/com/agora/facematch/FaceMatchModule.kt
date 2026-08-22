package com.agora.facematch

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.util.Base64
import com.facebook.react.bridge.Arguments
import com.facebook.react.bridge.Promise
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.bridge.ReactContextBaseJavaModule
import com.facebook.react.bridge.ReactMethod
import com.google.android.gms.tasks.Tasks
import com.google.mlkit.vision.common.InputImage
import com.google.mlkit.vision.face.Face
import com.google.mlkit.vision.face.FaceDetection
import com.google.mlkit.vision.face.FaceDetectorOptions
import org.tensorflow.lite.Interpreter
import java.io.FileInputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.channels.FileChannel
import java.util.concurrent.TimeUnit
import kotlin.math.sqrt

/**
 * Compares a live-captured selfie against a passport's EF.DG2 photo using a
 * MobileFaceNet embedding model, entirely on-device — nothing here leaves
 * the phone, same as the ZK proving pipeline this feeds into.
 *
 * ## Model provenance — read before touching the model asset or `MATCH_THRESHOLD`
 * `mobile/android/app/src/main/assets/mobilefacenet.tflite` was fetched from
 * `MCarlomagno/FaceRecognitionAuth` on GitHub — **BSD-3-Clause**, confirmed
 * via `gh api repos/MCarlomagno/FaceRecognitionAuth` this session — blob sha
 * `057b98506a7fe9fb8ac8e64464bcacd6439084eb`, 5,233,552 bytes, source
 * `https://raw.githubusercontent.com/MCarlomagno/FaceRecognitionAuth/master/assets/mobilefacenet.tflite`.
 * That model traces back to `sirius-ai/MobileFaceNet_TF`, typically trained
 * on MS1M/MS-Celeb-1M-derived face data — a dataset Microsoft took down in
 * 2019 over consent concerns. No face this codebase ever processes is from
 * that dataset (a citizen's registration photo/selfie never leaves their
 * device, let alone becomes training data), but the *model's own*
 * training-data lineage carries that history — worth knowing before this
 * asset ships to real citizens of a government identity system. See
 * `docs/project/changelog/087.md` for the full decision record.
 *
 * Preprocessing (112×112 input, `(pixel - 127.5) / 128.0` normalization,
 * 192-float embedding output) follows `MobileFaceNet_TF`'s own documented
 * convention for this model family.
 *
 * `MATCH_THRESHOLD` is an **unvalidated placeholder** (0.5 cosine
 * similarity, a commonly-cited figure for this model family) — this
 * codebase has no real passport-photo/selfie corpus to calibrate a
 * false-accept/false-reject tradeoff against. Same honesty standard as the
 * OPRF committee's own "SLA window is an unmeasured placeholder" note
 * (`docs/project/next-steps.md` log #82).
 *
 * Written against TensorFlow Lite's and ML Kit's documented public APIs;
 * not source-verified or compiled this session (no Android SDK in this
 * environment — see CLAUDE.md's Current State).
 */
class FaceMatchModule(private val reactContext: ReactApplicationContext) :
  ReactContextBaseJavaModule(reactContext) {

  companion object {
    private const val MODEL_ASSET = "mobilefacenet.tflite"
    private const val EMBEDDING_SIZE = 192
    private const val INPUT_SIZE = 112

    /** Unvalidated — see module doc comment. */
    private const val MATCH_THRESHOLD = 0.5

    private val faceDetectorOptions = FaceDetectorOptions.Builder()
      .setPerformanceMode(FaceDetectorOptions.PERFORMANCE_MODE_ACCURATE)
      .build()
  }

  override fun getName(): String = "FaceMatchModule"

  private val interpreter: Interpreter by lazy {
    val assetFd = reactContext.assets.openFd(MODEL_ASSET)
    val modelBuffer = FileInputStream(assetFd.fileDescriptor).channel.map(
      FileChannel.MapMode.READ_ONLY,
      assetFd.startOffset,
      assetFd.declaredLength,
    )
    Interpreter(modelBuffer)
  }

  /**
   * Compares the passport's DG2 photo against one already-captured selfie
   * (the frontal baseline shot from `FaceCaptureModule.capturePhoto` — the
   * liveness-challenge shot's own signals are read directly off that call's
   * result and never routed through here).
   *
   * `dg2MimeType` not `"image/jpeg"` (i.e. JPEG2000 or unrecognized)
   * resolves `{matched: false, skipped: true, reason: ...}` rather than
   * rejecting — `BitmapFactory` cannot decode JPEG2000, a real, documented
   * gap (`docs/project/changelog/087.md`), not a bug to hide behind a thrown
   * error. `RegisterScreen.tsx` still gates registration on the liveness
   * signals already captured even when this resolves `skipped: true`.
   *
   * This is also where the registration selfie(s) `FaceCaptureModule.capturePhoto`
   * wrote to the app cache directory get cleaned up (`FaceCaptureModule.sweepCaptureFiles`
   * in the outer `finally`, covering every exit path below — the unsupported-format skip,
   * a thrown error, and a normal match result alike) — see that function's doc comment
   * for why a directory sweep rather than deleting just `capturedUri`.
   */
  @ReactMethod
  fun matchAgainstPassport(dg2Base64: String, dg2MimeType: String, capturedUri: String, promise: Promise) {
    try {
      if (dg2MimeType != "image/jpeg") {
        val result = Arguments.createMap().apply {
          putBoolean("matched", false)
          putBoolean("skipped", true)
          putString("reason", "unsupported DG2 image format: $dg2MimeType")
        }
        promise.resolve(result)
        return
      }
      var dg2Bitmap: Bitmap? = null
      var capturedBitmap: Bitmap? = null
      var dg2Crop: Bitmap? = null
      var capturedCrop: Bitmap? = null
      try {
        val dg2Bytes = Base64.decode(dg2Base64, Base64.DEFAULT)
        dg2Bitmap = BitmapFactory.decodeByteArray(dg2Bytes, 0, dg2Bytes.size)
          ?: throw IllegalStateException("Could not decode DG2 photo")
        val capturedPath = Uri.parse(capturedUri).path
          ?: throw IllegalStateException("Invalid captured photo URI: $capturedUri")
        capturedBitmap = BitmapFactory.decodeFile(capturedPath)
          ?: throw IllegalStateException("Could not decode captured photo")

        val dg2Face = detectLargestFace(dg2Bitmap)
        val capturedFace = detectLargestFace(capturedBitmap)
        if (dg2Face == null || capturedFace == null) {
          promise.reject("NO_FACE_DETECTED", "Could not detect a face in the passport photo or the captured selfie")
          return
        }

        dg2Crop = cropFace(dg2Bitmap, dg2Face)
        capturedCrop = cropFace(capturedBitmap, capturedFace)
        val dg2Embedding = embed(dg2Crop)
        val capturedEmbedding = embed(capturedCrop)
        val score = cosineSimilarity(dg2Embedding, capturedEmbedding)

        val result = Arguments.createMap().apply {
          putBoolean("matched", score >= MATCH_THRESHOLD)
          putBoolean("skipped", false)
          putDouble("score", score.toDouble())
        }
        promise.resolve(result)
      } catch (e: Exception) {
        promise.reject("FACE_MATCH_ERROR", e.message ?: "Face match failed", e)
      } finally {
        // Best-effort — recycling an already-recycled bitmap (e.g. if cropFace/
        // createScaledBitmap returned the same object it was given) is a documented no-op.
        dg2Bitmap?.recycle()
        capturedBitmap?.recycle()
        dg2Crop?.recycle()
        capturedCrop?.recycle()
      }
    } finally {
      FaceCaptureModule.sweepCaptureFiles(reactContext.cacheDir)
    }
  }

  private fun detectLargestFace(bitmap: Bitmap): Face? {
    val faces = Tasks.await(
      FaceDetection.getClient(faceDetectorOptions).process(InputImage.fromBitmap(bitmap, 0)),
      10, TimeUnit.SECONDS,
    )
    return faces.maxByOrNull { it.boundingBox.width() * it.boundingBox.height() }
  }

  private fun cropFace(bitmap: Bitmap, face: Face): Bitmap {
    val box = face.boundingBox
    val left = box.left.coerceIn(0, bitmap.width - 1)
    val top = box.top.coerceIn(0, bitmap.height - 1)
    val width = box.width().coerceAtMost(bitmap.width - left)
    val height = box.height().coerceAtMost(bitmap.height - top)
    val cropped = Bitmap.createBitmap(bitmap, left, top, width, height)
    val scaled = Bitmap.createScaledBitmap(cropped, INPUT_SIZE, INPUT_SIZE, true)
    // createBitmap/createScaledBitmap can each return the same object they were given
    // (no-op crop/scale) rather than a new one — only recycle the intermediate crop if
    // it's genuinely a distinct bitmap from what we're returning, and never the caller's
    // own `bitmap` (matchAgainstPassport still owns and recycles that one itself).
    if (cropped !== scaled && cropped !== bitmap) {
      cropped.recycle()
    }
    return scaled
  }

  /** `(pixel - 127.5) / 128.0` per MobileFaceNet_TF's own preprocessing convention — see module doc comment. */
  private fun embed(face: Bitmap): FloatArray {
    val input = ByteBuffer.allocateDirect(4 * INPUT_SIZE * INPUT_SIZE * 3).apply { order(ByteOrder.nativeOrder()) }
    for (y in 0 until INPUT_SIZE) {
      for (x in 0 until INPUT_SIZE) {
        val pixel = face.getPixel(x, y)
        input.putFloat((((pixel shr 16) and 0xFF) - 127.5f) / 128.0f)
        input.putFloat((((pixel shr 8) and 0xFF) - 127.5f) / 128.0f)
        input.putFloat(((pixel and 0xFF) - 127.5f) / 128.0f)
      }
    }
    input.rewind()
    val output = Array(1) { FloatArray(EMBEDDING_SIZE) }
    interpreter.run(input, output)
    return output[0]
  }

  private fun cosineSimilarity(a: FloatArray, b: FloatArray): Float {
    var dot = 0f
    var normA = 0f
    var normB = 0f
    for (i in a.indices) {
      dot += a[i] * b[i]
      normA += a[i] * a[i]
      normB += b[i] * b[i]
    }
    return dot / (sqrt(normA) * sqrt(normB))
  }
}
