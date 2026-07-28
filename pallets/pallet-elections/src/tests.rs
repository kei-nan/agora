use crate::{
    mock::*, BackingCount, BackingOf, BackingThreshold, BackingThresholdCeiling,
    BackingThresholdFloor, CandidateCount, Candidates, CandidateStatus, CitizenBackingCount,
    Commissioners, DelegateInfo, DelegateStatus, Delegates, Elections as ElectionMap,
    ElectionCycleBlocks, ElectionStatus, Error, Event, LastElectionBlock, LegislatureSeats,
    MandatoryBreakBlocks, MaxBackingsPerCitizen, MaxConsecutiveTerms, NextElectionId,
    TermLengthBlocks, WarningWindowPct,
};
use frame_support::{assert_noop, assert_ok, traits::Hooks, traits::ConstU32, BoundedVec};
use sp_runtime::DispatchError;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn office() -> BoundedVec<u8, ConstU32<64>> {
    BoundedVec::try_from(b"President".to_vec()).unwrap()
}

fn name() -> BoundedVec<u8, ConstU32<64>> {
    BoundedVec::try_from(b"Alice".to_vec()).unwrap()
}

fn ipfs(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn add_commissioner(who: u64) {
    assert_ok!(Elections::add_commissioner(RuntimeOrigin::root(), who));
}

/// Creates an election (as commissioner `by`) and returns its id.
fn create_election(by: u64) -> u32 {
    let id = NextElectionId::<Test>::get();
    assert_ok!(Elections::create_election(
        RuntimeOrigin::signed(by),
        office(),
        1,
        1000,
    ));
    id
}

fn register_candidate(who: u64, election_id: u32) {
    set_active_citizen(who, true);
    assert_ok!(Elections::register_candidate(RuntimeOrigin::signed(who), election_id, ipfs(1)));
}

fn register_delegate(who: u64) {
    set_active_citizen(who, true);
    assert_ok!(Elections::register_as_delegate(RuntimeOrigin::signed(who), name(), ipfs(2)));
}

fn back(who: u64, delegate: u64) {
    set_active_citizen(who, true);
    assert_ok!(Elections::back_delegate(RuntimeOrigin::signed(who), delegate));
}

fn delegate_info_with_status(status: DelegateStatus) -> DelegateInfo<u64> {
    DelegateInfo {
        display_name: name(),
        profile_ipfs_hash: ipfs(2),
        status,
        consecutive_terms: 0,
        term_start_block: None,
        break_until_block: None,
        warning_emitted: false,
    }
}

// ─── add_commissioner / remove_commissioner ─────────────────────────────────

#[test]
fn add_commissioner_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_eq!(Commissioners::<Test>::get().into_inner(), vec![1]);
        System::assert_last_event(Event::CommissionerAdded { account: 1 }.into());
    });
}

#[test]
fn add_commissioner_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::add_commissioner(RuntimeOrigin::signed(1), 2),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn add_commissioner_fails_when_already_commissioner() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_noop!(
            Elections::add_commissioner(RuntimeOrigin::root(), 1),
            Error::<Test>::AlreadyCommissioner
        );
    });
}

#[test]
fn add_commissioner_fails_when_too_many() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        for who in 1..=MAX_COMMISSIONERS as u64 {
            add_commissioner(who);
        }

        assert_noop!(
            Elections::add_commissioner(RuntimeOrigin::root(), MAX_COMMISSIONERS as u64 + 1),
            Error::<Test>::TooManyCommissioners
        );
    });
}

#[test]
fn remove_commissioner_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_ok!(Elections::remove_commissioner(RuntimeOrigin::root(), 1));

        assert!(Commissioners::<Test>::get().is_empty());
        System::assert_last_event(Event::CommissionerRemoved { account: 1 }.into());
    });
}

#[test]
fn remove_commissioner_is_noop_when_not_a_commissioner() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        // Removing an account that was never a commissioner succeeds (no error) but is a
        // silent no-op: no event, no storage change.
        assert_ok!(Elections::remove_commissioner(RuntimeOrigin::root(), 2));

        assert_eq!(Commissioners::<Test>::get().into_inner(), vec![1]);
        System::assert_last_event(Event::CommissionerAdded { account: 1 }.into());
    });
}

#[test]
fn remove_commissioner_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_noop!(
            Elections::remove_commissioner(RuntimeOrigin::signed(1), 1),
            DispatchError::BadOrigin
        );
    });
}

