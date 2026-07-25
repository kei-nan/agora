// In-memory citizen status for the session.
// Real implementation will read from the chain on focus via pallet-identity.
let _registered = false;
let _passportName: string | null = null;

export function setRegistered(value: boolean) { _registered = value; }
export function getRegistered() { return _registered; }

// Full legal name as read from the passport NFC chip during citizen registration.
// In the real system this comes from the MRZ data verified by the Rarimo ZK circuit.
// Null until the citizen completes registration.
export function setPassportName(name: string) { _passportName = name; }
export function getPassportName(): string | null { return _passportName; }

// In-memory delegation map: topicId → { delegate address, expiry timestamp (unix ms) }
export interface DelegationEntry {
  delegate: string;
  expiresAt: number; // unix ms
}

const _delegations = new Map<number, DelegationEntry>();

export function setDelegation(topicId: number, delegate: string, expiresAt: number) {
  _delegations.set(topicId, { delegate, expiresAt });
}
export function removeDelegation(topicId: number) { _delegations.delete(topicId); }
export function getDelegationFor(topicId: number): DelegationEntry | null { return _delegations.get(topicId) ?? null; }
export function getAllDelegations(): Map<number, DelegationEntry> { return new Map(_delegations); }

// In-memory backing set: addresses the citizen is currently backing
const _backing = new Set<string>();

export const MAX_BACKINGS_PER_CITIZEN = 5;

export function addBacking(delegate: string) {
  if (_backing.size >= MAX_BACKINGS_PER_CITIZEN) {
    throw new Error(`You can only back up to ${MAX_BACKINGS_PER_CITIZEN} delegates at a time.`);
  }
  _backing.add(delegate);
}
export function removeBacking(delegate: string) { _backing.delete(delegate); }
export function isBacking(delegate: string): boolean { return _backing.has(delegate); }
export function backingCount(): number { return _backing.size; }

// Mock delegate registry (replaces chain query until @polkadot/api is wired up)
export interface DelegateProfile {
  address: string;
  displayName: string;
  status: 'Active' | 'Pending' | 'OnBreak';
  backingCount: number;
  consecutiveTerms: number;
  maxConsecutiveTerms: number;
  termProgressPct: number;   // 0–100, only meaningful when Active
  warningEmitted: boolean;
  breakEndsInBlocks?: number;
}

const _registry: DelegateProfile[] = [
  {
    address: '5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty',
    displayName: 'Alice Johnson',
    status: 'Active',
    backingCount: 128,
    consecutiveTerms: 2,
    maxConsecutiveTerms: 3,
    termProgressPct: 78,
    warningEmitted: true,
  },
  {
    address: '5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y',
    displayName: 'Bob Smith',
    status: 'Active',
    backingCount: 84,
    consecutiveTerms: 1,
    maxConsecutiveTerms: 3,
    termProgressPct: 34,
    warningEmitted: false,
  },
  {
    address: '5DAAnrj7VHTznn2AWBemMuyBwZWs6FNFjdyVXUeYum3PTXFy',
    displayName: 'Carol Lee',
    status: 'Pending',
    backingCount: 12,
    consecutiveTerms: 0,
    maxConsecutiveTerms: 3,
    termProgressPct: 0,
    warningEmitted: false,
  },
  {
    address: '5HGjWAeFDfFCWPsjFQdVV2Msvz2XtMktvgocEZ5GPjGNRdnW',
    displayName: 'David Park',
    status: 'OnBreak',
    backingCount: 201,
    consecutiveTerms: 3,
    maxConsecutiveTerms: 3,
    termProgressPct: 100,
    warningEmitted: false,
    breakEndsInBlocks: 432000,
  },
];

export function getDelegateRegistry(): DelegateProfile[] { return [..._registry]; }
export function getDelegateProfile(address: string): DelegateProfile | null {
  return _registry.find(d => d.address === address) ?? null;
}
export function updateDelegateBackingCount(address: string, delta: number) {
  const d = _registry.find(r => r.address === address);
  if (d) d.backingCount = Math.max(0, d.backingCount + delta);
}
