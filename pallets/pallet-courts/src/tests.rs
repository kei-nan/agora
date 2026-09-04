use crate::{
	mock::*,
	pallet::{
		CaseBonds, CaseFiler, CaseStatus, CaseSubject, Error, Event, JuryPool, JuryRequestBlock,
		Verdict, CASE_FILING_SERVICE_SCOPE, CASE_FILING_SERVICE_SUBSCOPE,
	},
};
use frame_support::{assert_noop, assert_ok, traits::{ConstU32, EnsureOriginWithArg, Hooks}, BoundedVec};
use sp_core::H256;
use sp_runtime::{DispatchError, DispatchResult};

/// True for the case types `do_file_case` requires a ZK proof for — see `CaseFiler`'s doc
/// comment. Mirrors the pallet's own branching so tests don't need to hand-pick which branch
/// applies at each call site.
fn requires_zk_proof(subject: &CaseSubject) -> bool {
	matches!(
		subject,
		CaseSubject::LawChallenge { .. }
			| CaseSubject::TreasuryDispute { .. }
			| CaseSubject::TierConflict { .. }
	)
}

fn valid_proof() -> BoundedVec<u8, ConstU32<4096>> {
	BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap()
}

fn invalid_proof() -> BoundedVec<u8, ConstU32<4096>> {
	BoundedVec::try_from(vec![INVALID_PROOF_MARKER]).unwrap()
}

/// A minimal, structurally valid ZKPassport `count_4` outer-circuit public-input array,
/// carrying this pallet's own case-filing scope/subscope and the given nullifier — mirrors
/// `pallet_anticorruption::tests::public_inputs_with`'s identical layout.
fn public_inputs_with(
	scope: [u8; 32],
	subscope: [u8; 32],
	nullifier: [u8; 32],
) -> BoundedVec<[u8; 32], ConstU32<16>> {
	BoundedVec::try_from(vec![
		[9u8; 32],  // 0: certificate_registry_root (unused by this pallet)
		[2u8; 32],  // 1: circuit_registry_root
		[0u8; 32],  // 2: current_date
		scope,      // 3: service_scope
		subscope,   // 4: service_subscope
		[3u8; 32],  // 5: param_commitments[0]
		[4u8; 32],  // 6: nullifier_type
		nullifier,  // 7 = len - 2: scoped_nullifier
		[6u8; 32],  // 8 = len - 1: oprf_pk_hash
	])
	.unwrap()
}

/// Convenience wrapper: correct case-filing domain-separation scope/subscope, caller-chosen
/// nullifier.
fn public_inputs(nullifier: [u8; 32]) -> BoundedVec<[u8; 32], ConstU32<16>> {
	public_inputs_with(CASE_FILING_SERVICE_SCOPE, CASE_FILING_SERVICE_SUBSCOPE, nullifier)
}

fn empty_public_inputs() -> BoundedVec<[u8; 32], ConstU32<16>> {
	BoundedVec::try_from(Vec::new()).unwrap()
}

/// Structurally too-short (fewer than the real layout's 9-element floor) but non-empty.
fn too_short_public_inputs() -> BoundedVec<[u8; 32], ConstU32<16>> {
	BoundedVec::try_from(vec![[1u8; 32]; 5]).unwrap()
}

/// Deterministic-but-arbitrary nullifier derived from an account id — used by
/// `file_case_as`/`file_ai_rule_and_appeal` so a citizen's own registered nullifier (via
/// `set_citizen_nullifier`) matches the nullifier stored in a case they filed, letting
/// `is_filer_or_oracle`'s nullifier-matching branch recognize them as the filer for
/// `appeal_ruling`/`select_jury` on anonymized case types, exactly as a real citizen re-proving
/// their own passport would.
fn nullifier_for(who: AccountId) -> [u8; 32] {
	let mut n = [7u8; 32];
	n[0..8].copy_from_slice(&who.to_le_bytes());
	n
}

/// Files `subject` as `who`, transparently supplying a valid ZK proof (and registering `who`'s
/// matching nullifier via `set_citizen_nullifier`, so later `appeal_ruling`/`select_jury` calls
/// signed by `who` are recognized as the filer) when `subject` is one of the anonymized case
/// types, or `None`/`None` otherwise — mirrors `do_file_case`'s own branching so call sites don't
/// need to hand-pick which applies.
fn file_case_as(who: AccountId, subject: CaseSubject) -> DispatchResult {
	if requires_zk_proof(&subject) {
		let nullifier = nullifier_for(who);
		set_citizen_nullifier(who, nullifier);
		Courts::file_case(RuntimeOrigin::signed(who), subject, Some(valid_proof()), Some(public_inputs(nullifier)))
	} else {
		Courts::file_case(RuntimeOrigin::signed(who), subject, None, None)
	}
}

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
	assert_ok!(file_case_as(filer, subject));
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

// ─── select_jury: conflict-of-interest exclusion ────────────────────────────
//
// Regression coverage for the fix closing `pick_random_jurors`' missing exclusion: previously
// the case's own filer, and (for `CaseSubject::CitizenConduct`) the accused, could be drawn
// onto their own jury. Both tests below shrink the citizen pool to exactly
// `jury_size + number_excluded` citizens, so excluding the right accounts leaves *exactly*
// enough eligible citizens to seat the jury — forcing a specific, fully-deterministic jury
// (every remaining citizen, in order) rather than merely asserting an outcome that could also
// happen to hold by chance against a large pool.

