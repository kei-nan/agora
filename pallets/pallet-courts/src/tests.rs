use crate::{
	mock::*,
	pallet::{CaseBonds, CaseStatus, CaseSubject, Error, Event, JuryPool, JuryRequestBlock, Verdict},
};
use frame_support::{assert_noop, assert_ok, traits::Hooks};
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

/// The account used as the sole Oracle Council member in tests that don't care about the M-of-N
/// mechanics themselves — mirrors `approve_first_ai_model`'s single-member shortcut for the
/// (separate) AI Model Governance Council. A 1-member council trivially satisfies the mock's
/// 1/2 majority threshold (`1 * 2 > 1 * 1`), so a lone `add_oracle_member` + one signed call
/// from this account resolves a ruling submission/finalization immediately — the same
/// observable behavior these tests were originally written against back when there was a single
/// `OracleAccount` and calls were signed `RuntimeOrigin::root()`.
const DEFAULT_ORACLE: AccountId = 50;

/// Registers `DEFAULT_ORACLE` as an Oracle Council member (idempotent per fresh `new_test_ext()`)
/// and returns it, ready to sign `submit_ai_ruling`/`approve_ai_ruling`/`finalize_ruling` calls.
fn setup_oracle_member() -> AccountId {
	assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), DEFAULT_ORACLE));
	DEFAULT_ORACLE
}

/// File a case, submit an AI ruling (with an arbitrary `Verdict::Upheld` — the jury-appeal
/// tests that use this helper always re-derive the real verdict from actual jury votes, so
/// the AI-submitted one is just a placeholder here), and appeal it — leaving the case in
/// `InJuryAppeal` with `JuryRequestBlock` set to the current block. Returns the case id.
fn file_ai_rule_and_appeal(filer: AccountId, subject: CaseSubject) -> u32 {
	let case_id = crate::NextCaseId::<Test>::get();
	assert_ok!(Courts::file_case(RuntimeOrigin::signed(filer), subject));
	let model_version = approve_first_ai_model([7u8; 32]);
	let oracle = setup_oracle_member();
	assert_ok!(Courts::submit_ai_ruling(
		RuntimeOrigin::signed(oracle),
		case_id,
		[7u8; 32],
		model_version,
		Verdict::Upheld
	));
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

/// Block at which the seed window for `case_id` closes (inclusive) — the jury seed is
/// captured by `on_initialize` at `window_end(case_id) + 1`, see `capture_jury_seed`.
fn window_end(case_id: u32) -> u64 {
	JuryRequestBlock::<Test>::get(case_id).unwrap() + 3 // JurySeedDelayBlocks = 3 in the mock.
}

/// Advance to the first block after `case_id`'s seed window closes and run the pallet's
/// `on_initialize` hook there, so `CapturedJurySeed` gets populated — mirrors what the
/// runtime's block-authoring executive does automatically outside of these unit tests (which
/// only advance `System::block_number()` directly and never run hooks on their own unless
/// explicitly told to). Returns the block number reached.
fn capture_jury_seed(case_id: u32) -> u64 {
	let capture_block = window_end(case_id) + 1;
	System::set_block_number(capture_block);
	let _ = Courts::on_initialize(capture_block);
	capture_block
}

#[test]
fn select_jury_fails_before_seed_window_elapses() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		// Right at the appeal block: nowhere near ready, and on_initialize hasn't run yet.
		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7),
			Error::<Test>::JurySeedNotReady
		);
		// Exactly at window_end (inclusive), still without running on_initialize: still not
		// ready — capture only happens at window_end + 1, and select_jury reads only the
		// captured seed, never live block-hash storage.
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
		capture_jury_seed(case_id);

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
		capture_jury_seed(case_id);

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
		// Capture, then call select_jury the block right after the window closes.
		capture_jury_seed(case_id);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	let jurors_called_late = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &window);
		// Capture right at window close, as usual.
		capture_jury_seed(case_id);
		// Populate an *outside-the-window* block with a completely different hash, and wait
		// several extra blocks before finally calling select_jury -- must have zero effect now
		// that the seed was captured once and select_jury only ever reads the captured value.
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
		capture_jury_seed(case_id);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	let jurors_b = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &[H256::repeat_byte(0xEE); 3]);
		capture_jury_seed(case_id);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	assert_ne!(jurors_a, jurors_b, "different window hashes should (almost certainly) produce a different jury");
}

