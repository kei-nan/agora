/**
 * Cross-pallet governance reads/writes for the mobile UI.
 *
 * Wraps pallet-voting / pallet-constitution calls that already have wrappers
 * in voting.ts / constitution.ts, and adds the storage-reading list views
 * (referenda, laws, petitions, the delegate registry) plus the
 * pallet-elections delegate-registry calls (register/back/remove-backing),
 * which didn't have a home in any of the other chain/*.ts files yet.
 *
 * Runtime pallet → @polkadot/api section names (see runtime/src/lib.rs
 * construct_runtime! for the canonical list):
 *   Voting          -> api.query.voting / api.tx.voting
 *   Constitution    -> api.query.constitution / api.tx.constitution
 *   PalletElections -> api.query.palletElections / api.tx.palletElections
 */
import { ApiPromise } from '@polkadot/api';
import { KeyringPair } from '@polkadot/keyring/types';
import { getApi } from './api';
import * as votingChain from './voting';
import * as constitutionChain from './constitution';
import { removeDelegation, setDelegation } from './citizenState';
import { clearBacking, isBackingDelegateLocally, recordBacking } from './backingState';
import { OprfCommitteeKeyHashes } from './identity';
import { submitExtrinsic } from './submitExtrinsic';

// 12s block time (see runtime/src/lib.rs MILLI_SECS_PER_BLOCK) => 7200
// blocks/day. Matches the same constant already assumed for term-limit
// display math elsewhere in the UI (DelegateScreen/DelegateDetailScreen).
export const BLOCKS_PER_DAY = 7200;

export interface Proposal {
  id: number;
  state: 'Voting' | 'Passed' | 'Failed';
  tier: 'Ordinary' | 'Constitutional' | 'Foundational';
  topicHash: string;
  votesFor: number;
  votesAgainst: number;
}

export interface Law {
  id: number;
  /**
   * pallet-constitution's Laws storage only holds (tier, status, version,
   * content_hash) — there is no on-chain title. Real content (including a
   * title) lives on IPFS at contentHash and isn't fetched yet (see
   * CLAUDE.md's "IPFS content fetching" remaining-work item). This is a
   * placeholder until that's wired in.
   */
  title: string;
  tier: 'Constitutional' | 'Ordinary';
  status: 'Active' | 'Paused' | 'Repealed';
  version: number;
  contentHash: string;
}

export interface Petition {
  id: number;
  /** Placeholder — see Law.title doc; petitions only store a topic_hash on-chain. */
  title: string;
  description: string;
  topicHash: string;
  sigCount: number;
  threshold: number;
}

export interface DelegateProfile {
  address: string;
  displayName: string;
  status: 'Active' | 'Pending' | 'OnBreak';
  backingCount: number;
  consecutiveTerms: number;
  maxConsecutiveTerms: number;
  termProgressPct: number; // 0-100, only meaningful when Active
  warningEmitted: boolean;
  breakEndsInBlocks?: number;
}

async function currentBlockNumber(api: ApiPromise): Promise<number> {
  const header = await api.rpc.chain.getHeader();
  return header.number.toNumber();
}

// ── Referenda (pallet-voting) ────────────────────────────────────────────

export async function fetchProposals(): Promise<Proposal[]> {
  const api = await getApi();
  const entries = await api.query.voting.referenda.entries();
  const proposals = await Promise.all(
    entries.map(async ([key, value]) => {
      if ((value as any).isNone) return null;
      const id = (key.args[0] as any).toNumber();
      const [, topicHash, , state, tier] = (value as any).unwrap();
      const [yes, no] = (await api.query.voting.referendumTally(id)) as any;
      const proposal: Proposal = {
        id,
        state: state.type,
        tier: tier.type,
        topicHash: topicHash.toHex(),
        votesFor: yes.toNumber(),
        votesAgainst: no.toNumber(),
      };
      return proposal;
    }),
  );
  return proposals
    .filter((p): p is Proposal => p !== null)
    .sort((a, b) => a.id - b.id);
}

export async function voteOnReferendum(
  keypair: KeyringPair,
  id: number,
  inFavor: boolean,
): Promise<void> {
  await votingChain.voteReferendum(keypair, id, inFavor);
}

/**
 * Whether `address` has already cast a vote in referendum `id` — pallet-voting's
 * `ReferendumHasVoted` (`(referendum_id, account) -> bool`, `ValueQuery`, so no
 * isSome/unwrap needed; a missing entry just reads back `false`). Used to seed
 * ProposalsScreen's already-voted guard from real chain state rather than only
 * this session's in-memory votes, since a citizen who voted in an earlier
 * session/app-install would otherwise see live Vote buttons again.
 */
