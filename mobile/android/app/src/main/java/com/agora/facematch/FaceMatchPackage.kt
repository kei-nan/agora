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
 * Registered manually in `MainApplication.kt` (autolinking doesn't apply to
 * modules that live inside this app's own source tree), mirroring
 * `NfcPassportPackage.kt`.
 */
class FaceMatchPackage : ReactPackage {
  override fun createNativeModules(reactContext: ReactApplicationContext): List<NativeModule> =
    listOf(FaceCaptureModule(reactContext), FaceMatchModule(reactContext))

  override fun createViewManagers(reactContext: ReactApplicationContext): List<ViewManager<*, *>> =
    listOf(FaceCameraViewManager())
}
