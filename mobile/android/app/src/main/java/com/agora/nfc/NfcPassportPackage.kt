package com.agora.nfc

import com.facebook.react.ReactPackage
import com.facebook.react.bridge.NativeModule
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.uimanager.ViewManager

/**
 * Classic (pre-Codegen) `ReactPackage` registering `NfcPassportModule`. This
 * app has `newArchEnabled=false` (mobile/android/gradle.properties) and no
 * TurboModule spec exists for this module — see `NfcPassportModule.kt`'s
 * module doc comment for why that matters here.
 *
 * Registered manually in `MainApplication.kt` (autolinking doesn't apply to
 * modules that live inside this app's own source tree).
 */
class NfcPassportPackage : ReactPackage {
  override fun createNativeModules(reactContext: ReactApplicationContext): List<NativeModule> =
    listOf(NfcPassportModule(reactContext))

  override fun createViewManagers(reactContext: ReactApplicationContext): List<ViewManager<*, *>> =
    emptyList()
}
