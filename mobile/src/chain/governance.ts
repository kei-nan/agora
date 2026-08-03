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
import { stringToU8a } from '@polkadot/util';
import { sha256AsU8a } from '@polkadot/util-crypto';
import { getApi } from './api';
import * as votingChain from './voting';
import * as constitutionChain from './constitution';
import { removeDelegation, setDelegation } from './citizenState';
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

export async function backDelegate(keypair: KeyringPair, address: string): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.palletElections.backDelegate(address), keypair);
}

export async function removeBackingFromDelegate(keypair: KeyringPair, address: string): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.palletElections.removeBacking(address), keypair);
}

/**
 * `address` is the checking citizen's own address, not the delegate's — pass
 * ''/undefined-ish values are treated as "not signed in yet" and short-circuit
 * to false rather than issuing an invalid storage query.
 */
export async function isBackingDelegate(address: string, delegate: string): Promise<boolean> {
  if (!address) return false;
  const api = await getApi();
  const entry = await api.query.palletElections.backingOf(address, delegate);
  return (entry as any).isSome;
}

export async function registerAsDelegate(
  keypair: KeyringPair,
  displayName: string,
  bio: string,
): Promise<void> {
  const api = await getApi();
  // register_as_delegate requires a real 32-byte profile_ipfs_hash. No IPFS
  // upload client exists yet in this app (see CLAUDE.md's P1 remaining-work
  // item / constitution.ts's TODO for the same gap on proposeAmendment). This
  // hashes the bio text locally as a placeholder purely so the call is
  // well-typed — it is NOT a real IPFS content hash and nothing is actually
  // pinned anywhere.
  const profileHash = sha256AsU8a(stringToU8a(bio || displayName));
  return submitExtrinsic(api.tx.palletElections.registerAsDelegate(displayName, profileHash), keypair);
}