#[test]
fn select_jury_excludes_the_case_filer() {
	new_test_ext().execute_with(|| {
		// 8 citizens, 7-person jury (General is a Level-1 subject): excluding the filer
		// (account 1) leaves exactly the other 7 — {2..=8} — as the only possible jury.
		set_citizen_count(8);
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::General);
		set_window_hashes(
			case_id,
			&[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)],
		);
		capture_jury_seed(case_id);

		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		let jury = JuryPool::<Test>::get(case_id).unwrap();
		assert_eq!(jury.len(), 7);
		assert!(!jury.contains(&1), "the case's own filer must never be seated on its jury");
		let mut sorted = jury.into_inner();
		sorted.sort();
		assert_eq!(sorted, vec![2, 3, 4, 5, 6, 7, 8]);
	});
}

#[test]
fn select_jury_excludes_the_citizen_conduct_defendant() {
	new_test_ext().execute_with(|| {
		// 9 citizens, 7-person jury: excluding the filer (account 1) AND the defendant
		// (account 2, matched via their registered nullifier) leaves exactly {3..=9}.
		set_citizen_count(9);
		let defendant_nullifier = [42u8; 32];
		set_citizen_nullifier(2, defendant_nullifier);
		let case_id = file_ai_rule_and_appeal(
			1,
			CaseSubject::CitizenConduct { nullifier: defendant_nullifier, suspension_blocks: Some(10) },
		);
		set_window_hashes(
			case_id,
			&[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)],
		);
		capture_jury_seed(case_id);

		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		let jury = JuryPool::<Test>::get(case_id).unwrap();
		assert_eq!(jury.len(), 7);
		assert!(!jury.contains(&1), "the filer must never be seated on its own case's jury");
		assert!(
			!jury.contains(&2),
			"the accused (matched by their registered nullifier) must never sit on their own jury"
		);
		let mut sorted = jury.into_inner();
		sorted.sort();
		assert_eq!(sorted, vec![3, 4, 5, 6, 7, 8, 9]);
	});
}

// ─── select_jury: eligibility check must account for pick_random_jurors' exclusions ───────
//
// Regression coverage for the fix closing a gap in the eligibility check itself (distinct
// from the exclusion tests above, which cover `pick_random_jurors` actually excluding the
// right accounts once selection runs): `total >= required_size` alone doesn't guarantee a
// jury can actually be filled once the filer (and, for CitizenConduct, the defendant) are
// excluded from the draw. Before this fix, a pool sized exactly at `required_size` with an
// excluded party inside it passed this check and only failed later, deep inside
// `pick_random_jurors`'s retry loop, after burning through its attempt budget — same
// terminal `NotEnoughCitizens` error, but reached the wrong way, and the case would have
// been permanently stranded in `InJuryAppeal` had `pick_random_jurors`'s failure not also
// mapped to the same error (it does, but only by chance of shared error variant, not because
// anything upstream actually validated the pool was big enough).

#[test]
fn select_jury_eligibility_check_accounts_for_filer_exclusion() {
	new_test_ext().execute_with(|| {
		// Exactly 7 citizens (== the Level-1 required size), and the filer (account 1) is one
		// of them. `pick_random_jurors` always excludes the filer, so only 6 citizens are
		// actually eligible -- one short. This must be rejected immediately by the eligibility
		// check itself, not merely eventually via pick_random_jurors' retries.
		set_citizen_count(7);
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
fn select_jury_eligibility_check_accounts_for_citizen_conduct_exclusions() {
	new_test_ext().execute_with(|| {
		// 8 citizens -- one short of the 9 a CitizenConduct case actually needs (7-juror
		// requirement + filer + defendant both excluded = 9). Filer is account 1, defendant is
		// account 2 (matched via their registered nullifier), both within the pool of 8.
		set_citizen_count(8);
		let defendant_nullifier = [42u8; 32];
		set_citizen_nullifier(2, defendant_nullifier);
		let case_id = file_ai_rule_and_appeal(
			1,
			CaseSubject::CitizenConduct { nullifier: defendant_nullifier, suspension_blocks: Some(10) },
		);
		set_window_hashes(case_id, &[H256::repeat_byte(0x11); 3]);
		capture_jury_seed(case_id);

		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7),
			Error::<Test>::NotEnoughCitizens
		);
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
			None,
			None,
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

// ─── duplicate LawChallenge rejection ───────────────────────────────────────
//
// Regression coverage for the fix closing the revert-and-strand bug: previously, nothing
// stopped two `LawChallenge` cases from being open against the same `law_id` at once. If the
// first resolved Overturned (pausing the law via `T::LawEnforcer::invalidate_law`), the
// second's own later `invalidate_law` call would fail because the law was no longer `Active`
// — and because `auto_finalize` runs inside the same transactional extrinsic as its caller
// (`cast_jury_vote`/`finalize_ruling`), that whole call reverted, permanently stranding the
// second case (and its filing bond). `file_case`/`auto_file_case` now reject a second
// `LawChallenge` for a `law_id` that already has one open.

#[test]
fn file_case_rejects_duplicate_law_challenge_while_one_is_open() {
	new_test_ext().execute_with(|| {
		let first_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::LawChallenge { law_id: 42 }));

		// A second, different citizen filing against the same law_id is rejected while the
		// first case is still open (Filed, i.e. nowhere near resolved).
		assert_noop!(
			file_case_as(2, CaseSubject::LawChallenge { law_id: 42 }),
			Error::<Test>::DuplicateLawChallenge
		);
		// No state from the rejected attempt: no second case, no reserve taken from account 2,
		// NextCaseId unchanged.
		assert_eq!(Balances::reserved_balance(2), 0);
		assert_eq!(crate::NextCaseId::<Test>::get(), first_id + 1);

		// A LawChallenge against a *different* law_id is unaffected.
		assert_ok!(file_case_as(2, CaseSubject::LawChallenge { law_id: 7 }));
		// As is a non-LawChallenge case entirely.
		assert_ok!(file_case_as(3, CaseSubject::General));
	});
}

#[test]
fn file_case_allows_new_law_challenge_once_the_previous_one_finalizes() {
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::LawChallenge { law_id: 42 });
		set_window_hashes(case_id, &[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)]);
		capture_jury_seed(case_id);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 21));
		let jury = JuryPool::<Test>::get(case_id).unwrap();
		for juror in jury.iter().take(11) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Overturned));
		}
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Enforced);

		// Now that the case has fully resolved, OpenLawChallengeCase[42] must have been
		// cleared — filing a fresh challenge against the same law must succeed.
		assert_ok!(file_case_as(2, CaseSubject::LawChallenge { law_id: 42 }));
	});
}

