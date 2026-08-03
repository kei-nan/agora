/**
 * Tests `submitExtrinsic`'s status-callback state machine against a fake
 * `SubmittableExtrinsic`-shaped object, covering the two bugs it was written
 * to fix (see its own module doc comment) that the duplicated inline
 * `signAndSend` pattern it replaced did not handle:
 *  - a call that never reaches `isFinalized` or a `dispatchError` times out
 *    instead of hanging forever;
 *  - `unsub` is captured and called exactly once, including when it arrives
 *    *after* the call already settled via the timeout path (the race the
 *    module doc comment calls out explicitly).
 */
import { submitExtrinsic, DEFAULT_SUBMIT_TIMEOUT_MS } from './submitExtrinsic';

type StatusCallback = (result: any) => void;

/** A fake tx whose `signAndSend` gives the test full control over when the
 * status callback fires and when `signAndSend`'s own promise resolves with
 * `unsub` — the two independent timelines `submitExtrinsic` has to reconcile. */
function fakeTx() {
  const unsub = jest.fn();
  let capturedCallback: StatusCallback | null = null;
  let resolveSignAndSend: (() => void) | null = null;

  const tx = {
    signAndSend: jest.fn((_pair: unknown, callback: StatusCallback) => {
      capturedCallback = callback;
      return new Promise<() => void>((resolve) => {
        resolveSignAndSend = () => resolve(unsub);
      });
    }),
  };

  return {
    tx,
    unsub,
    fireStatus: (result: any) => capturedCallback!(result),
    resolveSignAndSendPromise: () => resolveSignAndSend!(),
  };
}

const fakePair = {} as any;

describe('submitExtrinsic', () => {
  it('resolves once status.isFinalized fires with no dispatchError', async () => {
    const { tx, fireStatus, resolveSignAndSendPromise } = fakeTx();
    const promise = submitExtrinsic(tx as any, fakePair);
    fireStatus({ status: { isFinalized: true }, events: [], dispatchError: undefined });
    resolveSignAndSendPromise();
    await expect(promise).resolves.toBeUndefined();
  });

  it('rejects on dispatchError without waiting for isFinalized', async () => {
    const { tx, fireStatus, resolveSignAndSendPromise } = fakeTx();
    const promise = submitExtrinsic(tx as any, fakePair);
    fireStatus({ status: { isFinalized: false }, events: [], dispatchError: { toString: () => 'module.SomeError' } });
    resolveSignAndSendPromise();
    await expect(promise).rejects.toThrow('module.SomeError');
  });

  it('calls unsub exactly once after resolving', async () => {
    const { tx, unsub, fireStatus, resolveSignAndSendPromise } = fakeTx();
    const promise = submitExtrinsic(tx as any, fakePair);
    fireStatus({ status: { isFinalized: true }, events: [], dispatchError: undefined });
    resolveSignAndSendPromise();
    await promise;
    expect(unsub).toHaveBeenCalledTimes(1);
  });

  it('invokes onEvents on every non-error status update before resolving', async () => {
    const { tx, fireStatus, resolveSignAndSendPromise } = fakeTx();
    const seen: unknown[][] = [];
    const promise = submitExtrinsic(tx as any, fakePair, {
      onEvents: (events) => seen.push(events),
    });
    fireStatus({ status: { isFinalized: false }, events: ['inBlock-event'], dispatchError: undefined });
    fireStatus({ status: { isFinalized: true }, events: ['finalized-event'], dispatchError: undefined });
    resolveSignAndSendPromise();
    await promise;
    expect(seen).toEqual([['inBlock-event'], ['finalized-event']]);
  });

  it('times out and rejects if neither isFinalized nor dispatchError ever arrives', async () => {
    jest.useFakeTimers();
    try {
      const { tx, unsub } = fakeTx();
      // Deliberately never call fireStatus or resolveSignAndSendPromise — the
      // real-world case of a transaction that gets dropped/retracted.
      const promise = submitExtrinsic(tx as any, fakePair, { timeoutMs: 5_000 });
      const expectation = expect(promise).rejects.toThrow(/timed out|no finalization/i);
      jest.advanceTimersByTime(5_000);
      await expectation;
      // signAndSend's own promise never resolved in this scenario, so unsub
      // was never available to call — the timeout path must still settle
      // the outer promise without it.
      expect(unsub).not.toHaveBeenCalled();
    } finally {
      jest.useRealTimers();
    }
  });

  it('defaults to DEFAULT_SUBMIT_TIMEOUT_MS when no timeoutMs is given', async () => {
    jest.useFakeTimers();
    try {
      const { tx } = fakeTx();
      const promise = submitExtrinsic(tx as any, fakePair);
      const expectation = expect(promise).rejects.toThrow();
      jest.advanceTimersByTime(DEFAULT_SUBMIT_TIMEOUT_MS);
      await expectation;
    } finally {
      jest.useRealTimers();
    }
  });

  it('calls unsub immediately if signAndSend resolves with it after the call already timed out', async () => {
    jest.useFakeTimers();
    try {
      const { tx, unsub, resolveSignAndSendPromise } = fakeTx();
      const promise = submitExtrinsic(tx as any, fakePair, { timeoutMs: 1_000 });
      const expectation = expect(promise).rejects.toThrow();
      jest.advanceTimersByTime(1_000);
      await expectation;
      expect(unsub).not.toHaveBeenCalled();

      // signAndSend's own promise resolves late, well after the timeout
      // already rejected the outer promise — this is the exact race the
      // module doc comment describes: `unsub` must still be called so the
      // status subscription doesn't outlive the call that owns it.
      resolveSignAndSendPromise();
      await Promise.resolve(); // let the .then() microtask run
      expect(unsub).toHaveBeenCalledTimes(1);
    } finally {
      jest.useRealTimers();
    }
  });

  it('ignores a status update that arrives after the call already settled', async () => {
    const { tx, fireStatus, resolveSignAndSendPromise } = fakeTx();
    const promise = submitExtrinsic(tx as any, fakePair);
    fireStatus({ status: { isFinalized: true }, events: [], dispatchError: undefined });
    resolveSignAndSendPromise();
    await promise;

    // A late status update (e.g. the node still streaming updates for a
    // finalized extrinsic) must not throw or otherwise disturb an
    // already-settled promise.
    expect(() =>
      fireStatus({ status: { isFinalized: false }, events: [], dispatchError: { toString: () => 'late.Error' } }),
    ).not.toThrow();
  });
});
