/**
 * OPRF committee-duty pallet integration — the "check for pending duties" /
 * "fulfill duty" flow described in `docs/project/changelog/082.md` entry 82.
 *
 * # Migrated to the real two-round protocol
 *
 * `pallet-identity` was rewritten from a single-response design
 * (`submit_oprf_response`, call index 16, and its `pendingOprfQueries`/`oprfResponses`
 * storage) into a genuine `t`-of-`n` threshold protocol — Option B in
 * `docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`.
 * `committee-node/src/extrinsic.rs` and `wasm_host.rs` were updated for this already;
 * this file previously was not (it still called the retired `submitOprfResponse` and
 * queried the retired `oprfResponses` storage — neither exists on-chain any more).
 *
 * The real calls, confirmed against `pallets/pallet-identity/src/lib.rs`:
 *  - `submit_oprf_round1(query_id: u64, committee_slot: u8, r_i: [u8; 64], d_g: [u8;
 *    64], d_q: [u8; 64], e_g: [u8; 64], e_q: [u8; 64])` — call index 16.
 *  - `submit_oprf_round2(query_id: u64, committee_slot: u8, z_i: [u8; 32])` — call
 *    index 17.
 *  - storage double-map `oprfRound1Commitments: (query_id, committee_slot) ->
 *    BoundedVec<OprfRound1Commitment>` — one entry per member who has submitted round 1
 *    for that pair, `ValueQuery` (empty vec, not `None`, when nothing has been
 *    submitted yet).
 *  - storage double-map `oprfRound2Responses: (query_id, committee_slot) ->
 *    BoundedVec<OprfRound2Response>` — same shape/locking pattern as round 1.
 *  - pallet constant `oprfThreshold` (`T::OprfThreshold`) — how many round-1
 *    commitments lock a `(query_id, committee_slot)`'s qualifying set before round 2
 *    opens. Read via `api.consts.identity.oprfThreshold`, not a storage query — it's a
 *    `#[pallet::constant]`, part of chain metadata, not chain state.
 *
 * `pendingOprfQueries`/`committeeMembers` are unchanged from the original (correct)
 * implementation — only the response-submission and "is this pair still outstanding"
 * storage changed. See `getMemberCommitteeSlots`/`fetchPendingDuties`'s own doc
 * comments: the "poll every slot, cross-reference the roster" shape those two already
 * used is untouched by this migration.
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

/** Which round this member still owes a submission for on a given `(query, slot)` pair. */
export type DutyPhase = 'round1' | 'round2';