// ─── create_election ─────────────────────────────────────────────────────────

#[test]
fn create_election_works_by_commissioner() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_ok!(Elections::create_election(RuntimeOrigin::signed(1), office(), 1, 1000));

        let election = ElectionMap::<Test>::get(0).unwrap();
        assert_eq!(election.status, ElectionStatus::Open);
        assert_eq!(NextElectionId::<Test>::get(), 1);
        System::assert_last_event(Event::ElectionCreated { id: 0 }.into());
    });
}

#[test]
fn create_election_works_by_root() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Elections::create_election(RuntimeOrigin::root(), office(), 1, 1000));

        assert!(ElectionMap::<Test>::get(0).is_some());
    });
}

#[test]
fn create_election_fails_for_non_commissioner_signed_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::create_election(RuntimeOrigin::signed(1), office(), 1, 1000),
            Error::<Test>::NotCommissioner
        );
    });
}

#[test]
fn create_election_increments_next_election_id() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        assert_ok!(Elections::create_election(RuntimeOrigin::signed(1), office(), 1, 1000));
        assert_ok!(Elections::create_election(RuntimeOrigin::signed(1), office(), 1, 1000));

        assert_eq!(NextElectionId::<Test>::get(), 2);
        assert!(ElectionMap::<Test>::get(0).is_some());
        assert!(ElectionMap::<Test>::get(1).is_some());
    });
}

// ─── register_candidate ──────────────────────────────────────────────────────

#[test]
fn register_candidate_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);

        register_candidate(2, id);

        let info = Candidates::<Test>::get(id, 2).unwrap();
        assert_eq!(info.status, CandidateStatus::Registered);
        assert_eq!(info.deposit, CANDIDATE_DEPOSIT);
        assert_eq!(Balances::reserved_balance(2), CANDIDATE_DEPOSIT);
        System::assert_last_event(Event::CandidateRegistered { election_id: id, candidate: 2 }.into());
    });
}

#[test]
fn register_candidate_fails_when_not_active_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);

        assert_noop!(
            Elections::register_candidate(RuntimeOrigin::signed(2), id, ipfs(1)),
            Error::<Test>::NotActiveCitizen
        );
        assert_eq!(Balances::reserved_balance(2), 0);
    });
}

#[test]
fn register_candidate_fails_when_election_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_active_citizen(2, true);

        assert_noop!(
            Elections::register_candidate(RuntimeOrigin::signed(2), 42, ipfs(1)),
            Error::<Test>::ElectionNotFound
        );
    });
}

#[test]
fn register_candidate_fails_when_election_not_open() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);
        assert_ok!(Elections::submit_results(RuntimeOrigin::signed(1), id, 2, ipfs(9)));

        // Election is now ResultsSubmitted, not Open.
        set_active_citizen(3, true);
        assert_noop!(
            Elections::register_candidate(RuntimeOrigin::signed(3), id, ipfs(1)),
            Error::<Test>::ElectionNotOpen
        );
    });
}

#[test]
fn register_candidate_fails_when_already_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);

        assert_noop!(
            Elections::register_candidate(RuntimeOrigin::signed(2), id, ipfs(3)),
            Error::<Test>::CandidateAlreadyRegistered
        );
        // Still only reserved once, not double-reserved.
        assert_eq!(Balances::reserved_balance(2), CANDIDATE_DEPOSIT);
    });
}

#[test]
fn register_candidate_fails_with_insufficient_balance_and_leaves_no_dangling_reserve() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        // Account 99 was never funded in genesis.
        set_active_citizen(99, true);

        assert_noop!(
            Elections::register_candidate(RuntimeOrigin::signed(99), id, ipfs(1)),
            Error::<Test>::InsufficientBalance
        );
        assert_eq!(Balances::reserved_balance(99), 0);
        assert!(Candidates::<Test>::get(id, 99).is_none());
    });
}

#[test]
fn register_candidate_beyond_max_candidates_per_election_is_rejected() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);

        for who in 1..=MAX_CANDIDATES_PER_ELECTION as u64 {
            register_candidate(who, id);
        }
        assert_eq!(CandidateCount::<Test>::get(id), MAX_CANDIDATES_PER_ELECTION);

        // One more, once the election is already at capacity, must be rejected — and must not
        // reserve a deposit or otherwise mutate state.
        let extra = MAX_CANDIDATES_PER_ELECTION as u64 + 1;
        set_active_citizen(extra, true);
        assert_noop!(
            Elections::register_candidate(RuntimeOrigin::signed(extra), id, ipfs(1)),
            Error::<Test>::TooManyCandidates
        );
        assert_eq!(
            Candidates::<Test>::iter_prefix(id).count(),
            MAX_CANDIDATES_PER_ELECTION as usize
        );
    });
}

