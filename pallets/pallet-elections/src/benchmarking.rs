//! Benchmarking setup for pallet-elections.
//!
//! Real, compiling `#[benchmarks]` scaffolding — sanity-checked against this pallet's own
//! mock via `impl_benchmark_test_suite!` below. See `weights.rs`'s module doc comment for why
//! the numbers currently in `weights.rs` are manual estimates rather than this benchmark's
//! actual output.
//!
//! `register_candidate`/`register_as_delegate`/`back_delegate` depend on cross-pallet state
//! (`CitizenChecker`) that a generic pallet-elections benchmark can't set directly — see
//! `Config::BenchmarkHelper` in `lib.rs`. The pallet's own mock wires a real implementation
//! (so these benchmarks run against `crate::mock::Test`), but the runtime-side
//! `RuntimeBenchmarkHelper` in `runtime/src/configs/mod.rs` is currently a no-op: running
//! `benchmark pallet --pallet pallet_elections` against the real built runtime will still fail
//! those extrinsics until pallet-identity-zk grows an equivalent benchmark-only hook.
//! Documented rather than silently broken.

use super::*;
use frame_benchmarking::v2::*;
use frame_support::traits::{ConstU32, Currency, EnsureOrigin, EnsureOriginWithArg, Get};
use frame_support::BoundedVec;
use frame_system::RawOrigin;
use sp_runtime::traits::Saturating;

#[allow(unused)]
use crate::Pallet as ElectionsPallet;

fn governance_origin<T: Config>(tag: &'static [u8], params: impl codec::Encode) -> T::RuntimeOrigin {
	let hash = legislature_call_hash(tag, params);
	T::GovernanceOrigin::try_successful_origin(&hash).unwrap()
}

