/**
 * The one piece of `RegisterScreen.tsx`'s liveness-gate logic pulled out
 * into its own RN-free module so it's actually unit-testable: `faceMatch.ts`
 * (and transitively `FaceCameraView.tsx`'s `requireNativeComponent` call)
 * can't be imported under this repo's plain-`node` jest environment (see
 * `__mocks__/react-native.js`'s doc comment — the mock only stubs
 * `NativeModules`/`Platform`), so `RegisterScreen.tsx` itself can't be
 * required from a test either. This file has no `react-native` import at
 * runtime (`MatchResult` below is `import type`-only, erased at compile
 * time), so it can be tested directly.
 */
import type { MatchResult } from '../native/faceMatch';

/**
 * Whether a `matchAgainstPassport()` result should block registration
 * (require the user to retry the face-match capture) rather than let
 * `runLivenessGate()` resolve and registration proceed to `proving`.
 *
 * A real mismatch — the native module actually ran the comparison
 * (`skipped: false`) and it came back negative (`matched: false`) — blocks.
 * A skip (e.g. an undecodable DG2 photo format — see `FaceMatchModule.kt`'s
 * doc comment) is not a failure: registration is meant to still gate on the
 * liveness signals already captured and proceed, so a skip must never block
 * here, regardless of what `matched` happens to be alongside it (the native
 * side always sends `matched: false` alongside `skipped: true`, but that's
 * incidental — `skipped` is the actual signal to check).
 */
export function shouldBlockOnFaceMismatch(match: Pick<MatchResult, 'matched' | 'skipped'>): boolean {
  return !match.matched && !match.skipped;
}