// ─── certify_candidate ───────────────────────────────────────────────────────

#[test]
fn certify_candidate_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);

        assert_ok!(Elections::certify_candidate(RuntimeOrigin::signed(1), id, 2));

        assert_eq!(Candidates::<Test>::get(id, 2).unwrap().status, CandidateStatus::Certified);
        System::assert_last_event(Event::CandidateCertified { election_id: id, candidate: 2 }.into());
    });
}

#[test]
fn certify_candidate_fails_for_non_commissioner() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);

        assert_noop!(
            Elections::certify_candidate(RuntimeOrigin::signed(2), id, 2),
            Error::<Test>::NotCommissioner
        );
    });
}

#[test]
fn certify_candidate_fails_when_election_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_noop!(
            Elections::certify_candidate(RuntimeOrigin::signed(1), 42, 2),
            Error::<Test>::ElectionNotFound
        );
    });
}

#[test]
fn certify_candidate_fails_when_candidate_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);

        assert_noop!(
            Elections::certify_candidate(RuntimeOrigin::signed(1), id, 2),
            Error::<Test>::CandidateNotFound
        );
    });
}

// ─── submit_results / certify_results ───────────────────────────────────────

#[test]
fn submit_results_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);

        assert_ok!(Elections::submit_results(RuntimeOrigin::signed(1), id, 2, ipfs(9)));

        let election = ElectionMap::<Test>::get(id).unwrap();
        assert_eq!(election.status, ElectionStatus::ResultsSubmitted);
        assert_eq!(election.winner, Some(2));
        assert_eq!(election.results_ipfs_hash, Some(ipfs(9)));
        System::assert_last_event(Event::ResultsSubmitted { election_id: id, winner: 2 }.into());
    });
}

#[test]
fn submit_results_fails_for_non_commissioner() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);

        assert_noop!(
            Elections::submit_results(RuntimeOrigin::signed(2), id, 2, ipfs(9)),
            Error::<Test>::NotCommissioner
        );
    });
}

#[test]
fn submit_results_fails_when_election_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_noop!(
            Elections::submit_results(RuntimeOrigin::signed(1), 42, 2, ipfs(9)),
            Error::<Test>::ElectionNotFound
        );
    });
}

#[test]
fn submit_results_fails_when_not_open() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        assert_ok!(Elections::submit_results(RuntimeOrigin::signed(1), id, 2, ipfs(9)));

        assert_noop!(
            Elections::submit_results(RuntimeOrigin::signed(1), id, 3, ipfs(8)),
            Error::<Test>::ElectionNotOpen
        );
    });
}

#[test]
fn certify_results_works_and_unreserves_all_candidate_deposits() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);
        register_candidate(3, id);
        assert_ok!(Elections::submit_results(RuntimeOrigin::signed(1), id, 2, ipfs(9)));

        assert_ok!(Elections::certify_results(RuntimeOrigin::signed(1), id));

        assert_eq!(ElectionMap::<Test>::get(id).unwrap().status, ElectionStatus::Certified);
        // Both the winner and the loser get their deposits back.
        assert_eq!(Balances::reserved_balance(2), 0);
        assert_eq!(Balances::reserved_balance(3), 0);
        System::assert_last_event(Event::ElectionCertified { election_id: id, winner: 2 }.into());
    });
}

#[test]
fn certify_results_fails_for_non_commissioner() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);
        assert_ok!(Elections::submit_results(RuntimeOrigin::signed(1), id, 2, ipfs(9)));

        assert_noop!(
            Elections::certify_results(RuntimeOrigin::signed(2), id),
            Error::<Test>::NotCommissioner
        );
    });
}

#[test]
fn certify_results_fails_when_election_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);

        assert_noop!(
            Elections::certify_results(RuntimeOrigin::signed(1), 42),
            Error::<Test>::ElectionNotFound
        );
    });
}