fn fund<T: Config>(who: &T::AccountId) {
	let amount = T::CandidateDeposit::get().saturating_mul(1000u32.into());
	T::Currency::make_free_balance_be(who, amount);
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn add_commissioner() {
		let account: T::AccountId = whitelisted_caller();

		#[extrinsic_call]
		add_commissioner(RawOrigin::Root, account.clone());

		assert!(Commissioners::<T>::get().contains(&account));
	}

	#[benchmark]
	fn remove_commissioner() {
		let account: T::AccountId = whitelisted_caller();
		Pallet::<T>::add_commissioner(RawOrigin::Root.into(), account.clone()).unwrap();

		#[extrinsic_call]
		remove_commissioner(RawOrigin::Root, account.clone());

		assert!(!Commissioners::<T>::get().contains(&account));
	}

	#[benchmark]
	fn create_election() {
		let office: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"president".to_vec()).unwrap();

		#[extrinsic_call]
		create_election(RawOrigin::Root, office, 0u32.into(), 100u32.into());

		assert_eq!(NextElectionId::<T>::get(), 1);
	}

	#[benchmark]
	fn register_candidate() {
		let office: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"president".to_vec()).unwrap();
		Pallet::<T>::create_election(RawOrigin::Root.into(), office, 0u32.into(), 100u32.into()).unwrap();
		let caller: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&caller);
		fund::<T>(&caller);

		#[extrinsic_call]
		register_candidate(RawOrigin::Signed(caller.clone()), 0u32, [1u8; 32]);

		assert!(Candidates::<T>::contains_key(0u32, &caller));
	}

	#[benchmark]
	fn certify_candidate() {
		let office: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"president".to_vec()).unwrap();
		Pallet::<T>::create_election(RawOrigin::Root.into(), office, 0u32.into(), 100u32.into()).unwrap();
		let candidate: T::AccountId = account("candidate", 0, 0);
		T::BenchmarkHelper::make_active_citizen(&candidate);
		fund::<T>(&candidate);
		Pallet::<T>::register_candidate(RawOrigin::Signed(candidate.clone()).into(), 0u32, [2u8; 32]).unwrap();
		let commissioner: T::AccountId = whitelisted_caller();
		Pallet::<T>::add_commissioner(RawOrigin::Root.into(), commissioner.clone()).unwrap();

		#[extrinsic_call]
		certify_candidate(RawOrigin::Signed(commissioner), 0u32, candidate.clone());

		assert_eq!(Candidates::<T>::get(0u32, &candidate).unwrap().status, CandidateStatus::Certified);
	}

	#[benchmark]
	fn submit_results() {
		let office: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"president".to_vec()).unwrap();
		Pallet::<T>::create_election(RawOrigin::Root.into(), office, 0u32.into(), 100u32.into()).unwrap();
		let winner: T::AccountId = account("winner", 0, 0);
		let commissioner: T::AccountId = whitelisted_caller();
		Pallet::<T>::add_commissioner(RawOrigin::Root.into(), commissioner.clone()).unwrap();

		#[extrinsic_call]
		submit_results(RawOrigin::Signed(commissioner), 0u32, winner, [3u8; 32]);

		assert_eq!(Elections::<T>::get(0u32).unwrap().status, ElectionStatus::ResultsSubmitted);
	}

	#[benchmark]
	fn certify_results() {
		let office: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"president".to_vec()).unwrap();
		Pallet::<T>::create_election(RawOrigin::Root.into(), office, 0u32.into(), 100u32.into()).unwrap();
		let winner: T::AccountId = account("winner", 0, 0);
		let commissioner: T::AccountId = whitelisted_caller();
		Pallet::<T>::add_commissioner(RawOrigin::Root.into(), commissioner.clone()).unwrap();
		Pallet::<T>::submit_results(
			RawOrigin::Signed(commissioner.clone()).into(), 0u32, winner, [4u8; 32],
		).unwrap();

		#[extrinsic_call]
		certify_results(RawOrigin::Signed(commissioner), 0u32);

		assert_eq!(Elections::<T>::get(0u32).unwrap().status, ElectionStatus::Certified);
	}

	#[benchmark]
	fn register_as_delegate() {
		let caller: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&caller);
		let name: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"alice".to_vec()).unwrap();

		#[extrinsic_call]
		register_as_delegate(RawOrigin::Signed(caller.clone()), name, [5u8; 32]);

		assert!(Delegates::<T>::contains_key(&caller));
	}

	#[benchmark]
	fn back_delegate() {
		let delegate: T::AccountId = account("delegate", 0, 0);
		T::BenchmarkHelper::make_active_citizen(&delegate);
		let name: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"bob".to_vec()).unwrap();
		Pallet::<T>::register_as_delegate(RawOrigin::Signed(delegate.clone()).into(), name, [6u8; 32]).unwrap();
		let backer: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&backer);

		#[extrinsic_call]
		back_delegate(RawOrigin::Signed(backer.clone()), delegate.clone());

		assert!(BackingOf::<T>::contains_key(&backer, &delegate));
	}

	#[benchmark]
	fn remove_backing() {
		let delegate: T::AccountId = account("delegate", 0, 0);
		T::BenchmarkHelper::make_active_citizen(&delegate);
		let name: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"carol".to_vec()).unwrap();
		Pallet::<T>::register_as_delegate(RawOrigin::Signed(delegate.clone()).into(), name, [7u8; 32]).unwrap();
		let backer: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&backer);
		Pallet::<T>::back_delegate(RawOrigin::Signed(backer.clone()).into(), delegate.clone()).unwrap();

		#[extrinsic_call]
		remove_backing(RawOrigin::Signed(backer.clone()), delegate.clone());

		assert!(!BackingOf::<T>::contains_key(&backer, &delegate));
	}

	#[benchmark]
	fn set_backing_threshold() {
		let floor = BackingThresholdFloor::<T>::get();
		let origin = governance_origin::<T>(b"pallet-elections::set_backing_threshold", floor);

		#[extrinsic_call]
		set_backing_threshold(origin, floor);

		assert_eq!(BackingThreshold::<T>::get(), floor);
	}

	#[benchmark]
	fn set_backing_bounds() {
		let origin = T::ConstitutionalOrigin::try_successful_origin().unwrap();

		#[extrinsic_call]
		set_backing_bounds(origin, 1u32, 50u32);

		assert_eq!(BackingThresholdCeiling::<T>::get(), 50u32);
	}

	#[benchmark]
	fn set_term_params() {
		let origin = T::ConstitutionalOrigin::try_successful_origin().unwrap();

		#[extrinsic_call]
		set_term_params(origin, 1000u32.into(), 2u32, 100u32.into(), 20u8);

		assert_eq!(MaxConsecutiveTerms::<T>::get(), 2u32);
	}

	#[benchmark]
	fn set_election_params() {
		let origin = T::ConstitutionalOrigin::try_successful_origin().unwrap();

		#[extrinsic_call]
		set_election_params(origin, Some(5u32), Some(1000u32), Some(10u32));

		assert_eq!(LegislatureSeats::<T>::get(), 5u32);
	}

	impl_benchmark_test_suite!(ElectionsPallet, crate::mock::new_test_ext(), crate::mock::Test);
}