#[test]
fn auto_file_case_rejects_duplicate_law_challenge_against_a_citizen_filed_one() {
	new_test_ext().execute_with(|| {
		assert_ok!(file_case_as(1, CaseSubject::LawChallenge { law_id: 9 }));
		assert_noop!(
			Courts::auto_file_case(CaseSubject::LawChallenge { law_id: 9 }),
			Error::<Test>::DuplicateLawChallenge
		);
	});
}

#[test]
fn file_case_rejects_duplicate_law_challenge_against_an_auto_filed_one() {
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::auto_file_case(CaseSubject::LawChallenge { law_id: 9 }));
		assert_noop!(
			file_case_as(1, CaseSubject::LawChallenge { law_id: 9 }),
			Error::<Test>::DuplicateLawChallenge
		);
		assert_eq!(Balances::reserved_balance(1), 0);
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
		assert_ok!(file_case_as(1, CaseSubject::General));

		assert_eq!(Balances::reserved_balance(1), CASE_FILING_BOND);
		assert_eq!(CaseBonds::<Test>::get(case_id), Some(CASE_FILING_BOND));
		let (filer, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		// General is not one of the anonymized case types -- the real AccountId is stored.
		assert_eq!(filer, CaseFiler::Account(1));
		assert_eq!(status, CaseStatus::Filed);
		System::assert_last_event(
			Event::CaseFiled { case_id, filer: CaseFiler::Account(1), subject: CaseSubject::General }.into(),
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
			file_case_as(999, CaseSubject::General),
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));

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
		// auto_file_case always uses CaseFiler::Account(AutoChallengeAccount) -- there's no
		// citizen filer to anonymize for a system-initiated case.
		let CaseFiler::Account(filer_account) = filer else {
			panic!("auto_file_case must use CaseFiler::Account");
		};
		// AutoChallengeAccountId (account 0) is the system filer and is unfunded — if this
		// path tried to reserve a bond it would fail with InsufficientBalance, but it
		// succeeded above, and nothing is reserved.
		assert_eq!(Balances::reserved_balance(filer_account), 0);
		assert!(CaseBonds::<Test>::get(case_id).is_none());
	});
}

#[test]
fn submit_ai_ruling_succeeds_with_current_approved_version() {
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::LawChallenge { law_id: 7 }));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
			None,
			None,
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
			None,
			None,
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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
		assert_ok!(file_case_as(1, CaseSubject::General));
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

#[test]
fn remove_oracle_member_purges_stale_approval_from_in_flight_proposal() {
	new_test_ext().execute_with(|| {
		let (m1, m2, m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		// m1 proposes and is auto-approved as the proposer -- 1 of 3, below the 2-of-3
		// threshold, so the case is still just Filed.
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		assert!(crate::pallet::OracleApprovals::<Test>::get(case_id).unwrap().contains(&m1));

		// m1's key is compromised; root removes them from the council mid-proposal --
		// exactly the incident-response path this council exists to survive.
		assert_ok!(Courts::remove_oracle_member(RuntimeOrigin::root(), m1));

		// The stale approval must be purged from the in-flight proposal, not just m1's
		// membership.
		assert!(!crate::pallet::OracleApprovals::<Test>::get(case_id).unwrap().contains(&m1));

		// Remaining council is {m2, m3}: 1/2 strict majority now needs 2 of 2. If m1's
		// stale approval still counted, m2 approving alone would wrongly reach quorum
		// (stale m1 + fresh m2 == 2 of a size-2 council).
		assert_ok!(Courts::approve_ai_ruling(RuntimeOrigin::signed(m2), case_id));
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Filed, "m2 alone must not reach quorum on a 2-member council");
		assert_eq!(ruling_hash, None);

		// A genuine second approval from the remaining council is required to finalize.
		assert_ok!(Courts::approve_ai_ruling(RuntimeOrigin::signed(m3), case_id));
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued);
		assert_eq!(ruling_hash, Some([7u8; 32]));
	});
}

#[test]
fn remove_oracle_member_reresolves_case_action_crossing_threshold_after_shrink() {
	// The exact stuck-proposal scenario the fix closes: a 2-member council where only the
	// proposer's own approval exists (1 of 2, below the strict-majority threshold). Removing
	// the *other* (non-proposing) member shrinks the council to 1, and the proposer's
	// already-cast approval alone now satisfies a 1-of-1 threshold -- this must resolve the
	// ruling as a side effect of `remove_oracle_member`, not leave it stranded (before the fix,
	// the proposer could not re-propose -- `OracleActionAlreadyProposed` -- and no one else
	// could approve -- `AlreadyApprovedOracleAction` / nobody else on the council).
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 60));
		assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 61));
		let (proposer, other) = (60, 61);

		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		// proposer submits and is auto-approved -- 1 of 2, strictly below the 2-of-2 majority
		// a 2-member council needs, so the case stays Filed.
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(proposer),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Filed, "1 of 2 must not resolve yet");
		assert_eq!(ruling_hash, None);

		// Removing the *other*, non-proposing member (e.g. their key was compromised) shrinks
		// the council to just {proposer}. The proposer's surviving approval alone now meets
		// the shrunk 1-of-1 threshold, so this call must resolve the ruling itself.
		assert_ok!(Courts::remove_oracle_member(RuntimeOrigin::root(), other));
		assert_eq!(crate::pallet::OracleMembers::<Test>::get().len(), 1);

		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(
			status,
			CaseStatus::AIRulingIssued,
			"the shrunk threshold is already met by the proposer's own approval -- must \
			 auto-resolve instead of staying stuck"
		);
		assert_eq!(ruling_hash, Some([7u8; 32]));
		// Resolved proposals are cleared, not left dangling.
		assert!(crate::pallet::PendingOracleProposal::<Test>::get(case_id).is_none());
		assert!(crate::pallet::OracleApprovals::<Test>::get(case_id).is_none());
	});
}