/// Proves the actual security property Fix 2 is about: even once the seed window's live block
/// hashes have been wiped from `frame_system::BlockHash` (simulating `BlockHashCount`-based
/// pruning long after the fact -- what would happen in real deployment ~4h after window close),
/// `select_jury` still produces the same, real, unpredictable-at-commit-time jury, because it
/// now reads a value captured (via `on_initialize`) at window-close time rather than
/// recomputing `anchored_entropy` from live storage. Before this fix, the exact same setup
/// would degrade to fully-computable "zero-hash" entropy once the window's blocks were pruned.
#[test]
fn select_jury_seed_survives_blockhash_pruning_once_captured() {
	let window = [H256::repeat_byte(0x5A), H256::repeat_byte(0x5B), H256::repeat_byte(0x5C)];

	// Baseline: seed captured and consumed promptly, well within any pruning window.
	let baseline = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &window);
		capture_jury_seed(case_id);
		assert!(
			crate::pallet::CapturedJurySeed::<Test>::contains_key(case_id),
			"on_initialize must have captured the seed at window-close time"
		);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	// Adversarial: capture the seed at the correct block (while the window's hashes are still
	// live), then wipe those same live BlockHash entries and jump far into the future before
	// finally calling select_jury. A live recompute of anchored_entropy at that point would see
	// all-zero hashes for the whole window (frame_system's actual pruning behavior); the
	// captured value must be immune to this entirely.
	let after_pruning = new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &window);
		let request_block = JuryRequestBlock::<Test>::get(case_id).unwrap();
		capture_jury_seed(case_id);
		assert!(crate::pallet::CapturedJurySeed::<Test>::contains_key(case_id));

		// Simulate BlockHashCount pruning: wipe the window's live block hashes...
		for (offset, _) in window.iter().enumerate() {
			frame_system::BlockHash::<Test>::remove(request_block + 1 + offset as u64);
		}
		// ...and advance far past where select_jury is first callable, well beyond any
		// plausible pruning horizon.
		System::set_block_number(window_end(case_id) + 10_000);

		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		JuryPool::<Test>::get(case_id).unwrap().into_inner()
	});

	assert_eq!(
		baseline, after_pruning,
		"a captured seed must produce the same jury regardless of later block-hash pruning \
		 or how much later select_jury is actually called"
	);
}

#[test]
fn select_jury_authorization_rejects_unrelated_signed_account() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11); 3]);
		capture_jury_seed(case_id);

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
		capture_jury_seed(case_id);

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
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		assert_ok!(Courts::appeal_ruling(RuntimeOrigin::signed(9), case_id));
		set_window_hashes(case_id, &[H256::repeat_byte(0x11); 3]);
		capture_jury_seed(case_id);

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
		capture_jury_seed(case_id);
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
		let select_block = capture_jury_seed(case_id);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		let jury = JuryPool::<Test>::get(case_id).unwrap();

		for juror in jury.iter().take(4) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Overturned));
		}

		// suspension_blocks (a duration) is converted to an absolute block number by adding it
		// to "now" at finalization time (the block the 4th, majority-clinching vote lands in,
		// which here is still `select_block` since capture_jury_seed's on_initialize call and
		// the votes below all happen at that same block number).
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
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Overturned
		));

		// Let the appeal window (100 blocks in the mock) lapse without an appeal, then finalize.
		// finalize_ruling no longer takes a verdict argument -- it applies the Overturned
		// verdict committed above by submit_ai_ruling.
		System::set_block_number(200);
		assert_ok!(Courts::finalize_ruling(RuntimeOrigin::signed(oracle), case_id));

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
		capture_jury_seed(case_id);
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
// `file_case` reserves `CaseFilingBond` from the filer as a spam-prevention deposit. The bond
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
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		assert_eq!(Balances::reserved_balance(1), CASE_FILING_BOND);

		// Let the appeal window (100 blocks in the mock) lapse, then finalize. finalize_ruling
		// applies the Upheld verdict committed above, with no argument of its own.
		System::set_block_number(200);
		assert_ok!(Courts::finalize_ruling(RuntimeOrigin::signed(oracle), case_id));

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
		let oracle = setup_oracle_member();
		assert_noop!(
			Courts::submit_ai_ruling(RuntimeOrigin::signed(oracle), case_id, [7u8; 32], 0, Verdict::Upheld),
			Error::<Test>::NoApprovedAIModel
		);
		assert_noop!(
			Courts::submit_ai_ruling(RuntimeOrigin::signed(oracle), case_id, [7u8; 32], 1, Verdict::Upheld),
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
		capture_jury_seed(case_id);
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
		let oracle = setup_oracle_member();
		assert_noop!(
			Courts::submit_ai_ruling(RuntimeOrigin::signed(oracle), case_id, [7u8; 32], 1, Verdict::Upheld),
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
		let oracle = setup_oracle_member();

		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Overturned
		));

		assert_eq!(crate::AIRulingModelVersion::<Test>::get(case_id), Some(model_version));
		assert_eq!(crate::pallet::AIRulingVerdict::<Test>::get(case_id), Some(Verdict::Overturned));
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued);
		assert_eq!(ruling_hash, Some([7u8; 32]));
		System::assert_last_event(
			Event::AIRulingIssued { case_id, ruling_hash: [7u8; 32], model_version }.into(),
		);
	});
}

