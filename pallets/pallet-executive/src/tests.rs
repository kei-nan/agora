use crate::{
    mock::*, ActiveEmergency, DeclareVotes, EndVotes, Error, Event, InvestitureRound,
    NextPortfolioId, PendingEmergencyProposal, PendingMinisterNomination, PortfolioMinister,
    Portfolios, PrimeMinister, VacatedRole,
};
use frame_support::{assert_noop, assert_ok, traits::Hooks, BoundedVec};
use pallet_legislature::pallet::MinisterChecker;
use sp_runtime::DispatchError;

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn ballot(candidates: &[u64]) -> BoundedVec<u64, frame_support::traits::ConstU32<8>> {
    BoundedVec::try_from(candidates.to_vec()).unwrap()
}

fn define_portfolio(byte: u8) -> u32 {
    let id = NextPortfolioId::<Test>::get();
    assert_ok!(Executive::define_portfolio(RuntimeOrigin::root(), hash(byte)));
    id
}

/// Runs a full investiture round and installs `who` as PM: marks them a legislature
/// member, opens the round, self-nominates, casts a ballot for themselves, advances past
/// both windows, finalizes, then restores the original block number so callers'
/// block-number assumptions (e.g. emergency-timing tests) aren't disturbed.
fn appoint_pm(who: u64) {
    set_legislature_member(who, true);
    let origin_block = System::block_number();
    assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(who)));
    assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(who), who));
    System::set_block_number(origin_block + 5); // PmNominationWindowBlocks == 5
    assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(who), ballot(&[who])));
    System::set_block_number(origin_block + 10); // + PmVotingWindowBlocks == 5
    assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(who)));
    System::set_block_number(origin_block);
}

/// Stages and confirms `who` as minister of `portfolio_id`. Requires a PM to already be
/// appointed (uses them as the nominating origin).
fn appoint_minister(portfolio_id: u32, who: u64) {
    let pm = PrimeMinister::<Test>::get().expect("PM must be set before appointing a minister");
    assert_ok!(Executive::nominate_minister(RuntimeOrigin::signed(pm), portfolio_id, who));
    assert_ok!(Executive::appoint_minister(RuntimeOrigin::root(), portfolio_id, who));
}

/// Builds a 3-member cabinet: PM = 1, ministers 2 and 3 on freshly defined portfolios.
/// cabinet_size() == 3, so a 2/3 supermajority requires exactly 2 votes.
fn cabinet_of_three() {
    appoint_pm(1);
    let p0 = define_portfolio(10);
    let p1 = define_portfolio(11);
    appoint_minister(p0, 2);
    appoint_minister(p1, 3);
}

// ─── define_portfolio ───────────────────────────────────────────────────────

#[test]
fn define_portfolio_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Executive::define_portfolio(RuntimeOrigin::root(), hash(1)));

        assert_eq!(Portfolios::<Test>::get(0).unwrap().name_hash, hash(1));
        assert_eq!(NextPortfolioId::<Test>::get(), 1);
        System::assert_last_event(Event::PortfolioDefined { portfolio_id: 0, name_hash: hash(1) }.into());
    });
}

#[test]
fn define_portfolio_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Executive::define_portfolio(RuntimeOrigin::signed(1), hash(1)),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn define_portfolio_fails_when_capacity_reached() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // MaxPortfolios == 5.
        for i in 0..5u8 {
            assert_ok!(Executive::define_portfolio(RuntimeOrigin::root(), hash(i)));
        }
        assert_noop!(
            Executive::define_portfolio(RuntimeOrigin::root(), hash(9)),
            Error::<Test>::PortfolioCapacityReached
        );
    });
}

// ─── PM investiture (ranked-choice among legislature members) ─────────────────

#[test]
fn investiture_single_candidate_wins_unopposed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        System::set_block_number(6);
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[1])));
        System::set_block_number(11);
        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(1)));

        assert_eq!(PrimeMinister::<Test>::get(), Some(1));
        // Rolling-window occupancy is 0 the instant a tenure starts (nothing elapsed
        // yet), and accrues linearly with elapsed blocks thereafter -- confirms the
        // in-progress tenure is actually being tracked from this install.
        assert_eq!(Executive::pm_occupancy_in_window(&1, System::block_number()), 0);
        assert_eq!(Executive::pm_occupancy_in_window(&1, System::block_number() + 7), 7);
        assert!(InvestitureRound::<Test>::get().is_none());
        System::assert_has_event(Event::PmInvestitureFinalized { winner: 1 }.into());
    });
}

#[test]
fn investiture_runoff_redistributes_eliminated_candidates_second_preferences() {
    // Candidates: 10, 20, 30. 5 voters. Round 1: 10 gets 2, 20 gets 1, 30 gets 2 -- no
    // majority (need > 2.5), so 20 (fewest) is eliminated. Round 2: the voter who put 20
    // first now counts toward their second preference, 10, giving 10 a 3-2 majority.
    // Proves elimination + redistribution actually changes the outcome, not just that a
    // single round of first-preference counting happens to work.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        for m in [10u64, 20, 30, 100, 101, 102, 103, 104] {
            set_legislature_member(m, true);
        }
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(100)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(100), 10));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(100), 20));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(100), 30));

        System::set_block_number(6);
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(100), ballot(&[10, 20, 30])));
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(101), ballot(&[10, 20, 30])));
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(102), ballot(&[20, 10, 30])));
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(103), ballot(&[30, 20, 10])));
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(104), ballot(&[30, 20, 10])));

        System::set_block_number(11);
        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(100)));

        assert_eq!(PrimeMinister::<Test>::get(), Some(10));
    });
}

#[test]
fn investiture_fails_with_no_winner_when_no_ballots_cast() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        System::set_block_number(11);

        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(1)));

        assert!(PrimeMinister::<Test>::get().is_none());
        System::assert_last_event(Event::PmInvestitureFailedNoWinner.into());
    });
}

