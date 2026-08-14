// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
// For more information, please refer to <http://unlicense.org>

//! Weights for pallet_elections.
//!
//! NOT machine-benchmarked. `pallets/pallet-elections/src/benchmarking.rs` contains real,
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
//!   --chain dev --pallet pallet_elections --extrinsic '*' \
//!   --steps 50 --repeat 20 --wasm-execution compiled \
//!   --output pallets/pallet-elections/src/weights.rs \
//!   --template .maintain/frame-weight-template.hbs
//! ```

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for pallet_elections.
pub trait WeightInfo {
	fn register_as_delegate() -> Weight;
	fn back_delegate() -> Weight;
	fn remove_backing() -> Weight;
	fn set_backing_threshold() -> Weight;
	fn set_backing_bounds() -> Weight;
	fn set_term_params() -> Weight;
	fn set_election_params() -> Weight;
}

/// Weights for pallet_elections.
///
/// See the module doc comment: manually estimated, not machine-benchmarked.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// Reads `Delegates`; writes `Delegates`.
	fn register_as_delegate() -> Weight {
		Weight::from_parts(14_000_000, 2_600)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// Reads `Delegates`, `BackingOf`, `CitizenBackingCount`, `BackingCount`,
	/// `BackingThreshold`; writes `BackingOf`, `CitizenBackingCount`, `BackingCount`, and
	/// (on the activation path) `Delegates` — costed at the activation worst case.
	fn back_delegate() -> Weight {
		Weight::from_parts(22_000_000, 4_600)
			.saturating_add(T::DbWeight::get().reads(5_u64))
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}
	/// Same shape as `back_delegate` (mirror-image deactivation path).
	fn remove_backing() -> Weight {
		Weight::from_parts(22_000_000, 4_600)
			.saturating_add(T::DbWeight::get().reads(4_u64))
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}
	/// Reads `BackingThresholdFloor`, `BackingThresholdCeiling`; writes `BackingThreshold`.
	fn set_backing_threshold() -> Weight {
		Weight::from_parts(11_000_000, 1_800)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// Reads `BackingThreshold`; writes `BackingThresholdFloor`, `BackingThresholdCeiling`,
	/// and (on the clamping path) `BackingThreshold`.
	fn set_backing_bounds() -> Weight {
		Weight::from_parts(12_000_000, 1_800)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// Writes `TermLengthBlocks`, `MaxConsecutiveTerms`, `MandatoryBreakBlocks`,
	/// `WarningWindowPct` unconditionally.
	fn set_term_params() -> Weight {
		Weight::from_parts(11_000_000, 1_600)
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}
	/// Reads and conditionally writes `LegislatureSeats`, `ElectionCycleBlocks`,
	/// `MaxBackingsPerCitizen` — costed at the all-three-set worst case.
	fn set_election_params() -> Weight {
		Weight::from_parts(13_000_000, 2_400)
			.saturating_add(T::DbWeight::get().reads(3_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
}

// For backwards compatibility and tests.
impl WeightInfo for () {
	fn register_as_delegate() -> Weight {
		Weight::from_parts(14_000_000, 2_600)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn back_delegate() -> Weight {
		Weight::from_parts(22_000_000, 4_600)
			.saturating_add(RocksDbWeight::get().reads(5_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}
	fn remove_backing() -> Weight {
		Weight::from_parts(22_000_000, 4_600)
			.saturating_add(RocksDbWeight::get().reads(4_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}
	fn set_backing_threshold() -> Weight {
		Weight::from_parts(11_000_000, 1_800)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn set_backing_bounds() -> Weight {
		Weight::from_parts(12_000_000, 1_800)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn set_term_params() -> Weight {
		Weight::from_parts(11_000_000, 1_600)
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}
	fn set_election_params() -> Weight {
		Weight::from_parts(13_000_000, 2_400)
			.saturating_add(RocksDbWeight::get().reads(3_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
}
