//! Benchmarking setup for pallet-accountability-council.
//!
//! Real, compiling `#[benchmarks]` scaffolding, mirroring
//! `pallets/pallet-emergency-council/src/benchmarking.rs`'s structure. See `weights.rs`'s module
//! doc comment for why the numbers currently in that file are manual estimates rather than this
//! benchmark's actual output, and for the one gap (the unbounded `PendingAction::translate` scan
//! in `remove_member`) this scaffolding does not attempt to price at all.

use super::*;
use alloc::vec::Vec;
use frame_benchmarking::v2::*;
use frame_support::traits::Get;
use frame_system::{pallet_prelude::BlockNumberFor, RawOrigin};
use sp_runtime::traits::Saturating;

#[allow(unused)]
use crate::Pallet as AccountabilityCouncil;

fn seed_accounts<T: Config>(n: u32) -> Vec<T::AccountId> {
	(0..n).map(|i| -> T::AccountId { account("council", i, 0) }).collect()
}

/// Adds `n` fresh accounts as Council members via the Root pre-bootstrap path. Leaves
/// `Bootstrapped == false`, so `propose_action`/`approve_action`/`clear_stale_action` (which
/// don't care about bootstrap status at all) can be benchmarked without the extra cost of
/// closing bootstrap first.
fn seed_members<T: Config>(n: u32) -> Vec<T::AccountId> {
	let members = seed_accounts::<T>(n);
	for m in &members {
		Pallet::<T>::add_member(RawOrigin::Root.into(), m.clone()).unwrap();
	}
	members
}

/// Same as `seed_members`, but also closes bootstrap — for `add_member`/`remove_member`
/// benchmarks, which need `Bootstrapped == true` to exercise their costlier post-bootstrap path
/// (an `EnsureAccountabilityCouncilApproved` token consumption instead of a bare Root check),
/// matching the worst case `weights.rs` prices for both calls.
fn bootstrap_and_close<T: Config>(n: u32) -> Vec<T::AccountId> {
	let members = seed_members::<T>(n);
	Pallet::<T>::close_bootstrap(RawOrigin::Root.into()).unwrap();
	members
}

/// Minimum approval count needed to cross the configured supermajority threshold for a council
/// of `council_size`, mirroring the (private, so not directly callable from here)
/// `Pallet::supermajority_reached` formula: `approvals * denominator >= council_size *
/// numerator`. Mirrors `pallet_emergency_council`'s benchmarking helper of the same name.
fn votes_needed<T: Config>(council_size: u32) -> usize {
	let numerator = T::SupermajorityNumerator::get() as u64;
	let denominator = T::SupermajorityDenominator::get().max(1) as u64;
	let size = council_size as u64;
	size.saturating_mul(numerator).div_ceil(denominator).max(1) as usize
}