#[test]
fn certify_results_fails_when_results_not_submitted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);

        assert_noop!(
            Elections::certify_results(RuntimeOrigin::signed(1), id),
            Error::<Test>::ResultsNotSubmitted
        );
    });
}

#[test]
fn certify_results_fails_when_already_certified() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_commissioner(1);
        let id = create_election(1);
        register_candidate(2, id);
        assert_ok!(Elections::submit_results(RuntimeOrigin::signed(1), id, 2, ipfs(9)));
        assert_ok!(Elections::certify_results(RuntimeOrigin::signed(1), id));

        assert_noop!(
            Elections::certify_results(RuntimeOrigin::signed(1), id),
            Error::<Test>::AlreadyCertified
        );
    });
}

// ─── register_as_delegate ────────────────────────────────────────────────────

#[test]
fn register_as_delegate_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);

        let info = Delegates::<Test>::get(1).unwrap();
        assert_eq!(info.status, DelegateStatus::Pending);
        assert_eq!(info.display_name, name());
        System::assert_last_event(
            Event::DelegateRegistered { delegate: 1, display_name: name() }.into(),
        );
    });
}

#[test]
fn register_as_delegate_fails_when_not_active_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::register_as_delegate(RuntimeOrigin::signed(1), name(), ipfs(2)),
            Error::<Test>::NotActiveCitizen
        );
    });
}

#[test]
fn register_as_delegate_fails_when_already_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);

        assert_noop!(
            Elections::register_as_delegate(RuntimeOrigin::signed(1), name(), ipfs(2)),
            Error::<Test>::AlreadyRegisteredAsDelegate
        );
    });
}

// ─── back_delegate / remove_backing ──────────────────────────────────────────

#[test]
fn back_delegate_stays_pending_below_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);

        back(2, 1); // 1 backer, threshold is 3

        assert_eq!(BackingCount::<Test>::get(1), 1);
        assert_eq!(Delegates::<Test>::get(1).unwrap().status, DelegateStatus::Pending);
        System::assert_last_event(Event::DelegateBacked { delegate: 1, backer: 2 }.into());
    });
}

#[test]
fn back_delegate_activates_at_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(5);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // 3rd backer crosses DEFAULT_BACKING_THRESHOLD (3)

        let info = Delegates::<Test>::get(1).unwrap();
        assert_eq!(info.status, DelegateStatus::Active);
        assert_eq!(info.term_start_block, Some(5));
        assert_eq!(BackingCount::<Test>::get(1), 3);
        System::assert_last_event(Event::DelegateActivated { delegate: 1 }.into());
    });
}

#[test]
fn back_delegate_fails_when_backer_not_active_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1),
            Error::<Test>::NotActiveCitizen
        );
    });
}

#[test]
fn back_delegate_fails_when_backing_self() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(1), 1),
            Error::<Test>::CannotBackSelf
        );
    });
}

#[test]
fn back_delegate_fails_when_delegate_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_active_citizen(2, true);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1),
            Error::<Test>::DelegateNotFound
        );
    });
}

#[test]
fn back_delegate_fails_when_delegate_on_break() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Seed a delegate directly in the OnBreak state to isolate this check from the
        // (separately tested) term-limit machinery that produces it in practice.
        Delegates::<Test>::insert(1, delegate_info_with_status(DelegateStatus::OnBreak));
        set_active_citizen(2, true);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1),
            Error::<Test>::DelegateOnBreak
        );
    });
}

#[test]
fn back_delegate_fails_when_already_backing() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1),
            Error::<Test>::AlreadyBacking
        );
    });
}

#[test]
fn back_delegate_fails_when_backing_limit_reached() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        register_delegate(2);
        register_delegate(3);
        register_delegate(4);
        register_delegate(5);
        register_delegate(6);
        register_delegate(7); // 7 delegates, cap is DEFAULT_MAX_BACKINGS_PER_CITIZEN (6)

        set_active_citizen(10, true);
        for delegate in 1..=6u64 {
            assert_ok!(Elections::back_delegate(RuntimeOrigin::signed(10), delegate));
        }
        assert_eq!(CitizenBackingCount::<Test>::get(10), 6);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(10), 7),
            Error::<Test>::BackingLimitReached
        );
    });
}