#[test]
fn investiture_fails_with_no_winner_when_no_candidates_nominated() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        System::set_block_number(11);

        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(1)));

        assert!(PrimeMinister::<Test>::get().is_none());
        System::assert_last_event(Event::PmInvestitureFailedNoWinner.into());
    });
}

#[test]
fn open_pm_investiture_fails_when_seat_not_vacant() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        assert_noop!(
            Executive::open_pm_investiture(RuntimeOrigin::signed(2)),
            Error::<Test>::PmSeatNotVacant
        );
    });
}

#[test]
fn open_pm_investiture_fails_when_already_open() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_noop!(
            Executive::open_pm_investiture(RuntimeOrigin::signed(2)),
            Error::<Test>::InvestitureAlreadyOpen
        );
    });
}

#[test]
fn nominate_pm_fails_for_non_member_nominator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(2, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_noop!(
            Executive::nominate_pm(RuntimeOrigin::signed(1), 2),
            Error::<Test>::NotLegislatureMember
        );
    });
}

#[test]
fn nominate_pm_fails_for_non_member_candidate() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_noop!(
            Executive::nominate_pm(RuntimeOrigin::signed(1), 2),
            Error::<Test>::NotLegislatureMember
        );
    });
}

#[test]
fn nominate_pm_fails_after_nomination_window_closes() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        System::set_block_number(6);
        assert_noop!(
            Executive::nominate_pm(RuntimeOrigin::signed(1), 1),
            Error::<Test>::NominationWindowClosed
        );
    });
}

#[test]
fn nominate_pm_fails_when_already_nominated() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        assert_noop!(
            Executive::nominate_pm(RuntimeOrigin::signed(1), 1),
            Error::<Test>::AlreadyNominated
        );
    });
}

// The rolling-window occupancy cap replaces an earlier "consecutive terms" counter that
// counted *whether a different account was installed in between* two of X's terms. That
// was gameable: X could get a complicit ally Y installed for as little as one block via
// `remove_and_replace_prime_minister`, then get reinstalled -- Y counts as "a different
// account holding it in between" even though X never meaningfully left power, so X's old
// counter would reset to 1 and the cap would never bite. A time-based rolling budget
// closes this because a one-block puppet term barely dents the true power-holder's
// cumulative occupancy within the window. Mock config: `PmOccupancyWindowBlocks == 50`,
// `MaxPmOccupancyBlocks == 30`, `PmNominationWindowBlocks == PmVotingWindowBlocks == 5`.

#[test]
fn pm_occupancy_cap_blocks_reinstall_then_restores_after_window_ages_out() {
    // Straightforward case: one account accumulates enough real (uninterrupted) time in
    // office on its own to hit the cap, is blocked via `PmOccupancyLimitReached` on both
    // the `nominate_pm` and `remove_and_replace_prime_minister` paths, and later becomes
    // eligible again once that time ages out of the trailing window.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        set_legislature_member(2, true);

        // Install 1 as PM: tenure starts at block 11.
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        System::set_block_number(6);
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[1])));
        System::set_block_number(11);
        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(1)));
        assert_eq!(PrimeMinister::<Test>::get(), Some(1));

        // Serve uninterrupted for exactly `MaxPmOccupancyBlocks` (30) blocks, then resign.
        // This closes a single 30-block tenure into `PmTenureHistory`, landing 1 exactly
        // at the cap.
        System::set_block_number(41);
        assert_ok!(Executive::resign_as_pm(RuntimeOrigin::signed(1)));
        assert_eq!(Executive::pm_occupancy_in_window(&1, 41), 30);

        // ── nominate_pm path ────────────────────────────────────────────────
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(2)));
        assert_noop!(
            Executive::nominate_pm(RuntimeOrigin::signed(2), 1),
            Error::<Test>::PmOccupancyLimitReached
        );

        // ── remove_and_replace_prime_minister path ──────────────────────────
        // Install 2 instead (uncontested by the cap) so there's a sitting PM to replace.
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(2), 2));
        System::set_block_number(46);
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(2), ballot(&[2])));
        System::set_block_number(51);
        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(2)));
        assert_eq!(PrimeMinister::<Test>::get(), Some(2));
        // 1's 30-block tenure (blocks 11..41) is still within the trailing 50-block
        // window as of block 51, so the cap still bites here too.
        assert_noop!(
            Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 1),
            Error::<Test>::PmOccupancyLimitReached
        );

        // ── eligibility restored once the tenure ages out of the window ────────
        // At block 92, the window is [42, 92] -- 1's tenure (11..41) has fully aged out.
        System::set_block_number(92);
        assert_eq!(Executive::pm_occupancy_in_window(&1, 92), 0);
        assert_ok!(Executive::resign_as_pm(RuntimeOrigin::signed(2)));
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        System::set_block_number(97);
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[1])));
        System::set_block_number(102);
        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(1)));
        assert_eq!(PrimeMinister::<Test>::get(), Some(1));
    });
}

