// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
// For more information, please refer to <http://unlicense.org>

//! Weights for pallet_emergency_council.
//!
//! NOT machine-benchmarked. `pallets/pallet-emergency-council/src/benchmarking.rs` contains
//! real, compiling `#[benchmarks]` scaffolding, but no `benchmark pallet` run has been
//! executed against a real built runtime to produce the numbers below. Values are manually
//! reasoned from each extrinsic's actual storage access pattern (see each function's doc
//! comment). `vote_declare_emergency` and `vote_end_emergency` each read one `DeclareVotes` /
//! `EndVotes` entry *per council member* to count votes (`council.iter().filter(...).count()`
//! in `lib.rs`) — this is genuinely O(`MaxCouncilSize`), not O(1), so the estimate below scales
//! the read count with `T::MaxCouncilSize` rather than using a flat number, unlike the other
//! (truly O(1)) calls in this pallet. Do not treat these as production-safe, DoS-resistant
//! weights — a real `benchmark pallet` run (see `pallets/pallet-legislature/src/weights.rs`'s
//! doc comment for the command shape) would also want a genuine linear component (`x in 0 ..
//! MaxCouncilSize`) rather than this fixed worst-case-at-max-size approximation.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for pallet_emergency_council.
pub trait WeightInfo {
	fn add_council_member() -> Weight;
	fn remove_council_member() -> Weight;
	fn vote_declare_emergency() -> Weight;
	fn vote_end_emergency() -> Weight;
	fn close_bootstrap() -> Weight;
}

/// Weights for pallet_emergency_council.
///
/// Bound on the full pallet `Config` (not just `frame_system::Config`) so the
/// council-iterating calls below can size their read count off `T::MaxCouncilSize` — see the
/// module doc comment.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: crate::Config> WeightInfo for SubstrateWeight<T> {
	/// 1 read + 1 write of `Council`.
	fn add_council_member() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// 1 read + 1 write of `Council`, plus 2 writes clearing this member's `DeclareVotes` /
	/// `EndVotes` entries.
	fn remove_council_member() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// Reads `Council`, `ActiveEmergency`, `DeclareVotes` (own entry), `PendingEmergencyProposal`
	/// (up to twice), plus one `DeclareVotes` read *per council member* to tally votes — sized
	/// to the constitutional ceiling `T::MaxCouncilSize`. Writes: `DeclareVotes` (this vote),
	/// and on the supermajority-reached path `ActiveEmergency`, `PendingEmergencyProposal`, plus
	/// clearing both vote maps — costed at that (more expensive) path.
	fn vote_declare_emergency() -> Weight {
		let council_reads = T::MaxCouncilSize::get() as u64;
		Weight::from_parts(18_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(4_u64.saturating_add(council_reads)))
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}
	/// Same shape as `vote_declare_emergency` but tallies `EndVotes` instead, and has no
	/// proposal-terms bookkeeping.
	fn vote_end_emergency() -> Weight {
		let council_reads = T::MaxCouncilSize::get() as u64;
		Weight::from_parts(16_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(3_u64.saturating_add(council_reads)))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// 2 reads (`Bootstrapped`, `Council`) + 1 write (`Bootstrapped`).
	fn close_bootstrap() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
}

// For backwards compatibility and tests. Uses a fixed generous council-size assumption (35,
// matching the OPRF-committee-scale figure used elsewhere in this codebase) since `()` carries
// no `Config` to read `MaxCouncilSize` from.
impl WeightInfo for () {
	fn add_council_member() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn remove_council_member() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn vote_declare_emergency() -> Weight {
		Weight::from_parts(18_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(39_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}
	fn vote_end_emergency() -> Weight {
		Weight::from_parts(16_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(38_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn close_bootstrap() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
}
