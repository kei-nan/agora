//! Benchmarking setup for pallet-emergency-council.
//!
//! Real, compiling `#[benchmarks]` scaffolding. See `weights.rs`'s module doc comment for why
//! the numbers currently in that file are manual estimates rather than this benchmark's actual
//! output.

use super::*;
use alloc::vec::Vec;
use frame_benchmarking::v2::*;
use frame_support::{traits::Get, BoundedVec};
use frame_system::RawOrigin;

#[allow(unused)]
use crate::Pallet as EmergencyCouncil;

fn seed_accounts<T: Config>(n: u32) -> Vec<T::AccountId> {
	(0..n).map(|i| -> T::AccountId { account("council", i, 0) }).collect()
}

/// Minimum vote count needed to cross the configured supermajority threshold for a council of
/// `council_size`, mirroring the (private, so not directly callable from here)
/// `Pallet::supermajority_reached` formula: `votes * denominator >= council_size * numerator`.
fn votes_needed<T: Config>(council_size: u32) -> usize {
	let numerator = T::SupermajorityNumerator::get() as u64;
	let denominator = T::SupermajorityDenominator::get().max(1) as u64;
	let size = council_size as u64;
	size.saturating_mul(numerator).div_ceil(denominator).max(1) as usize
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn add_council_member() {
		let max = T::MaxCouncilSize::get();
		let existing = seed_accounts::<T>(max.saturating_sub(1));
		Council::<T>::put(BoundedVec::<T::AccountId, T::MaxCouncilSize>::try_from(existing).unwrap());
		let new_member: T::AccountId = account("new_member", 0, 0);

		#[extrinsic_call]
		add_council_member(RawOrigin::Root, new_member.clone());

		assert!(Council::<T>::get().contains(&new_member));
	}

	#[benchmark]
	fn remove_council_member() {
		let max = T::MaxCouncilSize::get();
		let existing = seed_accounts::<T>(max);
		let target = existing.last().unwrap().clone();
		Council::<T>::put(BoundedVec::<T::AccountId, T::MaxCouncilSize>::try_from(existing).unwrap());

		#[extrinsic_call]
		remove_council_member(RawOrigin::Root, target.clone());

		assert!(!Council::<T>::get().contains(&target));
	}

	#[benchmark]
	fn vote_declare_emergency() {
		let max = T::MaxCouncilSize::get();
		let members = seed_accounts::<T>(max);
		Council::<T>::put(BoundedVec::<T::AccountId, T::MaxCouncilSize>::try_from(members.clone()).unwrap());
		let needed = votes_needed::<T>(max);

		// Cast (needed - 1) prior votes so the benchmarked call is the one that crosses the
		// supermajority threshold — the more expensive path (it also activates the emergency).
		for member in members.iter().take(needed.saturating_sub(1)) {
			Pallet::<T>::vote_declare_emergency(
				RawOrigin::Signed(member.clone()).into(),
				[3u8; 32],
				50,
			).unwrap();
		}
		let voter = members[needed.saturating_sub(1)].clone();

		#[extrinsic_call]
		vote_declare_emergency(RawOrigin::Signed(voter), [3u8; 32], 50);

		assert!(ActiveEmergency::<T>::get().is_some());
	}

	#[benchmark]
	fn vote_end_emergency() {
		let max = T::MaxCouncilSize::get();
		let members = seed_accounts::<T>(max);
		Council::<T>::put(BoundedVec::<T::AccountId, T::MaxCouncilSize>::try_from(members.clone()).unwrap());
		let needed = votes_needed::<T>(max);

		// Activate an emergency first.
		for member in members.iter().take(needed) {
			Pallet::<T>::vote_declare_emergency(
				RawOrigin::Signed(member.clone()).into(),
				[4u8; 32],
				50,
			).unwrap();
		}
		assert!(ActiveEmergency::<T>::get().is_some());

		// Cast (needed - 1) end-votes so the benchmarked call crosses the threshold again.
		for member in members.iter().take(needed.saturating_sub(1)) {
			Pallet::<T>::vote_end_emergency(RawOrigin::Signed(member.clone()).into()).unwrap();
		}
		let voter = members[needed.saturating_sub(1)].clone();

		#[extrinsic_call]
		vote_end_emergency(RawOrigin::Signed(voter));

		assert!(ActiveEmergency::<T>::get().is_none());
	}

	impl_benchmark_test_suite!(EmergencyCouncil, crate::mock::new_test_ext(), crate::mock::Test);
}
