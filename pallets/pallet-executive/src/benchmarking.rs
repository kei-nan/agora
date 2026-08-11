//! Benchmarking setup for pallet-executive.
//!
//! Real, compiling `#[benchmarks]` scaffolding. See `weights.rs`'s module doc comment for why
//! the numbers currently in that file are manual estimates rather than this benchmark's actual
//! output.

use super::*;
use alloc::vec::Vec;
use codec::Encode;
use frame_benchmarking::v2::*;
use frame_support::traits::{EnsureOriginWithArg, Get};
use frame_system::pallet_prelude::BlockNumberFor;
use frame_system::RawOrigin;
use sp_runtime::traits::Saturating;

#[allow(unused)]
use crate::Pallet as Cabinet;

/// Obtains a `LegislatureOrigin`-authorized origin for `tag`'s call with `params`, the same way
/// `EnsureLegislatureMotion::try_successful_origin` (pallet-legislature) or this pallet's own
/// mock's `AsEnsureOriginWithArg<EnsureRoot<_>>` do under `runtime-benchmarks`.
fn legislature_origin<T: Config>(tag: &'static [u8], params: impl Encode) -> T::RuntimeOrigin {
	let hash = legislature_call_hash(tag, params);
	T::LegislatureOrigin::try_successful_origin(&hash).unwrap()
}

/// Fills `Portfolios`/`PortfolioMinister`/`MinisterPortfolio` with `n` occupied portfolios,
/// returning the minister accounts in portfolio-id order.
fn seed_cabinet<T: Config>(n: u32) -> Vec<T::AccountId> {
	let mut members = Vec::new();
	for i in 0..n {
		let m: T::AccountId = account("minister", i, 0);
		Portfolios::<T>::insert(i, Portfolio { name_hash: [1u8; 32] });
		PortfolioMinister::<T>::insert(i, m.clone());
		MinisterPortfolio::<T>::insert(m.clone(), i);
		members.push(m);
	}
	NextPortfolioId::<T>::put(n);
	members
}