export async function hasVotedOnReferendum(address: string, referendumId: number): Promise<boolean> {
  const api = await getApi();
  const voted = await api.query.voting.referendumHasVoted([referendumId, address]);
  return Boolean((voted as any).toJSON());
}

/**
 * The referendum created for a petition once it crosses its signature
 * threshold — pallet-voting's `PetitionReferendum` (`petition_id -> referendum_id`).
 * `create_referendum` is called synchronously inside the same `sign_petition`
 * extrinsic that emits `PetitionThresholdReached`
 * (pallets/pallet-constitution/src/lib.rs), so once a petition's on-chain
 * signature count has reached its threshold this mapping is already
 * populated — `null` should only happen for petitions that haven't reached
 * threshold yet (callers are expected to check that first).
 */
export async function fetchReferendumIdForPetition(petitionId: number): Promise<number | null> {
  const api = await getApi();
  const entry = await api.query.voting.petitionReferendum(petitionId);
  if ((entry as any).isNone) return null;
  return (entry as any).unwrap().toNumber();
}

// ── Laws (pallet-constitution) ───────────────────────────────────────────

export async function fetchLaws(): Promise<Law[]> {
  const api = await getApi();
  const entries = await api.query.constitution.laws.entries();
  const laws: Law[] = [];
  for (const [key, value] of entries) {
    if ((value as any).isNone) continue;
    const id = (key.args[0] as any).toNumber();
    const [tier, status, version, contentHash] = (value as any).unwrap();
    laws.push({
      id,
      title: `Law #${id}`,
      // LawTier has three variants (Ordinary/Structural/Foundational); this
      // UI only distinguishes two, so Structural and Foundational both
      // surface as "Constitutional".
      tier: tier.type === 'Ordinary' ? 'Ordinary' : 'Constitutional',
      status: status.type,
      version: version.toNumber(),
      contentHash: contentHash.toHex(),
    });
  }
  return laws.sort((a, b) => a.id - b.id);
}

// ── Petitions (pallet-constitution) ──────────────────────────────────────

export async function fetchPetitions(): Promise<Petition[]> {
  const api = await getApi();
  const threshold = (api.consts.constitution.petitionThreshold as any).toNumber();
  const entries = await api.query.constitution.petitions.entries();
  const petitions: Petition[] = [];
  for (const [key, value] of entries) {
    if ((value as any).isNone) continue;
    const id = (key.args[0] as any).toNumber();
    const [, topicHash, sigCount] = (value as any).unwrap();
    petitions.push({
      id,
      title: `Petition #${id}`,
      description: '',
      topicHash: topicHash.toHex(),
      sigCount: sigCount.toNumber(),
      threshold,
    });
  }
  return petitions.sort((a, b) => a.id - b.id);
}

export async function signPetition(keypair: KeyringPair, petitionId: number): Promise<void> {
  await constitutionChain.signPetition(keypair, petitionId);
}

// ── Topic delegation (pallet-voting liquid democracy) ────────────────────

export async function getDelegation(address: string, topicId: number): Promise<string | null> {
  const api = await getApi();
  const record = await api.query.voting.delegations([address, topicId]);
  if ((record as any).isNone) return null;
  return (record as any).unwrap().delegate.toString();
}

export async function delegateVote(
  keypair: KeyringPair,
  delegate: string,
  topicId: number,
  durationDays: number,
): Promise<void> {
  const durationBlocks = Math.max(1, Math.round(durationDays * BLOCKS_PER_DAY));
  await votingChain.delegateVote(keypair, delegate, topicId, durationBlocks);
  // Mirror into the local cache so DelegateScreen's synchronous "My
  // Delegations" list (reads citizenState directly, not the chain) reflects
  // this without waiting for a refetch. The chain is the real source of
  // truth; this is a UI-convenience mirror only.
  setDelegation(topicId, delegate, Date.now() + durationDays * 86_400_000);
}

export async function revokeDelegation(keypair: KeyringPair, topicId: number): Promise<void> {
  await votingChain.revokeDelegation(keypair, topicId);
  removeDelegation(topicId);
}

// ── Delegate registry (pallet-elections) ─────────────────────────────────

