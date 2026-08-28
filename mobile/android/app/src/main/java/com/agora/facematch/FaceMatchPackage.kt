package com.agora.facematch

import com.facebook.react.ReactPackage
import com.facebook.react.bridge.NativeModule
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.uimanager.ViewManager

/**
 * Classic (pre-Codegen) `ReactPackage` registering the face-match/liveness
 * native modules + view manager. This app has `newArchEnabled=false`
 * (mobile/android/gradle.properties) — see `NfcPassportModule.kt`'s module
 * doc comment for why that matters here.
 *
 * `QrChallengeModule` lives here too, alongside `FaceCaptureModule`/
 * `FaceMatchModule`, even though it's a distinct feature (the QR-code
 * alternate liveness challenge, see that module's doc comment) — it shares
 * this package's live `<FaceCameraView>` camera preview
 * (`FaceCaptureModule.currentImageCapture()`), so it belongs alongside the
 * modules that actually own that preview rather than in a new package.
 *
 * Registered manually in `MainApplication.kt` (autolinking doesn't apply to
 * modules that live inside this app's own source tree), mirroring
 * `NfcPassportPackage.kt`.
 */
class FaceMatchPackage : ReactPackage {
  override fun createNativeModules(reactContext: ReactApplicationContext): List<NativeModule> =
    listOf(FaceCaptureModule(reactContext), FaceMatchModule(reactContext), QrChallengeModule(reactContext))

  override fun createViewManagers(reactContext: ReactApplicationContext): List<ViewManager<*, *>> =
    listOf(FaceCameraViewManager())
}
