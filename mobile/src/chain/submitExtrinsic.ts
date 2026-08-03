/**
 * Shared `signAndSend` wrapper for every chain-mutating call in this app
 * (identity.ts, voting.ts, governance.ts, constitution.ts, courts.ts).
 *
 * Replaces a `new Promise((resolve, reject) => tx.signAndSend(pair, ({ status,
 * dispatchError }) => ...))` pattern that was duplicated across all five of
 * those files with two bugs, present in every copy:
 *
 *  1. `signAndSend` resolves with an unsubscribe function once the callback
 *     is registered; none of the copies captured it, so the subscription
 *     was never torn down after the call settled — it kept listening (and
 *     re-invoking an already-resolved/rejected promise's dead callback) for
 *     the life of the socket.
 *  2. There was no timeout. A transaction that is dropped or retracted
 *     before ever reaching `isFinalized` or producing a `dispatchError`
 *     (chain restart, network partition, competing nonce, etc.) left the
 *     awaiting screen hung forever with no way out.
 *
 * This fixes both once, here, rather than in five places.
 */
import type { SubmittableExtrinsic } from '@polkadot/api/types';
import type { KeyringPair } from '@polkadot/keyring/types';
import type { EventRecord } from '@polkadot/types/interfaces';
import type { ISubmittableResult } from '@polkadot/types/types';

/** How long to wait for finalization before giving up. Generous for a 12s-block dev chain finalizing in ~2 blocks. */
export const DEFAULT_SUBMIT_TIMEOUT_MS = 60_000;

export interface SubmitExtrinsicOptions {
  /** Overrides {@link DEFAULT_SUBMIT_TIMEOUT_MS}. */
  timeoutMs?: number;
  /**
   * Called on every non-error status update with that update's events —
   * including ones before finalization — so callers that need to read an
   * event emitted by the call (e.g. `submitProposal` reading back the
   * assigned `ProposalCreated.id`) can do so before the promise resolves.
   */
  onEvents?: (events: EventRecord[]) => void;
}

/**
 * Signs and submits `tx`, resolving once it is finalized with no
 * `dispatchError`, rejecting on a `dispatchError` or on timeout. Always
 * unsubscribes from the status stream before settling, so no callback fires
 * again after this promise has resolved or rejected.
 */
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
        // The call could already have settled (e.g. hit the timeout) before
        // signAndSend's own promise resolved with the unsub function.
        if (settled) unsub();
      })
      .catch((e) => settle(() => reject(e)));
  });
}
