use crate::{
	mock::*,
	pallet::{CaseBonds, CaseStatus, CaseSubject, Error, Event, JuryPool, JuryRequestBlock, Verdict},
};
use frame_support::{assert_noop, assert_ok};
use sp_core::H256;
use sp_runtime::DispatchError;

/// Adds a single AI Model Governance Council member (account 100, otherwise unused in these
/// tests) and votes to approve `model_hash`. A 1-member council trivially satisfies the 2/3
/// supermajority threshold configured in the mock (`1 * 3 >= 1 * 2`), so one vote is enough.
/// Returns the resulting `CurrentAIModelVersion` (1 the first time this is called in a test).
fn approve_first_ai_model(model_hash: [u8; 32]) -> u32 {
	assert_ok!(Courts::add_ai_governance_member(RuntimeOrigin::root(), 100));
	assert_ok!(Courts::vote_approve_ai_model(RuntimeOrigin::signed(100), model_hash));
	crate::CurrentAIModelVersion::<Test>::get()
}

/// File a case, submit an AI ruling, and appeal it — leaving the case in `InJuryAppeal` with
/// `JuryRequestBlock` set to the current block. Returns the case id.
fn file_ai_rule_and_appeal(filer: AccountId, subject: CaseSubject) -> u32 {
	let case_id = crate::NextCaseId::<Test>::get();
	assert_ok!(Courts::file_case(RuntimeOrigin::signed(filer), subject));
	let model_version = approve_first_ai_model([7u8; 32]);
	assert_ok!(Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], model_version));
	assert_ok!(Courts::appeal_ruling(RuntimeOrigin::signed(filer), case_id));
	case_id
}

/// Populate the delayed-reveal seed window for `case_id` with the given block hashes
/// (one per block in the window, i.e. `hashes.len()` must equal `JurySeedDelayBlocks`).
fn set_window_hashes(case_id: u32, hashes: &[H256]) {
	let request_block = JuryRequestBlock::<Test>::get(case_id).expect("case must have appealed");
	for (offset, h) in hashes.iter().enumerate() {
		let n = request_block + 1 + offset as u64;
		frame_system::BlockHash::<Test>::insert(n, *h);
	}
}

/// Block at which the seed window for `case_id` closes (inclusive) — `select_jury` only
/// succeeds strictly after this block.
fn window_end(case_id: u32) -> u64 {
	JuryRequestBlock::<Test>::get(case_id).unwrap() + 3 // JurySeedDelayBlocks = 3 in the mock.
}

#[test]
fn select_jury_fails_before_seed_window_elapses() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		// Right at the appeal block: nowhere near ready.
		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7),
			Error::<Test>::JurySeedNotReady
		);
		// Exactly at window_end (inclusive): still not ready — the boundary is strict.
		System::set_block_number(window_end(case_id));
		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7),
			Error::<Test>::JurySeedNotReady
		);
	});
}

#[test]
fn select_jury_succeeds_once_window_elapses() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)]);
		System::set_block_number(window_end(case_id) + 1);

		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));

		let jury = JuryPool::<Test>::get(case_id).expect("jury pool set");
		assert_eq!(jury.len(), 7, "General case needs a 7-person Level-1 jury");
		// No duplicate jurors.
		let mut sorted = jury.clone().into_inner();
		sorted.sort();
		sorted.dedup();
		assert_eq!(sorted.len(), 7);
		// Every juror must come from the configured citizen pool (accounts 1..=30).
		for j in jury.iter() {
			assert!((1..=30).contains(j));
		}
		// Case has advanced past InJuryAppeal.
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::JurySeated);
	});
}

#[test]
fn select_jury_requires_21_jurors_for_law_challenge() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::LawChallenge { law_id: 5 });
		set_window_hashes(case_id, &[H256::repeat_byte(0x44), H256::repeat_byte(0x55), H256::repeat_byte(0x66)]);
		System::set_block_number(window_end(case_id) + 1);

		// Wrong size for a constitutional (Level 2) case is rejected.
		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7),
			Error::<Test>::InvalidJurySize
		);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 21));
		let jury = JuryPool::<Test>::get(case_id).unwrap();
		assert_eq!(jury.len(), 21);
	});
}

