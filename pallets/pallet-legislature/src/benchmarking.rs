//! Benchmarking setup for pallet-legislature.
//!
//! Real, compiling `#[benchmarks]` scaffolding — runnable via `cargo test --features
//! runtime-benchmarks` (sanity-checks against this pallet's own mock, see
//! `impl_benchmark_test_suite!` below) and via the node's `benchmark pallet` subcommand once
//! this pallet is wired into `runtime/src/benchmarks.rs` (it is). See `weights.rs`'s module
//! doc comment for why the numbers currently in that file are manual estimates rather than
//! this benchmark's actual output.

use super::*;
use alloc::vec::Vec;
use frame_benchmarking::v2::*;
use frame_support::{traits::Get, BoundedVec};
use frame_system::RawOrigin;

#[allow(unused)]
use crate::Pallet as Legislature;

/// Fills `Members` with `n` distinct accounts.
fn seed_members<T: Config>(n: u32) -> Vec<T::AccountId> {
	(0..n).map(|i| -> T::AccountId { account("member", i, 0) }).collect()
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn add_member() {
		// Worst case: the council is already one seat short of `MaxMembers`.
		let max = T::MaxMembers::get();
		let existing = seed_members::<T>(max.saturating_sub(1));
		Members::<T>::put(BoundedVec::<T::AccountId, T::MaxMembers>::try_from(existing).unwrap());
		let new_member: T::AccountId = account("new_member", 0, 0);

		#[extrinsic_call]
		add_member(RawOrigin::Root, new_member.clone());

		assert!(Members::<T>::get().contains(&new_member));
	}

	#[benchmark]
	fn remove_member() {
		let max = T::MaxMembers::get();
		let existing = seed_members::<T>(max);
		let target = existing.last().unwrap().clone();
		Members::<T>::put(BoundedVec::<T::AccountId, T::MaxMembers>::try_from(existing).unwrap());

		#[extrinsic_call]
		remove_member(RawOrigin::Root, target.clone());

		assert!(!Members::<T>::get().contains(&target));
	}

	#[benchmark]
	fn propose_motion() {
		let max = T::MaxMembers::get();
		let mut existing = seed_members::<T>(max.saturating_sub(1));
		let proposer: T::AccountId = whitelisted_caller();
		existing.push(proposer.clone());
		Members::<T>::put(BoundedVec::<T::AccountId, T::MaxMembers>::try_from(existing).unwrap());
		let call_hash = [7u8; 32];

		#[extrinsic_call]
		propose_motion(RawOrigin::Signed(proposer), call_hash);

		assert_eq!(NextMotionId::<T>::get(), 1);
	}

	#[benchmark]
	fn vote_motion() {
		let max = T::MaxMembers::get();
		let mut existing = seed_members::<T>(max.saturating_sub(2).max(1));
		let proposer: T::AccountId = account("proposer", 0, 0);
		let voter: T::AccountId = whitelisted_caller();
		existing.push(proposer.clone());
		existing.push(voter.clone());
		Members::<T>::put(BoundedVec::<T::AccountId, T::MaxMembers>::try_from(existing).unwrap());
		Pallet::<T>::propose_motion(RawOrigin::Signed(proposer).into(), [1u8; 32]).unwrap();

		#[extrinsic_call]
		vote_motion(RawOrigin::Signed(voter.clone()), 0u32, true);

		assert!(MotionVotes::<T>::contains_key((0u32, voter)));
	}

	#[benchmark]
	fn close_motion() {
		let max = T::MaxMembers::get();
		let mut existing = seed_members::<T>(max.saturating_sub(1));
		let proposer: T::AccountId = account("proposer", 0, 0);
		existing.push(proposer.clone());
		Members::<T>::put(BoundedVec::<T::AccountId, T::MaxMembers>::try_from(existing).unwrap());
		Pallet::<T>::propose_motion(RawOrigin::Signed(proposer).into(), [2u8; 32]).unwrap();

		let end_block = Motions::<T>::get(0u32).unwrap().end_block;
		frame_system::Pallet::<T>::set_block_number(end_block);
		let closer: T::AccountId = whitelisted_caller();

		#[extrinsic_call]
		close_motion(RawOrigin::Signed(closer), 0u32);

		assert!(Motions::<T>::get(0u32).unwrap().executed);
	}

	impl_benchmark_test_suite!(Legislature, crate::mock::new_test_ext(), crate::mock::Test);
}
