//! Benchmarking setup for pallet-constitution.
//!
//! Real, compiling `#[benchmarks]` scaffolding — sanity-checked against this pallet's own
//! mock via `impl_benchmark_test_suite!` below. See `weights.rs`'s module doc comment for why
//! the numbers currently in `weights.rs` are manual estimates rather than this benchmark's
//! actual output.
//!
//! `submit_petition`/`sign_petition`/`reaffirm_amendment` depend on cross-pallet state
//! (`CitizenChecker`, `FreshLegislatureChecker`) that a generic pallet-constitution benchmark
//! can't set directly — see `Config::BenchmarkHelper` in `lib.rs`. The pallet's own mock wires
//! a real implementation (so these benchmarks run against `crate::mock::Test`), but the
//! runtime-side `RuntimeBenchmarkHelper` in `runtime/src/configs/mod.rs` is currently a no-op:
//! running `benchmark pallet --pallet pallet_constitution` against the real built runtime will
//! still fail those three extrinsics until pallet-identity-zk/pallet-elections grow an
//! equivalent benchmark-only hook. Documented rather than silently broken.

use super::*;
use codec::Encode;
use frame_benchmarking::v2::*;
use frame_support::traits::{EnsureOriginWithArg, Get};
use frame_system::pallet_prelude::BlockNumberFor;
use frame_system::RawOrigin;
use sp_runtime::traits::Saturating;

#[allow(unused)]
use crate::Pallet as Constitution;