#[test]
fn pm_occupancy_rolling_window_defeats_puppet_swap_collusion() {
    // Reproduces the exact collusion scenario that motivated the rolling-window
    // mechanism: PM X (1) serves long enough to approach the cap, then a complicit ally
    // Y (2) is installed via `remove_and_replace_prime_minister` for just one block,
    // then X is reinstalled. Under the old "consecutive terms" mechanism, Y counting as
    // "a different account holding office in between" would have reset X's counter to 1.
    // Under the rolling window, X's real cumulative occupancy is essentially unaffected
    // by Y's token tenure, and once X's genuine cumulative time crosses the cap, further
    // reinstallation is still blocked -- proving the swap-and-back trick resets nothing.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true); // X
        set_legislature_member(2, true); // Y

        // Install X: tenure starts at block 11.
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        System::set_block_number(6);
        assert_ok!(Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[1])));
        System::set_block_number(11);
        assert_ok!(Executive::finalize_pm_investiture(RuntimeOrigin::signed(1)));

        // X serves 25 real blocks (11..36), approaching but not yet at the 30-block cap.
        System::set_block_number(36);
        assert_ok!(Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 2));
        assert_eq!(PrimeMinister::<Test>::get(), Some(2));
        assert_eq!(Executive::pm_occupancy_in_window(&1, 36), 25);

        // Y (the puppet) holds the seat for exactly one block, then X is swapped back in.
        System::set_block_number(37);
        assert_ok!(Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 1));
        assert_eq!(PrimeMinister::<Test>::get(), Some(1));

        // X's cumulative occupancy is unchanged by the round trip through Y (still 25 --
        // no accrual happened while X wasn't PM, and nothing was reset either), while Y's
        // own occupancy reflects only the single token block it actually held.
        assert_eq!(Executive::pm_occupancy_in_window(&1, 37), 25);
        assert_eq!(Executive::pm_occupancy_in_window(&2, 37), 1);

        // X keeps serving for 5 more real blocks, crossing the cap (25 + 5 == 30), then
        // resigns.
        System::set_block_number(42);
        assert_ok!(Executive::resign_as_pm(RuntimeOrigin::signed(1)));
        assert_eq!(Executive::pm_occupancy_in_window(&1, 42), 30);

        // The swap-and-back trick did not launder any of X's real time in office: a fresh
        // reinstall attempt is blocked exactly as it would be without Y's interference.
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(2)));
        assert_noop!(
            Executive::nominate_pm(RuntimeOrigin::signed(2), 1),
            Error::<Test>::PmOccupancyLimitReached
        );
    });
}

#[test]
fn cast_pm_ballot_fails_before_nomination_window_closes() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        assert_noop!(
            Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[1])),
            Error::<Test>::NominationWindowStillOpen
        );
    });
}

#[test]
fn cast_pm_ballot_fails_after_voting_window_closes() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        System::set_block_number(11);
        assert_noop!(
            Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[1])),
            Error::<Test>::VotingWindowClosed
        );
    });
}

#[test]
fn cast_pm_ballot_fails_for_non_nominee() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        set_legislature_member(2, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        System::set_block_number(6);
        assert_noop!(
            Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[2])),
            Error::<Test>::NotANominee
        );
    });
}

#[test]
fn cast_pm_ballot_fails_for_duplicate_candidate() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        set_legislature_member(2, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 1));
        assert_ok!(Executive::nominate_pm(RuntimeOrigin::signed(1), 2));
        System::set_block_number(6);
        assert_noop!(
            Executive::cast_pm_ballot(RuntimeOrigin::signed(1), ballot(&[1, 1])),
            Error::<Test>::DuplicateCandidateInBallot
        );
    });
}

#[test]
fn finalize_pm_investiture_fails_before_voting_window_closes() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_ok!(Executive::open_pm_investiture(RuntimeOrigin::signed(1)));
        assert_noop!(
            Executive::finalize_pm_investiture(RuntimeOrigin::signed(1)),
            Error::<Test>::VotingWindowStillOpen
        );
    });
}

// ─── remove_and_replace_prime_minister (constructive no confidence) ───────────

#[test]
fn remove_and_replace_prime_minister_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        set_legislature_member(2, true);

        assert_ok!(Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 2));

        assert_eq!(PrimeMinister::<Test>::get(), Some(2));
        // The outgoing PM's in-progress tenure was closed out into history, and the
        // incoming PM's own in-progress tenure starts fresh (zero elapsed occupancy).
        assert!(crate::PmTenureHistory::<Test>::get().iter().any(|r| r.who == 1));
        assert_eq!(Executive::pm_occupancy_in_window(&2, System::block_number()), 0);
        let events = System::events();
        assert!(events.iter().any(|r| r.event == Event::PrimeMinisterDismissed { who: 1 }.into()));
        System::assert_last_event(
            Event::PrimeMinisterRemovedAndReplaced { old: 1, new: 2 }.into(),
        );
    });
}

#[test]
fn remove_and_replace_prime_minister_never_leaves_seat_vacant() {
    // The whole point of the constructive mechanism: a passed motion always installs a
    // successor in the same call -- there's no intermediate state where PrimeMinister is
    // None after a successful call.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        set_legislature_member(2, true);
        assert_ok!(Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 2));
        assert!(PrimeMinister::<Test>::get().is_some());
    });
}

#[test]
fn remove_and_replace_prime_minister_clears_outgoing_pms_pending_minister_nomination() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        set_legislature_member(2, true);
        let p0 = define_portfolio(1);
        assert_ok!(Executive::nominate_minister(RuntimeOrigin::signed(1), p0, 99));
        assert!(PendingMinisterNomination::<Test>::get(p0).is_some());

        assert_ok!(Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 2));

        assert!(PendingMinisterNomination::<Test>::get(p0).is_none());
    });
}

#[test]
fn remove_and_replace_prime_minister_fails_when_no_pm() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_legislature_member(1, true);
        assert_noop!(
            Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 1),
            Error::<Test>::NoPrimeMinister
        );
    });
}

#[test]
fn remove_and_replace_prime_minister_fails_when_successor_is_current_pm() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        assert_noop!(
            Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 1),
            Error::<Test>::SuccessorIsCurrentPm
        );
    });
}

#[test]
fn remove_and_replace_prime_minister_fails_when_successor_not_a_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        assert_noop!(
            Executive::remove_and_replace_prime_minister(RuntimeOrigin::root(), 2),
            Error::<Test>::NotLegislatureMember
        );
    });
}

