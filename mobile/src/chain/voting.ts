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

export async function commitVote(
  pair: KeyringPair,
  proposalId: number,
  nullifier: Uint8Array,
  commitment: Uint8Array,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .commitVote(proposalId, nullifier, commitment)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) { reject(new Error(dispatchError.toString())); return; }
        if (status.isFinalized) resolve();
      })
      .catch(reject);
  });
}

export async function delegateVote(
  pair: KeyringPair,
  delegate: string,
  topicId: number,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .delegateVote(delegate, topicId)
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
