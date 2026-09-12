// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
// For more information, please refer to <http://unlicense.org>

//! Weights for pallet_accountability_council.
//!
//! NOT machine-benchmarked. `pallets/pallet-accountability-council/src/benchmarking.rs`
//! contains real, compiling `#[benchmarks]` scaffolding (mirroring
//! `pallets/pallet-emergency-council/src/benchmarking.rs`), but no `benchmark pallet` run has
//! been executed against a real built runtime to produce the numbers below — see that file's
//! doc comment for the command shape this would need.
//!
//! One known gap these manual numbers do NOT capture, same as `pallet_courts::
//! remove_oracle_member`'s identical situation (that pallet has no `weights.rs`/benchmarking of
//! its own at all): `remove_member`'s `PendingAction::<T>::translate(...)` purge walks *every*
//! in-flight pending action, a count this pallet's `Config` has no bound on (unlike
//! `MaxCouncilSize`, which bounds each individual entry's approver list). The estimate below
//! prices `remove_member`'s bounded re-resolution loop
//! (`MAX_ACTIONS_RERESOLVED_PER_REMOVAL` = 20 iterations) but, like `remove_oracle_member`'s own
//! unbenchmarked flat weight, does not add a per-entry cost for the translate scan itself — a
//! real `benchmark pallet` run would need a genuine linear component
//! (`p in 0 .. MaxPendingActions` or similar) to make that call's weight actually DoS-resistant
//! against an unbounded number of pending actions at removal time.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for pallet_accountability_council.
pub trait WeightInfo {
	fn add_member() -> Weight;
	fn remove_member() -> Weight;
	fn close_bootstrap() -> Weight;
	fn propose_action() -> Weight;
	fn approve_action() -> Weight;
	fn clear_stale_action() -> Weight;
}

/// Weights for pallet_accountability_council.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// Worst case (post-bootstrap): `Bootstrapped` + `Members` (origin check) +
	/// `ApprovedAction` reads/remove, the legislature/executive overlap checks (2 cross-pallet
	/// reads), and the `Members` read+write to insert the new member.
	fn add_member() -> Weight {
		Weight::from_parts(15_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(7_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// Worst case (post-bootstrap): same origin-check cost as `add_member` minus the overlap
	/// checks, plus the `Members` read+write to remove the member, plus up to
	/// `MAX_ACTIONS_RERESOLVED_PER_REMOVAL` (20) re-resolutions of in-flight `PendingAction`s,
	/// each costed like `approve_action`'s own `try_resolve_action` path (2 reads + 2 writes).
	/// Does NOT price the unbounded `PendingAction::translate` purge scan itself — see the
	/// module doc comment.
	fn remove_member() -> Weight {
		Weight::from_parts(20_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(4_u64.saturating_add(20_u64.saturating_mul(2))))
			.saturating_add(T::DbWeight::get().writes(2_u64.saturating_add(20_u64.saturating_mul(2))))
	}
	/// 2 reads (`Bootstrapped`, `Members`) + 1 write (`Bootstrapped`).
	fn close_bootstrap() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// Reads `Members` (membership check), `PendingAction` and `ApprovedAction` (duplicate
	/// checks), then `try_resolve_action`'s own `PendingAction` + `Members` reads. Writes the
	/// new `PendingAction` entry, plus on the (worst-case, immediately-resolved) supermajority
	/// path: `PendingAction` remove + `ApprovedAction` insert.
	fn propose_action() -> Weight {
		Weight::from_parts(14_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(5_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// Same shape as `propose_action` but mutates the existing `PendingAction` entry (read +
	/// write) instead of inserting a fresh one.
	fn approve_action() -> Weight {
		Weight::from_parts(14_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(4_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// 2 reads (`Members`, `ApprovedAction`) + 1 write (`ApprovedAction` remove).
	fn clear_stale_action() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
}

// For backwards compatibility and tests.
impl WeightInfo for () {
	fn add_member() -> Weight {
		Weight::from_parts(15_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(7_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn remove_member() -> Weight {
		Weight::from_parts(20_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(44_u64))
			.saturating_add(RocksDbWeight::get().writes(42_u64))
	}
	fn close_bootstrap() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn propose_action() -> Weight {
		Weight::from_parts(14_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(5_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn approve_action() -> Weight {
		Weight::from_parts(14_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(4_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn clear_stale_action() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
}
