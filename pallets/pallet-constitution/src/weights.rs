// This is free and unencumbered software released into the public domain.
//
// Anyone is free to copy, modify, publish, use, compile, sell, or
// distribute this software, either in source code form or as a compiled
// binary, for any purpose, commercial or non-commercial, and by any
// means.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND.
// For more information, please refer to <http://unlicense.org>

//! Weights for pallet_constitution.
//!
//! NOT machine-benchmarked. `pallets/pallet-constitution/src/benchmarking.rs` contains real,
//! compiling `#[benchmarks]` scaffolding, but no `benchmark pallet` run has been executed
//! against a real built runtime to produce the numbers below. Values are manually reasoned
//! from each extrinsic's actual storage reads/writes (see each function's doc comment) — all
//! O(1) keyed lookups in this pallet, no unbounded iteration, so no linear-in-collection-size
//! component is needed here (contrast `pallet-emergency-council`/`pallet-executive`, which do
//! have genuinely O(n) calls). Do not treat these as production-safe, DoS-resistant weights —
//! see `pallets/pallet-legislature/src/weights.rs`'s doc comment for the real-benchmark command
//! shape that would replace this file.

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

/// Weight functions needed for pallet_constitution.
pub trait WeightInfo {
	fn enact_law() -> Weight;
	fn invalidate_law() -> Weight;
	fn propose_amendment() -> Weight;
	fn ratify_amendment() -> Weight;
	fn submit_petition() -> Weight;
	fn sign_petition() -> Weight;
	fn repeal_law() -> Weight;
	fn propose_constitutional_amendment() -> Weight;
	fn reaffirm_amendment() -> Weight;
	fn advance_to_entrenched() -> Weight;
	fn revoke_amendment() -> Weight;
	fn challenge_law_tier() -> Weight;
}

/// Weights for pallet_constitution.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// 1 read (`NextLawId`) + 2 writes (`Laws`, `NextLawId`); Structural/Foundational laws also
	/// call `AutoChallengeHook::auto_challenge_law` (best-effort, cost folded into the base).
	fn enact_law() -> Weight {
		Weight::from_parts(16_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// `Laws::try_mutate` — 1 read + 1 write.
	fn invalidate_law() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// 2 reads (`Laws`, `PendingAmendments::contains_key`) + 1 write (`PendingAmendments`).
	fn propose_amendment() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// `PendingAmendments::take` (read+write) + `Laws::try_mutate` (read+write).
	fn ratify_amendment() -> Weight {
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// 1 read (`NextPetitionId`) + 3 writes (`Petitions`, `PetitionSignatures`,
	/// `NextPetitionId`); may also call `PetitionApprover::create_referendum` when
	/// `PetitionThreshold == 1` (cost folded into the base).
	fn submit_petition() -> Weight {
		Weight::from_parts(15_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// 2 reads (`PetitionSignatures`, `Petitions`) + 2 writes (`Petitions`,
	/// `PetitionSignatures`); may also call `PetitionApprover::create_referendum` on the
	/// threshold-crossing path (cost folded into the base).
	fn sign_petition() -> Weight {
		Weight::from_parts(15_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// `Laws::try_mutate` (read+write) + 2 writes (`PendingAmendments::remove`,
	/// `ConstitutionalAmendments::remove`).
	fn repeal_law() -> Weight {
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}
	/// 2 reads (`Laws`, `ConstitutionalAmendments::contains_key`) + 2 writes (`Laws`,
	/// `ConstitutionalAmendments`).
	fn propose_constitutional_amendment() -> Weight {
		Weight::from_parts(15_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// `ConstitutionalAmendments::try_mutate` (read+write); also calls
	/// `FreshLegislatureChecker::has_election_occurred_since` (external read elsewhere, folded
	/// into the base).
	fn reaffirm_amendment() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// `ConstitutionalAmendments::try_mutate` — 1 read + 1 write. Permissionless, so no origin
	/// check overhead beyond a plain signed check.
	fn advance_to_entrenched() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	/// `ConstitutionalAmendments::take` (read+write) + `Laws::try_mutate` (read+write).
	fn revoke_amendment() -> Weight {
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	/// 1 read (`Laws::contains_key`) plus the folded cost of
	/// `TierConflictHook::file_tier_conflict_case`, which — unlike this pallet's other hook
	/// calls — performs a real ZK pairing check inside pallet-courts (see
	/// `pallet_anticorruption::submit_whistleblower_report`'s identical-shaped weight for the
	/// same reasoning), so this is priced heavier than this pallet's storage-only calls rather
	/// than folded in as a negligible extra the way `enact_law`'s `AutoChallengeHook` call is.
	fn challenge_law_tier() -> Weight {
		Weight::from_parts(20_000_000, 3_600)
			.saturating_add(T::DbWeight::get().reads(1_u64))
	}
}

// For backwards compatibility and tests.
impl WeightInfo for () {
	fn enact_law() -> Weight {
		Weight::from_parts(16_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn invalidate_law() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn propose_amendment() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn ratify_amendment() -> Weight {
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn submit_petition() -> Weight {
		Weight::from_parts(15_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn sign_petition() -> Weight {
		Weight::from_parts(15_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn repeal_law() -> Weight {
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}
	fn propose_constitutional_amendment() -> Weight {
		Weight::from_parts(15_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn reaffirm_amendment() -> Weight {
		Weight::from_parts(13_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn advance_to_entrenched() -> Weight {
		Weight::from_parts(11_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}
	fn revoke_amendment() -> Weight {
		Weight::from_parts(14_000_000, 1_957)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}
	fn challenge_law_tier() -> Weight {
		Weight::from_parts(20_000_000, 3_600)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
	}
}
