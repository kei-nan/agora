use crate::{
    mock::*, ActiveEmergency, Bootstrapped, Council, CooldownUntil, DeclareVotes, EndVotes,
    EnsureActiveEmergency, Error, Event, PendingEmergencyProposal,
};
use frame_support::{assert_noop, assert_ok, traits::{EnsureOrigin, Hooks}};
use sp_runtime::DispatchError;

const REASON_A: [u8; 32] = [1u8; 32];
const REASON_B: [u8; 32] = [2u8; 32];

/// Adds `accounts` to the council via root, in order.
fn add_members(accounts: &[u64]) {
    for a in accounts {
        assert_ok!(EmergencyCouncil::add_council_member(RuntimeOrigin::root(), *a));
    }
}

// ─── add_council_member ─────────────────────────────────────────────────────

#[test]
fn add_council_member_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_ok!(EmergencyCouncil::add_council_member(RuntimeOrigin::root(), 1));

        assert_eq!(Council::<Test>::get().into_inner(), vec![1]);
        System::assert_last_event(Event::CouncilMemberAdded { who: 1 }.into());
    });
}

#[test]
fn add_council_member_fails_when_already_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1]);

        assert_noop!(
            EmergencyCouncil::add_council_member(RuntimeOrigin::root(), 1),
            Error::<Test>::AlreadyCouncilMember
        );
    });
}

#[test]
fn add_council_member_fails_when_at_capacity() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // MAX_COUNCIL_SIZE is 10; fill it exactly.
        let full: Vec<u64> = (1..=MAX_COUNCIL_SIZE as u64).collect();
        add_members(&full);
        assert_eq!(Council::<Test>::get().len(), MAX_COUNCIL_SIZE as usize);

        assert_noop!(
            EmergencyCouncil::add_council_member(
                RuntimeOrigin::root(),
                MAX_COUNCIL_SIZE as u64 + 1
            ),
            Error::<Test>::CouncilAtCapacity
        );
    });
}

#[test]
fn add_council_member_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_noop!(
            EmergencyCouncil::add_council_member(RuntimeOrigin::signed(1), 2),
            DispatchError::BadOrigin
        );
    });
}

// ─── remove_council_member ──────────────────────────────────────────────────

#[test]
fn remove_council_member_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2]);

        assert_ok!(EmergencyCouncil::remove_council_member(RuntimeOrigin::root(), 1));

        assert_eq!(Council::<Test>::get().into_inner(), vec![2]);
        System::assert_last_event(Event::CouncilMemberRemoved { who: 1 }.into());
    });
}

#[test]
fn remove_council_member_fails_when_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1]);

        assert_noop!(
            EmergencyCouncil::remove_council_member(RuntimeOrigin::root(), 2),
            Error::<Test>::MemberNotFound
        );
    });
}

#[test]
fn remove_council_member_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1]);

        assert_noop!(
            EmergencyCouncil::remove_council_member(RuntimeOrigin::signed(1), 1),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn remove_council_member_clears_pending_declare_vote() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // 3-member council; a single vote does not reach the 2/3 supermajority.
        add_members(&[1, 2, 3]);
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            10
        ));
        assert!(DeclareVotes::<Test>::get(1));

        assert_ok!(EmergencyCouncil::remove_council_member(RuntimeOrigin::root(), 1));

        // The removed member's declare vote is cleared, not just orphaned.
        assert!(!DeclareVotes::<Test>::get(1));
    });
}

// ─── vote_declare_emergency: membership / basic gating ─────────────────────

#[test]
fn vote_declare_emergency_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);

        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(99), REASON_A, 10),
            Error::<Test>::NotCouncilMember
        );
    });
}

#[test]
fn vote_declare_emergency_single_vote_does_not_activate() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);

        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            10
        ));

        assert!(ActiveEmergency::<Test>::get().is_none());
        // No EmergencyDeclared event should have fired.
        assert!(System::events()
            .iter()
            .all(|r| !matches!(r.event, RuntimeEvent::EmergencyCouncil(Event::EmergencyDeclared { .. }))));
    });
}

