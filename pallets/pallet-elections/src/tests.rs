use crate::{
    mock::*, BackingCount, BackingOf, BackingThreshold, BackingThresholdCeiling,
    BackingThresholdFloor, CitizenBackingCount,
    DelegateInfo, DelegateStatus, DelegateSweepCursor, Delegates,
    ElectionCycleBlocks, Error, Event,
    LastElectionBlock, LegislatureSeats, MandatoryBreakBlocks, MaxBackingsPerCitizen,
    MaxConsecutiveTerms, TermLengthBlocks, WarningWindowPct,
};
use frame_support::{assert_noop, assert_ok, traits::Hooks, traits::ConstU32, BoundedVec};
use sp_runtime::DispatchError;

// ─── helpers ─────────────────────────────────────────────────────────────────

fn name() -> BoundedVec<u8, ConstU32<64>> {
    BoundedVec::try_from(b"Alice".to_vec()).unwrap()
}

fn ipfs(byte: u8) -> [u8; 32] {
    [byte; 32]
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

// ─── on_initialize: bounded delegate sweep (LOW-severity griefing fix) ───────
//
// Previously `on_initialize` collected and iterated *every* `Delegates` entry, every block,
// unconditionally -- with `MaxDelegates` in the thousands in production, that's unbounded
// per-block weight. The fix bounds each block's sweep to `MaxDelegateSweepPerBlock` entries
// (10 in this mock) and resumes via `DelegateSweepCursor` on subsequent blocks.

#[test]
fn on_initialize_sweep_is_bounded_per_block_and_resumes_via_cursor() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // 12 delegates -- more than MAX_DELEGATE_SWEEP_PER_BLOCK (10) -- each activated by
        // three dedicated backers (so none of the per-backer/per-delegate caps interact),
        // all with term_start_block = 1.
        for d in 1..=12u64 {
            register_delegate(d);
            let backer_base = 100 + d * 10;
            back(backer_base, d);
            back(backer_base + 1, d);
            back(backer_base + 2, d);
            assert_eq!(Delegates::<Test>::get(d).unwrap().status, DelegateStatus::Active);
        }
        assert!(DelegateSweepCursor::<Test>::get().is_none());

        // warning_offset = (100 / 100) * (100 - 20) = 80 blocks after term start.
        System::set_block_number(1 + 80);
        let _ = Elections::on_initialize(System::block_number());

        let warned_after_first =
            (1..=12u64).filter(|d| Delegates::<Test>::get(d).unwrap().warning_emitted).count();
        // The actual fix: at most MAX_DELEGATE_SWEEP_PER_BLOCK delegates get examined in one
        // block, not all 12 -- before the fix this would already be 12 here.
        assert_eq!(warned_after_first, MAX_DELEGATE_SWEEP_PER_BLOCK as usize);
        // A full batch was consumed with more delegates left -- the cursor must be set so the
        // next block resumes after the last one examined, not from the beginning.
        assert!(DelegateSweepCursor::<Test>::get().is_some());

        // A second call (same block) resumes from the cursor and finishes the remaining 2.
        let _ = Elections::on_initialize(System::block_number());
        let warned_after_second =
            (1..=12u64).filter(|d| Delegates::<Test>::get(d).unwrap().warning_emitted).count();
        assert_eq!(warned_after_second, 12);
        // The sweep reached the end of the map and wrapped back to the start.
        assert!(DelegateSweepCursor::<Test>::get().is_none());
    });
}

#[test]
fn on_initialize_sweep_wraps_around_when_delegate_count_is_under_the_batch_size() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1);

        System::set_block_number(1 + 80);
        let _ = Elections::on_initialize(System::block_number());

        // Fewer delegates than the batch size -- the sweep reaches the end of the map in one
        // call, so the cursor must not be left dangling.
        assert!(Delegates::<Test>::get(1).unwrap().warning_emitted);
        assert!(DelegateSweepCursor::<Test>::get().is_none());
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

#[test]
fn on_initialize_election_excludes_suspended_delegates() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // crosses the backing threshold -> Active

        register_delegate(5);
        back(2, 5);
        back(3, 5);
        back(4, 5); // also Active

        assert_eq!(Delegates::<Test>::get(1).unwrap().status, DelegateStatus::Active);
        assert_eq!(Delegates::<Test>::get(5).unwrap().status, DelegateStatus::Active);

        // Delegate 1 was an active citizen when they registered and built backing, but has
        // since been suspended (e.g. an Overturned CitizenConduct court ruling). Their
        // DelegateStatus is still Active -- nothing re-runs registration -- so only the
        // seating-time re-check can catch this.
        set_active_citizen(1, false);

        System::set_block_number(DEFAULT_ELECTION_CYCLE_BLOCKS as u64);
        let _ = Elections::on_initialize(System::block_number());

        assert_eq!(seat_calls(), vec![vec![5]]);
    });
}

// ── legislature_call_hash (HIGH-severity motion-hijack fix) ────────────────────
//
// See the equivalent block in pallet-constitution's tests for the full rationale. This
// pallet has only one `GovernanceOrigin`-gated call, so there's no sibling call to collide
// with here -- we just confirm different arguments to `set_backing_threshold` hash
// differently (the property the origin's mismatch check depends on).
#[test]
fn legislature_call_hash_differs_for_different_thresholds() {
    let a = crate::pallet::legislature_call_hash(b"pallet-elections::set_backing_threshold", 3u32);
    let b = crate::pallet::legislature_call_hash(b"pallet-elections::set_backing_threshold", 4u32);
    assert_ne!(a, b);
}
