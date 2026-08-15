/**
 * Reconciles a citizen's locally-persisted registration pipeline record
 * (`registrationState.ts`) against live pallet-identity chain state, and
 * returns the single `RegistrationStatus` the UI should actually show.
 *
 * The chain is always checked first and, whenever it has an opinion
 * (`CitizenNullifier` is set — i.e. `register_citizen` has already landed),
 * it wins outright: `Active`/`ReverificationDue`/`Suspended` are facts the
 * chain answers authoritatively, and none of the three are ever persisted
 * locally (see `registrationState.ts`'s `PersistableStatus`). Only when the
 * chain has no nullifier on file yet — the citizen hasn't completed
 * registration — does this fall back to the locally-persisted pipeline
 * record, and even then it re-polls whatever on-chain evidence exists for
 * that stage (OPRF round 1/round 2 commitment counts, SLA expiry) rather
 * than trusting the local record blindly.
 *
 * Storage/const names below are pallet-identity's Rust items
 * (`pallets/pallet-identity/src/lib.rs`) run through polkadot-js's standard
 * PascalCase -> camelCase derivation, the same convention `identity.ts`'s
 * `citizenNullifier` and `governance.ts`'s `palletElections.backingOf`
 * already rely on.
 */
import { getApi } from './api';
import {
  PersistableStatus,
  RegistrationStatus,
  clearRegistrationStatus,
  readRegistrationStatus,
  writeRegistrationStatus,
} from './registrationState';

/** Per-committee-slot OPRF response counts — every registration query goes to all 5 committees symmetrically (changelog 073), so progress is tracked per slot, not as a single scalar. */
export interface CommitteeSlotProgress {
  slot: number;
  round1Count: number;
  round2Count: number;
}

export interface ReconciliationResult {
  status: RegistrationStatus;
  oprfProgress?: { threshold: number; slots: CommitteeSlotProgress[] };
}

/** Pipeline stages that carry OPRF query tracking fields — factored out since three stages share this exact shape. */
type OprfPendingRecord = Extract<
  PersistableStatus,
  { stage: 'OprfQuerySubmitted' | 'AwaitingCommitteeRound1' | 'AwaitingCommitteeRound2' }
>;

function isOprfPendingRecord(record: PersistableStatus): record is OprfPendingRecord {
  return (
    record.stage === 'OprfQuerySubmitted' ||
    record.stage === 'AwaitingCommitteeRound1' ||
    record.stage === 'AwaitingCommitteeRound2'
  );
}

