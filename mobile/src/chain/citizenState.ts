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

// Local mirror of a citizen's pallet-voting topic delegations, keyed by
// topicId. The chain (pallet-voting's Delegations storage map) is the real
// source of truth once governance.ts's delegateVote/revokeDelegation submit
// real extrinsics — this cache exists only so DelegateScreen's "My
// Delegations" section (which reads it synchronously, without awaiting a
// chain query) can reflect a just-submitted action immediately.
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