#[test]
fn remove_oracle_member_reresolves_finalization_crossing_threshold_after_shrink() {
	// Same scenario as the submission-side test above, but for the Finalization pending action
	// (`finalize_ruling`) -- the bug report's other named entry point.
	new_test_ext().execute_with(|| {
		assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 60));
		assert_ok!(Courts::add_oracle_member(RuntimeOrigin::root(), 61));
		let (proposer, other) = (60, 61);

		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		// Get the case to AIRulingIssued first (2-of-2 submission, both members approve).
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(proposer),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		assert_ok!(Courts::approve_ai_ruling(RuntimeOrigin::signed(other), case_id));
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued);

		System::set_block_number(200); // past the 100-block appeal window in the mock.

		// proposer proposes finalization alone -- 1 of 2, below threshold, stays pending.
		assert_ok!(Courts::finalize_ruling(RuntimeOrigin::signed(proposer), case_id));
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued, "1 of 2 must not finalize yet");
		assert!(crate::pallet::Rulings::<Test>::get(case_id).is_none());

		// Removing the other member shrinks the council to {proposer}; their surviving
		// finalization approval alone now meets the shrunk 1-of-1 threshold.
		assert_ok!(Courts::remove_oracle_member(RuntimeOrigin::root(), other));

		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(
			status,
			CaseStatus::FinalRuling,
			"the shrunk threshold is already met -- finalization must auto-resolve"
		);
		assert_eq!(crate::pallet::Rulings::<Test>::get(case_id), Some(Verdict::Upheld));
		assert!(crate::pallet::PendingOracleProposal::<Test>::get(case_id).is_none());
	});
}

// ── Admin actions (EnsureOracleCouncilApproved) ────────────────────────────────
//
// These cover the fix for the gap where pallet-constitution::invalidate_law and
// pallet-identity::suspend_citizen/restore_citizen_rights were gated only by the bare
// single-member `EnsureOracle` check: any one Oracle Council member could pause any law or
// suspend any citizen unilaterally, with none of the M-of-N approval submit_ai_ruling/
// approve_ai_ruling/finalize_ruling require. `propose_admin_action`/`approve_admin_action`
// plus `EnsureOracleCouncilApproved` close that gap for any manual-override extrinsic gated
// on this origin, regardless of which other pallet actually wires it in.

#[test]
fn admin_action_single_member_call_is_rejected() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		let call_hash = [42u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));

		// Only 1 of 3 approvals -- not yet resolved, so the proposer cannot consume it.
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_none());
		assert!(
			crate::pallet::EnsureOracleCouncilApproved::<Test>::try_origin(
				RuntimeOrigin::signed(m1),
				&call_hash,
			)
			.is_err()
		);
	});
}

#[test]
fn admin_action_majority_approved_call_succeeds_and_any_current_member_may_consume_it() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		let call_hash = [43u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		// Second approval reaches the 1/2 strict-majority threshold (2 of 3).
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));

		assert_eq!(
			crate::pallet::ApprovedAdminAction::<Test>::get(call_hash),
			Some((m1, System::block_number()))
		);
		assert!(crate::pallet::PendingAdminAction::<Test>::get(call_hash).is_none());

		// The proposer consumes it exactly once.
		assert!(
			crate::pallet::EnsureOracleCouncilApproved::<Test>::try_origin(
				RuntimeOrigin::signed(m1),
				&call_hash,
			)
			.is_ok()
		);
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_none());
		// A second attempt to consume the same (now-cleared) action fails.
		assert!(
			crate::pallet::EnsureOracleCouncilApproved::<Test>::try_origin(
				RuntimeOrigin::signed(m1),
				&call_hash,
			)
			.is_err()
		);
	});
}

/// The deadlock fix: any current Oracle Council member -- not only the original
/// `propose_admin_action` caller -- can consume an approved admin action token. The vote
/// that approved it is what legitimizes the action, so a proposer who goes offline or is
/// later removed must not be able to permanently block invalidate_law/suspend_citizen/
/// restore_citizen_rights. Mirrors pallet-legislature's
/// `ensure_legislature_motion_allows_any_current_member_to_consume_token`.
#[test]
fn ensure_oracle_council_approved_allows_any_current_member_to_consume_token() {
	new_test_ext().execute_with(|| {
		let (m1, m2, m3) = setup_three_member_oracle_council();
		let call_hash = [46u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_some());

		// m3, who is not the proposer, successfully consumes the token.
		assert!(
			crate::pallet::EnsureOracleCouncilApproved::<Test>::try_origin(
				RuntimeOrigin::signed(m3),
				&call_hash,
			)
			.is_ok()
		);
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_none());
	});
}

#[test]
fn ensure_oracle_council_approved_rejects_non_member() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		let call_hash = [47u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));

		// Account 999 was never an Oracle Council member.
		assert!(
			crate::pallet::EnsureOracleCouncilApproved::<Test>::try_origin(
				RuntimeOrigin::signed(999),
				&call_hash,
			)
			.is_err()
		);
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_some());
	});
}

// ─── clear_stale_admin_action ───────────────────────────────────────────────