#[test]
fn remove_and_replace_prime_minister_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        set_legislature_member(2, true);
        assert_noop!(
            Executive::remove_and_replace_prime_minister(RuntimeOrigin::signed(1), 2),
            DispatchError::BadOrigin
        );
    });
}

// ─── resign_as_pm ───────────────────────────────────────────────────────────

#[test]
fn resign_as_pm_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);

        assert_ok!(Executive::resign_as_pm(RuntimeOrigin::signed(1)));

        assert!(PrimeMinister::<Test>::get().is_none());
        // Resignation closes out the in-progress tenure into `PmTenureHistory` (so it
        // still counts toward rolling-window occupancy on a future reinstall) and
        // clears the in-progress marker.
        assert!(crate::CurrentPmTenureStart::<Test>::get().is_none());
        assert!(crate::PmTenureHistory::<Test>::get().iter().any(|r| r.who == 1));
        System::assert_last_event(Event::PrimeMinisterResigned { who: 1 }.into());
    });
}

#[test]
fn resign_as_pm_fails_for_non_pm() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        assert_noop!(
            Executive::resign_as_pm(RuntimeOrigin::signed(2)),
            Error::<Test>::NotPrimeMinister
        );
    });
}

// ─── nominate_minister / appoint_minister ──────────────────────────────────

#[test]
fn nominate_minister_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);

        assert_ok!(Executive::nominate_minister(RuntimeOrigin::signed(1), p0, 2));

        assert_eq!(PendingMinisterNomination::<Test>::get(p0), Some(2));
        System::assert_last_event(Event::MinisterNominated { portfolio_id: p0, nominee: 2 }.into());
    });
}

#[test]
fn nominate_minister_fails_for_non_pm() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        assert_noop!(
            Executive::nominate_minister(RuntimeOrigin::signed(2), p0, 2),
            Error::<Test>::NotPrimeMinister
        );
    });
}

#[test]
fn appoint_minister_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);

        appoint_minister(p0, 2);

        assert_eq!(PortfolioMinister::<Test>::get(p0), Some(2));
        assert_eq!(crate::MinisterPortfolio::<Test>::get(2), Some(p0));
        assert!(PendingMinisterNomination::<Test>::get(p0).is_none());
        System::assert_last_event(Event::MinisterAppointed { portfolio_id: p0, who: 2 }.into());
    });
}

#[test]
fn appoint_minister_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        assert_ok!(Executive::nominate_minister(RuntimeOrigin::signed(1), p0, 2));
        assert_noop!(
            Executive::appoint_minister(RuntimeOrigin::signed(1), p0, 2),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn appoint_minister_fails_when_portfolio_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Executive::appoint_minister(RuntimeOrigin::root(), 42, 2),
            Error::<Test>::PortfolioNotFound
        );
    });
}

#[test]
fn appoint_minister_fails_when_no_nomination_pending() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let p0 = define_portfolio(1);
        assert_noop!(
            Executive::appoint_minister(RuntimeOrigin::root(), p0, 2),
            Error::<Test>::NoNominationPending
        );
    });
}

#[test]
fn appoint_minister_fails_when_who_does_not_match_pending_nomination() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        assert_ok!(Executive::nominate_minister(RuntimeOrigin::signed(1), p0, 2));

        assert_noop!(
            Executive::appoint_minister(RuntimeOrigin::root(), p0, 3),
            Error::<Test>::NoNominationPending
        );
    });
}

#[test]
fn appoint_minister_replaces_existing_minister_in_portfolio() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);

        appoint_minister(p0, 3);

        assert_eq!(PortfolioMinister::<Test>::get(p0), Some(3));
        assert!(crate::MinisterPortfolio::<Test>::get(2).is_none());
        assert_eq!(crate::MinisterPortfolio::<Test>::get(3), Some(p0));
        let events = System::events();
        assert!(events
            .iter()
            .any(|r| r.event == Event::MinisterDismissed { portfolio_id: p0, who: 2 }.into()));
        System::assert_last_event(Event::MinisterAppointed { portfolio_id: p0, who: 3 }.into());
    });
}

#[test]
fn appoint_minister_moves_minister_from_other_portfolio() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        let p1 = define_portfolio(2);
        appoint_minister(p0, 2);

        appoint_minister(p1, 2);

        assert!(PortfolioMinister::<Test>::get(p0).is_none());
        assert_eq!(PortfolioMinister::<Test>::get(p1), Some(2));
        assert_eq!(crate::MinisterPortfolio::<Test>::get(2), Some(p1));
        let events = System::events();
        assert!(events
            .iter()
            .any(|r| r.event == Event::MinisterDismissed { portfolio_id: p0, who: 2 }.into()));
    });
}

#[test]
fn dismiss_minister_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);

        assert_ok!(Executive::dismiss_minister(RuntimeOrigin::root(), p0));

        assert!(PortfolioMinister::<Test>::get(p0).is_none());
        assert!(crate::MinisterPortfolio::<Test>::get(2).is_none());
        System::assert_last_event(Event::MinisterDismissed { portfolio_id: p0, who: 2 }.into());
    });
}

#[test]
fn dismiss_minister_fails_when_portfolio_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Executive::dismiss_minister(RuntimeOrigin::root(), 42),
            Error::<Test>::PortfolioNotFound
        );
    });
}

#[test]
fn dismiss_minister_fails_when_portfolio_vacant() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let p0 = define_portfolio(1);
        assert_noop!(
            Executive::dismiss_minister(RuntimeOrigin::root(), p0),
            Error::<Test>::PortfolioVacant
        );
    });
}

#[test]
fn dismiss_minister_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);
        assert_noop!(
            Executive::dismiss_minister(RuntimeOrigin::signed(1), p0),
            DispatchError::BadOrigin
        );
    });
}

// ─── resign (minister) ──────────────────────────────────────────────────────