#[test]
fn remove_backing_works_and_deactivates_below_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // activates at 3

        assert_ok!(Elections::remove_backing(RuntimeOrigin::signed(4), 1));

        assert_eq!(BackingCount::<Test>::get(1), 2);
        assert_eq!(CitizenBackingCount::<Test>::get(4), 0);
        assert!(!BackingOf::<Test>::contains_key(4, 1));
        assert_eq!(Delegates::<Test>::get(1).unwrap().status, DelegateStatus::Pending);
        System::assert_last_event(Event::DelegateDeactivated { delegate: 1 }.into());
    });
}

#[test]
fn remove_backing_works_without_deactivating_when_still_above_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1);
        back(5, 1); // 4 backers, still above threshold(3) after one removal

        assert_ok!(Elections::remove_backing(RuntimeOrigin::signed(5), 1));

        assert_eq!(BackingCount::<Test>::get(1), 3);
        assert_eq!(Delegates::<Test>::get(1).unwrap().status, DelegateStatus::Active);
        System::assert_last_event(
            Event::DelegateBackingRemoved { delegate: 1, backer: 5 }.into(),
        );
    });
}

#[test]
fn remove_backing_fails_when_not_backing() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);

        assert_noop!(
            Elections::remove_backing(RuntimeOrigin::signed(2), 1),
            Error::<Test>::NotBacking
        );
    });
}

// ─── set_backing_threshold / set_backing_bounds ──────────────────────────────

#[test]
fn set_backing_threshold_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Elections::set_backing_threshold(RuntimeOrigin::root(), 5));

        assert_eq!(BackingThreshold::<Test>::get(), 5);
        System::assert_last_event(Event::BackingThresholdChanged { new_threshold: 5 }.into());
    });
}

#[test]
fn set_backing_threshold_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_backing_threshold(RuntimeOrigin::signed(1), 5),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn set_backing_threshold_fails_below_floor() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Default floor is DEFAULT_BACKING_THRESHOLD_FLOOR (1).
        assert_noop!(
            Elections::set_backing_threshold(RuntimeOrigin::root(), 0),
            Error::<Test>::ThresholdBelowFloor
        );
    });
}

#[test]
fn set_backing_threshold_fails_above_ceiling() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Default ceiling is DEFAULT_BACKING_THRESHOLD_CEILING (10).
        assert_noop!(
            Elections::set_backing_threshold(RuntimeOrigin::root(), 11),
            Error::<Test>::ThresholdAboveCeiling
        );
    });
}

#[test]
fn set_backing_bounds_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Elections::set_backing_bounds(RuntimeOrigin::root(), 2, 8));

        assert_eq!(BackingThresholdFloor::<Test>::get(), 2);
        assert_eq!(BackingThresholdCeiling::<Test>::get(), 8);
        System::assert_last_event(Event::BackingBoundsChanged { floor: 2, ceiling: 8 }.into());
    });
}

#[test]
fn set_backing_bounds_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_backing_bounds(RuntimeOrigin::signed(1), 2, 8),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn set_backing_bounds_fails_when_floor_exceeds_ceiling() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_backing_bounds(RuntimeOrigin::root(), 8, 2),
            Error::<Test>::FloorExceedsCeiling
        );
    });
}

#[test]
fn set_backing_bounds_clamps_current_threshold_down_to_new_ceiling() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Current threshold defaults to 3; lowering the ceiling below it must clamp down.
        assert_ok!(Elections::set_backing_bounds(RuntimeOrigin::root(), 1, 2));

        assert_eq!(BackingThreshold::<Test>::get(), 2);
    });
}

#[test]
fn set_backing_bounds_clamps_current_threshold_up_to_new_floor() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Current threshold defaults to 3; raising the floor above it must clamp up.
        assert_ok!(Elections::set_backing_bounds(RuntimeOrigin::root(), 5, 10));

        assert_eq!(BackingThreshold::<Test>::get(), 5);
    });
}

// ─── set_term_params / set_election_params ───────────────────────────────────

#[test]
fn set_term_params_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Elections::set_term_params(RuntimeOrigin::root(), 50, 3, 20, 10));

        assert_eq!(TermLengthBlocks::<Test>::get(), 50);
        assert_eq!(MaxConsecutiveTerms::<Test>::get(), 3);
        assert_eq!(MandatoryBreakBlocks::<Test>::get(), 20);
        assert_eq!(WarningWindowPct::<Test>::get(), 10);
        System::assert_last_event(
            Event::TermParamsChanged {
                term_length: 50,
                max_consecutive: 3,
                mandatory_break: 20,
                warning_pct: 10,
            }
            .into(),
        );
    });
}

