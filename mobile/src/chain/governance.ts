// Stub implementations — real chain calls need @polkadot/api polyfills for React Native
import {
  getDelegationFor, setDelegation, removeDelegation,
  getDelegateRegistry, getDelegateProfile, updateDelegateBackingCount,
  addBacking, removeBacking, isBacking,
  DelegateProfile,
} from './citizenState';

export type { DelegateProfile };

export interface Proposal {
  id: number;
  title: string;
  description: string;
  votesFor: number;
  votesAgainst: number;
  status: 'active' | 'passed' | 'rejected';
}

export interface Law {
  id: number;
  title: string;
  tier: 'Constitutional' | 'Ordinary';
  status: 'Active' | 'Paused' | 'Repealed';
  version: number;
  contentHash: string;
}

export interface Petition {
  id: number;
  title: string;
  description: string;
  topicHash: string;
  sigCount: number;
  threshold: number;
}

const MOCK_LAWS: Law[] = [
  {
    id: 1,
    title: 'Public Transport Accessibility Act',
    tier: 'Constitutional',
    status: 'Active',
    version: 2,
    contentHash: 'bafybeiemxf5abjwjbikoz4mc3a3dla6ual3jsgpdr4cjr3oz3evfyavhwq',
  },
  {
    id: 2,
    title: 'Clean Air Standards Regulation',
    tier: 'Ordinary',
    status: 'Active',
    version: 1,
    contentHash: 'bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi',
  },
  {
    id: 3,
    title: 'Digital Privacy & Data Sovereignty Act',
    tier: 'Constitutional',
    status: 'Paused',
    version: 1,
    contentHash: 'bafybeihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku',
  },
  {
    id: 4,
    title: 'Municipal Budget Transparency Law',
    tier: 'Ordinary',
    status: 'Active',
    version: 3,
    contentHash: 'bafybeiczsscdsbs7ffqz55asqdf3smv6klcw3gofszvwlyarci47bgf4ch',
  },
  {
    id: 5,
    title: 'Electoral Reform (Proportional Representation) Act',
    tier: 'Constitutional',
    status: 'Repealed',
    version: 1,
    contentHash: 'bafybeif2fdfijc7xhf7dvulzftedr35zpjcvudhyjrgbln77kzodkf3dca',
  },
];

const MOCK_PETITIONS: Petition[] = [
  {
    id: 1,
    title: 'Universal Basic Income Pilot Program',
    description: 'A 2-year UBI pilot of 800 AGR/month for citizens in the lowest two income quintiles, funded by a wealth tax surcharge.',
    topicHash: '0x3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c0d1e2f3a',
    sigCount: 847,
    threshold: 1000,
  },
  {
    id: 2,
    title: 'Expand Renewable Energy Subsidies',
    description: 'Triple the existing solar and wind installation grants, with priority for rural communities currently reliant on imported fossil fuels.',
    topicHash: '0x1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b',
    sigCount: 1243,
    threshold: 1000,
  },
  {
    id: 3,
    title: 'Free Public Transit for Under-18s',
    description: 'Zero-fare access to all bus and metro routes for citizens under 18, offset by a modest increase in peak-hour adult fares.',
    topicHash: '0x9c0d1e2f3a4b5c6d7e8f9a0b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f7a8b9c',
    sigCount: 234,
    threshold: 1000,
  },
  {
    id: 4,
    title: 'Open Source Government Software Mandate',
    description: 'Require all government-commissioned software to be open-source by default, with exceptions subject to legislature approval.',
    topicHash: '0x5d6e7f8a9b0c1d2e3f4a5b6c7d8e9f0a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d',
    sigCount: 678,
    threshold: 1000,
  },
];

export async function fetchProposals(): Promise<Proposal[]> { return []; }
export async function fetchLaws(): Promise<Law[]> { return MOCK_LAWS; }
export async function fetchPetitions(): Promise<Petition[]> { return MOCK_PETITIONS; }
export async function voteOnReferendum(_id: number, _vote: boolean, _keypair: any): Promise<void> {}
export async function signPetition(_id: number, _keypair: any): Promise<void> {}
export async function getDelegation(_address: string, topicId: number): Promise<string | null> {
  return getDelegationFor(topicId)?.delegate ?? null;
}
export async function delegateVote(_keypair: any, delegate: string, topicId: number, durationDays: number): Promise<void> {
  const expiresAt = Date.now() + durationDays * 86_400_000;
  setDelegation(topicId, delegate, expiresAt);
}
export async function revokeDelegation(_keypair: any, topicId: number): Promise<void> {
  removeDelegation(topicId);
}
export async function fetchDelegateRegistry(): Promise<DelegateProfile[]> {
  return getDelegateRegistry();
}
export async function fetchDelegateProfile(address: string): Promise<DelegateProfile | null> {
  return getDelegateProfile(address);
}
export async function backDelegate(_keypair: any, address: string): Promise<void> {
  addBacking(address);
  updateDelegateBackingCount(address, 1);
}
export async function removeBackingFromDelegate(_keypair: any, address: string): Promise<void> {
  removeBacking(address);
  updateDelegateBackingCount(address, -1);
}
export async function isBackingDelegate(_address: string, delegate: string): Promise<boolean> {
  return isBacking(delegate);
}
export async function registerAsDelegate(_keypair: any, displayName: string, _profileHash: string): Promise<void> {
  // Stub: in real impl posts register_as_delegate extrinsic
  void displayName;
}
