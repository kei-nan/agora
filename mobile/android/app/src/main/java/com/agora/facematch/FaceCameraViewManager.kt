package com.agora.facematch

import android.util.Log
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageCapture
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import com.facebook.react.uimanager.SimpleViewManager
import com.facebook.react.uimanager.ThemedReactContext

/**
 * Live front-camera preview for the face-match/liveness capture flow
 * (`RegisterScreen.tsx`). `SimpleViewManager` predates Fabric/TurboModules,
 * so it's safe under this app's classic bridge (`newArchEnabled=false`,
 * mobile/android/gradle.properties) — see `NfcPassportModule.kt`'s doc
 * comment for why that distinction matters here; it's also the reason this
 * feature uses a custom CameraX-based module instead of a third-party RN
 * camera library (`react-native-vision-camera` v4 is JSI-oriented and v5 is
 * a New-Architecture-only Nitro rewrite — see `docs/project/changelog/087.md`
 * for the full reasoning).
 *
 * Written against CameraX's (`androidx.camera:camera-core`/`camera-lifecycle`/
 * `camera-view`) documented public API. Unlike `NfcPassportModule.kt`'s JMRTD
 * calls, this was not cross-checked against downloaded library source this
 * session — CameraX is a mainstream, stable Jetpack API, but treat this file
 * as unverified until it's actually compiled (no Android SDK in this
 * environment — see CLAUDE.md's Current State).
 *
 * Binds CameraX's `Preview` (shown here) and `ImageCapture` use cases
 * together in a single `bindToLifecycle` call, then publishes the resulting
 * `ImageCapture` instance to `FaceCaptureModule` via `bindImageCapture` — the
 * same "companion object holds shared live state" pattern `NfcPassportModule.kt`
 * uses for `activeInstance`/`onTagDiscovered` — so `FaceCaptureModule`'s
 * `capturePhoto` native-module method can drive a still capture against the
 * same session this view is previewing.
 */
class FaceCameraViewManager : SimpleViewManager<PreviewView>() {

  override fun getName(): String = "FaceCameraView"

  override fun createViewInstance(themedReactContext: ThemedReactContext): PreviewView {
    val previewView = PreviewView(themedReactContext)
    // ThemedReactContext itself is not a LifecycleOwner — the underlying
    // Activity is (MainActivity : ReactActivity : AppCompatActivity, and
    // AppCompatActivity implements LifecycleOwner via ComponentActivity;
    // confirmed from react-android's own ReactActivity.java source this
    // session), reached via getCurrentActivity().
    val lifecycleOwner = themedReactContext.currentActivity as? LifecycleOwner
    if (lifecycleOwner == null) {
      Log.e("FaceCameraView", "No LifecycleOwner activity available — camera preview cannot bind")
      return previewView
    }
    val providerFuture = ProcessCameraProvider.getInstance(themedReactContext)
    providerFuture.addListener({
      try {
        val cameraProvider = providerFuture.get()
        val preview = Preview.Builder().build().also {
          it.setSurfaceProvider(previewView.surfaceProvider)
        }
        val imageCapture = ImageCapture.Builder().build()
        cameraProvider.unbindAll()
        cameraProvider.bindToLifecycle(
          lifecycleOwner,
          CameraSelector.DEFAULT_FRONT_CAMERA,
          preview,
          imageCapture,
        )
        FaceCaptureModule.bindImageCapture(imageCapture)
      } catch (e: Exception) {
        Log.e("FaceCameraView", "Failed to bind CameraX preview/capture", e)
      }
    }, ContextCompat.getMainExecutor(themedReactContext))
    return previewView
  }

  override fun onDropViewInstance(view: PreviewView) {
    super.onDropViewInstance(view)
    FaceCaptureModule.bindImageCapture(null)
  }
}