/** A `PendingOprfQuery` this member specifically owes a response for, on one committee slot. */
export interface PendingDuty extends PendingOprfQuery {
  committeeSlot: number;
  phase: DutyPhase;
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
 *
 * Unchanged by the two-round migration — confirmed still correct against the real
 * pallet, do not modify.
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

/** Decodes an `OprfRound1Commitments`/`OprfRound2Responses` `BoundedVec` entry's `member` field. */
function memberOf(entry: any): string {
  return String(entry.member);
}

/**
 * Which round (if any) `address` still owes a submission for on this `(queryId,
 * committeeSlot)` pair, given `threshold` (`T::OprfThreshold`):
 *
 *  - not yet in the round-1 set, and the set hasn't locked without them -> `'round1'`.
 *  - not yet in the round-1 set, but the set already locked (reached `threshold`
 *    members) without them -> `null` (missed it; nothing this member can do for this
 *    pair, matching `submit_oprf_round1`'s `OprfRound1SetLocked` rejection).
 *  - in the round-1 set, but it hasn't locked yet -> `null` (round 2 isn't open yet;
 *    `submit_oprf_round2` would reject with `OprfRound1NotLocked`).
 *  - in the (locked) round-1 set, and not yet in the round-2 set -> `'round2'`.
 *  - in both -> `null` (this member's duty for this pair is done).
 *
 * Replaces the retired `hasAlreadyResponded`, which checked presence in the (also
 * retired) `oprfResponses` double-map. Same double-map/`ValueQuery` shape convention
 * `OprfRound1Commitments`/`OprfRound2Responses` share — an absent entry decodes as an
 * empty `BoundedVec`, not `None`.
 */
async function dutyPhaseFor(
  api: Awaited<ReturnType<typeof getApi>>,
  queryId: number,
  committeeSlot: number,
  address: string,
  threshold: number,
): Promise<DutyPhase | null> {
  const round1Raw = await (api.query.identity as any).oprfRound1Commitments(queryId, committeeSlot);
  const round1: any[] = (round1Raw as any).toJSON() ?? [];
  const inRound1 = round1.some((c) => memberOf(c) === address);

  if (!inRound1) {
    return round1.length < threshold ? 'round1' : null;
  }
  if (round1.length < threshold) {
    return null; // in the set, but it hasn't locked yet — round 2 isn't open
  }

  const round2Raw = await (api.query.identity as any).oprfRound2Responses(queryId, committeeSlot);
  const round2: any[] = (round2Raw as any).toJSON() ?? [];
  const inRound2 = round2.some((r) => memberOf(r) === address);
  return inRound2 ? null : 'round2';
}

/**
 * Polls chain state for this member's pending committee duties: every
 * `pendingOprfQueries` entry, cross-referenced against `committeeMembers` for which
 * slot(s) `address` serves on, filtered down to `(query, slot)` pairs where `address`
 * currently owes either a round-1 or round-2 submission (see `dutyPhaseFor`).
 *
 * This is the whole "check" side of the poll-on-open pattern changelog 082 specifies
 * — no push notifications, no background service; call this when the app is opened or
 * the screen is refreshed, matching how `pallet-courts`' jury-duty selection is
 * checked by the citizen rather than pushed to them.
 *
 * The slot-iteration shape (poll every slot the member serves on, per query) is
 * unchanged by the two-round migration — confirmed already correct, do not modify;
 * only `dutyPhaseFor`'s underlying storage changed.
 */
export async function fetchPendingDuties(address: string): Promise<PendingDuty[]> {
  const api = await getApi();
  const mySlots = await getMemberCommitteeSlots(address);
  if (mySlots.length === 0) return [];

  const threshold = (api.consts.identity as any).oprfThreshold.toNumber();
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
      const phase = await dutyPhaseFor(api, queryId, committeeSlot, address, threshold);
      if (phase) {
        duties.push({ queryId, submitter, blindedQuery, postedAt, committeeSlot, phase });
      }
    }
  }

  return duties.sort((a, b) => a.queryId - b.queryId || a.committeeSlot - b.committeeSlot);
}

export interface SubmitRound1Params {
  duty: Pick<PendingDuty, 'queryId' | 'committeeSlot' | 'blindedQuery'>;
  /** This member's OPRF secret share (see `storage/keyStorage.ts` — DEV-ONLY today). */
  secretShareBytes: Uint8Array;
  /** Signs the `submit_oprf_round1` transaction (the chain-account key, distinct from `secretShareBytes`). */
  pair: KeyringPair;
  /** Overrides the module-wide `CommitteeCrypto` — tests inject a fixture here. */
  crypto?: CommitteeCrypto;
  /**
   * Fresh 32-byte per-query randomness for the FROST-style nonce commitments —
   * required, not generated internally, because the *caller* must retain this exact
   * value and replay it into {@link submitRound2} for the same `(queryId,
   * committeeSlot)` pair (see `CommitteeCrypto.ts`'s doc comment on `round1`). Tests
   * pass a fixed fixture; real callers should draw this from
   * `crypto/wasmCommitteeCrypto.ts`'s `freshOprfSeed()`.
   */
  seed: Uint8Array;
}

