/**
 * Covers `shouldBlockOnFaceMismatch` — the decision `RegisterScreen.tsx`'s
 * `handleLivenessCapture` uses to gate registration on the on-device face
 * match result. See that file's `matchAgainstPassport` call site and
 * `faceMatchGating.ts`'s own doc comment for why this lives in its own
 * RN-free module rather than being tested by rendering the screen.
 */
import { shouldBlockOnFaceMismatch } from './faceMatchGating';

describe('shouldBlockOnFaceMismatch', () => {
  it('blocks on a real mismatch (comparison ran, faces did not match)', () => {
    expect(shouldBlockOnFaceMismatch({ matched: false, skipped: false })).toBe(true);
  });

  it('does not block on a real match (comparison ran, faces matched)', () => {
    expect(shouldBlockOnFaceMismatch({ matched: true, skipped: false })).toBe(false);
  });

  it('does not block on a legitimate skip even though matched is false', () => {
    // FaceMatchModule.kt always sends matched: false alongside skipped: true
    // (e.g. an undecodable DG2 photo format) — this must fall through to
    // registration proceeding on liveness signals alone, not be treated as
    // a failed match.
    expect(shouldBlockOnFaceMismatch({ matched: false, skipped: true })).toBe(false);
  });

  it('does not block a skip even if matched were somehow true', () => {
    // Defensive: skipped should be authoritative regardless of matched.
    expect(shouldBlockOnFaceMismatch({ matched: true, skipped: true })).toBe(false);
  });
});
