//! Benchmarking setup for pallet-elections.
//!
//! Real, compiling `#[benchmarks]` scaffolding — sanity-checked against this pallet's own
//! mock via `impl_benchmark_test_suite!` below. See `weights.rs`'s module doc comment for why
//! the numbers currently in `weights.rs` are manual estimates rather than this benchmark's
//! actual output.
//!
//! `register_as_delegate`/`back_delegate` depend on cross-pallet state (`CitizenChecker`)
//! that a generic pallet-elections benchmark can't set directly — see `Config::BenchmarkHelper`
//! in `lib.rs`. The pallet's own mock wires a real implementation (so these benchmarks run
//! against `crate::mock::Test`), but the runtime-side `RuntimeBenchmarkHelper` in
//! `runtime/src/configs/mod.rs` is currently a no-op: running `benchmark pallet --pallet
//! pallet_elections` against the real built runtime will still fail those extrinsics until
//! pallet-identity-zk grows an equivalent benchmark-only hook. Documented rather than
//! silently broken.

use super::*;
use frame_benchmarking::v2::*;
use frame_support::traits::{ConstU32, EnsureOrigin, EnsureOriginWithArg};
use frame_support::BoundedVec;
use frame_system::RawOrigin;

#[allow(unused)]
use crate::Pallet as ElectionsPallet;

fn governance_origin<T: Config>(tag: &'static [u8], params: impl codec::Encode) -> T::RuntimeOrigin {
	let hash = legislature_call_hash(tag, params);
	T::GovernanceOrigin::try_successful_origin(&hash).unwrap()
}

#[benchmarks]
mod benchmarks {
	use super::*;

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