#[test]
fn clear_stale_admin_action_fails_when_not_yet_expired() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		let call_hash = [48u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));

		System::set_block_number(System::block_number() + ADMIN_ACTION_EXPIRY as u64 - 1);
		assert_noop!(
			Courts::clear_stale_admin_action(RuntimeOrigin::signed(m1), call_hash),
			Error::<Test>::ApprovalNotYetStale
		);
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_some());
	});
}

#[test]
fn clear_stale_admin_action_fails_when_none_approved() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		assert_noop!(
			Courts::clear_stale_admin_action(RuntimeOrigin::signed(m1), [49u8; 32]),
			Error::<Test>::NoApprovedAdminAction
		);
	});
}

#[test]
fn clear_stale_admin_action_fails_for_non_member() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		let call_hash = [50u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));
		System::set_block_number(System::block_number() + ADMIN_ACTION_EXPIRY as u64);

		assert_noop!(
			Courts::clear_stale_admin_action(RuntimeOrigin::signed(999), call_hash),
			Error::<Test>::OracleMemberNotFound
		);
	});
}

/// The deadlock recovery this fix adds: once an approved admin action token sits unconsumed
/// past `AdminActionExpiryBlocks`, *any* current Oracle Council member (not necessarily the
/// stuck proposer) can clear it, and a fresh proposal for the same call_hash is then free to
/// be raised again -- proving the court system is no longer permanently stuck. Mirrors
/// pallet-legislature's `clear_stale_approval_unblocks_a_new_motion_after_expiry`.
#[test]
fn clear_stale_admin_action_unblocks_a_new_proposal_after_expiry() {
	new_test_ext().execute_with(|| {
		let (m1, m2, m3) = setup_three_member_oracle_council();
		let call_hash = [51u8; 32];

		// The action is approved but nobody (e.g. an offline/removed proposer) ever consumes it.
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));
		let approved_at = System::block_number();

		// Before expiry, the call_hash cannot be re-proposed.
		assert_noop!(
			Courts::propose_admin_action(RuntimeOrigin::signed(m3), call_hash),
			Error::<Test>::OracleActionAlreadyProposed
		);

		// Move past the expiry window and clear the stale token -- called by m3, who is not
		// the original proposer.
		System::set_block_number(approved_at + ADMIN_ACTION_EXPIRY as u64);
		assert_ok!(Courts::clear_stale_admin_action(RuntimeOrigin::signed(m3), call_hash));
		System::assert_last_event(Event::AdminActionExpired { call_hash }.into());
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_none());

		// The court system is unblocked: the same call_hash can be proposed again.
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert!(crate::pallet::PendingAdminAction::<Test>::get(call_hash).is_some());
	});
}

// ─── clear_stale_oracle_proposal ─────────────────────────────────────────────
//
// Case-based counterpart of the `clear_stale_admin_action` coverage above: closes the gap
// where a `submit_ai_ruling`/`finalize_ruling` proposal that never reaches the Oracle
// Council's M-of-N threshold (e.g. because members are offline/non-participating) would
// otherwise strand `case_id` forever behind `Error::OracleActionAlreadyProposed`, with no
// recovery path.

#[test]
fn clear_stale_oracle_proposal_fails_when_not_yet_expired() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		// Only 1 of 3 approvals -- proposal stays pending, not yet stale.
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));

		System::set_block_number(System::block_number() + ORACLE_PROPOSAL_EXPIRY as u64 - 1);
		assert_noop!(
			Courts::clear_stale_oracle_proposal(RuntimeOrigin::signed(m1), case_id),
			Error::<Test>::OracleProposalNotYetStale
		);
		assert!(crate::pallet::PendingOracleProposal::<Test>::get(case_id).is_some());
	});
}

#[test]
fn clear_stale_oracle_proposal_fails_when_none_pending() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		assert_noop!(
			Courts::clear_stale_oracle_proposal(RuntimeOrigin::signed(m1), 0),
			Error::<Test>::NoPendingOracleAction
		);
	});
}

#[test]
fn clear_stale_oracle_proposal_fails_for_non_member() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		System::set_block_number(System::block_number() + ORACLE_PROPOSAL_EXPIRY as u64);

		assert_noop!(
			Courts::clear_stale_oracle_proposal(RuntimeOrigin::signed(999), case_id),
			Error::<Test>::OracleMemberNotFound
		);
	});
}

/// The deadlock recovery this fix adds: once a pending oracle proposal sits without reaching
/// threshold past `OracleProposalExpiryBlocks`, *any* current Oracle Council member (not
/// necessarily the stuck proposer) can clear it, and a fresh proposal for the same case_id can
/// then be raised and successfully resolved -- proving the court system is no longer
/// permanently stuck. Mirrors `clear_stale_admin_action_unblocks_a_new_proposal_after_expiry`.
#[test]
fn clear_stale_oracle_proposal_unblocks_a_new_proposal_after_expiry() {
	new_test_ext().execute_with(|| {
		let (m1, m2, m3) = setup_three_member_oracle_council();
		let case_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::General));
		let model_version = approve_first_ai_model([7u8; 32]);

		// m1 proposes; only 1 of 3 approvals ever lands (m2/m3 never approve -- offline).
		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[7u8; 32],
			model_version,
			Verdict::Upheld
		));
		let proposed_at = System::block_number();

		// Before expiry, the case cannot be re-proposed.
		assert_noop!(
			Courts::submit_ai_ruling(
				RuntimeOrigin::signed(m3),
				case_id,
				[8u8; 32],
				model_version,
				Verdict::Upheld
			),
			Error::<Test>::OracleActionAlreadyProposed
		);

		// Move past the expiry window and clear the stale proposal -- called by m3, who never
		// approved the original proposal.
		System::set_block_number(proposed_at + ORACLE_PROPOSAL_EXPIRY as u64);
		assert_ok!(Courts::clear_stale_oracle_proposal(RuntimeOrigin::signed(m3), case_id));
		System::assert_last_event(Event::OracleProposalCleared { case_id }.into());
		assert!(crate::pallet::PendingOracleProposal::<Test>::get(case_id).is_none());
		assert!(crate::pallet::OracleApprovals::<Test>::get(case_id).is_none());

		// The case is still Filed (the stale proposal was never applied), so a fresh
		// submission can be proposed and this time driven to threshold.
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Filed);
		assert_eq!(ruling_hash, None);

		assert_ok!(Courts::submit_ai_ruling(
			RuntimeOrigin::signed(m1),
			case_id,
			[9u8; 32],
			model_version,
			Verdict::Upheld
		));
		assert_ok!(Courts::approve_ai_ruling(RuntimeOrigin::signed(m2), case_id));
		let (_, status, ruling_hash, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::AIRulingIssued);
		assert_eq!(ruling_hash, Some([9u8; 32]));
	});
}