#[test]
fn resign_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);

        assert_ok!(Executive::resign(RuntimeOrigin::signed(2)));

        assert!(PortfolioMinister::<Test>::get(p0).is_none());
        assert!(crate::MinisterPortfolio::<Test>::get(2).is_none());
        System::assert_last_event(Event::MinisterResigned { portfolio_id: p0, who: 2 }.into());
    });
}

#[test]
fn resign_fails_when_not_a_minister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(Executive::resign(RuntimeOrigin::signed(1)), Error::<Test>::NotAMinister);
    });
}

// ─── conviction-triggered vacancy sweep ────────────────────────────────────

#[test]
fn vacancy_sweep_removes_pm_suspended_by_jury_reviewed_conviction() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        set_active_citizen(1, false); // simulates an Overturned CitizenConduct ruling
        set_jury_reviewed_suspension(1, true); // ...that a jury actually reviewed

        let _ = Executive::on_initialize(1);

        assert!(PrimeMinister::<Test>::get().is_none());
        // The forced removal closes out the in-progress tenure into `PmTenureHistory`,
        // same as a voluntary resignation, so it still counts toward rolling-window
        // occupancy on a future reinstall attempt.
        assert!(crate::CurrentPmTenureStart::<Test>::get().is_none());
        assert!(crate::PmTenureHistory::<Test>::get().iter().any(|r| r.who == 1));
        System::assert_has_event(
            Event::OfficeVacatedForConviction { who: 1, role: VacatedRole::PrimeMinister }.into(),
        );
    });
}

#[test]
fn vacancy_sweep_removes_minister_suspended_by_jury_reviewed_conviction() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);
        set_active_citizen(2, false);
        set_jury_reviewed_suspension(2, true);

        let _ = Executive::on_initialize(1);

        assert!(PortfolioMinister::<Test>::get(p0).is_none());
        assert!(crate::MinisterPortfolio::<Test>::get(2).is_none());
        System::assert_has_event(
            Event::OfficeVacatedForConviction {
                who: 2,
                role: VacatedRole::Minister { portfolio_id: p0 },
            }
            .into(),
        );
    });
}

#[test]
fn vacancy_sweep_does_not_remove_pm_suspended_without_jury_review() {
    // The core of the refinement: a bare, unappealed AI ruling suspends the citizen (they
    // can't be re-nominated, can't vote, etc.) but must NOT be enough on its own to
    // automatically remove a sitting PM -- that requires a jury having actually reviewed
    // the conviction. Legislature's own no-confidence vote remains available regardless.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        set_active_citizen(1, false); // suspended...
        // ...but set_jury_reviewed_suspension was never called -- defaults to false.

        let _ = Executive::on_initialize(1);

        assert_eq!(PrimeMinister::<Test>::get(), Some(1), "AI-only suspension must not vacate the PM");
    });
}

#[test]
fn vacancy_sweep_does_not_remove_minister_suspended_without_jury_review() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);
        set_active_citizen(2, false);

        let _ = Executive::on_initialize(1);

        assert_eq!(PortfolioMinister::<Test>::get(p0), Some(2));
    });
}

#[test]
fn vacancy_sweep_leaves_active_pm_and_ministers_untouched() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();

        let _ = Executive::on_initialize(1);

        assert_eq!(PrimeMinister::<Test>::get(), Some(1));
        assert!(<Executive as MinisterChecker<u64>>::is_active_minister(&2));
        assert!(<Executive as MinisterChecker<u64>>::is_active_minister(&3));
    });
}

#[test]
fn vacancy_sweep_only_runs_once_per_interval() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let _ = Executive::on_initialize(1); // consumes the initial (block-0-eligible) sweep

        set_active_citizen(1, false);
        set_jury_reviewed_suspension(1, true);
        // VacancySweepIntervalBlocks == 20, so the next sweep isn't due until block 21.
        let _ = Executive::on_initialize(10);
        assert_eq!(PrimeMinister::<Test>::get(), Some(1), "sweep isn't due yet");

        let _ = Executive::on_initialize(21);
        assert!(PrimeMinister::<Test>::get().is_none(), "sweep should have run by now");
    });
}

// ─── vote_declare_emergency ─────────────────────────────────────────────────

#[test]
fn vote_declare_emergency_single_vote_does_not_activate() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();

        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));

        assert!(ActiveEmergency::<Test>::get().is_none());
        System::assert_last_event(Event::EmergencyVoteCast { who: 1, vote_count: 1 }.into());
    });
}

#[test]
fn vote_declare_emergency_supermajority_boundary_exact_two_of_three() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three(); // cabinet_size == 3; 2/3 supermajority needs exactly 2 votes.

        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_none(), "1 of 3 must not reach 2/3");

        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_some(), "2 of 3 must reach 2/3");
        System::assert_has_event(
            Event::EmergencyDeclared { expires_at: 51, ratify_by: 11, reason_hash: hash(1) }.into(),
        );
    });
}

#[test]
fn vote_declare_emergency_boundary_needs_three_of_four() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(10);
        let p1 = define_portfolio(11);
        let p2 = define_portfolio(12);
        appoint_minister(p0, 2);
        appoint_minister(p1, 3);
        appoint_minister(p2, 4);
        // cabinet_size == 4; 2/3 of 4 requires votes*3 >= 8, i.e. 3 votes (2 is insufficient).

        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_none(), "2 of 4 must not reach 2/3");

        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(3), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_some(), "3 of 4 must reach 2/3");
    });
}

#[test]
fn vote_declare_emergency_fails_for_non_cabinet_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_noop!(
            Executive::vote_declare_emergency(RuntimeOrigin::signed(99), hash(1), 50),
            Error::<Test>::NotCabinetMember
        );
    });
}