function decodeDelegateInfo(
  address: string,
  info: any,
  backingCount: number,
  maxConsecutiveTerms: number,
  termLengthBlocks: number,
  now: number,
): DelegateProfile {
  const status: DelegateProfile['status'] = info.status.type;
  const consecutiveTerms = info.consecutiveTerms.toNumber();
  const warningEmitted = Boolean(info.warningEmitted.toJSON());

  let termProgressPct = 0;
  if (status === 'Active' && info.termStartBlock.isSome && termLengthBlocks > 0) {
    const start = info.termStartBlock.unwrap().toNumber();
    termProgressPct = Math.max(0, Math.min(100, Math.round(((now - start) / termLengthBlocks) * 100)));
  }

  let breakEndsInBlocks: number | undefined;
  if (status === 'OnBreak' && info.breakUntilBlock.isSome) {
    breakEndsInBlocks = Math.max(0, info.breakUntilBlock.unwrap().toNumber() - now);
  }

  return {
    address,
    displayName: info.displayName.toUtf8(),
    status,
    backingCount,
    consecutiveTerms,
    maxConsecutiveTerms,
    termProgressPct,
    warningEmitted,
    breakEndsInBlocks,
  };
}

export async function fetchDelegateRegistry(): Promise<DelegateProfile[]> {
  const api = await getApi();
  const [entries, maxTermsRaw, termLengthRaw, now] = await Promise.all([
    api.query.palletElections.delegates.entries(),
    api.query.palletElections.maxConsecutiveTerms(),
    api.query.palletElections.termLengthBlocks(),
    currentBlockNumber(api),
  ]);
  const maxConsecutiveTerms = (maxTermsRaw as any).toNumber();
  const termLengthBlocks = (termLengthRaw as any).toNumber();

  const profiles = await Promise.all(
    entries.map(async ([key, value]) => {
      if ((value as any).isNone) return null;
      const address = (key.args[0] as any).toString();
      const backing = await api.query.palletElections.backingCount(address);
      return decodeDelegateInfo(
        address,
        (value as any).unwrap(),
        (backing as any).toNumber(),
        maxConsecutiveTerms,
        termLengthBlocks,
        now,
      );
    }),
  );
  return profiles.filter((p): p is DelegateProfile => p !== null);
}

export async function fetchDelegateProfile(address: string): Promise<DelegateProfile | null> {
  const api = await getApi();
  const [infoOpt, backing, maxTermsRaw, termLengthRaw, now] = await Promise.all([
    api.query.palletElections.delegates(address),
    api.query.palletElections.backingCount(address),
    api.query.palletElections.maxConsecutiveTerms(),
    api.query.palletElections.termLengthBlocks(),
    currentBlockNumber(api),
  ]);
  if ((infoOpt as any).isNone) return null;
  return decodeDelegateInfo(
    address,
    (infoOpt as any).unwrap(),
    (backing as any).toNumber(),
    (maxTermsRaw as any).toNumber(),
    (termLengthRaw as any).toNumber(),
    now,
  );
}

/**
 * A proved `backing-nullifier` proof, ready for `back_delegate`/`remove_backing` — the exact
 * shape `zkProving.ts`'s `proveBackingNullifier` returns. `remove_backing` requires *the same*
 * proof `back_delegate` originally submitted (same `backingRootSecret`/`slotIndex`, hence the
 * same `backing_nullifier`) — see `pallet-elections`' `remove_backing` doc comment for why a
 * fresh proof over a different slot cannot unback an existing one.
 */
export interface BackingProofSubmission {
  zkProof: Uint8Array;
  publicInputs: readonly [Uint8Array, Uint8Array, Uint8Array, Uint8Array];
}

/**
 * Backs `delegate` with a real `backing-nullifier` proof (`pallet_elections::back_delegate`,
 * call index 8, commit 786b792). `slotIndex` is whichever of the citizen's
 * `0..maxBackingsPerCitizen` slots `proof` was built for (see `backingState.ts`'s
 * `nextFreeBackingSlot`) — recorded locally on success via `backingState.ts`'s
 * `recordBacking`, since nothing on-chain can answer "which slot did I use for this delegate"
 * back to this wallet later (see that module's doc comment).
 */
export async function backDelegate(
  keypair: KeyringPair,
  delegate: string,
  proof: BackingProofSubmission,
  slotIndex: number,
): Promise<void> {
  const api = await getApi();
  await submitExtrinsic(
    api.tx.palletElections.backDelegate(delegate, proof.zkProof, proof.publicInputs),
    keypair,
  );
  recordBacking(delegate, slotIndex);
}

/**
 * Removes an existing backing of `delegate` (`pallet_elections::remove_backing`, call index 9).
 * `proof` must be *the same* proof (same `backingRootSecret`/`slotIndex`, hence the same
 * `backing_nullifier`) that originally backed `delegate` — see `BackingProofSubmission`'s doc
 * comment.
 */
