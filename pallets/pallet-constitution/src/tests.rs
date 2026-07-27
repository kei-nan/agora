use crate::{
	mock::*, ConstitutionalAmendments, Error, Event, LawStatus, LawTier, Laws, MaturityStage,
	NextLawId, NextPetitionId, PendingAmendments, PetitionSignatures, Petitions,
};
use frame_support::{assert_noop, assert_ok};
use sp_runtime::DispatchError;

fn h(n: u8) -> [u8; 32] {
	[n; 32]
}

/// Enacts a law of the given tier with hash `h(seed)` via root and returns its law_id.
fn enact(tier: LawTier, seed: u8) -> u32 {
	let id = NextLawId::<Test>::get();
	assert_ok!(Constitution::enact_law(RuntimeOrigin::root(), tier, h(seed)));
	id
}

// ── enact_law ────────────────────────────────────────────────────────────────

#[test]
fn enact_law_works_and_deposits_event() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		assert_ok!(Constitution::enact_law(RuntimeOrigin::root(), LawTier::Ordinary, h(1)));
		assert_eq!(Laws::<Test>::get(0), Some((LawTier::Ordinary, LawStatus::Active, 1, h(1))));
		assert_eq!(NextLawId::<Test>::get(), 1);
		System::assert_last_event(
			Event::LawEnacted { law_id: 0, tier: LawTier::Ordinary, content_hash: h(1) }.into(),
		);
	});
}

#[test]
fn enact_law_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::enact_law(RuntimeOrigin::signed(1), LawTier::Ordinary, h(1)),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn enact_law_ordinary_does_not_fire_auto_challenge() {
	new_test_ext().execute_with(|| {
		enact(LawTier::Ordinary, 1);
		assert_eq!(auto_challenges(), Vec::<u32>::new());
	});
}

#[test]
fn enact_law_structural_fires_auto_challenge() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_eq!(auto_challenges(), vec![id]);
	});
}

#[test]
fn enact_law_foundational_fires_auto_challenge() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Foundational, 1);
		assert_eq!(auto_challenges(), vec![id]);
	});
}

// ── invalidate_law ───────────────────────────────────────────────────────────

#[test]
fn invalidate_law_works() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::invalidate_law(RuntimeOrigin::root(), id));
		assert_eq!(Laws::<Test>::get(id).unwrap().1, LawStatus::Paused);
		System::assert_last_event(Event::LawInvalidated { law_id: id }.into());
	});
}

#[test]
fn invalidate_law_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_noop!(
			Constitution::invalidate_law(RuntimeOrigin::signed(1), id),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn invalidate_law_fails_law_not_found() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::invalidate_law(RuntimeOrigin::root(), 999),
			Error::<Test>::LawNotFound
		);
	});
}

#[test]
fn invalidate_law_fails_when_already_paused() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::invalidate_law(RuntimeOrigin::root(), id));
		assert_noop!(
			Constitution::invalidate_law(RuntimeOrigin::root(), id),
			Error::<Test>::LawNotActive
		);
	});
}

// ── propose_amendment / ratify_amendment (Ordinary) ─────────────────────────

#[test]
fn propose_amendment_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_noop!(
			Constitution::propose_amendment(RuntimeOrigin::signed(1), id, h(2)),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn propose_amendment_fails_law_not_found() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::propose_amendment(RuntimeOrigin::root(), 999, h(2)),
			Error::<Test>::LawNotFound
		);
	});
}

#[test]
fn propose_amendment_fails_law_not_active() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::invalidate_law(RuntimeOrigin::root(), id));
		assert_noop!(
			Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)),
			Error::<Test>::LawNotActive
		);
	});
}

#[test]
fn propose_amendment_fails_for_non_ordinary_tier() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_noop!(
			Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)),
			Error::<Test>::UseConstitutionalAmendmentCall
		);
	});
}

#[test]
fn propose_amendment_works_and_deposits_event() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_eq!(PendingAmendments::<Test>::get(id), Some((h(2), 1)));
		System::assert_last_event(
			Event::AmendmentProposed { law_id: id, proposed_hash: h(2) }.into(),
		);
	});
}

