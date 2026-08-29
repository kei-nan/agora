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
//!
//! The same limitation now also applies, independently, to every ZK-proof-gated call
//! (`register_as_delegate`/`back_delegate`/`remove_backing`): there is no generic way for this
//! benchmark to construct a *genuinely valid* ZK proof for an arbitrary `T::Config` (that would
//! require a real Noir witness/proving run, not benchmark-harness code). The fixtures below are
//! shaped only to satisfy `crate::mock`'s deterministic marker-byte test doubles
//! (`TestZkVerifier`/`TestDelegatePersonaVerifier`/`TestCommitteeKeyChecker`/
//! `TestBackingProofVerifier`/`TestBackingRootChecker` — see that module's doc comments), so
//! these three benchmarks are only meaningful via `impl_benchmark_test_suite!` against
//! `crate::mock::Test` below, same as the pre-existing `CitizenChecker` limitation this doc
//! comment already described.

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

	/// Byte marker `crate::mock::TestZkVerifier`/`TestBackingProofVerifier` treat as "valid" —
	/// duplicated here (rather than importing `crate::mock`, which is `#[cfg(test)]`-only and
	/// unavailable when this benchmark is compiled into a real runtime) since it only needs to
	/// match when this benchmark actually runs, i.e. via `impl_benchmark_test_suite!` below.
	const VALID_PROOF_MARKER: u8 = 1;

	fn delegate_persona_proof<T: Config>(
		delegate_persona_id: [u8; 32],
		persona_account: &T::AccountId,
	) -> (BoundedVec<u8, ConstU32<4096>>, BoundedVec<[u8; 32], ConstU32<18>>) {
		let persona_bytes = T::AccountIdToBytes::to_bytes(persona_account);
		let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(alloc::vec![
			[0u8; 32], [0u8; 32], [0u8; 32],
			AGORA_ELECTIONS_SERVICE_SCOPE,
			AGORA_ELECTIONS_DELEGATE_REG_SUBSCOPE,
			delegate_persona_id,
			persona_bytes,
			[0u8; 32], [0u8; 32],
		])
		.unwrap();
		(BoundedVec::try_from(alloc::vec![VALID_PROOF_MARKER]).unwrap(), public_inputs)
	}

	fn backing_proof<T: Config>(
		delegate_persona_id: [u8; 32],
		backing_nullifier: [u8; 32],
	) -> (BoundedVec<u8, ConstU32<8192>>, [[u8; 32]; 4]) {
		let mut max_backings = [0u8; 32];
		max_backings[28..32].copy_from_slice(&MaxBackingsPerCitizen::<T>::get().to_be_bytes());
		let inputs = [[3u8; 32], delegate_persona_id, max_backings, backing_nullifier];
		(BoundedVec::try_from(alloc::vec![VALID_PROOF_MARKER]).unwrap(), inputs)
	}

	#[benchmark]
	fn register_as_delegate() {
		let caller: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&caller);
		let name: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"alice".to_vec()).unwrap();
		let delegate_persona_id = [5u8; 32];
		let (zk_proof, public_inputs) = delegate_persona_proof::<T>(delegate_persona_id, &caller);

		#[extrinsic_call]
		register_as_delegate(
			RawOrigin::Signed(caller.clone()),
			caller.clone(),
			delegate_persona_id,
			zk_proof,
			public_inputs,
			1,
			[[0u8; 32]; 5],
			name,
			[5u8; 32],
		);

		assert!(Delegates::<T>::contains_key(&caller));
	}

	#[benchmark]
	fn back_delegate() {
		let delegate: T::AccountId = account("delegate", 0, 0);
		T::BenchmarkHelper::make_active_citizen(&delegate);
		let name: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"bob".to_vec()).unwrap();
		let delegate_persona_id = [6u8; 32];
		let (persona_proof, persona_inputs) =
			delegate_persona_proof::<T>(delegate_persona_id, &delegate);
		Pallet::<T>::register_as_delegate(
			RawOrigin::Signed(delegate.clone()).into(),
			delegate.clone(),
			delegate_persona_id,
			persona_proof,
			persona_inputs,
			1,
			[[0u8; 32]; 5],
			name,
			[6u8; 32],
		)
		.unwrap();
		let backer: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&backer);
		let backing_nullifier = [10u8; 32];
		let (zk_proof, public_inputs) = backing_proof::<T>(delegate_persona_id, backing_nullifier);

		#[extrinsic_call]
		back_delegate(RawOrigin::Signed(backer.clone()), delegate.clone(), zk_proof, public_inputs);

		assert!(UsedBackingNullifier::<T>::contains_key(backing_nullifier));
	}

	#[benchmark]
	fn remove_backing() {
		let delegate: T::AccountId = account("delegate", 0, 0);
		T::BenchmarkHelper::make_active_citizen(&delegate);
		let name: BoundedVec<u8, ConstU32<64>> = BoundedVec::try_from(b"carol".to_vec()).unwrap();
		let delegate_persona_id = [7u8; 32];
		let (persona_proof, persona_inputs) =
			delegate_persona_proof::<T>(delegate_persona_id, &delegate);
		Pallet::<T>::register_as_delegate(
			RawOrigin::Signed(delegate.clone()).into(),
			delegate.clone(),
			delegate_persona_id,
			persona_proof,
			persona_inputs,
			1,
			[[0u8; 32]; 5],
			name,
			[7u8; 32],
		)
		.unwrap();
		let backer: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&backer);
		let backing_nullifier = [11u8; 32];
		let (back_proof, back_inputs) = backing_proof::<T>(delegate_persona_id, backing_nullifier);
		Pallet::<T>::back_delegate(
			RawOrigin::Signed(backer.clone()).into(),
			delegate.clone(),
			back_proof,
			back_inputs,
		)
		.unwrap();
		let (zk_proof, public_inputs) = backing_proof::<T>(delegate_persona_id, backing_nullifier);

		#[extrinsic_call]
		remove_backing(RawOrigin::Signed(backer.clone()), delegate.clone(), zk_proof, public_inputs);

		assert!(!UsedBackingNullifier::<T>::contains_key(backing_nullifier));
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