export async function removeBackingFromDelegate(
  keypair: KeyringPair,
  delegate: string,
  proof: BackingProofSubmission,
): Promise<void> {
  const api = await getApi();
  await submitExtrinsic(
    api.tx.palletElections.removeBacking(delegate, proof.zkProof, proof.publicInputs),
    keypair,
  );
  clearBacking(delegate);
}

/**
 * Whether this session's wallet currently records backing `delegate`.
 *
 * Unlike before commit 786b792, this is **not** a chain query — `pallet-elections`' old
 * plaintext `BackingOf(citizen, delegate)` map is gone, replaced by a nullifier that reveals
 * nothing about which delegate it targets to an outside observer (that is the entire point of
 * the redesign — see `backingState.ts`'s doc comment). A citizen's own wallet is now the only
 * thing that can answer this question, and only for the session it made the backing in (no
 * durable persistence yet — same doc comment). This is a synchronous local lookup, kept `async`
 * only so existing `await isBackingDelegate(...)` call sites did not need to change.
 */
export async function isBackingDelegate(delegate: string): Promise<boolean> {
  return isBackingDelegateLocally(delegate);
}

/** `pallet-elections`' `DelegatePersonaIdOf(delegate)` — the delegate's stable persona id, needed as a `backing-nullifier` proof's `delegate_persona_id` public input. `null` if `delegate` has no registered persona (not actually a delegate). */
export async function fetchDelegatePersonaId(delegate: string): Promise<Uint8Array | null> {
  const api = await getApi();
  const entry = await api.query.palletElections.delegatePersonaIdOf(delegate);
  if ((entry as any).isNone) return null;
  return (entry as any).unwrap().toU8a() as Uint8Array;
}

/** `pallet-elections`' live `MaxBackingsPerCitizen` governance parameter — needed as a `backing-nullifier` proof's `max_backings_per_citizen` public input, and by `backingState.ts`'s `nextFreeBackingSlot`. */
export async function fetchMaxBackingsPerCitizen(): Promise<number> {
  const api = await getApi();
  const value = await api.query.palletElections.maxBackingsPerCitizen();
  return (value as any).toNumber();
}

/**
 * Parameters for `register_as_delegate` (`pallet_elections`, call index 7, commits 2e07f68/
 * 786b792): a real delegate-persona ZK proof (`zkProving.ts`'s `proveDelegatePersona`) plus the
 * OPRF material it was derived under. Must be submitted signed by the delegate-persona account
 * itself (`persona_account == ensure_signed(origin)`, `Error::PersonaAccountMismatch`
 * otherwise) — see `registerAsDelegate`'s `personaKeypair` parameter, sourced from
 * `keystoreWallet.ts`'s `getOrCreateDelegatePersonaKeypair`, never the citizen's main wallet
 * keypair.
 */
export interface RegisterAsDelegateParams {
  delegatePersonaId: Uint8Array;
  zkProof: Uint8Array;
  publicInputs: readonly Uint8Array[];
  schemeVersion: number;
  oprfPkHashes: OprfCommitteeKeyHashes;
  displayName: string;
  /**
   * A real 32-byte IPFS content hash. No IPFS upload client exists yet in this app (see
   * CLAUDE.md's P1 remaining-work item / constitution.ts's TODO for the same gap on
   * proposeAmendment) — a caller with nothing real to upload yet may pass a locally-hashed
   * placeholder (e.g. `sha256AsU8a(stringToU8a(bio))`), but that is NOT a real IPFS content
   * hash and nothing is actually pinned anywhere; this function does not fabricate one itself.
   */
  profileIpfsHash: Uint8Array;
}

/**
 * Registers `personaKeypair`'s account as a delegate under a fresh, ZK-derived persona.
 *
 * `personaKeypair` — NOT the citizen's regular signing keypair — is both the transaction's
 * signer and the `persona_account` the proof binds (see `RegisterAsDelegateParams`'s doc
 * comment); this is what makes the resulting delegate identity genuinely separate from the
 * citizen's own on-chain activity, unlike the pre-786b792 scheme where delegate registration
 * reused the citizen's own account outright.
 */
export async function registerAsDelegate(
  personaKeypair: KeyringPair,
  params: RegisterAsDelegateParams,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(
    api.tx.palletElections.registerAsDelegate(
      personaKeypair.address,
      params.delegatePersonaId,
      params.zkProof,
      params.publicInputs,
      params.schemeVersion,
      params.oprfPkHashes,
      params.displayName,
      params.profileIpfsHash,
    ),
    personaKeypair,
  );
}