#[test]
fn propose_amendment_fails_when_already_pending() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_noop!(
			Constitution::propose_amendment(RuntimeOrigin::root(), id, h(3)),
			Error::<Test>::AmendmentAlreadyPending
		);
	});
}

#[test]
fn ratify_amendment_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_noop!(
			Constitution::ratify_amendment(RuntimeOrigin::signed(1), id),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn ratify_amendment_fails_amendment_not_found() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_noop!(
			Constitution::ratify_amendment(RuntimeOrigin::root(), id),
			Error::<Test>::AmendmentNotFound
		);
	});
}

#[test]
fn ratify_amendment_fails_before_deliberation_elapsed() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)));
		// ORDINARY_DELIBERATION_BLOCKS == 5; proposed at block 1, so blocks 1..5 must fail.
		System::set_block_number(1 + ORDINARY_DELIBERATION_BLOCKS as u64 - 1);
		assert_noop!(
			Constitution::ratify_amendment(RuntimeOrigin::root(), id),
			Error::<Test>::DeliberationPeriodActive
		);
	});
}

#[test]
fn ratify_amendment_works_after_deliberation_elapsed() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)));
		System::set_block_number(1 + ORDINARY_DELIBERATION_BLOCKS as u64);
		assert_ok!(Constitution::ratify_amendment(RuntimeOrigin::root(), id));
		assert_eq!(Laws::<Test>::get(id), Some((LawTier::Ordinary, LawStatus::Active, 2, h(2))));
		assert_eq!(PendingAmendments::<Test>::get(id), None);
		System::assert_last_event(Event::AmendmentRatified { law_id: id, new_hash: h(2) }.into());
	});
}

#[test]
fn ratify_amendment_fails_when_law_paused_after_proposal() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::propose_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_ok!(Constitution::invalidate_law(RuntimeOrigin::root(), id));
		System::set_block_number(1 + ORDINARY_DELIBERATION_BLOCKS as u64);
		assert_noop!(
			Constitution::ratify_amendment(RuntimeOrigin::root(), id),
			Error::<Test>::LawNotActive
		);
	});
}

// ── repeal_law ───────────────────────────────────────────────────────────────

#[test]
fn repeal_law_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_noop!(
			Constitution::repeal_law(RuntimeOrigin::signed(1), id),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn repeal_law_fails_law_not_found() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::repeal_law(RuntimeOrigin::root(), 999),
			Error::<Test>::LawNotFound
		);
	});
}

#[test]
fn repeal_law_fails_when_already_repealed() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::repeal_law(RuntimeOrigin::root(), id));
		assert_noop!(
			Constitution::repeal_law(RuntimeOrigin::root(), id),
			Error::<Test>::LawAlreadyRepealed
		);
	});
}

#[test]
fn repeal_law_works_and_cleans_up_pending_amendments() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let ordinary_id = enact(LawTier::Ordinary, 1);
		assert_ok!(Constitution::propose_amendment(RuntimeOrigin::root(), ordinary_id, h(2)));

		let structural_id = enact(LawTier::Structural, 3);
		assert_ok!(Constitution::propose_constitutional_amendment(
			RuntimeOrigin::root(),
			structural_id,
			h(4)
		));

		assert_ok!(Constitution::repeal_law(RuntimeOrigin::root(), ordinary_id));
		assert_eq!(Laws::<Test>::get(ordinary_id).unwrap().1, LawStatus::Repealed);
		assert_eq!(PendingAmendments::<Test>::get(ordinary_id), None);
		System::assert_last_event(Event::LawRepealed { law_id: ordinary_id }.into());

		assert_ok!(Constitution::repeal_law(RuntimeOrigin::root(), structural_id));
		assert_eq!(ConstitutionalAmendments::<Test>::get(structural_id), None);
	});
}

// ── petitions: submit_petition / sign_petition ──────────────────────────────

#[test]
fn submit_petition_fails_for_inactive_citizen() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::submit_petition(RuntimeOrigin::signed(1), h(1)),
			Error::<Test>::CitizenNotActive
		);
	});
}