#[test]
fn set_term_params_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_term_params(RuntimeOrigin::signed(1), 50, 3, 20, 10),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn set_term_params_fails_when_warning_pct_zero() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_term_params(RuntimeOrigin::root(), 50, 3, 20, 0),
            Error::<Test>::WarningPctInvalid
        );
    });
}

#[test]
fn set_term_params_fails_when_warning_pct_above_fifty() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_term_params(RuntimeOrigin::root(), 50, 3, 20, 51),
            Error::<Test>::WarningPctInvalid
        );
    });
}

#[test]
fn set_election_params_works_with_partial_update() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Elections::set_election_params(RuntimeOrigin::root(), Some(7), None, Some(2)));

        assert_eq!(LegislatureSeats::<Test>::get(), 7);
        // cycle_blocks left unchanged.
        assert_eq!(ElectionCycleBlocks::<Test>::get(), DEFAULT_ELECTION_CYCLE_BLOCKS);
        assert_eq!(MaxBackingsPerCitizen::<Test>::get(), 2);
        System::assert_last_event(
            Event::ElectionParamsChanged {
                seats: 7,
                cycle_blocks: DEFAULT_ELECTION_CYCLE_BLOCKS,
                max_backings_per_citizen: 2,
            }
            .into(),
        );
    });
}

#[test]
fn set_election_params_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_election_params(RuntimeOrigin::signed(1), Some(7), None, None),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn set_election_params_fails_when_seats_zero() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_election_params(RuntimeOrigin::root(), Some(0), None, None),
            Error::<Test>::ElectionSeatsZero
        );
    });
}

#[test]
fn set_election_params_fails_when_cycle_blocks_zero() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Elections::set_election_params(RuntimeOrigin::root(), None, Some(0), None),
            Error::<Test>::ElectionCycleBlocksZero
        );
    });
}

// ─── on_initialize: term warnings, expirations, mandatory breaks ────────────

#[test]
fn on_initialize_emits_term_warning_within_window() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // Active, term_start_block = 1

        // warning_offset = (100 / 100) * (100 - 20) = 80 blocks after term start.
        System::set_block_number(1 + 80);
        let _ = Elections::on_initialize(System::block_number());

        assert!(Delegates::<Test>::get(1).unwrap().warning_emitted);
        System::assert_last_event(
            Event::DelegateTermWarning { delegate: 1, blocks_remaining: 20 }.into(),
        );
    });
}

#[test]
fn on_initialize_does_not_emit_warning_before_window() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1);

        System::set_block_number(1 + 79);
        let _ = Elections::on_initialize(System::block_number());

        assert!(!Delegates::<Test>::get(1).unwrap().warning_emitted);
    });
}

#[test]
fn on_initialize_expires_term_and_renews_when_under_max_consecutive() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // term_start_block = 1, term_length = 100

        System::set_block_number(1 + 100);
        let _ = Elections::on_initialize(System::block_number());

        let info = Delegates::<Test>::get(1).unwrap();
        assert_eq!(info.consecutive_terms, 1);
        assert_eq!(info.status, DelegateStatus::Active);
        assert_eq!(info.term_start_block, Some(101));
        assert!(!info.warning_emitted);
        System::assert_last_event(Event::DelegateTermExpired { delegate: 1 }.into());
    });
}

#[test]
fn on_initialize_triggers_mandatory_break_at_max_consecutive_terms() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // term_start_block = 1

        // First term expiry: consecutive_terms 0 -> 1 (under max of 2), term renews.
        System::set_block_number(1 + 100);
        let _ = Elections::on_initialize(System::block_number());
        // Second term expiry: consecutive_terms 1 -> 2 (== max), triggers OnBreak.
        System::set_block_number(101 + 100);
        let _ = Elections::on_initialize(System::block_number());

        let info = Delegates::<Test>::get(1).unwrap();
        assert_eq!(info.consecutive_terms, 2);
        assert_eq!(info.status, DelegateStatus::OnBreak);
        assert_eq!(info.break_until_block, Some(201 + DEFAULT_MANDATORY_BREAK_BLOCKS as u64));
    });
}