/// The core security property of the delayed-reveal scheme: the resulting jury depends only
/// on the fixed window `[request_block + 1, request_block + JurySeedDelayBlocks]`, not on
/// anything that happens afterwards — not the block `select_jury` is actually called in, and
/// not block hashes outside the window. This is what closes the old "grind by delaying
/// submission" hole: there is nothing left to grind, because the caller's timing no longer
/// affects the seed.
#[test]
fn select_jury_result_is_independent_of_call_time_and_outside_hashes() {
	let window = [H256::repeat_byte(0xAB), H256::repeat_byte(0xCD), H256::repeat_byte(0xEF)];

	let jurors_called_early = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &window);
		// Call select_jury the block right after the window closes.
		System::set_block_number(window_end(case_id) + 1);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	let jurors_called_late = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &window);
		// Populate an *outside-the-window* block with a completely different hash, and wait
		// several extra blocks before finally calling select_jury.
		frame_system::BlockHash::<Test>::insert(window_end(case_id) + 2, H256::repeat_byte(0x99));
		System::set_block_number(window_end(case_id) + 5);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	assert_eq!(
		jurors_called_early, jurors_called_late,
		"jury must depend only on the fixed seed window, not on call timing or outside hashes"
	);
}

/// Changing the block hashes inside the seed window changes the outcome — i.e. the window
/// content actually matters and isn't just being ignored.
#[test]
fn select_jury_result_changes_with_window_hashes() {
	let jurors_a = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11); 3]);
		System::set_block_number(window_end(case_id) + 1);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	let jurors_b = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &[H256::repeat_byte(0xEE); 3]);
		System::set_block_number(window_end(case_id) + 1);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	assert_ne!(jurors_a, jurors_b, "different window hashes should (almost certainly) produce a different jury");
}

#[test]
fn select_jury_authorization_rejects_unrelated_signed_account() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11); 3]);
		System::set_block_number(window_end(case_id) + 1);

		// Account 2 is neither the filer nor the oracle, and this isn't a system case.
		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(2), case_id, 7),
			Error::<Test>::NotAuthorized
		);
		// The filer themself is authorized.
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
	});
}

#[test]
fn select_jury_fails_when_not_enough_citizens() {
	new_test_ext().execute_with(|| {
		set_citizen_count(5); // fewer than the 7 required for a Level-1 jury.
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11); 3]);
		System::set_block_number(window_end(case_id) + 1);

		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7),
			Error::<Test>::NotEnoughCitizens
		);
	});
}

#[test]
fn select_jury_system_case_requires_active_citizen() {
	new_test_ext().execute_with(|| {
		// System-initiated case: filer is the AutoChallengeAccount (account 0 in the mock).
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::auto_file_case(CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		assert_ok!(Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], model_version));
		assert_ok!(Courts::appeal_ruling(RuntimeOrigin::signed(9), case_id));
		set_window_hashes(case_id, &[H256::repeat_byte(0x11); 3]);
		System::set_block_number(window_end(case_id) + 1);

		// A suspended (non-active) citizen may not trigger jury selection even for a system case.
		set_suspended(6);
		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(6), case_id, 7),
			Error::<Test>::NotAuthorized
		);
		// Any other active citizen may, since there's no designated filer to restrict it to.
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(9), case_id, 7));
	});
}

#[test]
fn jury_vote_majority_freezes_department_for_treasury_dispute() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::TreasuryDispute { department_id: 3 });
		set_window_hashes(case_id, &[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)]);
		System::set_block_number(window_end(case_id) + 1);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		let jury = JuryPool::<Test>::get(case_id).unwrap();

		for juror in jury.iter().take(4) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Overturned));
		}

		assert_eq!(frozen_departments(), vec![3]);
	});
}

#[test]
fn jury_vote_majority_suspends_citizen_for_conduct_case() {
	new_test_ext().execute_with(|| {
		let nullifier = [9u8; 32];
		let case_id = file_ai_rule_and_appeal(
			1,
			CaseSubject::CitizenConduct { nullifier, suspension_blocks: Some(50) },
		);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)]);
		let select_block = window_end(case_id) + 1;
		System::set_block_number(select_block);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		let jury = JuryPool::<Test>::get(case_id).unwrap();

		for juror in jury.iter().take(4) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Overturned));
		}

		// suspension_blocks (a duration) is converted to an absolute block number by adding it
		// to "now" at finalization time (the block the 4th, majority-clinching vote lands in).
		// jury_reviewed is true here: the case reached JurySeated before auto_finalize ran.
		assert_eq!(suspended_citizens(), vec![(nullifier, Some(select_block + 50), true)]);
	});
}

#[test]
fn unappealed_ai_ruling_suspends_citizen_without_jury_review_flag() {
	// Same CitizenConduct enforcement, but via the *other* path into auto_finalize:
	// finalize_ruling on a case nobody ever appealed. The suspension still happens, but
	// jury_reviewed must be false — no jury ever saw this case.
	new_test_ext().execute_with(|| {
		let nullifier = [9u8; 32];
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(
			RuntimeOrigin::signed(1),
			CaseSubject::CitizenConduct { nullifier, suspension_blocks: Some(50) },
		));
		let model_version = approve_first_ai_model([7u8; 32]);
		assert_ok!(Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], model_version));

		// Let the appeal window (100 blocks in the mock) lapse without an appeal, then finalize.
		System::set_block_number(200);
		assert_ok!(Courts::finalize_ruling(RuntimeOrigin::root(), case_id, Verdict::Overturned));

		assert_eq!(suspended_citizens(), vec![(nullifier, Some(200 + 50), false)]);
	});
}

