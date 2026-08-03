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
import { submitExtrinsic } from './submitExtrinsic';

export async function submitPetition(
  pair: KeyringPair,
  topicHash: Uint8Array,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.constitution.submitPetition(topicHash), pair);
}

export async function signPetition(
  pair: KeyringPair,
  petitionId: number,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.constitution.signPetition(petitionId), pair);
}

/**
 * `propose_amendment` requires `T::LegislatureOrigin::ensure_origin`
 * (`pallets/pallet-constitution/src/lib.rs`) — a collective/legislature
 * origin, not a bare signed citizen origin. Calling this with an ordinary
 * citizen `KeyringPair` will dispatch successfully off-chain but always
 * fail on-chain with `BadOrigin`; this wrapper doesn't check that itself
 * (the pallet is the source of truth for who counts as "the legislature"),
 * it's just worth knowing before spending time debugging a well-formed call
 * that can never succeed from a plain citizen key.
 */
export async function proposeAmendment(
  pair: KeyringPair,
  lawId: number,
  proposedHash: Uint8Array,
): Promise<void> {
  const api = await getApi();
  return submitExtrinsic(api.tx.constitution.proposeAmendment(lawId, proposedHash), pair);
}
