// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
// For more information, please refer to <http://unlicense.org>

//! Weights for pallet_executive.
//!
//! NOT machine-benchmarked. `pallets/pallet-executive/src/benchmarking.rs` contains real,
//! compiling `#[benchmarks]` scaffolding, but no `benchmark pallet` run has been executed
//! against a real built runtime to produce the numbers below. Values are manually reasoned
//! from each extrinsic's actual storage reads/writes (see each function's doc comment).
//! `cabinet_size()` and `count_declare_votes()` (called by `vote_declare_emergency`) and the
//! equivalent tally in `vote_end_emergency`/`retract_emergency_vote` each iterate the full
//! `PortfolioMinister` / `DeclareVotes` / `EndVotes` maps — genuinely O(`MaxPortfolios`), not
//! O(1) — so those four functions scale their read count with `T::MaxPortfolios` rather than
//! using a flat number, unlike this pallet's other (truly O(1)) calls. Do not treat these as
//! production-safe, DoS-resistant weights — see `pallets/pallet-legislature/src/weights.rs`'s
//! doc comment for the real-benchmark command shape that would replace this file.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for pallet_executive.
pub trait WeightInfo {
	fn define_portfolio() -> Weight;
	fn appoint_prime_minister() -> Weight;
	fn dismiss_prime_minister() -> Weight;
	fn appoint_minister() -> Weight;
	fn dismiss_minister() -> Weight;
	fn resign() -> Weight;
	fn vote_declare_emergency() -> Weight;
	fn ratify_emergency() -> Weight;
	fn vote_end_emergency() -> Weight;
	fn retract_emergency_vote() -> Weight;
}

/// Weights for pallet_executive.
///
/// Bound on the full pallet `Config` (not just `frame_system::Config`) so the
/// cabinet-iterating calls below can size their read count off `T::MaxPortfolios` — see the
/// module doc comment.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: crate::Config> WeightInfo for SubstrateWeight<T> {
	/// 1 read (`NextPortfolioId`) + 2 writes (`Portfolios`, `NextPortfolioId`).
	fn define_portfolio() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// Worst case: an outgoing PM is replaced. 1 read (`PrimeMinister`) + 3 writes
	/// (`DeclareVotes` x2, `PrimeMinister`).
	fn appoint_prime_minister() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// `PrimeMinister::take` (read+write) + `DeclareVotes::remove` (write).
	fn dismiss_prime_minister() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// Worst case: both the outgoing portfolio-holder and the incoming account's prior
	/// portfolio must be vacated. 3 reads (`Portfolios::contains_key`, `PortfolioMinister::get`,
	/// `MinisterPortfolio::get`) + 6 writes (2x `MinisterPortfolio`/`PortfolioMinister` removal
	/// pairs, `DeclareVotes::remove`, plus the final insert pair).
	fn appoint_minister() -> Weight {
		Weight::from_parts(17_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(3_u64))
			.saturating_add(T::DbWeight::get().writes(6_u64))
	}
	/// 2 reads (`Portfolios::contains_key`, `PortfolioMinister::take`) + 3 writes
	/// (`PortfolioMinister`, `MinisterPortfolio`, `DeclareVotes`).
	fn dismiss_minister() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// `MinisterPortfolio::take` (read+write) + 2 writes (`PortfolioMinister`, `DeclareVotes`).
	fn resign() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// `is_cabinet_member` (2 reads) + `ActiveEmergency`/`DeclareVotes`/`PendingEmergencyProposal`
	/// (up to 4 reads) + `cabinet_size()` and `count_declare_votes()`, each an O(`MaxPortfolios`)
	/// map iteration — the dominant cost for a large cabinet. Costed at the
	/// supermajority-reached (most expensive) path: up to 4 writes.
	fn vote_declare_emergency() -> Weight {
		let cabinet_reads = 2u64.saturating_mul(T::MaxPortfolios::get() as u64);
		Weight::from_parts(20_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(6_u64.saturating_add(cabinet_reads)))
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}
	/// `ActiveEmergency::try_mutate` — 1 read + 1 write. O(1), unlike the vote/tally calls.
	fn ratify_emergency() -> Weight {
		Weight::from_parts(10_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// Same O(`MaxPortfolios`) tally shape as `vote_declare_emergency` (`cabinet_size()` plus a
	/// full `EndVotes` scan), costed at the supermajority-reached path: up to 4 writes.
	fn vote_end_emergency() -> Weight {
		let cabinet_reads = 2u64.saturating_mul(T::MaxPortfolios::get() as u64);
		Weight::from_parts(19_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(4_u64.saturating_add(cabinet_reads)))
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}
	/// `is_cabinet_member` (2 reads) + `ActiveEmergency`/`DeclareVotes` (2 reads) + a full
	/// `DeclareVotes` scan to check whether any votes remain (O(`MaxPortfolios`)). Up to 2 writes.
	fn retract_emergency_vote() -> Weight {
		let scan_reads = T::MaxPortfolios::get() as u64;
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(4_u64.saturating_add(scan_reads)))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
}

// For backwards compatibility and tests. Uses a fixed generous portfolio-count assumption
// (20, matching this pallet's own `MaxPortfolios` doc-comment example) since `()` carries no
// `Config` to read `MaxPortfolios` from.
impl WeightInfo for () {
	fn define_portfolio() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn appoint_prime_minister() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn dismiss_prime_minister() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn appoint_minister() -> Weight {
		Weight::from_parts(17_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(3_u64))
			.saturating_add(RocksDbWeight::get().writes(6_u64))
	}
	fn dismiss_minister() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn resign() -> Weight {
		Weight::from_parts(12_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn vote_declare_emergency() -> Weight {
		Weight::from_parts(20_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(46_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}
	fn ratify_emergency() -> Weight {
		Weight::from_parts(10_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn vote_end_emergency() -> Weight {
		Weight::from_parts(19_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(44_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}
	fn retract_emergency_vote() -> Weight {
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(24_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
}
