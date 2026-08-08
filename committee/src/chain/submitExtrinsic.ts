/**
 * Shared `signAndSend` wrapper for chain-mutating calls in the committee-duty app.
 *
 * Deliberate near-duplicate of `mobile/src/chain/submitExtrinsic.ts` — see this
 * package's `chain/api.ts` doc comment for why this app doesn't depend on mobile/'s
 * modules even for small shared pieces like this one. Behavior is identical: resolves
 * once `tx` is finalized with no `dispatchError`, rejects on a `dispatchError` or on
 * timeout, and always unsubscribes from the status stream before settling so no
 * callback fires again after the promise has settled.
 */
import type { SubmittableExtrinsic } from '@polkadot/api/types';
import type { KeyringPair } from '@polkadot/keyring/types';
import type { EventRecord } from '@polkadot/types/interfaces';
import type { ISubmittableResult } from '@polkadot/types/types';

/** How long to wait for finalization before giving up. */
export const DEFAULT_SUBMIT_TIMEOUT_MS = 60_000;

export interface SubmitExtrinsicOptions {
  /** Overrides {@link DEFAULT_SUBMIT_TIMEOUT_MS}. */
  timeoutMs?: number;
  /** Called on every non-error status update with that update's events. */
  onEvents?: (events: EventRecord[]) => void;
}

export function submitExtrinsic(
  tx: SubmittableExtrinsic<'promise'>,
  pair: KeyringPair,
  options: SubmitExtrinsicOptions = {},
): Promise<void> {
  const { timeoutMs = DEFAULT_SUBMIT_TIMEOUT_MS, onEvents } = options;

  return new Promise<void>((resolve, reject) => {
    let settled = false;
    let unsub: (() => void) | null = null;

    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      unsub?.();
      reject(
        new Error(
          `submitExtrinsic: no finalization or error after ${timeoutMs}ms — the transaction may have been dropped`,
        ),
      );
    }, timeoutMs);

    const settle = (fn: () => void) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      unsub?.();
      fn();
    };

    tx.signAndSend(pair, (result: ISubmittableResult) => {
      if (settled) return;
      const { status, events, dispatchError } = result;
      if (dispatchError) {
        settle(() => reject(new Error(dispatchError.toString())));
        return;
      }
      onEvents?.(events);
      if (status.isFinalized) {
        settle(resolve);
      }
    })
      .then((u) => {
        unsub = u;
        if (settled) unsub();
      })
      .catch((e) => settle(() => reject(e)));
  });
}
