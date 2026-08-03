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
import { submitExtrinsic } from './submitExtrinsic';

export type CaseSubject =
  | { General: null }
  | { LawChallenge: { law_id: number } }
  | { TreasuryDispute: { department_id: number } }
  | { CitizenConduct: { nullifier: Uint8Array; suspension_blocks: number | null } };

/** Mirrors `pallet_courts::Verdict` (`pallets/pallet-courts/src/lib.rs`) — a fieldless enum. */
export type Verdict = 'Upheld' | 'Overturned';

export async function fileCase(
  pair: KeyringPair,
  subject: CaseSubject,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.courts.fileCase(subject), pair);
}

export async function appealRuling(
  pair: KeyringPair,
  caseId: number,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.courts.appealRuling(caseId), pair);
}

/**
 * `cast_jury_vote(origin, case_id, verdict)` takes a `Verdict` enum
 * (`Upheld`/`Overturned`), not a bare boolean — a previous version of this wrapper
 * took `verdict: boolean` and passed it straight through, which either fails to
 * encode or silently coerces to the wrong variant, potentially recording a jury
 * vote as the opposite of what the caller intended.
 */
export async function castJuryVote(
  pair: KeyringPair,
  caseId: number,
  verdict: Verdict,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.courts.castJuryVote(caseId, verdict), pair);
}
