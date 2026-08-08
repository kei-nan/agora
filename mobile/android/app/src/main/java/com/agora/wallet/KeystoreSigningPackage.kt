package com.agora.wallet

import com.facebook.react.ReactPackage
import com.facebook.react.bridge.NativeModule
import com.facebook.react.bridge.ReactApplicationContext
import com.facebook.react.uimanager.ViewManager

/**
 * Classic (pre-Codegen) `ReactPackage` registering `KeystoreSigningModule`.
 * Mirrors `../nfc/NfcPassportPackage.kt` — see that file's doc comment for
 * why this app uses the classic bridge registration pattern instead of a
 * TurboModule spec (`newArchEnabled=false`).
 *
 * Registered manually in `MainApplication.kt` (autolinking doesn't apply to
 * modules that live inside this app's own source tree).
 */
class KeystoreSigningPackage : ReactPackage {
  override fun createNativeModules(reactContext: ReactApplicationContext): List<NativeModule> =
    listOf(KeystoreSigningModule(reactContext))

  override fun createViewManagers(reactContext: ReactApplicationContext): List<ViewManager<*, *>> =
    emptyList()
}