/// Minimum vote count needed to cross the configured supermajority threshold for a cabinet of
/// `cabinet_size`, mirroring the (private, so not directly callable from here)
/// `Pallet::supermajority_reached` formula.
fn votes_needed<T: Config>(cabinet_size: u32) -> usize {
	let numerator = T::SupermajorityNumerator::get() as u64;
	let denominator = T::SupermajorityDenominator::get().max(1) as u64;
	(cabinet_size as u64).saturating_mul(numerator).div_ceil(denominator).max(1) as usize
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn define_portfolio() {
		let name_hash = [1u8; 32];
		let origin = legislature_origin::<T>(b"pallet-executive::define_portfolio", name_hash);

		#[extrinsic_call]
		define_portfolio(origin, name_hash);

		assert_eq!(NextPortfolioId::<T>::get(), 1);
	}

	#[benchmark]
	fn appoint_prime_minister() {
		// Worst case: an outgoing PM is replaced.
		let old_pm: T::AccountId = account("old_pm", 0, 0);
		PrimeMinister::<T>::put(old_pm);
		let new_pm: T::AccountId = account("new_pm", 0, 0);
		let origin = legislature_origin::<T>(
			b"pallet-executive::appoint_prime_minister",
			new_pm.clone(),
		);

		#[extrinsic_call]
		appoint_prime_minister(origin, new_pm.clone());

		assert_eq!(PrimeMinister::<T>::get(), Some(new_pm));
	}

	#[benchmark]
	fn dismiss_prime_minister() {
		let pm: T::AccountId = account("pm", 0, 0);
		PrimeMinister::<T>::put(pm);
		let origin = legislature_origin::<T>(b"pallet-executive::dismiss_prime_minister", ());

		#[extrinsic_call]
		dismiss_prime_minister(origin);

		assert!(PrimeMinister::<T>::get().is_none());
	}

	#[benchmark]
	fn appoint_minister() {
		// Worst case: the target portfolio is occupied *and* the incoming minister already
		// holds a different portfolio, so both must be vacated.
		Portfolios::<T>::insert(0u32, Portfolio { name_hash: [1u8; 32] });
		Portfolios::<T>::insert(1u32, Portfolio { name_hash: [2u8; 32] });
		NextPortfolioId::<T>::put(2u32);

		let old_holder: T::AccountId = account("old_holder", 0, 0);
		PortfolioMinister::<T>::insert(0u32, old_holder.clone());
		MinisterPortfolio::<T>::insert(old_holder, 0u32);

		let incoming: T::AccountId = account("incoming", 0, 0);
		PortfolioMinister::<T>::insert(1u32, incoming.clone());
		MinisterPortfolio::<T>::insert(incoming.clone(), 1u32);

		let origin = legislature_origin::<T>(
			b"pallet-executive::appoint_minister",
			(0u32, incoming.clone()),
		);

		#[extrinsic_call]
		appoint_minister(origin, 0u32, incoming.clone());

		assert_eq!(MinisterPortfolio::<T>::get(&incoming), Some(0u32));
	}

	#[benchmark]
	fn dismiss_minister() {
		Portfolios::<T>::insert(0u32, Portfolio { name_hash: [1u8; 32] });
		let holder: T::AccountId = account("holder", 0, 0);
		PortfolioMinister::<T>::insert(0u32, holder.clone());
		MinisterPortfolio::<T>::insert(holder, 0u32);
		let origin = legislature_origin::<T>(b"pallet-executive::dismiss_minister", 0u32);

		#[extrinsic_call]
		dismiss_minister(origin, 0u32);

		assert!(PortfolioMinister::<T>::get(0u32).is_none());
	}

	#[benchmark]
	fn resign() {
		Portfolios::<T>::insert(0u32, Portfolio { name_hash: [1u8; 32] });
		let minister: T::AccountId = whitelisted_caller();
		PortfolioMinister::<T>::insert(0u32, minister.clone());
		MinisterPortfolio::<T>::insert(minister.clone(), 0u32);

		#[extrinsic_call]
		resign(RawOrigin::Signed(minister.clone()));

		assert!(MinisterPortfolio::<T>::get(&minister).is_none());
	}

	#[benchmark]
	fn vote_declare_emergency() {
		let max = T::MaxPortfolios::get();
		let members = seed_cabinet::<T>(max);
		let needed = votes_needed::<T>(max);

		for member in members.iter().take(needed.saturating_sub(1)) {
			Pallet::<T>::vote_declare_emergency(
				RawOrigin::Signed(member.clone()).into(),
				[5u8; 32],
				10,
			).unwrap();
		}
		let voter = members[needed.saturating_sub(1)].clone();

		#[extrinsic_call]
		vote_declare_emergency(RawOrigin::Signed(voter), [5u8; 32], 10);

		assert!(ActiveEmergency::<T>::get().is_some());
	}

	#[benchmark]
	fn ratify_emergency() {
		let now = frame_system::Pallet::<T>::block_number();
		ActiveEmergency::<T>::put(EmergencyInfo {
			declared_at: now,
			expires_at: now.saturating_add(BlockNumberFor::<T>::from(1_000u32)),
			ratify_by: now.saturating_add(BlockNumberFor::<T>::from(500u32)),
			reason_hash: [6u8; 32],
			ratified: false,
		});
		let origin = legislature_origin::<T>(b"pallet-executive::ratify_emergency", ());

		#[extrinsic_call]
		ratify_emergency(origin);

		assert!(ActiveEmergency::<T>::get().unwrap().ratified);
	}

	#[benchmark]
	fn vote_end_emergency() {
		let max = T::MaxPortfolios::get();
		let members = seed_cabinet::<T>(max);
		let needed = votes_needed::<T>(max);

		for member in members.iter().take(needed) {
			Pallet::<T>::vote_declare_emergency(
				RawOrigin::Signed(member.clone()).into(),
				[7u8; 32],
				10,
			).unwrap();
		}
		assert!(ActiveEmergency::<T>::get().is_some());

		for member in members.iter().take(needed.saturating_sub(1)) {
			Pallet::<T>::vote_end_emergency(RawOrigin::Signed(member.clone()).into()).unwrap();
		}
		let voter = members[needed.saturating_sub(1)].clone();

		#[extrinsic_call]
		vote_end_emergency(RawOrigin::Signed(voter));

		assert!(ActiveEmergency::<T>::get().is_none());
	}

	#[benchmark]
	fn retract_emergency_vote() {
		let max = T::MaxPortfolios::get();
		let members = seed_cabinet::<T>(max);
		let voter = members[0].clone();
		Pallet::<T>::vote_declare_emergency(
			RawOrigin::Signed(voter.clone()).into(),
			[8u8; 32],
			10,
		).unwrap();

		#[extrinsic_call]
		retract_emergency_vote(RawOrigin::Signed(voter.clone()));

		assert!(!DeclareVotes::<T>::get(&voter));
	}

	impl_benchmark_test_suite!(Cabinet, crate::mock::new_test_ext(), crate::mock::Test);
}