#[test]
fn vote_declare_emergency_fails_on_double_vote() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            10
        ));

        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(1), REASON_A, 10),
            Error::<Test>::AlreadyVotedToDeclare
        );
    });
}

#[test]
fn vote_declare_emergency_fails_when_already_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);
        // 2 of 3 reaches the 2/3 supermajority and activates the emergency.
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            10
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            10
        ));
        assert!(ActiveEmergency::<Test>::get().is_some());

        // Member 3 never voted (votes were reset on activation) but a new declare
        // attempt must still be rejected because an emergency is already active.
        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(3), REASON_B, 10),
            Error::<Test>::AlreadyActiveEmergency
        );
    });
}

// ─── vote_declare_emergency: supermajority boundary ─────────────────────────

#[test]
fn vote_declare_emergency_activates_at_exact_supermajority_boundary_council_of_3() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Council of 3, 2/3 threshold: votes*3 >= 3*2=6 => votes>=2.
        add_members(&[1, 2, 3]);

        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            10
        ));
        assert!(ActiveEmergency::<Test>::get().is_none());

        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            10
        ));

        let info = ActiveEmergency::<Test>::get().expect("emergency should be active");
        assert_eq!(info.declared_at, 1);
        assert_eq!(info.expires_at, 11);
        assert_eq!(info.reason_hash, REASON_A);
        assert_eq!(info.votes_to_declare, 2);
        System::assert_last_event(
            Event::EmergencyDeclared { expires_at: 11, reason_hash: REASON_A }.into(),
        );

        // Vote maps are reset once the emergency activates.
        assert!(!DeclareVotes::<Test>::get(1));
        assert!(!DeclareVotes::<Test>::get(2));
        assert!(PendingEmergencyProposal::<Test>::get().is_none());
    });
}

#[test]
fn vote_declare_emergency_one_vote_short_of_supermajority_council_of_5_does_not_activate() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Council of 5, 2/3 threshold: votes*3 >= 5*2=10 => votes>=4 (ceil(10/3)=4).
        add_members(&[1, 2, 3, 4, 5]);

        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            10
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            10
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(3),
            REASON_A,
            10
        ));
        // 3 votes: 3*3=9 >= 10? No. Must not activate yet.
        assert!(ActiveEmergency::<Test>::get().is_none());

        // The 4th vote reaches 4*3=12 >= 10 and activates.
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(4),
            REASON_A,
            10
        ));
        assert!(ActiveEmergency::<Test>::get().is_some());
    });
}

// ─── vote_declare_emergency: duration clamping & proposal lock-in ──────────

#[test]
fn vote_declare_emergency_clamps_duration_to_max_emergency_blocks() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);
        let huge_duration = MAX_EMERGENCY_BLOCKS * 10;

        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            huge_duration
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            huge_duration
        ));

        let info = ActiveEmergency::<Test>::get().expect("emergency should be active");
        // Clamped to declared_at (1) + MAX_EMERGENCY_BLOCKS (100), not the requested duration.
        assert_eq!(info.expires_at, 1 + MAX_EMERGENCY_BLOCKS as u64);
    });
}

#[test]
fn vote_declare_emergency_does_not_clamp_duration_under_max() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);
        let small_duration = MAX_EMERGENCY_BLOCKS - 1;

        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            small_duration
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            small_duration
        ));

        let info = ActiveEmergency::<Test>::get().expect("emergency should be active");
        assert_eq!(info.expires_at, 1 + small_duration as u64);
    });
}

#[test]
fn vote_declare_emergency_locks_in_first_voters_terms() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);

        // First voter sets the terms: REASON_A, duration 5.
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            5
        ));
        // Second voter votes with the *same* terms as the first — counts normally.
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            5
        ));

        let info = ActiveEmergency::<Test>::get().expect("emergency should be active");
        assert_eq!(info.reason_hash, REASON_A);
        assert_eq!(info.expires_at, 1 + 5);
    });
}

