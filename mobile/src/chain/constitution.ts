/**
 * Constitution pallet integration.
 *
 * Laws are stored on IPFS with their content hash committed on-chain.
 * Petitions gather citizen signatures until a threshold triggers a referendum.
 *
 * TODO: integrate IPFS client for content upload before calling enactLaw.
 */
import { KeyringPair } from '@polkadot/keyring/types';
import { getApi } from './api';

export async function submitPetition(
  pair: KeyringPair,
  topicHash: Uint8Array,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.constitution
      .submitPetition(topicHash)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) {
          reject(new Error(dispatchError.toString()));
        } else if (status.isFinalized) {
          resolve();
        }
      })
      .catch(reject);
  });
}

export async function signPetition(
  pair: KeyringPair,
  petitionId: number,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.constitution
      .signPetition(petitionId)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) {
          reject(new Error(dispatchError.toString()));
        } else if (status.isFinalized) {
          resolve();
        }
      })
      .catch(reject);
  });
}

export async function proposeAmendment(
  pair: KeyringPair,
  lawId: number,
  proposedHash: Uint8Array,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.constitution
      .proposeAmendment(lawId, proposedHash)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) {
          reject(new Error(dispatchError.toString()));
        } else if (status.isFinalized) {
          resolve();
        }
      })
      .catch(reject);
  });
}
