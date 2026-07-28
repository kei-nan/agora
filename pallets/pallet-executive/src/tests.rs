use crate::{
    mock::*, ActiveEmergency, DeclareVotes, EndVotes, Error, Event, MinisterPortfolio,
    NextPortfolioId, PendingEmergencyProposal, PortfolioMinister, Portfolios, PrimeMinister,
};
use frame_support::{assert_noop, assert_ok, traits::Hooks};
use pallet_legislature::pallet::MinisterChecker;
use sp_runtime::DispatchError;

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn define_portfolio(byte: u8) -> u32 {
    let id = NextPortfolioId::<Test>::get();
    assert_ok!(Executive::define_portfolio(RuntimeOrigin::root(), hash(byte)));
    id
}

fn appoint_pm(who: u64) {
    assert_ok!(Executive::appoint_prime_minister(RuntimeOrigin::root(), who));
}

fn appoint_minister(portfolio_id: u32, who: u64) {
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

// ─── appoint_prime_minister / dismiss_prime_minister ───────────────────────

#[test]
fn appoint_prime_minister_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Executive::appoint_prime_minister(RuntimeOrigin::root(), 1));

        assert_eq!(PrimeMinister::<Test>::get(), Some(1));
        System::assert_last_event(Event::PrimeMinisterAppointed { who: 1 }.into());
    });
}

#[test]
fn appoint_prime_minister_replaces_existing_pm() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);

        assert_ok!(Executive::appoint_prime_minister(RuntimeOrigin::root(), 2));

        assert_eq!(PrimeMinister::<Test>::get(), Some(2));
        let events = System::events();
        assert!(events.iter().any(|r| r.event == Event::PrimeMinisterDismissed { who: 1 }.into()));
        System::assert_last_event(Event::PrimeMinisterAppointed { who: 2 }.into());
    });
}

#[test]
fn appoint_prime_minister_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Executive::appoint_prime_minister(RuntimeOrigin::signed(1), 1),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn dismiss_prime_minister_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);

        assert_ok!(Executive::dismiss_prime_minister(RuntimeOrigin::root()));

        assert!(PrimeMinister::<Test>::get().is_none());
        System::assert_last_event(Event::PrimeMinisterDismissed { who: 1 }.into());
    });
}

#[test]
fn dismiss_prime_minister_fails_when_none_appointed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Executive::dismiss_prime_minister(RuntimeOrigin::root()),
            Error::<Test>::NoPrimeMinister
        );
    });
}

#[test]
fn dismiss_prime_minister_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        appoint_pm(1);
        assert_noop!(
            Executive::dismiss_prime_minister(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

// ─── appoint_minister / dismiss_minister ───────────────────────────────────

#[test]
fn appoint_minister_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let p0 = define_portfolio(1);

        assert_ok!(Executive::appoint_minister(RuntimeOrigin::root(), p0, 2));

        assert_eq!(PortfolioMinister::<Test>::get(p0), Some(2));
        assert_eq!(MinisterPortfolio::<Test>::get(2), Some(p0));
        System::assert_last_event(Event::MinisterAppointed { portfolio_id: p0, who: 2 }.into());
    });
}

#[test]
fn appoint_minister_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let p0 = define_portfolio(1);
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
fn appoint_minister_replaces_existing_minister_in_portfolio() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);

        assert_ok!(Executive::appoint_minister(RuntimeOrigin::root(), p0, 3));

        assert_eq!(PortfolioMinister::<Test>::get(p0), Some(3));
        assert!(MinisterPortfolio::<Test>::get(2).is_none());
        assert_eq!(MinisterPortfolio::<Test>::get(3), Some(p0));
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
        let p0 = define_portfolio(1);
        let p1 = define_portfolio(2);
        appoint_minister(p0, 2);

        assert_ok!(Executive::appoint_minister(RuntimeOrigin::root(), p1, 2));

        assert!(PortfolioMinister::<Test>::get(p0).is_none());
        assert_eq!(PortfolioMinister::<Test>::get(p1), Some(2));
        assert_eq!(MinisterPortfolio::<Test>::get(2), Some(p1));
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
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);

        assert_ok!(Executive::dismiss_minister(RuntimeOrigin::root(), p0));

        assert!(PortfolioMinister::<Test>::get(p0).is_none());
        assert!(MinisterPortfolio::<Test>::get(2).is_none());
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
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);
        assert_noop!(
            Executive::dismiss_minister(RuntimeOrigin::signed(1), p0),
            DispatchError::BadOrigin
        );
    });
}

// ─── resign ─────────────────────────────────────────────────────────────────

#[test]
fn resign_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let p0 = define_portfolio(1);
        appoint_minister(p0, 2);

        assert_ok!(Executive::resign(RuntimeOrigin::signed(2)));

        assert!(PortfolioMinister::<Test>::get(p0).is_none());
        assert!(MinisterPortfolio::<Test>::get(2).is_none());
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
        System::assert_last_event(Event::EmergencyLapsed.into());
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
        System::assert_last_event(Event::EmergencyExpired { at_block: 6 }.into());
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