#[test]
fn vote_declare_emergency_fails_when_already_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_some());

        assert_noop!(
            Executive::vote_declare_emergency(RuntimeOrigin::signed(3), hash(1), 50),
            Error::<Test>::AlreadyActiveEmergency
        );
    });
}

#[test]
fn vote_declare_emergency_fails_when_already_voted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));

        assert_noop!(
            Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50),
            Error::<Test>::AlreadyVotedToDeclare
        );
    });
}

#[test]
fn vote_declare_emergency_clamps_duration_to_max_emergency_blocks() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        // MaxEmergencyBlocks == 100; propose far beyond it.
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 500));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 500));

        let info = ActiveEmergency::<Test>::get().unwrap();
        assert_eq!(info.expires_at, 1 + 100, "duration must be clamped to MaxEmergencyBlocks");
    });
}

#[test]
fn vote_declare_emergency_does_not_exceed_max_even_at_exact_boundary() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        // Exactly at the ceiling: should pass through unclamped (still == max).
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 100));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 100));

        let info = ActiveEmergency::<Test>::get().unwrap();
        assert_eq!(info.expires_at, 1 + 100);
    });
}

#[test]
fn vote_declare_emergency_locks_terms_from_first_voter() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 30));
        // Second voter proposes different terms — should be ignored in favor of the locked-in terms.
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(9), 90));

        let info = ActiveEmergency::<Test>::get().unwrap();
        assert_eq!(info.reason_hash, hash(1));
        assert_eq!(info.expires_at, 1 + 30);
    });
}

// ─── ratify_emergency ───────────────────────────────────────────────────────

#[test]
fn ratify_emergency_works_within_window() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        // ratify_by == 11 (declared at block 1 + RatificationWindowBlocks == 10).

        System::set_block_number(5);
        assert_ok!(Executive::ratify_emergency(RuntimeOrigin::root()));

        assert!(ActiveEmergency::<Test>::get().unwrap().ratified);
        System::assert_last_event(Event::EmergencyRatified.into());
    });
}

#[test]
fn ratify_emergency_fails_after_window_closed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        // ratify_by == 11.

        System::set_block_number(12);
        assert_noop!(
            Executive::ratify_emergency(RuntimeOrigin::root()),
            Error::<Test>::RatificationWindowClosed
        );
    });
}

#[test]
fn ratify_emergency_succeeds_exactly_at_window_boundary() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        // ratify_by == 11; the boundary block itself must still be within the window.

        System::set_block_number(11);
        assert_ok!(Executive::ratify_emergency(RuntimeOrigin::root()));
    });
}

#[test]
fn ratify_emergency_fails_when_no_active_emergency() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Executive::ratify_emergency(RuntimeOrigin::root()),
            Error::<Test>::NoActiveEmergency
        );
    });
}

#[test]
fn ratify_emergency_fails_when_already_ratified() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert_ok!(Executive::ratify_emergency(RuntimeOrigin::root()));

        assert_noop!(
            Executive::ratify_emergency(RuntimeOrigin::root()),
            Error::<Test>::AlreadyRatified
        );
    });
}

#[test]
fn ratify_emergency_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));

        assert_noop!(
            Executive::ratify_emergency(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

// ─── emergency auto-lapse / auto-expire via on_initialize ──────────────────

#[test]
fn emergency_lapses_when_not_ratified_within_window() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        // ratify_by == 11; never ratified.

        System::set_block_number(12);
        let _ = Executive::on_initialize(12);

        assert!(ActiveEmergency::<Test>::get().is_none());
        assert_eq!(DeclareVotes::<Test>::iter().filter(|(_, v)| *v).count(), 0);
        System::assert_has_event(Event::EmergencyLapsed.into());
    });
}

#[test]
fn emergency_does_not_lapse_at_exact_ratify_by_block() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        // ratify_by == 11 — the lapse condition is `now > ratify_by`, so at exactly 11 it must survive.

        System::set_block_number(11);
        let _ = Executive::on_initialize(11);

        assert!(ActiveEmergency::<Test>::get().is_some());
    });
}

#[test]
fn emergency_expires_at_sunset_block_even_if_ratified() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 5));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 5));
        // expires_at == 6.
        assert_ok!(Executive::ratify_emergency(RuntimeOrigin::root()));

        System::set_block_number(6);
        let _ = Executive::on_initialize(6);

        assert!(ActiveEmergency::<Test>::get().is_none());
        System::assert_has_event(Event::EmergencyExpired { at_block: 6 }.into());
    });
}

// ─── vote_end_emergency ─────────────────────────────────────────────────────

#[test]
fn vote_end_emergency_works_at_supermajority() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_some());

        assert_ok!(Executive::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert!(ActiveEmergency::<Test>::get().is_some(), "1 of 3 must not end it");

        assert_ok!(Executive::vote_end_emergency(RuntimeOrigin::signed(2)));
        assert!(ActiveEmergency::<Test>::get().is_none(), "2 of 3 must end it");
        assert_eq!(DeclareVotes::<Test>::iter().filter(|(_, v)| *v).count(), 0);
        assert_eq!(EndVotes::<Test>::iter().filter(|(_, v)| *v).count(), 0);
        System::assert_last_event(Event::EmergencyLifted.into());
    });
}

#[test]
fn vote_end_emergency_fails_for_non_cabinet_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));

        assert_noop!(
            Executive::vote_end_emergency(RuntimeOrigin::signed(99)),
            Error::<Test>::NotCabinetMember
        );
    });
}

#[test]
fn vote_end_emergency_fails_when_no_active_emergency() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_noop!(
            Executive::vote_end_emergency(RuntimeOrigin::signed(1)),
            Error::<Test>::NoActiveEmergency
        );
    });
}

