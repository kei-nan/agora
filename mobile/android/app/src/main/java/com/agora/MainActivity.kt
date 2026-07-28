package com.agora

import android.app.PendingIntent
import android.content.Intent
import android.nfc.NfcAdapter
import android.nfc.Tag
import android.nfc.tech.IsoDep
import android.os.Build
import android.os.Bundle
import com.agora.nfc.NfcPassportModule
import com.facebook.react.ReactActivity
import com.facebook.react.ReactActivityDelegate
import com.facebook.react.defaults.DefaultNewArchitectureEntryPoint.fabricEnabled
import com.facebook.react.defaults.DefaultReactActivityDelegate

class MainActivity : ReactActivity() {

  /**
   * Returns the name of the main component registered from JavaScript. This is used to schedule
   * rendering of the component.
   */
  override fun getMainComponentName(): String = "Agora"

  /**
   * Returns the instance of the [ReactActivityDelegate]. We use [DefaultReactActivityDelegate]
   * which allows you to enable New Architecture with a single boolean flags [fabricEnabled]
   */
  override fun createReactActivityDelegate(): ReactActivityDelegate =
      DefaultReactActivityDelegate(this, mainComponentName, fabricEnabled)

  private var nfcAdapter: NfcAdapter? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    super.onCreate(savedInstanceState)
    nfcAdapter = NfcAdapter.getDefaultAdapter(this)
  }

  /**
   * Standard Android NFC foreground-dispatch setup, needed so this activity
   * (rather than the system's default tag handling) receives passport chip
   * taps while it's in the foreground — see
   * `com/agora/nfc/NfcPassportModule.kt`, which this feeds via
   * `onNewIntent` below. `android:launchMode="singleTask"` on this activity
   * (AndroidManifest.xml) is what makes `onNewIntent` fire on the existing
   * instance instead of a new one being created.
   *
   * `PendingIntent.FLAG_MUTABLE` is required from Android 12 (API 31) onward
   * — NFC dispatch needs to write the discovered `Tag` into the intent's
   * extras, which an immutable `PendingIntent` would reject. The flag's
   * underlying int value (`0x02000000`) is simply an unused bit on pre-31
   * devices (minSdkVersion 23 here), so this is safe across the whole
   * supported range — this is the standard pattern for NFC foreground
   * dispatch on modern Android.
   */
  override fun onResume() {
    super.onResume()
    val adapter = nfcAdapter ?: return
    val intent = Intent(this, javaClass).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP)
    val pendingIntent = PendingIntent.getActivity(
      this,
      0,
      intent,
      PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
    // Null intentFilters + a tech list means dispatch by NFC technology
    // (TECH_DISCOVERED) rather than by NDEF record type — correct here since
    // a passport chip's ICAO applet is read via raw ISO-DEP APDUs, not NDEF.
    val techLists = arrayOf(arrayOf(IsoDep::class.java.name))
    adapter.enableForegroundDispatch(this, pendingIntent, null, techLists)
  }

  override fun onPause() {
    super.onPause()
    nfcAdapter?.disableForegroundDispatch(this)
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    // Keeps getIntent() current for this singleTask activity — standard RN
    // recipe (see reactnative.dev/docs/linking#enabling-deep-links), also
    // needed for the democracychain:// auth deep link (AndroidManifest.xml)
    // to keep working on a warm start now that this override exists.
    setIntent(intent)
    val tag: Tag? = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
      intent.getParcelableExtra(NfcAdapter.EXTRA_TAG, Tag::class.java)
    } else {
      @Suppress("DEPRECATION")
      intent.getParcelableExtra(NfcAdapter.EXTRA_TAG)
    }
    if (tag != null) {
      NfcPassportModule.onTagDiscovered(tag)
    }
  }
}
