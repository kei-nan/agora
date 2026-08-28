/**
 * Thin JS/TS bridge to the Android-only native face-match/liveness modules
 * (`mobile/android/app/src/main/java/com/agora/facematch/`):
 * `FaceCaptureModule` (live-preview still capture + per-shot ML Kit liveness
 * signals) and `FaceMatchModule` (MobileFaceNet embedding comparison against
 * the passport's DG2 photo). See those files' doc comments for the
 * architecture decisions (custom CameraX module instead of a third-party RN
 * camera library, bundled ML Kit model, model-asset provenance) —
 * `docs/project/changelog/087.md` has the full write-up.
 *
 * Mirrors `./nfcPassportReader.ts`/`./keystoreSigner.ts`'s bridging pattern:
 * a `NativeModules` lookup that's `undefined` if the module isn't linked
 * (e.g. iOS, or a JS-only test environment), and an `isAvailable`-style guard
 * callers check before use. Live preview itself is a separate native view
 * component (`./FaceCameraView.tsx`, `requireNativeComponent`), not bridged
 * here.
 *
 * **Android only.** iOS has no equivalent yet since `ios/` itself doesn't
 * exist (see CLAUDE.md's Mobile App section).
 *
 * Not runtime-tested: no Android SDK, emulator, or physical device is
 * available in this development environment. Only `tsc --noEmit` and the
 * unit tests in `faceMatch.test.ts` (which mock the native bridge) have
 * verified this file.
 */
import { NativeModules, Platform } from 'react-native';
import { Buffer } from 'buffer';

/** One randomized liveness challenge asked of the citizen, plus the always-taken frontal baseline. See `RegisterScreen.tsx`. */
export type LivenessChallenge = 'baseline' | 'blink' | 'turn';

/** Per-shot signals `FaceCaptureModule.capturePhoto` reads off the frame via ML Kit, right after capture. */
export interface CapturedPhoto {
  /** file:// URI of the captured JPEG on-device — never uploaded anywhere. */
  uri: string;
  /** `-1` if ML Kit did not compute this (uncomputed probability), not a real 0-1 value. */
  leftEyeOpenProbability: number;
  rightEyeOpenProbability: number;
  /** Degrees the head is turned; positive = the subject's right. */
  headEulerAngleY: number;
}

export interface MatchResult {
  matched: boolean;
  /** `true` if the DG2 photo couldn't be decoded (e.g. JPEG2000 — see `FaceMatchModule.kt`) and no comparison ran at all. */
  skipped: boolean;
  score?: number;
  reason?: string;
}

/**
 * Result of {@link captureFaceAndQr} — the same per-shot liveness signals
 * {@link CapturedPhoto} carries, plus whatever QR code (if any) was decoded
 * from the very same frame. See that function's doc comment for why this
 * exists: a single frame proving both signals at once is the whole point of
 * the QR-liveness-challenge's two-shot redesign — see
 * `../screens/qrLivenessChallenge.ts`.
 */
export interface CapturedFaceAndQr {
  /** file:// URI of the captured JPEG on-device — never uploaded anywhere. */
  uri: string;
  /** `-1` if ML Kit found no face in the frame at all (uncomputed probability), not a real 0-1 value — same convention as `CapturedPhoto`. */
  leftEyeOpenProbability: number;
  rightEyeOpenProbability: number;
  headEulerAngleY: number;
  /** Raw decoded text of the first QR code found in the frame, or `null` if none was found. */
  qrText: string | null;
}

interface FaceCaptureModuleNative {
  hasCameraPermission(): Promise<boolean>;
  requestCameraPermission(): Promise<boolean>;
  capturePhoto(challenge: string): Promise<{
    uri: string;
    leftEyeOpenProbability: number;
    rightEyeOpenProbability: number;
    headEulerAngleY: number;
  }>;
  captureFaceAndQr(): Promise<{
    uri: string;
    leftEyeOpenProbability: number;
    rightEyeOpenProbability: number;
    headEulerAngleY: number;
    qrText: string | null;
  }>;
  deleteCaptureFile(uri: string): Promise<void>;
}

interface FaceMatchModuleNative {
  matchAgainstPassport(
    dg2Base64: string,
    dg2MimeType: string,
    capturedUri: string,
  ): Promise<{ matched: boolean; skipped: boolean; score?: number; reason?: string }>;
}

const FaceCaptureModule = NativeModules.FaceCaptureModule as FaceCaptureModuleNative | undefined;
const FaceMatchModule = NativeModules.FaceMatchModule as FaceMatchModuleNative | undefined;

/**
 * `true` only on Android with both native modules actually linked. Every
 * other export below throws if called while this is `false`.
 */
export function isFaceMatchAvailable(): boolean {
  return Platform.OS === 'android' && FaceCaptureModule != null && FaceMatchModule != null;
}

