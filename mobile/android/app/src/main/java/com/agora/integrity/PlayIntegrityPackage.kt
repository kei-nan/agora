package com.agora.integrity

import com.facebook.react.ReactPackage
import com.facebook.react.bridge.NativeModule
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.uimanager.ViewManager

/**
 * Classic (pre-Codegen) `ReactPackage` registering `PlayIntegrityModule` —
 * see that file's doc comment. Its own package, separate from
 * `com.agora.facematch`, since device-integrity attestation is an
 * independent concern from face-match/liveness with no shared state (unlike
 * `QrChallengeModule`, which shares `FaceMatchPackage`'s camera preview).
 *
 * Registered manually in `MainApplication.kt` (autolinking doesn't apply to
 * modules that live inside this app's own source tree), mirroring
 * `FaceMatchPackage.kt`/`NfcPassportPackage.kt`.
 */
class PlayIntegrityPackage : ReactPackage {
  override fun createNativeModules(reactContext: ReactApplicationContext): List<NativeModule> =
    listOf(PlayIntegrityModule(reactContext))

  override fun createViewManagers(reactContext: ReactApplicationContext): List<ViewManager<*, *>> = emptyList()
}
