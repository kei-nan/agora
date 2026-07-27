use crate::{
	mock::*,
	pallet::{CaseStatus, CaseSubject, Error, Event, JuryPool, JuryRequestBlock, Verdict},
};
use frame_support::{assert_noop, assert_ok};
use sp_core::H256;

/// File a case, submit an AI ruling, and appeal it — leaving the case in `InJuryAppeal` with
/// `JuryRequestBlock` set to the current block. Returns the case id.
fn file_ai_rule_and_appeal(filer: AccountId, subject: CaseSubject) -> u32 {
	let case_id = crate::NextCaseId::<Test>::get();
	assert_ok!(Courts::file_case(RuntimeOrigin::signed(filer), subject));
	assert_ok!(Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32]));
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
		assert_ok!(Courts::submit_ai_ruling(RuntimeOrigin::root(), case_id, [7u8; 32]));
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
		assert_eq!(suspended_citizens(), vec![(nullifier, Some(select_block + 50))]);
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