// Previously, a second voter's own `reason_hash`/`duration_blocks` arguments were silently
// discarded once `PendingEmergencyProposal` was locked in by the first voter — only their
// `who` counted as a yes-vote toward whatever the first caller had proposed, with no check
// their submitted terms actually matched. That's a confused-deputy gap: a council member's
// vote silently endorsed terms they never saw. Fixed: a mismatched vote is now rejected
// outright. Mirrors pallet-executive's
// `vote_declare_emergency_fails_when_terms_mismatch_pending_proposal`.
#[test]
fn vote_declare_emergency_fails_when_terms_mismatch_pending_proposal() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);

        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            5
        ));

        // Different reason_hash, same duration.
        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(2), REASON_B, 5),
            Error::<Test>::EmergencyProposalMismatch
        );
        // Same reason_hash, different duration.
        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(2), REASON_A, 50),
            Error::<Test>::EmergencyProposalMismatch
        );

        // The mismatched attempts must not have been recorded as a vote for account 2, nor
        // altered the locked-in proposal.
        assert!(!DeclareVotes::<Test>::get(2));
        assert_eq!(PendingEmergencyProposal::<Test>::get(), Some((REASON_A, 5)));
        assert!(ActiveEmergency::<Test>::get().is_none());

        // Voting with the correct, matching terms still works.
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            5
        ));
        let info = ActiveEmergency::<Test>::get().expect("emergency should be active");
        assert_eq!(info.reason_hash, REASON_A);
        assert_eq!(info.expires_at, 1 + 5);
    });
}

// ─── vote_end_emergency ──────────────────────────────────────────────────────

fn declare_active_emergency_with_council_of_3() {
    add_members(&[1, 2, 3]);
    assert_ok!(EmergencyCouncil::vote_declare_emergency(
        RuntimeOrigin::signed(1),
        REASON_A,
        50
    ));
    assert_ok!(EmergencyCouncil::vote_declare_emergency(
        RuntimeOrigin::signed(2),
        REASON_A,
        50
    ));
    assert!(ActiveEmergency::<Test>::get().is_some());
}

#[test]
fn vote_end_emergency_fails_when_no_active_emergency() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);

        assert_noop!(
            EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)),
            Error::<Test>::NoActiveEmergency
        );
    });
}

#[test]
fn vote_end_emergency_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();

        assert_noop!(
            EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(99)),
            Error::<Test>::NotCouncilMember
        );
    });
}

#[test]
fn vote_end_emergency_fails_on_double_vote() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));

        assert_noop!(
            EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)),
            Error::<Test>::AlreadyVotedToEnd
        );
    });
}

#[test]
fn vote_end_emergency_activates_at_exact_supermajority_boundary() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Council of 3, threshold 2/3 => 2 votes required.
        declare_active_emergency_with_council_of_3();

        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert!(ActiveEmergency::<Test>::get().is_some(), "1 vote should not lift the emergency");

        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(2)));

        assert!(ActiveEmergency::<Test>::get().is_none());
        System::assert_last_event(Event::EmergencyLifted.into());
        // Vote maps reset.
        assert!(!EndVotes::<Test>::get(1));
        assert!(!EndVotes::<Test>::get(2));
    });
}

#[test]
fn vote_end_emergency_one_vote_short_of_supermajority_council_of_5_does_not_lift() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3, 4, 5]);
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            50
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(2),
            REASON_A,
            50
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(3),
            REASON_A,
            50
        ));
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(4),
            REASON_A,
            50
        ));
        assert!(ActiveEmergency::<Test>::get().is_some());

        // 3 of 5 end-votes: 3*3=9 >= 10? No -> must remain active.
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(2)));
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(3)));
        assert!(ActiveEmergency::<Test>::get().is_some());

        // 4th end-vote reaches 4*3=12 >= 10 and lifts it.
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(4)));
        assert!(ActiveEmergency::<Test>::get().is_none());
    });
}

// ─── on_initialize: automatic expiry ────────────────────────────────────────

#[test]
fn emergency_does_not_expire_before_expires_at() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        let info = ActiveEmergency::<Test>::get().unwrap();
        assert_eq!(info.expires_at, 51);

        // One block before expiry: still active.
        System::set_block_number(50);
        EmergencyCouncil::on_initialize(50);
        assert!(ActiveEmergency::<Test>::get().is_some());
    });
}

