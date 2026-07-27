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

export async function submitProposal(
  pair: KeyringPair,
  durationBlocks: number,
): Promise<number> {
  const api = await getApi();
  let proposalId = -1;
  await new Promise<void>((resolve, reject) => {
    api.tx.voting
      .submitProposal(durationBlocks)
      .signAndSend(pair, ({ status, events, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) {
          for (const { event } of events) {
            if (api.events.voting.ProposalCreated.is(event)) {
              proposalId = (event.data as any).id.toNumber();
            }
          }
          resolve();
        }
      })
      .catch(reject);
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
  return new Promise((resolve, reject) => {
    api.tx.voting
      .commitVote(proposalId, commitment)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) resolve();
      })
      .catch(reject);
  });
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
  return new Promise((resolve, reject) => {
    api.tx.voting
      .delegateVote(delegate, topicId, durationBlocks)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) resolve();
      })
      .catch(reject);
  });
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
  return new Promise((resolve, reject) => {
    api.tx.voting
      .voteReferendum(referendumId, inFavor)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) resolve();
      })
      .catch(reject);
  });
}

export async function revokeDelegation(
  pair: KeyringPair,
  topicId: number,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .revokeDelegation(topicId)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) resolve();
      })
      .catch(reject);
  });
}

export async function claimFiscalYearTokens(
  pair: KeyringPair,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .claimFiscalYearTokens()
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) resolve();
      })
      .catch(reject);
  });
}

export async function allocateBudget(
  pair: KeyringPair,
  categoryId: number,
  voteCount: number,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .allocateBudget(categoryId, voteCount)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) resolve();
      })
      .catch(reject);
  });
}
