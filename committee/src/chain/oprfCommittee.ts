/**
 * OPRF committee-duty pallet integration — the "check for pending duties" /
 * "fulfill duty" flow described in `docs/project/changelog/082.md` entry 82.
 *
 * # Reconciled against the real, now-landed pallet-identity mailbox
 *
 * This module was originally built against a provisional/guessed interface, before
 * `pallets/pallet-identity/src/lib.rs` had any of these primitives. **All of it has
 * since been confirmed against the real pallet**:
 *
 *  - extrinsic `submit_oprf_response(query_id: u64, committee_slot: u8, evaluation:
 *    [u8; 64], dlog_proof: BoundedVec<u8, ConstU32<64>>)` — call index 16 (confirmed;
 *    `submit_oprf_query` is 15) — `@polkadot/api` auto-camelCases this to
 *    `submitOprfResponse`, as already assumed.
 *  - storage map `pendingOprfQueries: query_id -> { submitter: AccountId,
 *    blinded_query: [u8; 64], posted_at: BlockNumber }` — field order and the
 *    64-byte x-then-y-big-endian `blinded_query` layout both confirmed correct
 *    against the real `OprfQueryRecord`.
 *  - storage map `committeeMembers: committee_slot (u8, 0..NUM_COMMITTEES) ->
 *    BoundedVec<AccountId>` — confirmed, `Blake2_128Concat`-keyed like every other
 *    numeric-keyed map in this pallet.
 *
 * **One correction from the original guess**: the real `OprfResponses` storage is a
 * `StorageDoubleMap<(query_id, committee_slot)> -> OprfResponseRecord { responder,
 * evaluation, dlog_proof }` — **one optional record per `(query, slot)` pair, not a
 * list of accounts to search.** The pallet's own doc comment confirms this is
 * deliberate: "presence of an entry for a given pair is exactly the idempotency guard
 * `submit_oprf_response` uses" — i.e. the question was never "has *this* address
 * responded," it's "has *any* member of this slot's roster already responded" (any
 * one of them fulfilling the duty satisfies it for the whole slot). The original
 * `hasAlreadyResponded` implementation searched a list for a matching address, which
 * doesn't match this shape at all — fixed below to a simple presence check.
 */
import { KeyringPair } from '@polkadot/keyring/types';
import { getApi } from './api';
import { submitExtrinsic } from './submitExtrinsic';
import { CommitteeCrypto, getCommitteeCrypto } from '../crypto/CommitteeCrypto';

/**
 * Number of independent OPRF committees. Mirrors `mobile/src/chain/identity.ts`'s
 * `NUM_OPRF_COMMITTEES` (itself mirroring `pallet_identity_zk::NUM_COMMITTEES = 5`,
 * changelog entry 73). Redefined here rather than imported — this app deliberately
 * doesn't depend on `mobile/`'s modules (see `chain/api.ts`'s doc comment) — so this
 * constant must be kept in lockstep with both of those by hand, the same way
 * `identity.ts` already keeps its own mirrored constants in lockstep with `verifier.rs`.
 */
export const NUM_COMMITTEES = 5;

/** One entry from `pendingOprfQueries`, decoded. */
export interface PendingOprfQuery {
  queryId: number;
  submitter: string;
  /** 64 bytes: the blinded query point, first 32 bytes X, last 32 bytes Y. */
  blindedQuery: Uint8Array;
  postedAt: number;
}

/** A `PendingOprfQuery` this member specifically owes a response for, on one committee slot. */
export interface PendingDuty extends PendingOprfQuery {
  committeeSlot: number;
}

function assertValidCommitteeSlot(slot: number, label: string): void {
  if (!Number.isInteger(slot) || slot < 0 || slot >= NUM_COMMITTEES) {
    throw new RangeError(`${label}: committee slot ${slot} out of range 0..${NUM_COMMITTEES}`);
  }
}

function assertBlindedQueryLength(blindedQuery: Uint8Array, label: string): void {
  if (blindedQuery.length !== 64) {
    throw new RangeError(`${label}: blindedQuery is ${blindedQuery.length} bytes, expected 64`);
  }
}

/**
 * Which committee slot(s) `address` belongs to, found by checking `committeeMembers`
 * for each slot `0..NUM_COMMITTEES` — there is no reverse index, so this is
 * `NUM_COMMITTEES` (5) small storage reads, not one query. A member is expected to
 * belong to at most one slot in practice, but this returns every match rather than
 * assuming that.
 */
export async function getMemberCommitteeSlots(address: string): Promise<number[]> {
  const api = await getApi();
  const slots: number[] = [];
  for (let slot = 0; slot < NUM_COMMITTEES; slot++) {
    const members = await api.query.identity.committeeMembers(slot);
    const list = (members as any).toJSON() as unknown[] | null;
    if (list && list.some((m) => String(m) === address)) {
      slots.push(slot);
    }
  }
  return slots;
}