// ─── Fix 1: finalize_ruling applies the verdict bound at submit_ai_ruling time ─────────────

#[test]
fn finalize_ruling_applies_the_verdict_committed_at_submission_not_a_caller_supplied_one() {
	// The core property of the fix: finalize_ruling(case_id) takes no verdict argument at all
	// now, so whatever was committed by submit_ai_ruling is what gets applied and enforced --
	// there is no longer any way for the caller of finalize_ruling to choose a different one.
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::LawChallenge { law_id: 7 }));
		let model_version = approve_first_ai_model([7u8; 32]);
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Overturned
		));

		System::set_block_number(200);
		assert_ok!(Courts::finalize_ruling(RuntimeOrigin::signed(oracle), case_id));

		// The Overturned verdict committed at submission time was applied and enforced (law 7
		// invalidated), even though finalize_ruling's call site above supplied no verdict.
		assert_eq!(crate::Rulings::<Test>::get(case_id), Some(Verdict::Overturned));
		assert_eq!(invalidated_laws(), vec![7]);
	});
}

#[test]
fn finalize_ruling_fails_if_no_verdict_was_ever_recorded() {
	// Defensive case: AIRulingIssued status with no AIRulingVerdict entry shouldn't be
	// reachable through the normal call surface (submit_ai_ruling always sets both together),
	// but finalize_ruling must fail safe rather than panic if it somehow happens.
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		// Simulate a corrupted/partial state: verdict entry removed after submission.
		crate::pallet::AIRulingVerdict::<Test>::remove(case_id);

		System::set_block_number(200);
		assert_noop!(
			Courts::finalize_ruling(RuntimeOrigin::signed(oracle), case_id),
			Error::<Test>::NoRulingVerdict
		);
	});
}

// ─── Fix 3: appeal_ruling authorization ────────────────────────────────────────────────────

#[test]
fn appeal_ruling_rejects_unrelated_signed_account() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));

		// Account 2 is neither the filer nor the oracle, and this isn't a system case, and it
		// has no registered nullifier matching a CitizenConduct subject (this is General
		// anyway). Before the fix, any signed account could do this.
		assert_noop!(
			Courts::appeal_ruling(RuntimeOrigin::signed(2), case_id),
			Error::<Test>::NotAuthorized
		);
		// The filer themself is still authorized.
		assert_ok!(Courts::appeal_ruling(RuntimeOrigin::signed(1), case_id));
	});
}

#[test]
fn appeal_ruling_allows_the_designated_oracle() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		// DEFAULT_ORACLE (account 50) is registered as an Oracle Council member here, replacing
		// the old `set_oracle_account` — membership alone (via `is_filer_or_oracle`) is what
		// grants the independent appeal right tested below, regardless of who proposed/approved
		// this particular case's ruling.
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));

		// Account 50 is neither the filer nor a CitizenConduct nullifier match, but it is an
		// Oracle Council member, which is independently sufficient.
		assert_ok!(Courts::appeal_ruling(RuntimeOrigin::signed(oracle), case_id));
	});
}