/// End-to-end: jury selection feeds into voting, which feeds into auto-enforcement — makes
/// sure the new seed mechanism didn't break anything downstream of `select_jury`.
#[test]
fn jury_vote_majority_auto_finalizes_and_enforces_law_challenge() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::LawChallenge { law_id: 42 });
		set_window_hashes(case_id, &[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)]);
		System::set_block_number(window_end(case_id) + 1);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 21));
		let jury = JuryPool::<Test>::get(case_id).unwrap();

		// Strict majority (11 of 21) votes Overturned.
		for juror in jury.iter().take(11) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Overturned));
		}

		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Enforced);
		assert_eq!(crate::Rulings::<Test>::get(case_id), Some(Verdict::Overturned));
		System::assert_has_event(Event::RulingEnforced { case_id }.into());
		assert_eq!(invalidated_laws(), vec![42]);

		// A 12th juror voting after finalization is rejected — case is no longer JurySeated.
		let last = jury.iter().last().unwrap();
		assert_noop!(
			Courts::cast_jury_vote(RuntimeOrigin::signed(*last), case_id, Verdict::Upheld),
			Error::<Test>::InvalidStatus
		);
	});
}

// ─── file_case bond ────────────────────────────────────────────────────────
//
// `file_case` reserves `CaseFilingBond` from the filer as a spam-prevention deposit,
// mirroring pallet-elections' `CandidateDeposit` pattern on `register_candidate`. The bond
// is released in full once the case reaches a final status (`auto_finalize`, reached either
// via `finalize_ruling`'s no-appeal path or `cast_jury_vote`'s majority path). System-filed
// cases via `auto_file_case` never reserve anything.

#[test]
fn file_case_reserves_bond_when_filer_can_afford_it() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));

		assert_eq!(Balances::reserved_balance(1), CASE_FILING_BOND);
		assert_eq!(CaseBonds::<Test>::get(case_id), Some(CASE_FILING_BOND));
		let (filer, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(filer, 1);
		assert_eq!(status, CaseStatus::Filed);
		System::assert_last_event(
			Event::CaseFiled { case_id, filer: 1, subject: CaseSubject::General }.into(),
		);
	});
}

// ─── AI model governance (supermajority vote) ─────────────────────────────

#[test]
fn vote_approve_ai_model_reaches_supermajority_and_sets_current_version() {
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::add_ai_governance_member(RuntimeOrigin::root(), 101));
		assert_ok!(Courts::add_ai_governance_member(RuntimeOrigin::root(), 102));
		assert_ok!(Courts::add_ai_governance_member(RuntimeOrigin::root(), 103));

		let model_hash = [42u8; 32];
		// 2/3 of 3 members requires 2 votes: 1 * 3 = 3 >= 3 * 2 = 6 is false, so 1 vote alone
		// must not resolve the proposal.
		assert_ok!(Courts::vote_approve_ai_model(RuntimeOrigin::signed(101), model_hash));
		assert_eq!(crate::CurrentAIModelVersion::<Test>::get(), 0);
		assert!(crate::AIModelVersions::<Test>::get(1).is_none());

		// Second vote reaches the threshold (2 * 3 = 6 >= 3 * 2 = 6) and resolves it.
		assert_ok!(Courts::vote_approve_ai_model(RuntimeOrigin::signed(102), model_hash));

		assert_eq!(crate::CurrentAIModelVersion::<Test>::get(), 1);
		let info = crate::AIModelVersions::<Test>::get(1).expect("version 1 recorded");
		assert_eq!(info.model_hash, model_hash);
		System::assert_has_event(Event::AIModelApproved { version: 1, model_hash }.into());

		// A late vote from the third member after resolution doesn't error — it just starts
		// (and, being alone, doesn't resolve) a fresh round for the next proposal.
		assert_ok!(Courts::vote_approve_ai_model(RuntimeOrigin::signed(103), [99u8; 32]));
		assert_eq!(crate::CurrentAIModelVersion::<Test>::get(), 1);
	});
}

#[test]
fn vote_approve_ai_model_rejects_non_council_member() {
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::add_ai_governance_member(RuntimeOrigin::root(), 101));
		// Account 999 was never added to the council.
		assert_noop!(
			Courts::vote_approve_ai_model(RuntimeOrigin::signed(999), [1u8; 32]),
			Error::<Test>::NotAIGovernanceCouncilMember
		);
		assert_eq!(crate::CurrentAIModelVersion::<Test>::get(), 0);
	});
}