/// Builds a `LegislatureOrigin` for a call that requires `required_pct`% passage — the same
/// value the real call site would compute via `required_threshold` from the call's actual tier.
fn legislature_origin<T: Config>(
	tag: &'static [u8],
	params: impl Encode,
	required_pct: u8,
) -> T::RuntimeOrigin {
	let hash = legislature_call_hash(tag, params);
	T::LegislatureOrigin::try_successful_origin(&(hash, required_pct)).unwrap()
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn enact_law() {
		let content_hash = [1u8; 32];
		let origin = legislature_origin::<T>(
			b"pallet-constitution::enact_law",
			(LawTier::Ordinary, content_hash),
			T::OrdinaryPassageThreshold::get(),
		);

		#[extrinsic_call]
		enact_law(origin, LawTier::Ordinary, content_hash);

		assert_eq!(NextLawId::<T>::get(), 1);
	}

	#[benchmark]
	fn invalidate_law() {
		Laws::<T>::insert(0u32, (LawTier::Ordinary, LawStatus::Active, 1u32, [2u8; 32]));
		let hash = legislature_call_hash(b"pallet-constitution::invalidate_law", 0u32);
		let origin = T::CourtOrigin::try_successful_origin(&hash).unwrap();

		#[extrinsic_call]
		invalidate_law(origin, 0u32);

		assert_eq!(Laws::<T>::get(0u32).unwrap().1, LawStatus::Paused);
	}

	#[benchmark]
	fn propose_amendment() {
		Laws::<T>::insert(0u32, (LawTier::Ordinary, LawStatus::Active, 1u32, [3u8; 32]));
		let proposed_hash = [4u8; 32];
		let origin = legislature_origin::<T>(
			b"pallet-constitution::propose_amendment",
			(0u32, proposed_hash),
			T::OrdinaryPassageThreshold::get(),
		);

		#[extrinsic_call]
		propose_amendment(origin, 0u32, proposed_hash);

		assert!(PendingAmendments::<T>::contains_key(0u32));
	}

	#[benchmark]
	fn ratify_amendment() {
		Laws::<T>::insert(0u32, (LawTier::Ordinary, LawStatus::Active, 1u32, [5u8; 32]));
		PendingAmendments::<T>::insert(0u32, ([6u8; 32], frame_system::Pallet::<T>::block_number()));
		let deliberation = BlockNumberFor::<T>::from(T::OrdinaryAmendmentDeliberationBlocks::get());
		frame_system::Pallet::<T>::set_block_number(
			frame_system::Pallet::<T>::block_number().saturating_add(deliberation),
		);
		let origin = legislature_origin::<T>(
			b"pallet-constitution::ratify_amendment",
			0u32,
			T::OrdinaryPassageThreshold::get(),
		);

		#[extrinsic_call]
		ratify_amendment(origin, 0u32);

		assert_eq!(Laws::<T>::get(0u32).unwrap().2, 2u32);
	}

	#[benchmark]
	fn submit_petition() {
		let caller: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&caller);
		let topic_hash = [7u8; 32];

		#[extrinsic_call]
		submit_petition(RawOrigin::Signed(caller), topic_hash);

		assert_eq!(NextPetitionId::<T>::get(), 1);
	}

	#[benchmark]
	fn sign_petition() {
		let proposer: T::AccountId = account("proposer", 0, 0);
		T::BenchmarkHelper::make_active_citizen(&proposer);
		Pallet::<T>::submit_petition(RawOrigin::Signed(proposer).into(), [8u8; 32]).unwrap();

		let signer: T::AccountId = whitelisted_caller();
		T::BenchmarkHelper::make_active_citizen(&signer);

		#[extrinsic_call]
		sign_petition(RawOrigin::Signed(signer.clone()), 0u32);

		assert!(PetitionSignatures::<T>::get((0u32, &signer)));
	}

	#[benchmark]
	fn repeal_law() {
		Laws::<T>::insert(0u32, (LawTier::Ordinary, LawStatus::Active, 1u32, [9u8; 32]));
		let origin = legislature_origin::<T>(
			b"pallet-constitution::repeal_law",
			0u32,
			T::OrdinaryPassageThreshold::get(),
		);

		#[extrinsic_call]
		repeal_law(origin, 0u32);

		assert_eq!(Laws::<T>::get(0u32).unwrap().1, LawStatus::Repealed);
	}

	#[benchmark]
	fn propose_constitutional_amendment() {
		Laws::<T>::insert(0u32, (LawTier::Structural, LawStatus::Active, 1u32, [10u8; 32]));
		let new_hash = [11u8; 32];
		let origin = legislature_origin::<T>(
			b"pallet-constitution::propose_constitutional_amendment",
			(0u32, new_hash),
			T::ConstitutionalPassageThreshold::get(),
		);

		#[extrinsic_call]
		propose_constitutional_amendment(origin, 0u32, new_hash);

		assert!(ConstitutionalAmendments::<T>::contains_key(0u32));
	}

	#[benchmark]
	fn reaffirm_amendment() {
		Laws::<T>::insert(0u32, (LawTier::Structural, LawStatus::Active, 2u32, [12u8; 32]));
		ConstitutionalAmendments::<T>::insert(0u32, ConstitutionalAmendmentRecord::<T> {
			previous_hash: [13u8; 32],
			previous_version: 1u32,
			new_hash: [12u8; 32],
			proposed_at: frame_system::Pallet::<T>::block_number(),
			stage: MaturityStage::Provisional,
			legislature_reaffirmed: false,
		});
		let provisioning = BlockNumberFor::<T>::from(T::ProvisioningPeriodBlocks::get());
		frame_system::Pallet::<T>::set_block_number(
			frame_system::Pallet::<T>::block_number().saturating_add(provisioning),
		);
		T::BenchmarkHelper::make_legislature_fresh();
		let origin = legislature_origin::<T>(
			b"pallet-constitution::reaffirm_amendment",
			0u32,
			T::ConstitutionalPassageThreshold::get(),
		);

		#[extrinsic_call]
		reaffirm_amendment(origin, 0u32);

		assert_eq!(
			ConstitutionalAmendments::<T>::get(0u32).unwrap().stage,
			MaturityStage::Confirmed
		);
	}

	#[benchmark]
	fn advance_to_entrenched() {
		Laws::<T>::insert(0u32, (LawTier::Structural, LawStatus::Active, 2u32, [14u8; 32]));
		ConstitutionalAmendments::<T>::insert(0u32, ConstitutionalAmendmentRecord::<T> {
			previous_hash: [15u8; 32],
			previous_version: 1u32,
			new_hash: [14u8; 32],
			proposed_at: frame_system::Pallet::<T>::block_number(),
			stage: MaturityStage::Confirmed,
			legislature_reaffirmed: true,
		});
		let total = BlockNumberFor::<T>::from(
			T::ProvisioningPeriodBlocks::get().saturating_add(T::ConfirmationPeriodBlocks::get()),
		);
		frame_system::Pallet::<T>::set_block_number(
			frame_system::Pallet::<T>::block_number().saturating_add(total),
		);
		let caller: T::AccountId = whitelisted_caller();

		#[extrinsic_call]
		advance_to_entrenched(RawOrigin::Signed(caller), 0u32);

		assert_eq!(
			ConstitutionalAmendments::<T>::get(0u32).unwrap().stage,
			MaturityStage::Entrenched
		);
	}

	#[benchmark]
	fn revoke_amendment() {
		Laws::<T>::insert(0u32, (LawTier::Structural, LawStatus::Active, 2u32, [16u8; 32]));
		ConstitutionalAmendments::<T>::insert(0u32, ConstitutionalAmendmentRecord::<T> {
			previous_hash: [17u8; 32],
			previous_version: 1u32,
			new_hash: [16u8; 32],
			proposed_at: frame_system::Pallet::<T>::block_number(),
			stage: MaturityStage::Provisional,
			legislature_reaffirmed: false,
		});
		let origin = T::RevocationOrigin::try_successful_origin(&crate::pallet::revocation_threshold_for_stage(
			&MaturityStage::Provisional,
		))
		.unwrap();

		#[extrinsic_call]
		revoke_amendment(origin, 0u32);

		assert_eq!(Laws::<T>::get(0u32).unwrap().3, [17u8; 32]);
	}

	impl_benchmark_test_suite!(Constitution, crate::mock::new_test_ext(), crate::mock::Test);
}
