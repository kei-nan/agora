// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
// For more information, please refer to <http://unlicense.org>

//! Weights for pallet_legislature.
//!
//! NOT machine-benchmarked. `pallets/pallet-legislature/src/benchmarking.rs` contains real,
//! compiling `#[benchmarks]` scaffolding (built against this pallet's own mock runtime, same
//! shape as `pallet-template`'s), but no `benchmark pallet` run has been executed against a
//! real built runtime to produce the numbers below. Values are manually reasoned from each
//! extrinsic's actual storage reads/writes (see each function's doc comment) using
//! `T::DbWeight` for the database component and a small fixed per-extrinsic base cost for
//! decode/dispatch/event-deposit overhead — the same shape a real benchmark's output takes,
//! but not a substitute for one. Do not treat these as production-safe, DoS-resistant weights.
//!
//! To replace with real numbers once tooling allows it:
//! ```text
//! WASM_BUILD_RUSTFLAGS="-C link-arg=--allow-undefined" \
//!   cargo build --release --features runtime-benchmarks -p agora-node
//! ./target/release/agora-node benchmark pallet \
//!   --chain dev --pallet pallet_legislature --extrinsic '*' \
//!   --steps 50 --repeat 20 --wasm-execution compiled \
//!   --output pallets/pallet-legislature/src/weights.rs \
//!   --template .maintain/frame-weight-template.hbs
//! ```
//!
//! As of this writing, that full build additionally requires fixing an unrelated,
//! pre-existing gap in `pallet-courts`'s `EnsureOracle`'s `EnsureOrigin` impl (missing
//! `try_successful_origin`), which only surfaces once `runtime-benchmarks` is enabled
//! workspace-wide — `pallet-courts` has no `runtime-benchmarks` feature of its own yet
//! and was never previously compiled with this feature on. Not part of this pallet's
//! scope; noted here so the blocker is visible from the file a future benchmark run
//! would start from.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for pallet_legislature.
pub trait WeightInfo {
	fn add_member() -> Weight;
	fn remove_member() -> Weight;
	fn propose_motion() -> Weight;
	fn vote_motion() -> Weight;
	fn close_motion() -> Weight;
}

/// Weights for pallet_legislature.
///
/// See the module doc comment: manually estimated, not machine-benchmarked.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// 1 read (`Members`) + 1 write (`Members`). Worst case: council already near `MaxMembers`,
	/// so the `BoundedVec` push/re-encode dominates the base cost slightly more than a trivial
	/// storage write.
	fn add_member() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// 1 read + 1 write of `Members`, plus a linear scan of the member list to find the
	/// position to remove — same order of cost as `add_member`.
	fn remove_member() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// Reads `Members` (minister eligibility + membership check) and `NextMotionId`; writes
	/// `Motions`, `MotionVotes`, and `NextMotionId`. Three writes vs. one for add/remove_member,
	/// so a materially higher base.
	fn propose_motion() -> Weight {
		Weight::from_parts(18_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// Reads `Members`, `MotionVotes` (double-vote check), `Motions`; writes `Motions`
	/// (mutate ayes/nays) and `MotionVotes` (record the vote).
	fn vote_motion() -> Weight {
		Weight::from_parts(16_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(3_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// Reads `Motions`, `Members` (for the threshold denominator), `PendingLegislatureApproval`;
	/// writes `Motions` (mark executed) and, on the passing path, `PendingLegislatureApproval`.
	/// Costed at the passing-path worst case (two writes).
	fn close_motion() -> Weight {
		Weight::from_parts(17_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(3_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
}

// For backwards compatibility and tests.
impl WeightInfo for () {
	fn add_member() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn remove_member() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn propose_motion() -> Weight {
		Weight::from_parts(18_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn vote_motion() -> Weight {
		Weight::from_parts(16_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(3_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn close_motion() -> Weight {
		Weight::from_parts(17_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(3_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
}