#[test]
fn emergency_auto_expires_at_expires_at_block() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        let info = ActiveEmergency::<Test>::get().unwrap();
        assert_eq!(info.expires_at, 51);

        System::set_block_number(51);
        EmergencyCouncil::on_initialize(51);

        assert!(ActiveEmergency::<Test>::get().is_none());
        System::assert_last_event(Event::EmergencyExpired { at_block: 51 }.into());
    });
}

#[test]
fn emergency_auto_expiry_clears_vote_maps_and_pending_proposal() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        // Member 3 casts an end-vote before expiry, which should be cleared too.
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(3)));
        assert!(EndVotes::<Test>::get(3));

        System::set_block_number(51);
        EmergencyCouncil::on_initialize(51);

        assert!(!EndVotes::<Test>::get(3));
        assert!(PendingEmergencyProposal::<Test>::get().is_none());

        // A member who had already voted to declare the now-expired emergency can
        // immediately participate in declaring a fresh one without a stale
        // AlreadyVotedToDeclare rejection, once the post-emergency cooldown has passed.
        System::set_block_number(CooldownUntil::<Test>::get());
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_B,
            10
        ));
    });
}

// ─── post-emergency cooldown ────────────────────────────────────────────────

#[test]
fn vote_declare_emergency_fails_during_cooldown_after_auto_expiry() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();

        System::set_block_number(51);
        EmergencyCouncil::on_initialize(51);
        assert_eq!(CooldownUntil::<Test>::get(), 51 + EMERGENCY_COOLDOWN_BLOCKS as u64);

        // One block before the cooldown lifts: still blocked.
        System::set_block_number(51 + EMERGENCY_COOLDOWN_BLOCKS as u64 - 1);
        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(1), REASON_B, 10),
            Error::<Test>::EmergencyCooldownActive
        );
    });
}

#[test]
fn vote_declare_emergency_fails_during_cooldown_after_early_lift() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();

        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(2)));
        assert!(ActiveEmergency::<Test>::get().is_none());

        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(3), REASON_B, 10),
            Error::<Test>::EmergencyCooldownActive
        );
    });
}

#[test]
fn vote_declare_emergency_succeeds_once_cooldown_elapses() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();

        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(2)));

        System::set_block_number(CooldownUntil::<Test>::get());
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(3),
            REASON_B,
            10
        ));
        assert!(DeclareVotes::<Test>::get(3));
    });
}

// ─── EnsureActiveEmergency origin ───────────────────────────────────────────

#[test]
fn ensure_active_emergency_fails_when_no_emergency_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert!(ActiveEmergency::<Test>::get().is_none());

        // Root, but no active emergency: must fail.
        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::root()).is_err());
    });
}

#[test]
fn ensure_active_emergency_fails_for_signed_origin_even_with_active_emergency() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        assert!(ActiveEmergency::<Test>::get().is_some());

        // A signed origin (even a council member who helped declare the emergency) must
        // not succeed — only Root, combined with an active emergency, may.
        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::signed(1)).is_err());
        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::none()).is_err());
    });
}

#[test]
fn ensure_active_emergency_succeeds_for_root_when_emergency_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        assert!(ActiveEmergency::<Test>::get().is_some());

        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::root()).is_ok());
    });
}

#[test]
fn ensure_active_emergency_fails_again_after_emergency_lifted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::root()).is_ok());

        // Lift the emergency early via the real supermajority vote_end_emergency path.
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(2)));
        assert!(ActiveEmergency::<Test>::get().is_none());

        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::root()).is_err());
    });
}

#[test]
fn ensure_active_emergency_fails_again_after_auto_sunset_expiry() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::root()).is_ok());

        let info = ActiveEmergency::<Test>::get().unwrap();
        System::set_block_number(info.expires_at);
        EmergencyCouncil::on_initialize(info.expires_at);
        assert!(ActiveEmergency::<Test>::get().is_none());

        assert!(EnsureActiveEmergency::<Test>::try_origin(RuntimeOrigin::root()).is_err());
    });
}