export async function reconcileRegistrationStatus(address: string): Promise<ReconciliationResult> {
  const api = await getApi();

  // ── Step 1: chain-confirmed check — always runs first, and wins outright
  // whenever a nullifier is on file, since that means registration has
  // already landed on-chain and nothing about the local pipeline record
  // matters any more. ─────────────────────────────────────────────────────
  const nullifierOpt = await api.query.identity.citizenNullifier(address);
  if (!(nullifierOpt as any).isNone) {
    const header = await api.rpc.chain.getHeader();
    const currentBlock = header.number.toNumber();

    const deadlineOpt = await api.query.identity.reverificationDeadline(address);
    if ((deadlineOpt as any).isNone || currentBlock > (deadlineOpt as any).unwrap().toNumber()) {
      // A missing deadline means "past deadline", not "no deadline" — mirrors
      // is_active_citizen's own treatment of a missing ReverificationDeadline
      // entry (pallet-identity/src/lib.rs). Not cleared locally: reverifying
      // is the only thing that should move this forward again.
      return { status: { stage: 'ReverificationDue' } };
    }

    const nullifier = (nullifierOpt as any).unwrap();
    const suspendedOpt = await api.query.identity.suspendedNullifiers(nullifier);
    if ((suspendedOpt as any).isNone) {
      await clearRegistrationStatus(address).catch(() => undefined); // best-effort — a failed clear shouldn't break the status return
      return { status: { stage: 'Active' } };
    }

    const inner = (suspendedOpt as any).unwrap();
    if (inner.isNone) {
      return { status: { stage: 'Suspended', until: null } }; // indefinite suspension — do not clear, still suspended
    }
    const untilBlock = inner.unwrap().toNumber();
    if (currentBlock > untilBlock) {
      // Lazily-expired suspension, mirroring is_active_citizen's own
      // point-of-use expiry check — the chain only actually cleans this up
      // on the next extrinsic that happens to touch it.
      await clearRegistrationStatus(address).catch(() => undefined);
      return { status: { stage: 'Active' } };
    }
    return { status: { stage: 'Suspended', until: untilBlock } };
  }

  // ── Step 2: pipeline fallback — only reached when no nullifier exists yet. ─
  const record = await readRegistrationStatus(address);
  if (!record) return { status: { stage: 'NotStarted' } };

  if (
    record.stage === 'NotStarted' ||
    record.stage === 'PassportScanned' ||
    record.stage === 'ProofMaterialAssembled' ||
    record.stage === 'ProofReady' ||
    record.stage === 'ChainSubmissionPending' ||
    record.stage === 'Failed'
  ) {
    return { status: record };
  }

  if (record.stage === 'ProofCombining') {
    // Combination happens off-chain, client-side — no further chain evidence
    // exists for this stage beyond the nullifier check already done above.
    return { status: record };
  }

  if (isOprfPendingRecord(record)) {
    const header = await api.rpc.chain.getHeader();
    const currentBlock = header.number.toNumber();

    if (currentBlock > record.slaExpiresAtBlock) {
      const failed: PersistableStatus = {
        stage: 'Failed',
        failedStage: record.stage,
        reason: 'The OPRF committee did not respond within the SLA window.',
        retryable: true,
        expiresAtBlock: record.slaExpiresAtBlock,
      };
      await writeRegistrationStatus(address, failed);
      return { status: failed };
    }

    const threshold = (api.consts.identity.oprfThreshold as any).toNumber();

    // Every registration query goes to all 5 committees symmetrically (changelog
    // 073) — poll each slot in record.committeeSlots independently, rather than
    // just slot 0. Round 2 is only ever read for a slot once that slot's round 1
    // has hit threshold: round2 submissions can't exist on-chain before that
    // slot's round1 is locked, so skipping the read both bounds the number of
    // chain reads and matches on-chain reality.
    const slotProgress: Record<number, CommitteeSlotProgress> = {};
    for (const slot of record.committeeSlots) {
      const round1 = await api.query.identity.oprfRound1Commitments(record.queryId, slot);
      const round1Count = (round1 as any).length;

      let round2Count = 0;
      if (round1Count >= threshold) {
        const round2 = await api.query.identity.oprfRound2Responses(record.queryId, slot);
        round2Count = (round2 as any).length;
      }

      slotProgress[slot] = { slot, round1Count, round2Count };
    }

    const allRound1Locked = record.committeeSlots.every((s) => slotProgress[s].round1Count >= threshold);
    const allRound2Complete =
      allRound1Locked && record.committeeSlots.every((s) => slotProgress[s].round2Count >= threshold);

    let next: PersistableStatus;
    if (allRound2Complete) {
      next = { stage: 'ProofCombining', queryId: record.queryId, committeeSlots: record.committeeSlots };
    } else if (allRound1Locked) {
      next = {
        stage: 'AwaitingCommitteeRound2',
        queryId: record.queryId,
        committeeSlots: record.committeeSlots,
        submittedAtBlock: record.submittedAtBlock,
        slaExpiresAtBlock: record.slaExpiresAtBlock,
      };
    } else {
      next = {
        stage: 'AwaitingCommitteeRound1',
        queryId: record.queryId,
        committeeSlots: record.committeeSlots,
        submittedAtBlock: record.submittedAtBlock,
        slaExpiresAtBlock: record.slaExpiresAtBlock,
      };
    }
    if (next.stage !== record.stage) await writeRegistrationStatus(address, next);

    return {
      status: next,
      oprfProgress: { threshold, slots: record.committeeSlots.map((s) => slotProgress[s]) },
    };
  }

  // Exhaustive per PersistableStatus — TypeScript would flag a missing branch above.
  return { status: record };
}
