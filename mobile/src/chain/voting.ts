/**
 * Voting pallet integration.
 *
 * Vote commitments are MACI-encrypted on-device before being submitted.
 * Actual tally + ZK proof is produced off-chain by the MACI coordinator.
 *
 * TODO: integrate @maci-protocol/domainobjs for message encryption.
 */
import { KeyringPair } from '@polkadot/keyring/types';
import { getApi } from './api';
import { submitExtrinsic } from './submitExtrinsic';

/**
 * Mirrors `pallet_voting::ReferendumTier` (`pallets/pallet-voting/src/lib.rs`) — a
 * fieldless enum, so `@polkadot/api` accepts the bare variant name as a string.
 */
export type ReferendumTier = 'Ordinary' | 'Constitutional' | 'Foundational';

/**
 * `submit_proposal(origin, topic_hash, tier, duration_blocks)` (call index 0) takes
 * three arguments beyond origin — a previous version of this wrapper only took
 * `durationBlocks` and dropped `topic_hash`/`tier` entirely, so every call failed at
 * the chain (wrong argument count against the metadata) rather than at compile time,
 * since `api.tx.voting.submitProposal` isn't checked by TypeScript here.
 */
export async function submitProposal(
  pair: KeyringPair,
  topicHash: Uint8Array,
  tier: ReferendumTier,
  durationBlocks: number,
): Promise<number> {
  const api = await getApi();
  let proposalId = -1;
  await submitExtrinsic(api.tx.voting.submitProposal(topicHash, tier, durationBlocks), pair, {
    onEvents: (events) => {
      for (const { event } of events) {
        if (api.events.voting.ProposalCreated.is(event)) {
          proposalId = (event.data as any).id.toNumber();
        }
      }
    },
  });
  return proposalId;
}

/**
 * Commit an encrypted MACI vote for a 1p1v proposal.
 *
 * Note: the on-chain call is `commit_vote(proposal_id, commitment)` — the
 * nullifier is derived server-side from the caller's registered identity
 * (pallet-voting's NullifierProvider), not supplied by the caller. An earlier
 * version of this wrapper took a `nullifier` argument and passed it through
 * as a third extrinsic argument, which didn't match the pallet's call
 * signature at all; that argument has been removed.
 */
export async function commitVote(
  pair: KeyringPair,
  proposalId: number,
  commitment: Uint8Array,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.voting.commitVote(proposalId, commitment), pair);
}

/**
 * Delegate a citizen's referendum vote on `topicId` to `delegate` for
 * `durationBlocks` blocks. This is pallet-voting's liquid-democracy
 * delegation (distinct from pallet-elections' delegate-registry
 * backing/registration — see governance.ts for that).
 *
 * `durationBlocks` is required by the on-chain call
 * (`delegate_vote(delegate, topic_id, duration_blocks)`, checked against
 * MinDelegationDurationBlocks/MaxDelegationDurationBlocks) — an earlier
 * version of this wrapper omitted it entirely.
 */
export async function delegateVote(
  pair: KeyringPair,
  delegate: string,
  topicId: number,
  durationBlocks: number,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.voting.delegateVote(delegate, topicId, durationBlocks), pair);
}

/**
 * Cast a yes/no vote on an active referendum (`vote_referendum` — distinct
 * from `commit_vote`, which is for the separate MACI 1p1v proposal flow).
 */
export async function voteReferendum(
  pair: KeyringPair,
  referendumId: number,
  inFavor: boolean,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.voting.voteReferendum(referendumId, inFavor), pair);
}

export async function revokeDelegation(
  pair: KeyringPair,
  topicId: number,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.voting.revokeDelegation(topicId), pair);
}

export async function claimFiscalYearTokens(
  pair: KeyringPair,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.voting.claimFiscalYearTokens(), pair);
}

export async function allocateBudget(
  pair: KeyringPair,
  categoryId: number,
  voteCount: number,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.voting.allocateBudget(categoryId, voteCount), pair);
}