#[test]
fn appeal_ruling_rejects_suspended_citizen_for_system_case() {
	new_test_ext().execute_with(|| {
		// System-initiated case: filer is the AutoChallengeAccount (account 0 in the mock), so
		// there's no natural filer to restrict appeal to -- any *active* citizen should be
		// able to trigger it, but a suspended one must not.
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::auto_file_case(CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));

		set_suspended(6);
		assert_noop!(
			Courts::appeal_ruling(RuntimeOrigin::signed(6), case_id),
			Error::<Test>::NotAuthorized
		);
		assert_ok!(Courts::appeal_ruling(RuntimeOrigin::signed(9), case_id));
	});
}

#[test]
fn appeal_ruling_allows_verified_ruled_against_party_for_citizen_conduct_case() {
	// A CitizenConduct case's registered nullifier holder may appeal even though they didn't
	// file the case -- the genuine "losing party" appeal right described in the fix, verified
	// via CitizenChecker::citizen_nullifier (pallet-identity's real AccountId -> nullifier
	// reverse lookup in the runtime).
	new_test_ext().execute_with(|| {
		let nullifier = [42u8; 32];
		let case_id = crate::NextCaseId::<Test>::get();
		// Filed by account 1 (e.g. a prosecutor/complainant), against the citizen holding
		// `nullifier` -- account 5, who registers that nullifier via the mock helper.
		assert_ok!(Courts::file_case(
			RuntimeOrigin::signed(1),
			CaseSubject::CitizenConduct { nullifier, suspension_blocks: Some(10) },
		));
		set_citizen_nullifier(5, nullifier);
		let model_version = approve_first_ai_model([7u8; 32]);
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Overturned
		));

		// Account 5 is neither the filer nor the oracle nor a system case, but they *are* the
		// verified ruled-against party.
		assert_ok!(Courts::appeal_ruling(RuntimeOrigin::signed(5), case_id));
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::InJuryAppeal);
	});
}

#[test]
fn appeal_ruling_rejects_nullifier_mismatch_for_citizen_conduct_case() {
	new_test_ext().execute_with(|| {
		let nullifier = [42u8; 32];
		let other_nullifier = [99u8; 32];
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(
			RuntimeOrigin::signed(1),
			CaseSubject::CitizenConduct { nullifier, suspension_blocks: Some(10) },
		));
		// Account 5 is registered under a *different* nullifier than the one this case names.
		set_citizen_nullifier(5, other_nullifier);
		let model_version = approve_first_ai_model([7u8; 32]);
		let oracle = setup_oracle_member();
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(oracle),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Overturned
		));

		assert_noop!(
			Courts::appeal_ruling(RuntimeOrigin::signed(5), case_id),
			Error::<Test>::NotAuthorized
		);
	});
}

// ─── Oracle Council (M-of-N ruling approval) ───────────────────────────────────────────────
//
// Replaces the earlier single-`OracleAccount` design: `submit_ai_ruling`/`finalize_ruling`
// now only *propose* an action (and cast the proposer's own approval); it only takes effect
// once `OracleApprovalNumerator`/`Denominator` (1/2 -- strict majority -- in this mock, see
// mock.rs) of `OracleMembers` has approved via `approve_ai_ruling`. These tests use a
// 3-member council (accounts 60/61/62) so `DEFAULT_ORACLE`'s 1-member shortcut (which
// resolves on the very first call) doesn't mask the threshold logic: 1 of 3 approvals is
// `1*2=2 > 3*1=3`? false (not reached), 2 of 3 is `2*2=4 > 3` true (reached).

fn setup_three_member_oracle_council() -> (AccountId, AccountId, AccountId) {
	assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 60));
	assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 61));
	assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 62));
	(60, 61, 62)
}

#[test]
fn oracle_single_approval_does_not_trigger_ruling_below_threshold() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));

		// Only 1 of 3 approvals so far -- the case must still be Filed, not AIRulingIssued.
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Filed);
		assert_eq!(ruling_hash, None);
		assert!(crate::pallet::AIRulingVerdict::<Test>::get(case_id).is_none());
	});
}

