/**
 * Thin JS/TS bridge to the Android-only `QrChallengeModule` native module
 * (`mobile/android/app/src/main/java/com/agora/facematch/QrChallengeModule.kt`)
 * — the decode half of the QR-code alternate liveness challenge (see
 * `../screens/qrLivenessChallenge.ts` for the session/nonce logic this feeds,
 * and `RegisterScreen.tsx` for where it's offered as an accessible
 * alternative to the default blink/turn challenge).
 *
 * Captures a still frame from the same live front-camera preview
 * `../native/faceMatch.ts`'s `capturePhoto` uses (`<FaceCameraView>`, already
 * mounted for the liveness step regardless of which challenge the user
 * picks) and decodes any QR code found in it via ML Kit's *Barcode Scanning*
 * API — a different ML Kit module than `com.google.mlkit:face-detection`
 * (already a dependency, used for the blink/turn signals), but the same
 * bundled/on-device model family: no Play Services download, no network
 * call, consistent with "nothing leaves your phone." Checked first, per this
 * app's existing precedent of reusing what's already there before adding a
 * library — ML Kit ships barcode scanning as `com.google.mlkit:barcode-
 * scanning`, a natural sibling to the face-detection dependency already in
 * `android/app/build.gradle`, so it was added there rather than reaching for
 * a third-party RN camera/barcode library.
 *
 * Deliberately a single atomic native call (capture + decode), unlike the
 * face path's separate `capturePhoto`/`matchAgainstPassport` steps: there's
 * no reason to hand a QR-code photo back to JS or leave it on disk even
 * briefly — the native module deletes its temp file immediately after
 * decoding either way (see `QrChallengeModule.kt`).
 *
 * **Android only.** iOS has no equivalent yet since `ios/` itself doesn't
 * exist (see CLAUDE.md's Mobile App section) — same standing limitation as
 * every other native module in this app.
 *
 * Not runtime-tested: no Android SDK, emulator, or physical device is
 * available in this development environment. `qrLivenessChallenge.ts`'s pure
 * nonce/session/payload logic (the part that doesn't touch a native module)
 * is unit-tested directly.
 */
import { NativeModules, Platform } from 'react-native';

interface QrChallengeModuleNative {
  /**
   * Resolves with the raw decoded text of the first QR code found in the
   * captured frame, or `null` if none was found. Never resolves with a
   * face-detection-style rejection the way `FaceCaptureModule.capturePhoto`
   * does — a frame with no visible face is expected and fine here, since the
   * whole point is the user is holding up a QR code, not their face.
   */
  captureAndDecodeQrCode(): Promise<string | null>;
}

const QrChallengeModule = NativeModules.QrChallengeModule as QrChallengeModuleNative | undefined;

/** `true` only on Android with the native module actually linked. */
export function isQrChallengeScanAvailable(): boolean {
  return Platform.OS === 'android' && QrChallengeModule != null;
}

/**
 * Takes one still frame from the already-mounted `<FaceCameraView>` preview
 * and decodes any QR code in it. Throws if called before checking
 * {@link isQrChallengeScanAvailable}, or if the camera preview isn't bound
 * yet (`CAMERA_NOT_READY`, mirroring `capturePhoto`'s own error).
 */
export async function captureAndDecodeQrCode(): Promise<string | null> {
  if (!isQrChallengeScanAvailable()) {
    throw new Error(
      `qrChallenge: not available on this platform/build (Platform.OS=${Platform.OS}, ` +
        `QrChallengeModule linked=${QrChallengeModule != null}). Call isQrChallengeScanAvailable() first.`,
    );
  }
  return QrChallengeModule!.captureAndDecodeQrCode();
}