/// The second half of the finding: a removed/compromised member's still-pending admin-action
/// approval must no longer count toward the M-of-N threshold, exactly like the existing
/// `remove_oracle_member_purges_stale_approval_from_in_flight_proposal` coverage for case-based
/// oracle proposals.
#[test]
fn remove_oracle_member_purges_stale_approval_from_pending_admin_action() {
	new_test_ext().execute_with(|| {
		let (m1, m2, m3) = setup_three_member_oracle_council();
		let call_hash = [52u8; 32];

		// m1 proposes and is auto-approved -- 1 of 3, below the 2-of-3 threshold.
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_none());

		// m1's key is compromised; root removes them mid-proposal.
		assert_ok!(Courts::remove_oracle_member(RuntimeOrigin::root(), m1));

		// The stale approval must be purged -- the pending entry (if it still exists) must not
		// contain m1, and the action must not have been resolved off of m1's stale approval
		// plus a single fresh one from a council that's now only {m2, m3}.
		if let Some((_, approvals)) = crate::pallet::PendingAdminAction::<Test>::get(call_hash) {
			assert!(!approvals.contains(&m1));
		}
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_none());

		// Remaining council is {m2, m3}: 1/2 strict majority now needs 2 of 2. If m1's stale
		// approval still counted, m2 approving alone would wrongly reach quorum (stale m1 +
		// fresh m2 == 2 of a size-2 council).
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));
		assert!(
			crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_none(),
			"m2 alone must not reach quorum on a 2-member council"
		);

		// A genuine second approval from the remaining council is required to resolve.
		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m3), call_hash));
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_some());
	});
}

#[test]
fn admin_action_rejects_double_approval_and_non_member() {
	new_test_ext().execute_with(|| {
		let (m1, _m2, _m3) = setup_three_member_oracle_council();
		let call_hash = [44u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert_noop!(
			Courts::approve_admin_action(RuntimeOrigin::signed(m1), call_hash),
			Error::<Test>::AlreadyApprovedOracleAction
		);
		assert_noop!(
			Courts::approve_admin_action(RuntimeOrigin::signed(999), call_hash),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn admin_action_rejects_re_proposing_a_call_hash_already_pending_or_approved() {
	new_test_ext().execute_with(|| {
		let (m1, m2, _m3) = setup_three_member_oracle_council();
		let call_hash = [45u8; 32];
		assert_ok!(Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash));
		assert_noop!(
			Courts::propose_admin_action(RuntimeOrigin::signed(m2), call_hash),
			Error::<Test>::OracleActionAlreadyProposed
		);

		assert_ok!(Courts::approve_admin_action(RuntimeOrigin::signed(m2), call_hash));
		assert!(crate::pallet::ApprovedAdminAction::<Test>::get(call_hash).is_some());
		assert_noop!(
			Courts::propose_admin_action(RuntimeOrigin::signed(m1), call_hash),
			Error::<Test>::OracleActionAlreadyProposed
		);
	});
}

// ─── TierConflict (Change 1: constitutional-law-tier-laundering fix) ───────────────────────
//
// `CaseSubject::TierConflict { law_id }` lets any citizen permissionlessly open a case alleging
// `law_id` was enacted at the wrong `LawTier` -- in production reached via
// `pallet-constitution::challenge_law_tier`, which delegates into `file_case_for` (exercised
// directly here since this pallet has no notion of `LawTier`/`Laws` itself). An Overturned
// ruling applies the same remedy as `LawChallenge`: `T::LawEnforcer::invalidate_law`.

#[test]
fn tier_conflict_case_can_be_opened_and_overturned_ruling_invalidates_the_law() {
	new_test_ext().execute_with(|| {
		let case_id =
			file_ai_rule_and_appeal(1, CaseSubject::TierConflict { law_id: 77 });
		set_window_hashes(case_id, &[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)]);
		capture_jury_seed(case_id);
		// TierConflict is a Level-2 (21-juror) subject, same as LawChallenge.
		assert_noop!(
			Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7),
			Error::<Test>::InvalidJurySize
		);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 21));
		let jury = JuryPool::<Test>::get(case_id).unwrap();

		for juror in jury.iter().take(11) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Overturned));
		}

		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Enforced);
		assert_eq!(crate::Rulings::<Test>::get(case_id), Some(Verdict::Overturned));
		System::assert_has_event(Event::RulingEnforced { case_id }.into());
		// Same enforcement as a LawChallenge Overturned ruling: the law is invalidated.
		assert_eq!(invalidated_laws(), vec![77]);
	});
}