/**
 * True if this `(queryId, committeeSlot)` pair already has a response recorded —
 * `OprfResponses` is a `StorageDoubleMap` holding at most one `OprfResponseRecord`
 * per pair (see this module's doc comment for why: any one member of a slot's
 * roster fulfilling the duty satisfies it for the whole slot, so this is a presence
 * check, not a per-`address` search). The `address` parameter from the original
 * (wrong) guess is gone — it was never meaningful for this real storage shape.
 */
async function hasAlreadyResponded(
  api: Awaited<ReturnType<typeof getApi>>,
  queryId: number,
  committeeSlot: number,
): Promise<boolean> {
  const responded = await (api.query.identity as any).oprfResponses(queryId, committeeSlot);
  return !(responded as any).isNone && !(responded as any).isEmpty;
}

/**
 * Polls chain state for this member's pending committee duties: every
 * `pendingOprfQueries` entry, cross-referenced against `committeeMembers` for which
 * slot(s) `address` serves on, filtered down to `(query, slot)` pairs with no response
 * recorded yet in `oprfResponses` (see `hasAlreadyResponded` — a presence check per
 * pair, not per-address, since any one member of the slot's roster answering closes
 * the duty for the whole slot).
 *
 * This is the whole "check" side of the poll-on-open pattern changelog 082 specifies
 * — no push notifications, no background service; call this when the app is opened or
 * the screen is refreshed, matching how `pallet-courts`' jury-duty selection is
 * checked by the citizen rather than pushed to them.
 */
export async function fetchPendingDuties(address: string): Promise<PendingDuty[]> {
  const api = await getApi();
  const mySlots = await getMemberCommitteeSlots(address);
  if (mySlots.length === 0) return [];

  const entries = await api.query.identity.pendingOprfQueries.entries();
  const duties: PendingDuty[] = [];

  for (const [key, value] of entries) {
    if ((value as any).isNone) continue;
    const raw = (value as any).isSome ? (value as any).unwrap() : value;
    const queryId = (key.args[0] as any).toNumber();
    const submitter = raw.submitter.toString();
    const blindedQueryRaw = raw.blindedQuery ?? raw.blinded_query;
    const blindedQuery: Uint8Array = blindedQueryRaw.toU8a
      ? blindedQueryRaw.toU8a()
      : new Uint8Array(blindedQueryRaw);
    const postedAtRaw = raw.postedAt ?? raw.posted_at;
    const postedAt = postedAtRaw.toNumber();

    for (const committeeSlot of mySlots) {
      const responded = await hasAlreadyResponded(api, queryId, committeeSlot);
      if (!responded) {
        duties.push({ queryId, submitter, blindedQuery, postedAt, committeeSlot });
      }
    }
  }

  return duties.sort((a, b) => a.queryId - b.queryId || a.committeeSlot - b.committeeSlot);
}

export interface FulfillDutyParams {
  duty: Pick<PendingDuty, 'queryId' | 'committeeSlot' | 'blindedQuery'>;
  /** This member's OPRF secret key share (see `storage/keyStorage.ts` — DEV-ONLY today). */
  secretKeyBytes: Uint8Array;
  /** Signs the `submit_oprf_response` transaction (the chain-account key, distinct from `secretKeyBytes`). */
  pair: KeyringPair;
  /** Overrides the module-wide `CommitteeCrypto` — tests inject a fixture here. */
  crypto?: CommitteeCrypto;
}

/**
 * Fulfills one pending duty: evaluates the blinded query under this member's secret
 * share via {@link CommitteeCrypto.evaluateQuery} (still the stub — see
 * `CommitteeCrypto.ts`'s doc comment for what's real vs. not), then submits
 * `submit_oprf_response(query_id, committee_slot, evaluation, dlog_proof)`.
 *
 * `duty.blindedQuery` is now passed to `evaluateQuery` as a single 64-byte value
 * with no split/join step — the real Wasm ABI takes/returns it whole, matching
 * `PendingOprfQueries`/`OprfResponses`' own on-chain layout directly (the earlier
 * guessed ABI needed separate 32-byte X/Y halves; the real one doesn't).
 *
 * Deliberately does not itself decide whether the duty is still outstanding — call
 * `fetchPendingDuties` first and pass one of its results in. Validates
 * `duty.committeeSlot` and `duty.blindedQuery`'s length before ever calling the (stub)
 * crypto function or the chain, mirroring `identity.ts`'s "fail locally with a
 * specific message" convention.
 */
export async function fulfillDuty(params: FulfillDutyParams): Promise<void> {
  const { duty, secretKeyBytes, pair } = params;
  assertValidCommitteeSlot(duty.committeeSlot, 'fulfillDuty');
  assertBlindedQueryLength(duty.blindedQuery, 'fulfillDuty');

  const crypto = params.crypto ?? getCommitteeCrypto();
  const result = await crypto.evaluateQuery(secretKeyBytes, duty.blindedQuery);

  const api = await getApi();
  return submitExtrinsic(
    (api.tx.identity as any).submitOprfResponse(
      duty.queryId,
      duty.committeeSlot,
      result.evaluation,
      result.dlogProof,
    ),
    pair,
  );
}