/// Drives `call_hash` from unproposed to `ApprovedAction` using `members` (in order), stopping
/// as soon as it resolves. Assumes `members.len() as u32 >= votes_needed::<T>(members.len() as
/// u32)`, which holds for any non-degenerate council (true for both the mock and the runtime's
/// configured 9-member Council).
fn resolve_action<T: Config>(members: &[T::AccountId], call_hash: [u8; 32]) {
	let needed = votes_needed::<T>(members.len() as u32);
	for m in members.iter().take(needed) {
		if PendingAction::<T>::get(call_hash).is_none() && ApprovedAction::<T>::get(call_hash).is_none() {
			Pallet::<T>::propose_action(RawOrigin::Signed(m.clone()).into(), call_hash).unwrap();
		} else if ApprovedAction::<T>::get(call_hash).is_none() {
			Pallet::<T>::approve_action(RawOrigin::Signed(m.clone()).into(), call_hash).unwrap();
		}
	}
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn add_member() {
		let max = T::MaxCouncilSize::get();
		let members = bootstrap_and_close::<T>(max.saturating_sub(1));
		let new_member: T::AccountId = account("new_member", 0, 0);
		let call_hash =
			crate::accountability_call_hash(b"pallet-accountability-council::add_member", &new_member);
		resolve_action::<T>(&members, call_hash);
		assert!(ApprovedAction::<T>::get(call_hash).is_some());
		let caller = members[0].clone();

		#[extrinsic_call]
		add_member(RawOrigin::Signed(caller), new_member.clone());

		assert!(Members::<T>::get().contains(&new_member));
	}

	#[benchmark]
	fn remove_member() {
		let max = T::MaxCouncilSize::get();
		let members = bootstrap_and_close::<T>(max);
		let target = members.last().unwrap().clone();
		let call_hash = crate::accountability_call_hash(
			b"pallet-accountability-council::remove_member",
			&target,
		);
		resolve_action::<T>(&members, call_hash);
		assert!(ApprovedAction::<T>::get(call_hash).is_some());

		// Also seed one unrelated pending action for the removal's purge/re-resolution loop to
		// touch, so the benchmarked call exercises that path too — not the full
		// `MAX_ACTIONS_RERESOLVED_PER_REMOVAL` (20) worst case, which `weights.rs`'s doc comment
		// already documents as unpriced by a real benchmark run here.
		let other_hash = [9u8; 32];
		Pallet::<T>::propose_action(RawOrigin::Signed(members[0].clone()).into(), other_hash)
			.unwrap();

		let caller = members[0].clone();

		#[extrinsic_call]
		remove_member(RawOrigin::Signed(caller), target.clone());

		assert!(!Members::<T>::get().contains(&target));
	}

	#[benchmark]
	fn close_bootstrap() {
		let members = seed_members::<T>(1);
		let _ = members;

		#[extrinsic_call]
		close_bootstrap(RawOrigin::Root);

		assert!(Bootstrapped::<T>::get());
	}

	#[benchmark]
	fn propose_action() {
		// A 1-member council resolves on the proposer's own vote — the more expensive
		// (immediately-resolved) path `weights.rs` prices for this call.
		let members = seed_members::<T>(1);
		let call_hash = [1u8; 32];

		#[extrinsic_call]
		propose_action(RawOrigin::Signed(members[0].clone()), call_hash);

		assert!(ApprovedAction::<T>::get(call_hash).is_some());
	}

	#[benchmark]
	fn approve_action() {
		let max = T::MaxCouncilSize::get();
		let members = seed_members::<T>(max);
		let call_hash = [2u8; 32];
		let needed = votes_needed::<T>(max);
		Pallet::<T>::propose_action(RawOrigin::Signed(members[0].clone()).into(), call_hash)
			.unwrap();
		// Cast (needed - 2) more approvals so the benchmarked call is the one that crosses the
		// supermajority threshold — the more expensive path (it also moves the action to
		// `ApprovedAction`).
		for m in members.iter().skip(1).take(needed.saturating_sub(2)) {
			Pallet::<T>::approve_action(RawOrigin::Signed(m.clone()).into(), call_hash).unwrap();
		}
		let voter = members[needed.saturating_sub(1).max(1)].clone();

		#[extrinsic_call]
		approve_action(RawOrigin::Signed(voter), call_hash);

		assert!(ApprovedAction::<T>::get(call_hash).is_some());
	}

	#[benchmark]
	fn clear_stale_action() {
		let members = seed_members::<T>(T::MaxCouncilSize::get());
		let call_hash = [3u8; 32];
		resolve_action::<T>(&members, call_hash);
		assert!(ApprovedAction::<T>::get(call_hash).is_some());
		let expiry = T::ApprovalExpiryBlocks::get();
		let now = frame_system::Pallet::<T>::block_number();
		frame_system::Pallet::<T>::set_block_number(
			now.saturating_add(BlockNumberFor::<T>::from(expiry)).saturating_add(1u32.into()),
		);

		#[extrinsic_call]
		clear_stale_action(RawOrigin::Signed(members[0].clone()), call_hash);

		assert!(ApprovedAction::<T>::get(call_hash).is_none());
	}

	impl_benchmark_test_suite!(AccountabilityCouncil, crate::mock::new_test_ext(), crate::mock::Test);
}
