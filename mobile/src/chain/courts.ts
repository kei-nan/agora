/**
 * Courts pallet integration.
 *
 * Level 0 rulings are issued by an AI judge (hash stored on IPFS on-chain).
 * Citizens may appeal to a random jury (Level 1) or a constitutional panel (Level 2).
 *
 * TODO: wire jury-selection RNG to an on-chain VRF once available.
 */
import { KeyringPair } from '@polkadot/keyring/types';
import { getApi } from './api';

export type CaseSubject =
  | { General: null }
  | { LawChallenge: { law_id: number } }
  | { TreasuryDispute: { department_id: number } };

export async function fileCase(
  pair: KeyringPair,
  subject: CaseSubject,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.courts
      .fileCase(subject)
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

export async function appealRuling(
  pair: KeyringPair,
  caseId: number,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.courts
      .appealRuling(caseId)
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

export async function castJuryVote(
  pair: KeyringPair,
  caseId: number,
  verdict: boolean,
): Promise<void> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.courts
      .castJuryVote(caseId, verdict)
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