#[test]
fn on_initialize_ends_break_and_reactivates_when_still_above_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1);
        System::set_block_number(101);
        let _ = Elections::on_initialize(101);
        System::set_block_number(201);
        let _ = Elections::on_initialize(201); // now OnBreak, break_until = 211

        System::set_block_number(211);
        let _ = Elections::on_initialize(211);

        let info = Delegates::<Test>::get(1).unwrap();
        // Backers (2,3,4) never left, so BackingCount (3) is still >= threshold (3):
        // the delegate is reactivated directly rather than parked in Pending.
        assert_eq!(info.status, DelegateStatus::Active);
        assert_eq!(info.consecutive_terms, 0);
        assert_eq!(info.term_start_block, Some(211));
        System::assert_has_event(Event::DelegateBreakEnded { delegate: 1 }.into());
        System::assert_last_event(Event::DelegateActivated { delegate: 1 }.into());
    });
}

#[test]
fn on_initialize_ends_break_to_pending_when_below_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1);
        System::set_block_number(101);
        let _ = Elections::on_initialize(101);
        System::set_block_number(201);
        let _ = Elections::on_initialize(201); // now OnBreak, break_until = 211

        // Backers desert during the break, dropping BackingCount below threshold.
        assert_ok!(Elections::remove_backing(RuntimeOrigin::signed(2), 1));
        assert_ok!(Elections::remove_backing(RuntimeOrigin::signed(3), 1));

        System::set_block_number(211);
        let _ = Elections::on_initialize(211);

        let info = Delegates::<Test>::get(1).unwrap();
        assert_eq!(info.status, DelegateStatus::Pending);
        assert_eq!(info.consecutive_terms, 0);
        assert!(info.term_start_block.is_none());
        System::assert_last_event(Event::DelegateBreakEnded { delegate: 1 }.into());
    });
}

// ─── on_initialize: legislature election cycle ───────────────────────────────

#[test]
fn on_initialize_does_not_run_election_before_cycle_boundary() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1);

        System::set_block_number(DEFAULT_ELECTION_CYCLE_BLOCKS as u64 - 1);
        let _ = Elections::on_initialize(System::block_number());

        assert!(seat_calls().is_empty());
        assert_eq!(LastElectionBlock::<Test>::get(), 0);
    });
}

#[test]
fn on_initialize_runs_election_and_seats_top_n_active_delegates_by_backing() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // 4 delegates (accounts 1-4), backed by accounts 5-10, with distinct backing counts
        // and LegislatureSeats defaulting to 3 — so the lowest-backed delegate (4) must be
        // excluded even though it clears the Active threshold.
        for d in 1..=4u64 {
            register_delegate(d);
        }
        let backers: [u64; 6] = [5, 6, 7, 8, 9, 10];
        // Delegate 1: 6 backers, Delegate 2: 5, Delegate 3: 4, Delegate 4: 3.
        for (i, &b) in backers.iter().enumerate() {
            let _ = i;
            back(b, 1);
        }
        for &b in &backers[0..5] {
            back(b, 2);
        }
        for &b in &backers[0..4] {
            back(b, 3);
        }
        for &b in &backers[0..3] {
            back(b, 4);
        }

        assert_eq!(BackingCount::<Test>::get(1), 6);
        assert_eq!(BackingCount::<Test>::get(2), 5);
        assert_eq!(BackingCount::<Test>::get(3), 4);
        assert_eq!(BackingCount::<Test>::get(4), 3);
        for d in 1..=4u64 {
            assert_eq!(Delegates::<Test>::get(d).unwrap().status, DelegateStatus::Active);
        }

        System::set_block_number(DEFAULT_ELECTION_CYCLE_BLOCKS as u64);
        let _ = Elections::on_initialize(System::block_number());

        assert_eq!(seat_calls(), vec![vec![1, 2, 3]]);
        assert_eq!(LastElectionBlock::<Test>::get(), DEFAULT_ELECTION_CYCLE_BLOCKS as u64);
        System::assert_last_event(
            Event::LegislatureElectionRun {
                at_block: DEFAULT_ELECTION_CYCLE_BLOCKS as u64,
                seated: 3,
            }
            .into(),
        );
    });
}

#[test]
fn on_initialize_election_excludes_pending_delegates() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // Active

        register_delegate(5);
        back(6, 5); // only 1 backer, stays Pending

        System::set_block_number(DEFAULT_ELECTION_CYCLE_BLOCKS as u64);
        let _ = Elections::on_initialize(System::block_number());

        assert_eq!(seat_calls(), vec![vec![1]]);
    });
}