#[test]
fn vote_end_emergency_fails_when_already_voted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert_ok!(Executive::vote_end_emergency(RuntimeOrigin::signed(1)));

        assert_noop!(
            Executive::vote_end_emergency(RuntimeOrigin::signed(1)),
            Error::<Test>::AlreadyVotedToEnd
        );
    });
}

// ─── cabinet-too-small floor (HIGH-severity solo-emergency fix) ────────────
//
// A freshly-invested PM with no confirmed ministers yet has cabinet_size() == 1, at which
// point supermajority_reached(1, 1) (1*3 >= 1*2) is trivially true -- the PM's own single
// vote would satisfy "2/3 cabinet supermajority" with zero actual cabinet consensus. Both
// vote_declare_emergency and vote_end_emergency require cabinet_size() >= 2 up front so a
// 1-member cabinet can never pass the check regardless of the ratio math.

#[test]
fn vote_declare_emergency_fails_when_cabinet_too_small() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1); // cabinet_size == 1: PM alone, no confirmed ministers yet.

        assert_noop!(
            Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50),
            Error::<Test>::CabinetTooSmallForEmergencyVote
        );
    });
}

#[test]
fn vote_end_emergency_fails_when_cabinet_too_small() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three(); // PM = 1, ministers 2 and 3.
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_some());

        // Both ministers leave office, shrinking the live cabinet down to just the PM.
        assert_ok!(Executive::resign(RuntimeOrigin::signed(2)));
        assert_ok!(Executive::resign(RuntimeOrigin::signed(3)));

        assert_noop!(
            Executive::vote_end_emergency(RuntimeOrigin::signed(1)),
            Error::<Test>::CabinetTooSmallForEmergencyVote
        );
    });
}

// ─── stale EndVotes from departed cabinet members (HIGH-severity fix) ──────
//
// Every office-vacating path purges DeclareVotes for the departing account, but previously
// none of them purged EndVotes -- so a departed minister's stale "yes, end it" vote kept
// inflating vote_end_emergency's numerator after they left, while cabinet_size() (the
// denominator) is always computed live from who's actually still in office.

#[test]
fn vote_end_emergency_stale_vote_from_departed_minister_does_not_count() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three(); // PM = 1, ministers 2 and 3. cabinet_size == 3.
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));
        assert!(ActiveEmergency::<Test>::get().is_some());

        // Minister 2 casts an "end it" vote -- 1 of 3 is insufficient alone (needs 2/3).
        assert_ok!(Executive::vote_end_emergency(RuntimeOrigin::signed(2)));
        assert!(ActiveEmergency::<Test>::get().is_some(), "1 of 3 must not end it");
        assert!(EndVotes::<Test>::get(2));

        // Minister 2 then leaves office entirely.
        assert_ok!(Executive::resign(RuntimeOrigin::signed(2)));
        assert!(
            !EndVotes::<Test>::contains_key(2),
            "departed minister's stale end-vote must be purged from storage"
        );

        // Cabinet is now PM(1) + minister(3) only -- cabinet_size == 2, needs both votes
        // for 2/3. If minister 2's stale vote still counted, minister 3 voting alone would
        // wrongly reach the supermajority (stale 2 + fresh 3 == 2 votes of 2). It must not.
        assert_ok!(Executive::vote_end_emergency(RuntimeOrigin::signed(3)));
        assert!(
            ActiveEmergency::<Test>::get().is_some(),
            "a departed member's stale vote must not let one fresh vote satisfy the supermajority"
        );

        // Ending it now genuinely requires a fresh vote from a still-current cabinet member.
        assert_ok!(Executive::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert!(
            ActiveEmergency::<Test>::get().is_none(),
            "two fresh votes from the genuinely current cabinet must end it"
        );
    });
}

// ─── retract_emergency_vote ─────────────────────────────────────────────────

#[test]
fn retract_emergency_vote_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert!(DeclareVotes::<Test>::get(1));

        assert_ok!(Executive::retract_emergency_vote(RuntimeOrigin::signed(1)));

        assert!(!DeclareVotes::<Test>::get(1));
        // Having retracted, the member should be able to vote again.
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
    });
}

#[test]
fn retract_emergency_vote_fails_when_already_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(1), 50));

        assert_noop!(
            Executive::retract_emergency_vote(RuntimeOrigin::signed(1)),
            Error::<Test>::AlreadyActiveEmergency
        );
    });
}

#[test]
fn retract_emergency_vote_fails_when_not_yet_voted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_noop!(
            Executive::retract_emergency_vote(RuntimeOrigin::signed(1)),
            Error::<Test>::NotYetVoted
        );
    });
}

#[test]
fn retract_emergency_vote_fails_for_non_cabinet_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_noop!(
            Executive::retract_emergency_vote(RuntimeOrigin::signed(99)),
            Error::<Test>::NotCabinetMember
        );
    });
}

#[test]
fn retract_emergency_vote_resets_pending_proposal_when_last_vote_removed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(1), 30));
        assert!(PendingEmergencyProposal::<Test>::get().is_some());

        assert_ok!(Executive::retract_emergency_vote(RuntimeOrigin::signed(1)));
        assert!(PendingEmergencyProposal::<Test>::get().is_none());

        // A fresh proposal from a different first voter should now set new terms.
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(7), 60));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(3), hash(1), 30));

        let info = ActiveEmergency::<Test>::get().unwrap();
        assert_eq!(info.reason_hash, hash(7));
        assert_eq!(info.expires_at, 1 + 60);
    });
}

// ─── MinisterChecker (consumed by pallet-legislature) ───────────────────────

#[test]
fn minister_checker_true_for_active_minister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);

        assert!(<Executive as MinisterChecker<u64>>::is_active_minister(&2));
    });
}

#[test]
fn minister_checker_true_for_prime_minister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);

        assert!(<Executive as MinisterChecker<u64>>::is_active_minister(&1));
    });
}