#[test]
fn oracle_threshold_reached_triggers_ruling() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		// Second approval reaches the 1/2 strict-majority threshold (2 of 3).
		assert_ok!(Courts::approve_ai_ruling(RuntimeOrigin::signed(m2), case_id));

		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued);
		assert_eq!(ruling_hash, Some([7u8; 32]));
		assert_eq!(crate::pallet::AIRulingVerdict::<Test>::get(case_id), Some(Verdict::Upheld));
		System::assert_has_event(
			Event::AIRulingIssued { case_id, ruling_hash: [7u8; 32], model_version }.into(),
		);
		// The pending proposal/approval bookkeeping is cleared once resolved.
		assert!(crate::pallet::PendingOracleProposal::<Test>::get(case_id).is_none());
		assert!(crate::pallet::OracleApprovals::<Test>::get(case_id).is_none());
	});
}

#[test]
fn oracle_approve_ai_ruling_rejects_double_approval_from_same_member() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		// m1's proposal already counted as their approval -- calling approve_ai_ruling again
		// with the same member is rejected, not silently double-counted.
		assert_noop!(
			Courts::approve_ai_ruling(RuntimeOrigin::signed(m1), case_id),
			Error::<Test>::AlreadyApprovedOracleAction
		);
	});
}

#[test]
fn oracle_approve_ai_ruling_rejects_non_member() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		// Account 999 was never added to the Oracle Council.
		assert_noop!(
			Courts::approve_ai_ruling(RuntimeOrigin::signed(999), case_id),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn oracle_approve_ai_ruling_rejects_when_no_pending_action() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		// No case has ever been filed/proposed for id 0.
		assert_noop!(
			Courts::approve_ai_ruling(RuntimeOrigin::signed(m1), 0),
			Error::<Test>::NoPendingOracleAction
		);
	});
}

#[test]
fn oracle_finalize_ruling_requires_threshold_approval() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		// Get the case to AIRulingIssued first (2-of-3 submission).
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		assert_ok!(Courts::approve_ai_ruling(RuntimeOrigin::signed(m2), case_id));

		System::set_block_number(200); // past the 100-block appeal window in the mock.

		// m1 proposes finalization alone -- only 1 of 3 approvals, must not resolve yet.
		assert_ok!(Courts::finalize_ruling(RuntimeOrigin::signed(m1), case_id));
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued, "still pending: only 1 of 3 approved");
		assert!(crate::pallet::Rulings::<Test>::get(case_id).is_none());

		// Second approval reaches threshold and actually finalizes.
		assert_ok!(Courts::approve_ai_ruling(RuntimeOrigin::signed(m2), case_id));
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::FinalRuling);
		assert_eq!(crate::pallet::Rulings::<Test>::get(case_id), Some(Verdict::Upheld));
	});
}

#[test]
fn add_oracle_member_works_and_rejects_duplicate() {
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 60));
		assert!(crate::pallet::OracleMembers::<Test>::get().contains(&60));
		assert_noop!(
			Courts::add_oracle_member(RuntimeOrigin::root(), 60),
			Error::<Test>::AlreadyOracleMember
		);
	});
}

#[test]
fn add_oracle_member_requires_root() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Courts::add_oracle_member(RuntimeOrigin::signed(1), 60),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn remove_oracle_member_works_and_rejects_unknown_member() {
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 60));
		assert_ok!(Courts::remove_oracle_member(RuntimeOrigin::root(), 60));
		assert!(!crate::pallet::OracleMembers::<Test>::get().contains(&60));

		assert_noop!(
			Courts::remove_oracle_member(RuntimeOrigin::root(), 60),
			Error::<Test>::OracleMemberNotFound
		);
	});
}

#[test]
fn removed_oracle_member_can_no_longer_propose_or_approve() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		assert_ok!(Courts::remove_oracle_member(RuntimeOrigin::root(), m2));

		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(Courts::file_case(RuntimeOrigin::signed(1), CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		assert_noop!(
			Courts::approve_ai_ruling(RuntimeOrigin::signed(m2), case_id),
			DispatchError::BadOrigin
		);
	});
}