#[test]
fn submit_petition_works_proposer_counted_as_first_signer() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		set_active_citizens(vec![1]);
		assert_ok!(Constitution::submit_petition(RuntimeOrigin::signed(1), h(1)));
		assert_eq!(Petitions::<Test>::get(0), Some((1, h(1), 1, 1)));
		assert!(PetitionSignatures::<Test>::get((0, 1)));
		assert_eq!(NextPetitionId::<Test>::get(), 1);
		System::assert_has_event(
			Event::PetitionSubmitted { petition_id: 0, proposer: 1, topic_hash: h(1) }.into(),
		);
		System::assert_last_event(
			Event::PetitionSigned { petition_id: 0, signer: 1, signature_count: 1 }.into(),
		);
		// PETITION_THRESHOLD == 3, so a lone proposer signature must not cross it.
		assert_eq!(referenda_created(), Vec::<(u32, [u8; 32])>::new());
	});
}

#[test]
fn sign_petition_fails_for_inactive_citizen() {
	new_test_ext().execute_with(|| {
		set_active_citizens(vec![1]);
		assert_ok!(Constitution::submit_petition(RuntimeOrigin::signed(1), h(1)));
		assert_noop!(
			Constitution::sign_petition(RuntimeOrigin::signed(2), 0),
			Error::<Test>::CitizenNotActive
		);
	});
}

#[test]
fn sign_petition_fails_petition_not_found() {
	new_test_ext().execute_with(|| {
		set_active_citizens(vec![1]);
		assert_noop!(
			Constitution::sign_petition(RuntimeOrigin::signed(1), 999),
			Error::<Test>::PetitionNotFound
		);
	});
}

#[test]
fn sign_petition_fails_when_already_signed() {
	new_test_ext().execute_with(|| {
		set_active_citizens(vec![1]);
		assert_ok!(Constitution::submit_petition(RuntimeOrigin::signed(1), h(1)));
		// Proposer is auto-recorded as first signer; signing again must be rejected.
		assert_noop!(
			Constitution::sign_petition(RuntimeOrigin::signed(1), 0),
			Error::<Test>::AlreadySigned
		);
	});
}

#[test]
fn sign_petition_works_and_increments_count() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		set_active_citizens(vec![1, 2]);
		assert_ok!(Constitution::submit_petition(RuntimeOrigin::signed(1), h(1)));
		assert_ok!(Constitution::sign_petition(RuntimeOrigin::signed(2), 0));
		assert_eq!(Petitions::<Test>::get(0).unwrap().2, 2);
		System::assert_last_event(
			Event::PetitionSigned { petition_id: 0, signer: 2, signature_count: 2 }.into(),
		);
	});
}

#[test]
fn sign_petition_crossing_threshold_creates_referendum() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		set_active_citizens(vec![1, 2, 3]);
		assert_ok!(Constitution::submit_petition(RuntimeOrigin::signed(1), h(7)));
		assert_ok!(Constitution::sign_petition(RuntimeOrigin::signed(2), 0));
		// PETITION_THRESHOLD == 3: proposer (1) + signer 2 = 2, third signer crosses it.
		assert_eq!(referenda_created(), Vec::<(u32, [u8; 32])>::new());
		assert_ok!(Constitution::sign_petition(RuntimeOrigin::signed(3), 0));
		assert_eq!(Petitions::<Test>::get(0).unwrap().2, 3);
		System::assert_has_event(
			Event::PetitionThresholdReached { petition_id: 0, topic_hash: h(7) }.into(),
		);
		assert_eq!(referenda_created(), vec![(0, h(7))]);
	});
}

// ── propose_constitutional_amendment ────────────────────────────────────────

#[test]
fn propose_constitutional_amendment_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_noop!(
			Constitution::propose_constitutional_amendment(RuntimeOrigin::signed(1), id, h(2)),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn propose_constitutional_amendment_fails_law_not_found() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), 999, h(2)),
			Error::<Test>::LawNotFound
		);
	});
}

#[test]
fn propose_constitutional_amendment_fails_law_not_active() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::invalidate_law(RuntimeOrigin::root(), id));
		assert_noop!(
			Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)),
			Error::<Test>::LawNotActive
		);
	});
}

#[test]
fn propose_constitutional_amendment_fails_for_ordinary_tier() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Ordinary, 1);
		assert_noop!(
			Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)),
			Error::<Test>::UseOrdinaryAmendmentCall
		);
	});
}

