/**
 * Tests the `CommitteeCrypto` injection boundary itself: the default stub throws a
 * clear "not implemented" error and never fabricates a result, and
 * `setCommitteeCrypto`/`resetCommitteeCrypto` correctly swap the module-wide instance.
 * No real cryptography is exercised here — there is none in this file, by design.
 */
import {
  getCommitteeCrypto,
  notImplementedCommitteeCrypto,
  resetCommitteeCrypto,
  setCommitteeCrypto,
  type CommitteeCrypto,
} from './CommitteeCrypto';

afterEach(() => {
  resetCommitteeCrypto();
});

describe('notImplementedCommitteeCrypto', () => {
  it('rejects with a "not implemented" error rather than returning a fabricated result', async () => {
    await expect(
      notImplementedCommitteeCrypto.evaluateQuery(new Uint8Array(32), new Uint8Array(64)),
    ).rejects.toThrow(/not implemented/);
  });
});

describe('getCommitteeCrypto / setCommitteeCrypto / resetCommitteeCrypto', () => {
  it('defaults to notImplementedCommitteeCrypto', () => {
    expect(getCommitteeCrypto()).toBe(notImplementedCommitteeCrypto);
  });

  it('returns whatever implementation was installed via setCommitteeCrypto', () => {
    const fixture: CommitteeCrypto = {
      evaluateQuery: jest.fn().mockResolvedValue({
        pk: new Uint8Array(64),
        evaluation: new Uint8Array(64),
        dlogProof: new Uint8Array(64),
      }),
    };
    setCommitteeCrypto(fixture);
    expect(getCommitteeCrypto()).toBe(fixture);
  });

  it('resetCommitteeCrypto restores the default stub', () => {
    setCommitteeCrypto({ evaluateQuery: jest.fn() });
    resetCommitteeCrypto();
    expect(getCommitteeCrypto()).toBe(notImplementedCommitteeCrypto);
  });
});