// ─── cross-pallet sibling cooldown coordination (pallet-executive) ─────────
//
// pallet-executive has its own, independent cabinet-level emergency mechanism with its own
// `CooldownUntil`. Without `SiblingEmergencyCooldown`, a coalition controlling both bodies
// could declare an emergency via one, let it lapse, and immediately declare a fresh one via
// the other — this pallet's own `EmergencyCooldownBlocks` would never even see it happen. The
// mock sibling here (`mock::MockSiblingCooldown`) stands in for the real
// `Runtime`-level implementation that bridges to pallet-executive's actual `CooldownUntil`.

#[test]
fn emergency_auto_expiry_notifies_sibling_cooldown() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();
        assert!(sibling_notified_at().is_none());

        let info = ActiveEmergency::<Test>::get().unwrap();
        System::set_block_number(info.expires_at);
        EmergencyCouncil::on_initialize(info.expires_at);

        // The sibling (pallet-executive, in the real runtime) was told to start its own
        // cooldown at the same block this pallet started its own.
        assert_eq!(sibling_notified_at(), Some(info.expires_at));
    });
}

#[test]
fn vote_end_emergency_notifies_sibling_cooldown() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency_with_council_of_3();

        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(2)));
        assert!(ActiveEmergency::<Test>::get().is_none());

        assert_eq!(sibling_notified_at(), Some(1));
    });
}

#[test]
fn vote_declare_emergency_fails_while_sibling_in_cooldown_even_with_own_cooldown_elapsed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2, 3]);
        // This pallet's own CooldownUntil defaults to 0 (never touched) — only the sibling's
        // reported cooldown can be blocking the declare below.
        assert_eq!(CooldownUntil::<Test>::get(), 0);
        set_sibling_cooldown_until(1000);

        assert_noop!(
            EmergencyCouncil::vote_declare_emergency(RuntimeOrigin::signed(1), REASON_A, 10),
            Error::<Test>::EmergencyCooldownActive
        );

        // Once the sibling's (simulated) cooldown passes, declaration succeeds again — this
        // is the regression case: a coalition alternating between pallet-executive and
        // pallet-emergency-council can no longer chain declarations past either pallet's
        // cooldown, only past both.
        System::set_block_number(1000);
        assert_ok!(EmergencyCouncil::vote_declare_emergency(
            RuntimeOrigin::signed(1),
            REASON_A,
            10
        ));
    });
}

// ─── Bootstrap lock: Root can seed initial members, but not forever ────────────────────────

#[test]
fn root_can_add_and_remove_members_pre_bootstrap() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1, 2]);
        assert_ok!(EmergencyCouncil::remove_council_member(RuntimeOrigin::root(), 2));
        assert_eq!(Council::<Test>::get().into_inner(), vec![1]);
        assert!(!Bootstrapped::<Test>::get());
    });
}

#[test]
fn close_bootstrap_requires_root() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1]);
        assert_noop!(
            EmergencyCouncil::close_bootstrap(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn close_bootstrap_requires_at_least_one_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            EmergencyCouncil::close_bootstrap(RuntimeOrigin::root()),
            Error::<Test>::NoMembersToBootstrap
        );
    });
}

#[test]
fn close_bootstrap_cannot_be_called_twice() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1]);
        assert_ok!(EmergencyCouncil::close_bootstrap(RuntimeOrigin::root()));
        assert!(Bootstrapped::<Test>::get());
        System::assert_last_event(Event::BootstrapClosed.into());
        assert_noop!(
            EmergencyCouncil::close_bootstrap(RuntimeOrigin::root()),
            Error::<Test>::BootstrapClosed
        );
    });
}

/// The core fix this pallet gained: once bootstrapped, `Root` can never again unilaterally
/// add or remove an Emergency Council member — closing the gap where a compromised sudo key
/// could pack or purge the council forever.
#[test]
fn root_cannot_add_or_remove_members_after_bootstrap_closed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add_members(&[1]);
        assert_ok!(EmergencyCouncil::close_bootstrap(RuntimeOrigin::root()));

        assert_noop!(
            EmergencyCouncil::add_council_member(RuntimeOrigin::root(), 2),
            Error::<Test>::BootstrapClosed
        );
        assert_noop!(
            EmergencyCouncil::remove_council_member(RuntimeOrigin::root(), 1),
            Error::<Test>::BootstrapClosed
        );
        assert_eq!(Council::<Test>::get().into_inner(), vec![1]);
    });
}