#[test]
fn propose_constitutional_amendment_works_applies_hash_immediately() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		// Hash and version applied immediately, even though the amendment is only Provisional.
		assert_eq!(Laws::<Test>::get(id), Some((LawTier::Structural, LawStatus::Active, 2, h(2))));
		let record = ConstitutionalAmendments::<Test>::get(id).unwrap();
		assert_eq!(record.previous_hash, h(1));
		assert_eq!(record.previous_version, 1);
		assert_eq!(record.new_hash, h(2));
		assert_eq!(record.proposed_at, 1);
		assert_eq!(record.stage, MaturityStage::Provisional);
		assert!(!record.legislature_reaffirmed);
		System::assert_last_event(
			Event::ConstitutionalAmendmentProposed {
				law_id: id,
				new_hash: h(2),
				tier: LawTier::Structural,
			}
			.into(),
		);
	});
}

#[test]
fn propose_constitutional_amendment_fails_when_already_pending() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_noop!(
			Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(3)),
			Error::<Test>::ConstitutionalAmendmentAlreadyPending
		);
	});
}

// ── reaffirm_amendment (Provisional → Confirmed) ────────────────────────────

#[test]
fn reaffirm_amendment_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_noop!(
			Constitution::reaffirm_amendment(RuntimeOrigin::signed(1), id),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn reaffirm_amendment_fails_amendment_not_found() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_noop!(
			Constitution::reaffirm_amendment(RuntimeOrigin::root(), id),
			Error::<Test>::ConstitutionalAmendmentNotFound
		);
	});
}

#[test]
fn reaffirm_amendment_fails_before_provisioning_period_elapsed() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		set_fresh_legislature(true);
		// PROVISIONING_PERIOD_BLOCKS == 10; proposed at block 1, so block 10 must still fail.
		System::set_block_number(1 + PROVISIONING_PERIOD_BLOCKS as u64 - 1);
		assert_noop!(
			Constitution::reaffirm_amendment(RuntimeOrigin::root(), id),
			Error::<Test>::ProvisioningPeriodNotElapsed
		);
	});
}

#[test]
fn reaffirm_amendment_fails_when_legislature_not_fresh() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		// Provisioning period has elapsed, but no fresh election has occurred.
		System::set_block_number(1 + PROVISIONING_PERIOD_BLOCKS as u64);
		assert_noop!(
			Constitution::reaffirm_amendment(RuntimeOrigin::root(), id),
			Error::<Test>::LegislatureNotFresh
		);
	});
}

#[test]
fn reaffirm_amendment_works_after_period_elapsed_and_fresh_legislature() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		set_fresh_legislature(true);
		System::set_block_number(1 + PROVISIONING_PERIOD_BLOCKS as u64);
		assert_ok!(Constitution::reaffirm_amendment(RuntimeOrigin::root(), id));
		let record = ConstitutionalAmendments::<Test>::get(id).unwrap();
		assert_eq!(record.stage, MaturityStage::Confirmed);
		assert!(record.legislature_reaffirmed);
		System::assert_last_event(Event::AmendmentReaffirmed { law_id: id }.into());
	});
}

#[test]
fn reaffirm_amendment_fails_when_not_provisional() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		set_fresh_legislature(true);
		System::set_block_number(1 + PROVISIONING_PERIOD_BLOCKS as u64);
		assert_ok!(Constitution::reaffirm_amendment(RuntimeOrigin::root(), id));
		// Already Confirmed — a second reaffirm must fail with AmendmentNotProvisional
		// (the stage guard is checked before the already-reaffirmed guard).
		assert_noop!(
			Constitution::reaffirm_amendment(RuntimeOrigin::root(), id),
			Error::<Test>::AmendmentNotProvisional
		);
	});
}

// ── advance_to_entrenched (Confirmed → Entrenched) ──────────────────────────

#[test]
fn advance_to_entrenched_fails_amendment_not_found() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::advance_to_entrenched(RuntimeOrigin::signed(1), 999),
			Error::<Test>::ConstitutionalAmendmentNotFound
		);
	});
}

#[test]
fn advance_to_entrenched_fails_when_still_provisional() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		// Even after the full pipeline duration, an amendment that was never reaffirmed
		// (still Provisional) must not be advanceable.
		System::set_block_number(
			1 + PROVISIONING_PERIOD_BLOCKS as u64 + CONFIRMATION_PERIOD_BLOCKS as u64,
		);
		assert_noop!(
			Constitution::advance_to_entrenched(RuntimeOrigin::signed(1), id),
			Error::<Test>::AmendmentNotConfirmed
		);
	});
}