#[test]
fn file_case_for_opens_a_tier_conflict_case_the_same_way_file_case_would() {
	// `file_case_for` is what pallet-constitution's `challenge_law_tier` (via `TierConflictHook`)
	// calls into -- proves it reuses the exact same citizen-filing pipeline (active-citizen
	// gate, ZK verification, bond reservation, anonymized storage) as the `file_case`
	// dispatchable itself, not a parallel reimplementation.
	new_test_ext().execute_with(|| {
		let case_id = crate::NextCaseId::<Test>::get();
		let nullifier = nullifier_for(1);
		set_citizen_nullifier(1, nullifier);
		assert_ok!(Courts::file_case_for(
			1,
			CaseSubject::TierConflict { law_id: 5 },
			valid_proof(),
			public_inputs(nullifier),
		));

		assert_eq!(Balances::reserved_balance(1), CASE_FILING_BOND);
		let (filer, status, _, subject) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(filer, CaseFiler::Nullifier(nullifier));
		assert_eq!(status, CaseStatus::Filed);
		assert_eq!(subject, CaseSubject::TierConflict { law_id: 5 });
	});
}

#[test]
fn tier_conflict_rejects_duplicate_while_one_is_open_and_allows_a_fresh_one_once_finalized() {
	new_test_ext().execute_with(|| {
		let first_id = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(1, CaseSubject::TierConflict { law_id: 42 }));

		assert_noop!(
			file_case_as(2, CaseSubject::TierConflict { law_id: 42 }),
			Error::<Test>::DuplicateTierConflict
		);
		assert_eq!(Balances::reserved_balance(2), 0);
		assert_eq!(crate::NextCaseId::<Test>::get(), first_id + 1);

		// A LawChallenge against the *same* law_id is now ALSO rejected while the TierConflict
		// is open. `OpenCaseByLaw` is keyed on law_id ALONE, so a LawChallenge and a
		// TierConflict against the same law are mutually exclusive with *each other*, not just
		// each with itself -- this is the corrected behavior; the old, buggy behavior (two
		// independent `OpenLawChallengeCase`/`OpenTierConflictCase` maps letting both coexist)
		// reintroduced the exact revert-and-strand bug these guards exist to prevent. See
		// `OpenCaseByLaw`'s doc comment.
		assert_noop!(
			file_case_as(3, CaseSubject::LawChallenge { law_id: 42 }),
			Error::<Test>::DuplicateLawChallenge
		);
		assert_eq!(Balances::reserved_balance(3), 0);

		// A TierConflict against a *different* law_id remains fully independent.
		assert_ok!(file_case_as(2, CaseSubject::TierConflict { law_id: 7 }));
	});
}

#[test]
fn law_challenge_rejects_duplicate_while_a_tier_conflict_is_open_for_the_same_law() {
	// Symmetric to the test above: opening a LawChallenge first, then attempting a
	// TierConflict against the same law_id, must also be rejected.
	new_test_ext().execute_with(|| {
		assert_ok!(file_case_as(1, CaseSubject::LawChallenge { law_id: 42 }));

		assert_noop!(
			file_case_as(2, CaseSubject::TierConflict { law_id: 42 }),
			Error::<Test>::DuplicateTierConflict
		);
		assert_eq!(Balances::reserved_balance(2), 0);
	});
}

#[test]
fn law_challenge_can_open_once_a_tier_conflict_against_the_same_law_finalizes() {
	// Cross-kind release: `OpenCaseByLaw`'s slot for a law_id is freed once the case actually
	// holding it (a TierConflict here) reaches FinalRuling, regardless of what kind of
	// law-targeting case opens next against that same law_id.
	new_test_ext().execute_with(|| {
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::TierConflict { law_id: 42 });
		set_window_hashes(
			case_id,
			&[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)],
		);
		capture_jury_seed(case_id);
		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 21));
		let jury = JuryPool::<Test>::get(case_id).unwrap();
		for juror in jury.iter().take(11) {
			assert_ok!(Courts::cast_jury_vote(RuntimeOrigin::signed(*juror), case_id, Verdict::Overturned));
		}
		let (_, status, _, _) = crate::pallet::Cases::<Test>::get(case_id).unwrap();
		assert_eq!(status, CaseStatus::Enforced);

		// A fresh LawChallenge (a *different* kind than the one that just resolved) against
		// the same law_id now succeeds.
		assert_ok!(file_case_as(2, CaseSubject::LawChallenge { law_id: 42 }));
	});
}

// ─── Anonymized vs. plain-AccountId filer storage/event (Change 2) ────────────────────────
//
// `LawChallenge`/`TreasuryDispute`/`TierConflict` are "citizen vs. institutional power" and
// must file under a ZK nullifier, never the signer's `AccountId`; `CitizenConduct`/`General`
// are citizen-vs-citizen/general and keep filing under the signer's plain `AccountId`, unchanged
// from before this fix.

#[test]
fn law_challenge_and_treasury_dispute_store_and_emit_a_nullifier_not_an_account_id() {
	new_test_ext().execute_with(|| {
		let case_id_a = crate::NextCaseId::<Test>::get();
		let nullifier_a = nullifier_for(11);
		assert_ok!(Courts::file_case(
			RuntimeOrigin::signed(11),
			CaseSubject::LawChallenge { law_id: 100 },
			Some(valid_proof()),
			Some(public_inputs(nullifier_a)),
		));
		let (filer_a, _, _, _) = crate::pallet::Cases::<Test>::get(case_id_a).unwrap();
		assert_eq!(filer_a, CaseFiler::Nullifier(nullifier_a));
		System::assert_has_event(
			Event::CaseFiled {
				case_id: case_id_a,
				filer: CaseFiler::Nullifier(nullifier_a),
				subject: CaseSubject::LawChallenge { law_id: 100 },
			}
			.into(),
		);

		let case_id_b = crate::NextCaseId::<Test>::get();
		let nullifier_b = nullifier_for(12);
		assert_ok!(Courts::file_case(
			RuntimeOrigin::signed(12),
			CaseSubject::TreasuryDispute { department_id: 3 },
			Some(valid_proof()),
			Some(public_inputs(nullifier_b)),
		));
		let (filer_b, _, _, _) = crate::pallet::Cases::<Test>::get(case_id_b).unwrap();
		assert_eq!(filer_b, CaseFiler::Nullifier(nullifier_b));
		System::assert_has_event(
			Event::CaseFiled {
				case_id: case_id_b,
				filer: CaseFiler::Nullifier(nullifier_b),
				subject: CaseSubject::TreasuryDispute { department_id: 3 },
			}
			.into(),
		);

		// Neither filing account's raw AccountId (11 or 12) ever appears as a `CaseFiler` --
		// only their nullifier does.
		assert_ne!(filer_a, CaseFiler::Account(11));
		assert_ne!(filer_b, CaseFiler::Account(12));
	});
}

