/**
 * Local, in-memory record of which backing-nullifier "slot" this citizen has assigned to which
 * delegate — client-side bookkeeping the unlinkable backing design (commit 786b792,
 * `pallet-elections`' `verify_backing_proof`) deliberately pushes onto the wallet rather than
 * keeping on-chain.
 *
 * # Why this has to live on the client at all
 *
 * A `backing_nullifier` depends only on `(backing_root_secret, slot_index)` — never on which
 * delegate it targets (see `circuits/oprf-identity-anchor/backing-nullifier/src/main.nr`'s
 * `derive_backing_nullifier` doc comment for why: so a citizen can retarget the same slot to a
 * different delegate over time without changing its nullifier). That is exactly what makes
 * backing unlinkable: nothing on-chain can answer "is citizen X backing delegate Y" for any
 * observer — including X's own future session on a different device or app install, which has
 * no way to recover which of its `MaxBackingsPerCitizen` slots (if any) was ever pointed at a
 * given delegate without asking the wallet that made that choice. This is the intended privacy
 * property, not a gap to close — see `DelegateDetailScreen.tsx`'s "Your Backing" section for
 * what it means for the UI, and this pallet's own module doc comment in
 * `pallets/pallet-elections/src/lib.rs` for the on-chain side of the same story.
 *
 * # What this module is, and is not
 *
 * A session-only convenience cache, in the same spirit as `citizenState.ts`'s delegation
 * mirror — NOT durable, secret-bearing storage. It holds only `slotIndex` (a small integer with
 * no confidentiality requirement of its own), never `backingRootSecret` itself. Real durable
 * persistence of the slot assignment (so "which delegates am I backing" survives an app
 * reinstall) is meaningful only once there is a real `backingRootSecret` to persist it
 * alongside — which needs the same OPRF-committee round-trip `oprfCombine.ts`'s
 * `combineCommitteeSlotResponses` documents as unimplemented (see `zkProving.ts`'s
 * `proveBackingNullifier` doc comment). Until then, losing this cache (app restart) just means
 * re-deriving which slots are free is no longer possible from the client alone — a real, open
 * gap, not something to paper over with a false sense of persistence.
 */

const _backingSlots = new Map<string, number>();

/** The slot index this citizen has assigned to `delegate`, or `null` if none. */
export function getBackingSlotFor(delegate: string): number | null {
  return _backingSlots.get(delegate) ?? null;
}

/**
 * Whether this session's local record shows the citizen currently backing `delegate`. This is
 * the *only* way to answer that question — see this module's doc comment for why no chain query
 * can. `DelegateDetailScreen.tsx`'s "Your Backing" toggle reads this, not a chain query.
 */
export function isBackingDelegateLocally(delegate: string): boolean {
  return _backingSlots.has(delegate);
}

/** Records that `delegate` now occupies `slotIndex`. Called after a successful `back_delegate`. */
export function recordBacking(delegate: string, slotIndex: number): void {
  _backingSlots.set(delegate, slotIndex);
}

/** Frees `delegate`'s slot. Called after a successful `remove_backing`. */
export function clearBacking(delegate: string): void {
  _backingSlots.delete(delegate);
}

/** Every delegate this session currently records a backing slot for, as `(delegate, slotIndex)` pairs. */
export function getAllBackingSlots(): [string, number][] {
  return Array.from(_backingSlots.entries());
}

/**
 * The lowest slot index in `0..maxBackingsPerCitizen` this session hasn't already assigned to
 * some other delegate — the slot a fresh `back_delegate` proof for a new delegate should use.
 * Throws once every slot is in use, mirroring the circuit's own `assert_lt(slot_index,
 * max_backings_per_citizen)` cap (`Error::MaxBackingsMismatch` on-chain) — there is no way to
 * back a `maxBackingsPerCitizen + 1`th delegate simultaneously; an existing one must be
 * unbacked (`remove_backing`) first to free its slot.
 */
export function nextFreeBackingSlot(maxBackingsPerCitizen: number): number {
  const used = new Set(_backingSlots.values());
  for (let slot = 0; slot < maxBackingsPerCitizen; slot++) {
    if (!used.has(slot)) return slot;
  }
  throw new Error(
    `nextFreeBackingSlot: all ${maxBackingsPerCitizen} backing slots are already assigned in this session — ` +
      'remove an existing backing first to free one',
  );
}

/** Test-only escape hatch. */
export function _resetBackingStateForTests(): void {
  _backingSlots.clear();
}