/**
 * Submits round 1 of the threshold protocol for one pending duty: computes this
 * member's partial evaluation and nonce commitments via
 * {@link CommitteeCrypto.round1}, then submits `submit_oprf_round1(query_id,
 * committee_slot, r_i, d_g, d_q, e_g, e_q)`.
 *
 * Deliberately does not itself decide whether the duty is still outstanding or which
 * phase it's in — call `fetchPendingDuties` first and pass one of its `phase ===
 * 'round1'` results in. Validates `duty.committeeSlot`/`duty.blindedQuery`'s length
 * before ever calling the crypto core or the chain, mirroring `identity.ts`'s "fail
 * locally with a specific message" convention.
 *
 * The caller (not this function) is responsible for holding onto `params.seed` and
 * passing the *same* bytes into {@link submitRound2} once this pair's round-1 set
 * locks — a mismatched seed silently fails to combine into a valid proof rather than
 * erroring anywhere (see `CommitteeCrypto.ts`'s doc comment).
 */
export async function submitRound1(params: SubmitRound1Params): Promise<void> {
  const { duty, secretShareBytes, pair, seed } = params;
  assertValidCommitteeSlot(duty.committeeSlot, 'submitRound1');
  assertBlindedQueryLength(duty.blindedQuery, 'submitRound1');

  const crypto = params.crypto ?? getCommitteeCrypto();
  const commitment = await crypto.round1(secretShareBytes, duty.blindedQuery, seed);

  const api = await getApi();
  return submitExtrinsic(
    (api.tx.identity as any).submitOprfRound1(
      duty.queryId,
      duty.committeeSlot,
      commitment.rI,
      commitment.dG,
      commitment.dQ,
      commitment.eG,
      commitment.eQ,
    ),
    pair,
  );
}

export interface SubmitRound2Params {
  duty: Pick<PendingDuty, 'queryId' | 'committeeSlot'>;
  /** This member's OPRF secret share — the same value passed to `submitRound1`. */
  secretShareBytes: Uint8Array;
  /** Signs the `submit_oprf_round2` transaction. */
  pair: KeyringPair;
  /** Overrides the module-wide `CommitteeCrypto` — tests inject a fixture here. */
  crypto?: CommitteeCrypto;
  /** The exact seed passed to `submitRound1` for this same `(queryId, committeeSlot)` pair. */
  seed: Uint8Array;
  /**
   * This member's binding factor, Lagrange coefficient, and the shared round-2
   * challenge — public values computed from the now-locked round-1 set via
   * `oprf-committee-dev::threshold::binding_factor`/`lagrange_coefficient`/
   * `combined_challenge` (see `committee-node/src/main.rs::try_round2` for the
   * reference computation). **This app has no JS/TS port of that aggregation math** —
   * see `crypto/CommitteeCrypto.ts`'s module doc for why (it's deliberately
   * native-Rust-only, not behind the wasm FFI boundary, since it touches no secret
   * material). Callers must supply real values from elsewhere until such a port
   * exists; this function does not compute or default them.
   */
  rhoI: Uint8Array;
  lambdaI: Uint8Array;
  e: Uint8Array;
}

/**
 * Submits round 2 of the threshold protocol for one pending duty: computes this
 * member's response scalar via {@link CommitteeCrypto.round2Response}, then submits
 * `submit_oprf_round2(query_id, committee_slot, z_i)`.
 *
 * Call only after `fetchPendingDuties` reports `phase === 'round2'` for this pair
 * (meaning round 1 has locked and this member is in the qualifying set) — see
 * `dutyPhaseFor`. See `params.rhoI`/`lambdaI`/`e`'s doc comments for the real,
 * currently-unclosed gap in this app: nothing here computes those three values.
 */
export async function submitRound2(params: SubmitRound2Params): Promise<void> {
  const { duty, secretShareBytes, pair, seed, rhoI, lambdaI, e } = params;
  assertValidCommitteeSlot(duty.committeeSlot, 'submitRound2');

  const crypto = params.crypto ?? getCommitteeCrypto();
  const response = await crypto.round2Response(secretShareBytes, seed, rhoI, lambdaI, e);

  const api = await getApi();
  return submitExtrinsic(
    (api.tx.identity as any).submitOprfRound2(duty.queryId, duty.committeeSlot, response.zI),
    pair,
  );
}