#[test]
fn citizen_conduct_and_general_still_store_and_emit_a_plain_account_id() {
	new_test_ext().execute_with(|| {
		let case_id_a = crate::NextCaseId::<Test>::get();
		assert_ok!(file_case_as(21, CaseSubject::General));
		let (filer_a, _, _, _) = crate::pallet::Cases::<Test>::get(case_id_a).unwrap();
		assert_eq!(filer_a, CaseFiler::Account(21));

		let case_id_b = crate::NextCaseId::<Test>::get();
		let nullifier = [55u8; 32];
		assert_ok!(file_case_as(
			22,
			CaseSubject::CitizenConduct { nullifier, suspension_blocks: Some(5) },
		));
		let (filer_b, _, _, _) = crate::pallet::Cases::<Test>::get(case_id_b).unwrap();
		assert_eq!(filer_b, CaseFiler::Account(22));
		System::assert_has_event(
			Event::CaseFiled {
				case_id: case_id_b,
				filer: CaseFiler::Account(22),
				subject: CaseSubject::CitizenConduct { nullifier, suspension_blocks: Some(5) },
			}
			.into(),
		);
	});
}

// ─── file_case ZK-proof validation (anonymized case types) ────────────────────────────────

#[test]
fn file_case_rejects_law_challenge_with_missing_zk_proof() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Courts::file_case(
				RuntimeOrigin::signed(1),
				CaseSubject::LawChallenge { law_id: 1 },
				None,
				None,
			),
			Error::<Test>::MissingZkProof
		);
	});
}

#[test]
fn file_case_rejects_general_with_an_unexpected_zk_proof() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Courts::file_case(
				RuntimeOrigin::signed(1),
				CaseSubject::General,
				Some(valid_proof()),
				Some(public_inputs([1u8; 32])),
			),
			Error::<Test>::UnexpectedZkProof
		);
	});
}

#[test]
fn file_case_rejects_law_challenge_with_an_invalid_proof() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Courts::file_case(
				RuntimeOrigin::signed(1),
				CaseSubject::LawChallenge { law_id: 1 },
				Some(invalid_proof()),
				Some(public_inputs([1u8; 32])),
			),
			Error::<Test>::InvalidZkProof
		);
	});
}

#[test]
fn file_case_rejects_law_challenge_with_too_short_public_inputs() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Courts::file_case(
				RuntimeOrigin::signed(1),
				CaseSubject::LawChallenge { law_id: 1 },
				Some(valid_proof()),
				Some(too_short_public_inputs()),
			),
			Error::<Test>::MissingNullifierInput
		);
		assert_noop!(
			Courts::file_case(
				RuntimeOrigin::signed(1),
				CaseSubject::LawChallenge { law_id: 1 },
				Some(valid_proof()),
				Some(empty_public_inputs()),
			),
			Error::<Test>::MissingNullifierInput
		);
	});
}

#[test]
fn file_case_rejects_law_challenge_with_wrong_proof_scope() {
	new_test_ext().execute_with(|| {
		// Valid shape and a passing mock verifier, but scope/subscope from a different
		// purpose's proof (e.g. citizen registration) rather than this pallet's own
		// CASE_FILING_SERVICE_SCOPE/SUBSCOPE -- must not be accepted as a replay.
		assert_noop!(
			Courts::file_case(
				RuntimeOrigin::signed(1),
				CaseSubject::LawChallenge { law_id: 1 },
				Some(valid_proof()),
				Some(public_inputs_with([1u8; 32], [2u8; 32], [3u8; 32])),
			),
			Error::<Test>::InvalidProofScope
		);
	});
}

// ─── pick_random_jurors excludes an anonymized filer via their own registered nullifier ────

#[test]
fn select_jury_excludes_the_anonymized_law_challenge_filer_via_nullifier() {
	new_test_ext().execute_with(|| {
		// 8 citizens, 7-person jury: excluding the filer (account 1, matched via the nullifier
		// `file_case_as` registered for them) leaves exactly {2..=8}.
		set_citizen_count(8);
		let case_id = file_ai_rule_and_appeal(1, CaseSubject::TreasuryDispute { department_id: 9 });
		set_window_hashes(
			case_id,
			&[H256::repeat_byte(0x11), H256::repeat_byte(0x22), H256::repeat_byte(0x33)],
		);
		capture_jury_seed(case_id);

		assert_ok!(Courts::select_jury(RuntimeOrigin::signed(1), case_id, 7));
		let jury = JuryPool::<Test>::get(case_id).unwrap();
		assert_eq!(jury.len(), 7);
		assert!(
			!jury.contains(&1),
			"the anonymized case's own filer must never be seated on its jury, even though \
			 the case only stores their nullifier, not their AccountId"
		);
		let mut sorted = jury.into_inner();
		sorted.sort();
		assert_eq!(sorted, vec![2, 3, 4, 5, 6, 7, 8]);
	});
}
