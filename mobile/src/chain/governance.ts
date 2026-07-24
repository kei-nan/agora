/**
 * Chain reads for governance: proposals (referenda), laws, petitions, delegation.
 */
import { getApi } from './api';

export interface Proposal {
  id: number;
  topicHash: string;
  endBlock: number;
  state: 'Voting' | 'Passed' | 'Rejected' | 'Enacted';
  tier: 'Ordinary' | 'Constitutional';
  votesFor: number;
  votesAgainst: number;
}

export interface Law {
  id: number;
  tier: 'Ordinary' | 'Constitutional';
  status: 'Active' | 'Paused' | 'Repealed';
  version: number;
  contentHash: string;
}

export interface Petition {
  id: number;
  proposer: string;
  topicHash: string;
  sigCount: number;
  submittedAt: number;
}

export async function fetchProposals(): Promise<Proposal[]> {
  const api = await getApi();
  const entries = await api.query.voting.referenda.entries();
  const results: Proposal[] = [];

  for (const [key, value] of entries) {
    const id = (key.args[0] as any).toNumber();
    const tuple = (value as any).unwrapOrDefault();
    if (!tuple || !tuple[0]) continue;

    const [petitionId, topicHash, endBlock, stateEnum, tierEnum] = tuple;

    const stateMap: Record<number, Proposal['state']> = {
      0: 'Voting', 1: 'Passed', 2: 'Rejected', 3: 'Enacted',
    };
    const tierMap: Record<number, Proposal['tier']> = {
      0: 'Ordinary', 1: 'Constitutional',
    };

    const tallyOpt = await api.query.voting.referendumTally(id);
    const tally = (tallyOpt as any).unwrapOrDefault();
    const yes = tally?.[0]?.toNumber() ?? 0;
    const no = tally?.[1]?.toNumber() ?? 0;

    results.push({
      id,
      topicHash: topicHash.toHex ? topicHash.toHex() : topicHash.toString(),
      endBlock: endBlock.toNumber ? endBlock.toNumber() : Number(endBlock),
      state: stateMap[stateEnum?.toNumber?.() ?? 0] ?? 'Voting',
      tier: tierMap[tierEnum?.toNumber?.() ?? 0] ?? 'Ordinary',
      votesFor: yes,
      votesAgainst: no,
    });
  }

  return results.sort((a, b) => b.id - a.id);
}

export async function fetchLaws(): Promise<Law[]> {
  const api = await getApi();
  const entries = await api.query.constitution.laws.entries();
  const results: Law[] = [];

  for (const [key, value] of entries) {
    const id = (key.args[0] as any).toNumber();
    const tuple = (value as any).unwrapOrDefault();
    if (!tuple || !tuple[0]) continue;

    const [tierEnum, statusEnum, version, contentHash] = tuple;
    const tierMap: Record<number, Law['tier']> = { 0: 'Ordinary', 1: 'Constitutional' };
    const statusMap: Record<number, Law['status']> = { 0: 'Active', 1: 'Paused', 2: 'Repealed' };

    results.push({
      id,
      tier: tierMap[tierEnum?.toNumber?.() ?? 0] ?? 'Ordinary',
      status: statusMap[statusEnum?.toNumber?.() ?? 0] ?? 'Active',
      version: version?.toNumber?.() ?? 1,
      contentHash: contentHash?.toHex?.() ?? contentHash?.toString() ?? '0x',
    });
  }

  return results.sort((a, b) => b.id - a.id);
}

export async function fetchPetitions(): Promise<Petition[]> {
  const api = await getApi();
  const entries = await api.query.constitution.petitions.entries();
  const results: Petition[] = [];

  for (const [key, value] of entries) {
    const id = (key.args[0] as any).toNumber();
    const tuple = (value as any).unwrapOrDefault();
    if (!tuple || !tuple[0]) continue;

    const [proposer, topicHash, sigCount, submittedAt] = tuple;

    results.push({
      id,
      proposer: proposer?.toString() ?? '',
      topicHash: topicHash?.toHex?.() ?? topicHash?.toString() ?? '0x',
      sigCount: sigCount?.toNumber?.() ?? 0,
      submittedAt: submittedAt?.toNumber?.() ?? 0,
    });
  }

  return results.sort((a, b) => b.id - a.id);
}

export async function voteOnReferendum(
  pair: import('@polkadot/keyring/types').KeyringPair,
  referendumId: number,
  inFavor: boolean,
): Promise<string> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .voteReferendum(referendumId, inFavor)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) reject(new Error(dispatchError.toString()));
        else if (status.isFinalized) resolve(status.asFinalized.toString());
      })
      .catch(reject);
  });
}

export async function signPetition(
  pair: import('@polkadot/keyring/types').KeyringPair,
  petitionId: number,
): Promise<string> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.constitution
      .signPetition(petitionId)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) reject(new Error(dispatchError.toString()));
        else if (status.isFinalized) resolve(status.asFinalized.toString());
      })
      .catch(reject);
  });
}

export async function getDelegation(address: string, topicId: number): Promise<string | null> {
  const api = await getApi();
  const result = await api.query.voting.delegations([address, topicId]);
  if ((result as any).isNone) return null;
  return (result as any).unwrap().toString();
}

export async function delegateVote(
  pair: import('@polkadot/keyring/types').KeyringPair,
  delegate: string,
  topicId: number,
): Promise<string> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .delegateVote(delegate, topicId)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) reject(new Error(dispatchError.toString()));
        else if (status.isFinalized) resolve(status.asFinalized.toString());
      })
      .catch(reject);
  });
}

export async function revokeDelegation(
  pair: import('@polkadot/keyring/types').KeyringPair,
  topicId: number,
): Promise<string> {
  const api = await getApi();
  return new Promise((resolve, reject) => {
    api.tx.voting
      .revokeDelegation(topicId)
      .signAndSend(pair, ({ status, dispatchError }) => {
        if (dispatchError) reject(new Error(dispatchError.toString()));
        else if (status.isFinalized) resolve(status.asFinalized.toString());
      })
      .catch(reject);
  });
}