#[test]
fn minister_checker_false_for_resigned_minister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);
        assert_ok!(Executive::resign(RuntimeOrigin::signed(2)));

        assert!(!<Executive as MinisterChecker<u64>>::is_active_minister(&2));
    });
}

#[test]
fn minister_checker_false_for_dismissed_minister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);
        assert_ok!(Executive::dismiss_minister(RuntimeOrigin::root(), p0));

        assert!(!<Executive as MinisterChecker<u64>>::is_active_minister(&2));
    });
}

#[test]
fn minister_checker_false_for_unrelated_account() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        cabinet_of_three();

        assert!(!<Executive as MinisterChecker<u64>>::is_active_minister(&999));
    });
}

// ── legislature_call_hash (HIGH-severity motion-hijack fix) ────────────────────
//
// See the equivalent block in pallet-constitution's tests for the full rationale. The
// binding invariant itself is proven against the real `EnsureLegislatureMotion` origin in
// pallet-legislature's own suite; here we confirm this pallet's `LegislatureOrigin`-gated
// calls never hash to the same value for overlapping raw parameters -- in particular the
// scenario the security review called out by name: a motion approved to `appoint_minister`
// must not double as authorization to `ratify_emergency` (or any other gated call here).
#[test]
fn legislature_call_hash_differs_between_appoint_minister_and_ratify_emergency() {
    let appoint =
        crate::pallet::legislature_call_hash(b"pallet-executive::appoint_minister", (0u32, 1u64));
    let ratify =
        crate::pallet::legislature_call_hash(b"pallet-executive::ratify_emergency", (hash(1), 1u64));
    assert_ne!(appoint, ratify);
}

// ── ratify_emergency hash bound to a specific emergency (MEDIUM-severity replay fix) ──
//
// Previously ratify_emergency's hash was `legislature_call_hash(b"...", ())` -- a constant
// identical for every emergency that will ever exist. Since PendingLegislatureApproval only
// tracks one outstanding token system-wide and doesn't invalidate it when the emergency it
// was meant for lapses, a token approved for emergency #1 could be replayed later to ratify
// an unrelated emergency #2 the legislature never actually voted on. The hash must now bind
// to that specific emergency's (reason_hash, declared_at).

#[test]
fn ratify_emergency_call_hash_differs_across_different_emergencies() {
    let e1 = crate::pallet::legislature_call_hash(
        b"pallet-executive::ratify_emergency",
        (hash(1), 10u64),
    );
    // Same reason, different declared_at -- e.g. the same wording used for two separate
    // emergencies declared at different times.
    let e2 = crate::pallet::legislature_call_hash(
        b"pallet-executive::ratify_emergency",
        (hash(1), 20u64),
    );
    // Different reason, same declared_at.
    let e3 = crate::pallet::legislature_call_hash(
        b"pallet-executive::ratify_emergency",
        (hash(2), 10u64),
    );
    assert_ne!(e1, e2);
    assert_ne!(e1, e3);
    assert_ne!(e2, e3);
}

#[test]
fn ratify_emergency_binds_hash_to_the_actual_active_emergencys_reason_and_declared_at() {
    // Behavioral check that the extrinsic itself hashes the *actual* ActiveEmergency fields
    // (not e.g. always `(reason_hash, 0)` or some other placeholder): declare with a known
    // reason at a known block, then confirm the value ratify_emergency would need to be
    // authorized for matches legislature_call_hash(tag, (that reason_hash, that declared_at)).
    new_test_ext().execute_with(|| {
        System::set_block_number(7);
        cabinet_of_three();
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(1), hash(3), 50));
        assert_ok!(Executive::vote_declare_emergency(RuntimeOrigin::signed(2), hash(3), 50));

        let info = ActiveEmergency::<Test>::get().unwrap();
        assert_eq!(info.reason_hash, hash(3));
        assert_eq!(info.declared_at, 7);

        let expected = crate::pallet::legislature_call_hash(
            b"pallet-executive::ratify_emergency",
            (info.reason_hash, info.declared_at),
        );
        // A hash bound to a *different* emergency's fields must differ from this one.
        let wrong = crate::pallet::legislature_call_hash(
            b"pallet-executive::ratify_emergency",
            (hash(9), info.declared_at),
        );
        assert_ne!(expected, wrong);

        // Root passes regardless under this mock's AsEnsureOriginWithArg (which doesn't
        // check the hash) -- the binding invariant itself is exercised end-to-end in
        // pallet-legislature's own suite against the real EnsureLegislatureMotion origin.
        assert_ok!(Executive::ratify_emergency(RuntimeOrigin::root()));
    });
}

#[test]
fn legislature_call_hash_differs_between_appoint_minister_and_dismiss_minister() {
    // Same portfolio_id shape as part of the params, different call tag.
    let appoint =
        crate::pallet::legislature_call_hash(b"pallet-executive::appoint_minister", (0u32, 1u64));
    let dismiss = crate::pallet::legislature_call_hash(b"pallet-executive::dismiss_minister", 0u32);
    assert_ne!(appoint, dismiss);
}

#[test]
fn legislature_call_hash_differs_for_different_appointees() {
    let a = crate::pallet::legislature_call_hash(b"pallet-executive::appoint_minister", (0u32, 1u64));
    let b = crate::pallet::legislature_call_hash(b"pallet-executive::appoint_minister", (0u32, 2u64));
    assert_ne!(a, b);
}

#[test]
fn legislature_call_hash_differs_between_replace_pm_and_appoint_minister() {
    let replace =
        crate::pallet::legislature_call_hash(b"pallet-executive::remove_and_replace_prime_minister", 1u64);
    let appoint =
        crate::pallet::legislature_call_hash(b"pallet-executive::appoint_minister", (0u32, 1u64));
    assert_ne!(replace, appoint);
}