#[test]
fn advance_to_entrenched_fails_before_confirmation_period_elapsed() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		set_fresh_legislature(true);
		System::set_block_number(1 + PROVISIONING_PERIOD_BLOCKS as u64);
		assert_ok!(Constitution::reaffirm_amendment(RuntimeOrigin::root(), id));
		// One block short of the full pipeline (provisioning + confirmation).
		System::set_block_number(
			1 + PROVISIONING_PERIOD_BLOCKS as u64 + CONFIRMATION_PERIOD_BLOCKS as u64 - 1,
		);
		assert_noop!(
			Constitution::advance_to_entrenched(RuntimeOrigin::signed(1), id),
			Error::<Test>::ConfirmationPeriodNotElapsed
		);
	});
}

#[test]
fn advance_to_entrenched_works_after_full_pipeline_elapsed() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		set_fresh_legislature(true);
		System::set_block_number(1 + PROVISIONING_PERIOD_BLOCKS as u64);
		assert_ok!(Constitution::reaffirm_amendment(RuntimeOrigin::root(), id));
		System::set_block_number(
			1 + PROVISIONING_PERIOD_BLOCKS as u64 + CONFIRMATION_PERIOD_BLOCKS as u64,
		);
		// Permissionless: any signed account may call this.
		assert_ok!(Constitution::advance_to_entrenched(RuntimeOrigin::signed(42), id));
		let record = ConstitutionalAmendments::<Test>::get(id).unwrap();
		assert_eq!(record.stage, MaturityStage::Entrenched);
		System::assert_last_event(Event::AmendmentAdvancedToEntrenched { law_id: id }.into());
	});
}

// ── revoke_amendment ─────────────────────────────────────────────────────────

#[test]
fn revoke_amendment_fails_for_unauthorized_origin() {
	new_test_ext().execute_with(|| {
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_noop!(
			Constitution::revoke_amendment(RuntimeOrigin::signed(1), id),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn revoke_amendment_fails_amendment_not_found() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Constitution::revoke_amendment(RuntimeOrigin::root(), 999),
			Error::<Test>::ConstitutionalAmendmentNotFound
		);
	});
}

#[test]
fn revoke_amendment_works_at_provisional_stage_and_restores_law() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_eq!(Laws::<Test>::get(id), Some((LawTier::Structural, LawStatus::Active, 2, h(2))));

		assert_ok!(Constitution::revoke_amendment(RuntimeOrigin::root(), id));

		assert_eq!(Laws::<Test>::get(id), Some((LawTier::Structural, LawStatus::Active, 1, h(1))));
		assert_eq!(ConstitutionalAmendments::<Test>::get(id), None);
		System::assert_last_event(
			Event::AmendmentRevoked { law_id: id, restored_hash: h(1) }.into(),
		);
	});
}

#[test]
fn revoke_amendment_works_at_confirmed_stage_and_restores_law() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		set_fresh_legislature(true);
		System::set_block_number(1 + PROVISIONING_PERIOD_BLOCKS as u64);
		assert_ok!(Constitution::reaffirm_amendment(RuntimeOrigin::root(), id));

		assert_ok!(Constitution::revoke_amendment(RuntimeOrigin::root(), id));

		assert_eq!(Laws::<Test>::get(id), Some((LawTier::Structural, LawStatus::Active, 1, h(1))));
		assert_eq!(ConstitutionalAmendments::<Test>::get(id), None);
	});
}

#[test]
fn revoke_amendment_can_be_called_again_after_new_proposal() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1);
		let id = enact(LawTier::Structural, 1);
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(2)));
		assert_ok!(Constitution::revoke_amendment(RuntimeOrigin::root(), id));
		// After revocation the pipeline slot is free again for a new proposal.
		assert_ok!(Constitution::propose_constitutional_amendment(RuntimeOrigin::root(), id, h(3)));
		assert_eq!(Laws::<Test>::get(id), Some((LawTier::Structural, LawStatus::Active, 2, h(3))));
	});
}