#[test]
fn file_case_fails_with_insufficient_balance_and_leaves_no_dangling_reserve() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		// Account 999 was never funded in genesis (only 1..=30 are).
		assert_noop!(
			Courts::file_case(RuntimeOrigin::signed(999), CaseSubject::General),
			Error::<Test>::InsufficientBalance
		);
		assert_eq!(Balances::reserved_balance(999), 0);
		assert!(crate::pallet::Cases::<Test>::get(case_id).is_none());
		assert!(CaseBonds::<Test>::get(case_id).is_none());
		// NextCaseId must not have advanced — the call failed before any state was written.
		assert_eq!(crate::NextCaseId::<Test>::get(), case_id);
	});
}

#[test]
fn add_ai_governance_member_requires_root() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Courts::add_ai_governance_member(RuntimeOrigin::signed(1), 101),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn file_case_bond_is_released_when_finalized_without_appeal() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		assert_ok!(Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], model_version));
		assert_eq!(Balances::reserved_balance(1), CASE_FILING_BOND);

		// Let the appeal window (100 blocks in the mock) lapse, then finalize.
		System::set_block_number(200);
		assert_ok!(Courts::finalize_ruling(RuntimeOrigin::root(), case_id, Verdict::Upheld));

		assert_eq!(Balances::reserved_balance(1), 0);
		assert!(CaseBonds::<Test>::get(case_id).is_none());
	});
}

#[test]
fn vote_approve_ai_model_rejects_double_vote() {
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::add_ai_governance_member(RuntimeOrigin::root(), 101));
		assert_ok!(Courts::add_ai_governance_member(RuntimeOrigin::root(), 102));
		assert_ok!(Courts::vote_approve_ai_model(RuntimeOrigin::signed(101), [1u8; 32]));
		assert_noop!(
			Courts::vote_approve_ai_model(RuntimeOrigin::signed(101), [1u8; 32]),
			Error::<Test>::AlreadyVotedForAIModel
		);
	});
}

#[test]
fn submit_ai_ruling_rejects_when_no_model_approved() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		assert_noop!(
			Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], 0),
			Error::<Test>::NoApprovedAIModel
		);
		assert_noop!(
			Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], 1),
			Error::<Test>::NoApprovedAIModel
		);
	});
}

#[test]
fn file_case_bond_is_released_when_jury_finalizes_case() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		assert_eq!(Balances::reserved_balance(1), CASE_FILING_BOND);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)]);
		System::set_block_number(window_end(case_id) + 1);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		let jury = JuryPool::<Test>::get(case_id).unwrap();

		// Bond stays reserved through jury seating — only released on final status.
		assert_eq!(Balances::reserved_balance(1), CASE_FILING_BOND);

		for juror in jury.iter().take(4) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Upheld));
		}

		assert_eq!(Balances::reserved_balance(1), 0);
		assert!(CaseBonds::<Test>::get(case_id).is_none());
	});
}

#[test]
fn submit_ai_ruling_rejects_stale_model_version() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));

		// Approve version 1, then supersede it with version 2.
		let v1 = approve_first_ai_model([1u8; 32]);
		assert_eq!(v1, 1);
		assert_ok!(Courts::vote_approve_ai_model(RuntimeOrigin::signed(100), [2u8; 32]));
		assert_eq!(crate::CurrentAIModelVersion::<Test>::get(), 2);

		// Citing the now-stale version 1 is rejected even though it was once valid.
		assert_noop!(
			Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], 1),
			Error::<Test>::UnapprovedAIModel
		);
	});
}

#[test]
fn auto_file_case_does_not_reserve_bond() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::auto_file_case(CaseSubject::General));

		let (filer, _, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		// AutoChallengeAccountId (account 0) is the system filer and is unfunded — if this
		// path tried to reserve a bond it would fail with InsufficientBalance, but it
		// succeeded above, and nothing is reserved.
		assert_eq!(Balances::reserved_balance(filer), 0);
		assert!(CaseBonds::<Test>::get(case_id).is_none());
	});
}

#[test]
fn submit_ai_ruling_succeeds_with_current_approved_version() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([9u8; 32]);

		assert_ok!(Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32], model_version));

		assert_eq!(crate::AIRulingModelVersion::<Test>::get(case_id), Some(model_version));
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued);
		assert_eq!(ruling_hash, Some([7u8; 32]));
		System::assert_last_event(
			Event::AIRulingIssued { case_id, ruling_hash: [7u8; 32], model_version }.into(),
		);
	});
}