function requireCaptureModule(): FaceCaptureModuleNative {
  if (!isFaceMatchAvailable()) {
    throw new Error(
      `faceMatch: not available on this platform/build (Platform.OS=${Platform.OS}, ` +
        `FaceCaptureModule linked=${FaceCaptureModule != null}, FaceMatchModule linked=${FaceMatchModule != null}). ` +
        'Call isFaceMatchAvailable() first.',
    );
  }
  return FaceCaptureModule!;
}

function requireMatchModule(): FaceMatchModuleNative {
  if (!isFaceMatchAvailable()) {
    throw new Error(
      `faceMatch: not available on this platform/build (Platform.OS=${Platform.OS}, ` +
        `FaceCaptureModule linked=${FaceCaptureModule != null}, FaceMatchModule linked=${FaceMatchModule != null}). ` +
        'Call isFaceMatchAvailable() first.',
    );
  }
  return FaceMatchModule!;
}

/** Whether `CAMERA` runtime permission is already granted. */
export async function hasCameraPermission(): Promise<boolean> {
  return requireCaptureModule().hasCameraPermission();
}

/** Prompts the user for `CAMERA` runtime permission if not already granted. Resolves with the outcome, never rejects on a plain denial. */
export async function requestCameraPermission(): Promise<boolean> {
  return requireCaptureModule().requestCameraPermission();
}

/**
 * Takes one still frame from the already-mounted `<FaceCameraView>` preview
 * and returns it plus the liveness signals ML Kit read off it. `challenge`
 * is opaque to the native side — purely a label so multiple shots (the
 * frontal baseline plus a randomized liveness challenge) land in
 * distinguishable files; `RegisterScreen.tsx` is what actually evaluates
 * whether a shot satisfies whichever challenge it asked for.
 */
export async function capturePhoto(challenge: LivenessChallenge): Promise<CapturedPhoto> {
  return requireCaptureModule().capturePhoto(challenge);
}

/**
 * Takes one still frame from the already-mounted `<FaceCameraView>` preview
 * and runs BOTH ML Kit face detection and ML Kit barcode scanning against
 * it, returning the face signals {@link capturePhoto} would (or `-1`
 * probabilities if no face was found — never rejects for that, unlike
 * {@link capturePhoto}'s `NO_FACE_DETECTED`) plus whatever QR text was
 * decoded from that same frame (`null` if none). Backs the QR-liveness-
 * challenge's two-shot redesign in `RegisterScreen.tsx`: calling this twice,
 * with a freshly-regenerated QR session
 * (`../screens/qrLivenessChallenge.ts#createQrChallengeSession`) between
 * calls, is what proves a face and a live, freshly-issued nonce were both
 * presented together, at two distinct challenged moments — not two
 * separately-satisfiable checks. See `FaceCaptureModule.captureFaceAndQr`'s
 * doc comment for the full redesign and its residual-risk scope.
 */
export async function captureFaceAndQr(): Promise<CapturedFaceAndQr> {
  return requireCaptureModule().captureFaceAndQr();
}

/**
 * Deletes one specific capture file returned by an earlier {@link capturePhoto}
 * call, once the caller knows it's stale — e.g. a baseline/challenge shot
 * that failed its liveness check and is about to be retried, or a
 * passed-baseline shot left over because the user abandoned registration
 * before finishing the challenge substep. See `FaceCaptureModule.kt`'s
 * `deleteCaptureFile` doc comment for why this exists as a single-file
 * delete rather than reusing the `matchAgainstPassport` directory sweep
 * (a blanket sweep at these call sites could delete a *different*
 * still-needed capture file). Best-effort: this never throws — a cleanup
 * helper must not block or fail the registration flow it's cleaning up
 * after, so unavailability (e.g. iOS) or a native-side delete failure are
 * both silently swallowed.
 */
export async function deleteCaptureFile(uri: string): Promise<void> {
  try {
    await requireCaptureModule().deleteCaptureFile(uri);
  } catch {
    // Swallowed — see doc comment above.
  }
}

/**
 * Compares the passport's DG2 photo against the frontal baseline shot from
 * {@link capturePhoto}. `dg2MimeType` should be whatever
 * `RawPassportData.dg2MimeType` (`./nfcPassportReader.ts`) reported for this
 * passport — the native side resolves `{skipped: true}` rather than
 * rejecting when it can't decode the format (see `FaceMatchModule.kt`).
 */
export async function matchAgainstPassport(
  dg2: Uint8Array,
  dg2MimeType: string,
  capturedUri: string,
): Promise<MatchResult> {
  return requireMatchModule().matchAgainstPassport(Buffer.from(dg2).toString('base64'), dg2MimeType, capturedUri);
}
