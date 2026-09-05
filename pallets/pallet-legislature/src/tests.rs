use crate::{
    mock::*, Bootstrapped, EnsureLegislatureMotion, Error, Event, Members, Motions, MotionVotes,
    NextMotionId, PendingLegislatureApproval,
};
use frame_support::{assert_noop, assert_ok, traits::EnsureOriginWithArg};
use sp_runtime::DispatchError;

const CALL_HASH_A: [u8; 32] = [1u8; 32];
const CALL_HASH_B: [u8; 32] = [2u8; 32];

fn add(who: u64) {
    assert_ok!(Legislature::add_member(RuntimeOrigin::root(), who));
}

// ─── add_member ─────────────────────────────────────────────────────────────

#[test]
fn add_member_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);

        assert_eq!(Members::<Test>::get().to_vec(), vec![1]);
        System::assert_last_event(Event::MemberAdded { who: 1 }.into());
    });
}

#[test]
fn add_member_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Legislature::add_member(RuntimeOrigin::signed(1), 1),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn add_member_fails_for_duplicate() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);

        assert_noop!(
            Legislature::add_member(RuntimeOrigin::root(), 1),
            Error::<Test>::AlreadyMember
        );
    });
}

#[test]
fn add_member_fails_at_max_capacity() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // MAX_MEMBERS = 5.
        for who in 1..=MAX_MEMBERS as u64 {
            add(who);
        }
        assert_eq!(Members::<Test>::get().len(), MAX_MEMBERS as usize);

        assert_noop!(
            Legislature::add_member(RuntimeOrigin::root(), 999),
            Error::<Test>::MembersAtCapacity
        );
    });
}

// Regression test: bootstrap-phase `add_member` used to have no Accountability Council
// overlap check at all, unlike post-bootstrap automatic seating via
// `pallet_elections::SeatLegislature`, which already checks this. A sitting Council member
// must be refused here too.
#[test]
fn add_member_fails_for_sitting_accountability_council_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_accountability_council_member(1, true);

        assert_noop!(
            Legislature::add_member(RuntimeOrigin::root(), 1),
            Error::<Test>::AccountabilityCouncilMember
        );
        assert!(Members::<Test>::get().is_empty());
    });
}

// ─── remove_member ──────────────────────────────────────────────────────────

#[test]
fn remove_member_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);

        assert_ok!(Legislature::remove_member(RuntimeOrigin::root(), 1));

        assert_eq!(Members::<Test>::get().to_vec(), vec![2]);
        System::assert_last_event(Event::MemberRemoved { who: 1 }.into());
    });
}

#[test]
fn remove_member_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);

        assert_noop!(
            Legislature::remove_member(RuntimeOrigin::signed(1), 1),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn remove_member_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Legislature::remove_member(RuntimeOrigin::root(), 42),
            Error::<Test>::MemberNotFound
        );
    });
}

// ─── propose_motion ─────────────────────────────────────────────────────────

#[test]
fn propose_motion_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);

        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        let motion = Motions::<Test>::get(0).expect("motion 0 exists");
        assert_eq!(motion.call_hash, CALL_HASH_A);
        assert_eq!(motion.proposer, 1);
        assert_eq!(motion.ayes, 1);
        assert_eq!(motion.nays, 0);
        assert_eq!(motion.end_block, 1 + MOTION_DURATION as u64);
        assert!(!motion.executed);
        assert_eq!(MotionVotes::<Test>::get((0, 1)), Some(true));
        assert_eq!(NextMotionId::<Test>::get(), 1);

        let events = System::events();
        assert!(events.iter().any(|r| r.event
            == Event::MotionProposed { motion_id: 0, call_hash: CALL_HASH_A }.into()));
        System::assert_last_event(
            Event::VoteCast { motion_id: 0, member: 1, approve: true }.into(),
        );
    });
}

#[test]
fn propose_motion_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A),
            Error::<Test>::NotAMember
        );
    });
}

#[test]
fn propose_motion_fails_for_minister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        set_minister(1);

        assert_noop!(
            Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A),
            Error::<Test>::MinisterCannotVote
        );
    });
}

#[test]
fn propose_motion_allows_duplicate_call_hash_as_separate_motions() {
    // The pallet does not guard against re-proposing the same call_hash; each
    // proposal gets its own independent motion_id.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);

        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        assert_eq!(NextMotionId::<Test>::get(), 2);
        assert_eq!(Motions::<Test>::get(0).unwrap().call_hash, CALL_HASH_A);
        assert_eq!(Motions::<Test>::get(1).unwrap().call_hash, CALL_HASH_A);
    });
}

// ─── vote_motion ────────────────────────────────────────────────────────────

#[test]
fn vote_motion_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));

        let motion = Motions::<Test>::get(0).unwrap();
        assert_eq!(motion.ayes, 2);
        assert_eq!(motion.nays, 0);
        assert_eq!(MotionVotes::<Test>::get((0, 2)), Some(true));
        System::assert_last_event(
            Event::VoteCast { motion_id: 0, member: 2, approve: true }.into(),
        );
    });
}

#[test]
fn vote_motion_nay_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, false));

        let motion = Motions::<Test>::get(0).unwrap();
        assert_eq!(motion.ayes, 1);
        assert_eq!(motion.nays, 1);
        assert_eq!(MotionVotes::<Test>::get((0, 2)), Some(false));
    });
}

#[test]
fn vote_motion_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        assert_noop!(
            Legislature::vote_motion(RuntimeOrigin::signed(99), 0, true),
            Error::<Test>::NotAMember
        );
    });
}

/// The pallet's most important invariant: an active executive minister who is
/// also an enrolled legislature member must be rejected when voting.
#[test]
fn vote_motion_fails_for_active_minister() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        set_minister(2);

        assert_noop!(
            Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true),
            Error::<Test>::MinisterCannotVote
        );

        // Confirm the vote truly did not register.
        let motion = Motions::<Test>::get(0).unwrap();
        assert_eq!(motion.ayes, 1);
        assert_eq!(motion.nays, 0);
        assert!(MotionVotes::<Test>::get((0, 2)).is_none());
    });
}

/// Sanity check on the flip side of the invariant above: a member who is
/// *not* currently a minister votes successfully.
#[test]
fn vote_motion_succeeds_for_non_minister_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        assert_eq!(Motions::<Test>::get(0).unwrap().ayes, 2);
    });
}

#[test]
fn vote_motion_fails_on_double_vote() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));

        assert_noop!(
            Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true),
            Error::<Test>::AlreadyVoted
        );
    });
}

#[test]
fn vote_motion_fails_when_motion_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);

        assert_noop!(
            Legislature::vote_motion(RuntimeOrigin::signed(1), 0, true),
            Error::<Test>::MotionNotFound
        );
    });
}

#[test]
fn vote_motion_fails_after_voting_window_closed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        // end_block = 1 + MOTION_DURATION; move to exactly end_block.
        System::set_block_number(1 + MOTION_DURATION as u64);

        assert_noop!(
            Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true),
            Error::<Test>::VotingWindowClosed
        );
    });
}

#[test]
fn vote_motion_fails_on_already_executed_motion() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        // Member 3 never voted, so it clears the `AlreadyVoted` check and hits
        // `MotionAlreadyExecuted` inside the storage mutation instead.
        assert_noop!(
            Legislature::vote_motion(RuntimeOrigin::signed(3), 0, true),
            Error::<Test>::MotionAlreadyExecuted
        );
    });
}

// ─── close_motion ───────────────────────────────────────────────────────────

#[test]
fn close_motion_fails_while_still_open() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        // Still well before end_block.
        assert_noop!(
            Legislature::close_motion(RuntimeOrigin::signed(1), 0),
            Error::<Test>::MotionStillOpen
        );
    });
}

#[test]
fn close_motion_fails_when_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Legislature::close_motion(RuntimeOrigin::signed(1), 0),
            Error::<Test>::MotionNotFound
        );
    });
}

#[test]
fn close_motion_fails_when_already_executed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        assert_noop!(
            Legislature::close_motion(RuntimeOrigin::signed(1), 0),
            Error::<Test>::MotionAlreadyExecuted
        );
    });
}

/// PassageThreshold = 50%, evaluated over *total members* (not votes cast):
/// passed iff `ayes * 100 >= threshold * total_members`.
/// With 4 total members, exactly 2 ayes (50%) meets the threshold exactly —
/// including when the other 2 members never voted (abstained).
#[test]
fn close_motion_passes_at_exact_threshold_boundary_with_abstentions() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        add(4);
        // Proposer (1) auto-ayes; member 2 ayes. Members 3 and 4 abstain entirely.
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        let motion = Motions::<Test>::get(0).unwrap();
        assert!(motion.executed);
        assert_eq!(motion.ayes, 2);
        System::assert_last_event(
            Event::MotionPassed { motion_id: 0, call_hash: CALL_HASH_A }.into(),
        );
        assert_eq!(
            PendingLegislatureApproval::<Test>::get(),
            Some((CALL_HASH_A, 1, 2, 4, 1 + MOTION_DURATION as u64))
        );
    });
}

/// One aye short of the boundary above (1 of 4, i.e. 25% < 50%) fails.
#[test]
fn close_motion_fails_threshold_just_below_boundary() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        add(4);
        // Only the proposer's automatic aye — no one else votes.
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        let motion = Motions::<Test>::get(0).unwrap();
        assert!(motion.executed);
        assert_eq!(motion.ayes, 1);
        System::assert_last_event(Event::MotionFailed { motion_id: 0 }.into());
        assert!(PendingLegislatureApproval::<Test>::get().is_none());
    });
}

/// With an odd total membership (5), the threshold check is exact integer
/// comparison (no explicit rounding function): 2/5 ayes (40%) fails, 3/5
/// (60%) passes — the natural boundary lands between 2 and 3, not at a
/// "round" percentage.
#[test]
fn close_motion_threshold_odd_membership_boundary() {
    // 2 of 5 fails.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        add(4);
        add(5);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        // ayes = 2 (proposer + member 2), total_members = 5.
        // 2 * 100 = 200 < 50 * 5 = 250 -> fails.

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        System::assert_last_event(Event::MotionFailed { motion_id: 0 }.into());
    });

    // 3 of 5 passes.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        add(4);
        add(5);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(3), 0, true));
        // ayes = 3, total_members = 5. 3 * 100 = 300 >= 250 -> passes.

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        System::assert_last_event(
            Event::MotionPassed { motion_id: 0, call_hash: CALL_HASH_A }.into(),
        );
    });
}

#[test]
fn close_motion_can_be_called_by_any_signed_account() {
    // "Anyone may call this" per the doc comment -- not restricted to members.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(999), 0));
    });
}

/// A second motion that passes while an earlier passed motion's approval
/// token is still unconsumed must be rejected with `ApprovalPending` rather
/// than silently overwriting the first motion's token.
#[test]
fn close_motion_fails_with_approval_pending_when_prior_token_unconsumed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);

        // Motion A passes (single member, 100% ayes) and plants a token.
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));
        assert_eq!(
            PendingLegislatureApproval::<Test>::get(),
            Some((CALL_HASH_A, 1, 1, 1, 1 + MOTION_DURATION as u64))
        );

        // Motion B is proposed and also reaches passage, but closing it must
        // fail because motion A's token has not been consumed yet.
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_B));
        System::set_block_number(1 + 2 * MOTION_DURATION as u64);

        assert_noop!(
            Legislature::close_motion(RuntimeOrigin::signed(1), 1),
            Error::<Test>::ApprovalPending
        );

        // The original token from motion A must be untouched.
        assert_eq!(
            PendingLegislatureApproval::<Test>::get(),
            Some((CALL_HASH_A, 1, 1, 1, 1 + MOTION_DURATION as u64))
        );
    });
}

// ─── EnsureLegislatureMotion origin ─────────────────────────────────────────

#[test]
fn ensure_legislature_motion_succeeds_only_after_motion_passes_for_proposer() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);

        // No pending approval yet -- a member's signed origin is rejected.
        let origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        assert!(EnsureLegislatureMotion::<Test>::try_origin(origin, &CALL_HASH_A).is_err());

        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        // The proposer's origin succeeds when it presents the hash the motion approved.
        let proposer_origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        EnsureLegislatureMotion::<Test>::try_origin(proposer_origin, &CALL_HASH_A)
            .expect("proposer origin should be authorized after motion passed");

        // The token is consumed exactly once -- a second attempt fails.
        let proposer_origin_again: RuntimeOrigin = RuntimeOrigin::signed(1);
        assert!(
            EnsureLegislatureMotion::<Test>::try_origin(proposer_origin_again, &CALL_HASH_A)
                .is_err()
        );
        assert!(PendingLegislatureApproval::<Test>::get().is_none());
    });
}

/// The deadlock fix: any current legislature member -- not only the original
/// proposer -- can consume a passed motion's approval token. The vote that
/// passed the motion is what legitimizes the action, so a proposer who goes
/// offline or is later removed must not be able to permanently block it.
#[test]
fn ensure_legislature_motion_allows_any_current_member_to_consume_token() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        // Member 2, who is not the proposer, successfully consumes the token.
        let other_origin: RuntimeOrigin = RuntimeOrigin::signed(2);
        EnsureLegislatureMotion::<Test>::try_origin(other_origin, &CALL_HASH_A)
            .expect("any current member should be able to execute a passed motion");
        assert!(PendingLegislatureApproval::<Test>::get().is_none());
    });
}

/// The core fix for the HIGH-severity hijack: a token approved for one call (A) must not
/// authorize dispatch of a different call (B), even by the legitimate proposer.
#[test]
fn ensure_legislature_motion_rejects_mismatched_call_hash() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        // The proposer presents call B's hash instead of the approved call A's hash.
        let origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        assert!(EnsureLegislatureMotion::<Test>::try_origin(origin, &CALL_HASH_B).is_err());

        // The mismatched attempt must not consume the token -- it's still there, and
        // still only good for the call it was actually approved for (A).
        assert_eq!(
            PendingLegislatureApproval::<Test>::get(),
            Some((CALL_HASH_A, 1, 1, 1, 1 + MOTION_DURATION as u64))
        );
        let origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        assert!(EnsureLegislatureMotion::<Test>::try_origin(origin, &CALL_HASH_A).is_ok());
        assert!(PendingLegislatureApproval::<Test>::get().is_none());
    });
}

#[test]
fn ensure_legislature_motion_rejects_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        // Account 42 is not an enrolled member at all.
        let origin: RuntimeOrigin = RuntimeOrigin::signed(42);
        assert!(EnsureLegislatureMotion::<Test>::try_origin(origin, &CALL_HASH_A).is_err());
        // Token remains untouched since a non-member origin can't consume it.
        assert!(PendingLegislatureApproval::<Test>::get().is_some());
    });
}

#[test]
fn ensure_legislature_motion_rejects_root_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        let origin: RuntimeOrigin = RuntimeOrigin::root();
        assert!(EnsureLegislatureMotion::<Test>::try_origin(origin, &CALL_HASH_A).is_err());
    });
}

// ─── EnsureLegislatureMotion tier-aware ([u8; 32], u8) overload ────────────
//
// These exercise the mechanism pallet-constitution relies on for tier-aware supermajorities:
// the required percentage is checked against the *real* ayes/total tally frozen when the
// motion closed, independent of whether the motion cleared the pallet's own baseline
// `PassageThreshold` floor. See that pallet's module doc comment for how it derives the
// percentage it passes in from data that can't be spoofed by a proposer.

/// A motion with 60% support (3 of 5) clears the mock's 50% floor and closes as
/// `MotionPassed`, but must NOT be able to authorize a call that requires 75% — a real-world
/// stand-in for "a Foundational-tier enactment only got Ordinary-level support."
#[test]
fn tiered_origin_rejects_motion_below_required_percentage() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        add(4);
        add(5);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(3), 0, true));
        // ayes = 3 (proposer + 2 voters), total_members = 5 -> 60%.

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));
        System::assert_last_event(
            Event::MotionPassed { motion_id: 0, call_hash: CALL_HASH_A }.into(),
        );
        assert_eq!(
            PendingLegislatureApproval::<Test>::get(),
            Some((CALL_HASH_A, 1, 3, 5, 1 + MOTION_DURATION as u64))
        );

        // 60% < 75% required -- rejected even though the motion "passed" at the floor.
        let origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        let result = <EnsureLegislatureMotion<Test> as frame_support::traits::EnsureOriginWithArg<
            RuntimeOrigin,
            ([u8; 32], u8),
        >>::try_origin(origin, &(CALL_HASH_A, 75));
        assert!(result.is_err());
        // A rejected attempt must not consume the token -- it might still legitimately
        // authorize a lower-tier call.
        assert!(PendingLegislatureApproval::<Test>::get().is_some());
    });
}

/// The same mechanism accepts a tally that meets or exceeds the required percentage: 80%
/// support (4 of 5) clears a 75% requirement.
#[test]
fn tiered_origin_accepts_motion_at_or_above_required_percentage() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        add(4);
        add(5);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(3), 0, true));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(4), 0, true));
        // ayes = 4 (proposer + 3 voters), total_members = 5 -> 80%.

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        let origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        let result = <EnsureLegislatureMotion<Test> as frame_support::traits::EnsureOriginWithArg<
            RuntimeOrigin,
            ([u8; 32], u8),
        >>::try_origin(origin, &(CALL_HASH_A, 75));
        assert!(result.is_ok());
        // Consumed on success, same as the hash-only overload.
        assert!(PendingLegislatureApproval::<Test>::get().is_none());
    });
}

/// The critical tier-awareness property: the *exact same* 60% tally that fails a 75%
/// (Foundational-equivalent) requirement above succeeds against a 51% (Ordinary-equivalent)
/// requirement -- the mechanism enforces whatever the calling pallet says it needs, it doesn't
/// silently apply one flat bar to every call.
#[test]
fn tiered_origin_ordinary_requirement_satisfied_by_same_tally_that_fails_higher_tier() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        add(3);
        add(4);
        add(5);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(2), 0, true));
        assert_ok!(Legislature::vote_motion(RuntimeOrigin::signed(3), 0, true));
        // ayes = 3, total_members = 5 -> 60%, same tally as the rejection test above.

        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        let origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        let result = <EnsureLegislatureMotion<Test> as frame_support::traits::EnsureOriginWithArg<
            RuntimeOrigin,
            ([u8; 32], u8),
        >>::try_origin(origin, &(CALL_HASH_A, 51));
        assert!(result.is_ok());
    });
}

/// Gaming resistance at the mechanism level: presenting a *different* call_hash than the one
/// the motion actually approved is rejected regardless of the required percentage supplied --
/// even a trivially-low required percentage (1%) cannot substitute for hash-binding. This is
/// the piece pallet-constitution's tier derivation leans on: swapping the tier at execution
/// time changes the recomputed call_hash (tier is part of its preimage for `enact_law`), so it
/// can never match what the motion approved, independent of any threshold check.
#[test]
fn tiered_origin_rejects_mismatched_call_hash_even_with_trivial_required_percentage() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        // 100% support (single member) would clear any percentage requirement -- but the
        // caller presents CALL_HASH_B, not the approved CALL_HASH_A.
        let origin: RuntimeOrigin = RuntimeOrigin::signed(1);
        let result = <EnsureLegislatureMotion<Test> as frame_support::traits::EnsureOriginWithArg<
            RuntimeOrigin,
            ([u8; 32], u8),
        >>::try_origin(origin, &(CALL_HASH_B, 1));
        assert!(result.is_err());
        assert!(PendingLegislatureApproval::<Test>::get().is_some());
    });
}

// ─── clear_stale_approval ───────────────────────────────────────────────────

#[test]
fn clear_stale_approval_fails_when_not_yet_expired() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));

        // Still within the expiry window (APPROVAL_EXPIRY = 20 blocks after planting).
        System::set_block_number(1 + MOTION_DURATION as u64 + APPROVAL_EXPIRY as u64 - 1);
        assert_noop!(
            Legislature::clear_stale_approval(RuntimeOrigin::signed(1)),
            Error::<Test>::ApprovalNotYetStale
        );
        assert!(PendingLegislatureApproval::<Test>::get().is_some());
    });
}

#[test]
fn clear_stale_approval_fails_when_none_pending() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_noop!(
            Legislature::clear_stale_approval(RuntimeOrigin::signed(1)),
            Error::<Test>::NoPendingApproval
        );
    });
}

#[test]
fn clear_stale_approval_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));
        System::set_block_number(1 + MOTION_DURATION as u64 + APPROVAL_EXPIRY as u64);

        assert_noop!(
            Legislature::clear_stale_approval(RuntimeOrigin::signed(999)),
            Error::<Test>::NotAMember
        );
    });
}

/// The deadlock recovery this fix adds: once a passed motion's token sits unconsumed past
/// `PendingApprovalExpiryBlocks`, *any* current member (not necessarily the stuck proposer)
/// can clear it, and a subsequent motion is then free to pass and plant a new token --
/// proving the legislature is no longer permanently stuck.
#[test]
fn clear_stale_approval_unblocks_a_new_motion_after_expiry() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);

        // Motion A passes and plants a token that nobody ever consumes (simulating an
        // offline/removed proposer).
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(1 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 0));
        let planted_at = 1 + MOTION_DURATION as u64;

        // Before expiry, a second passed motion cannot plant its own token.
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(2), CALL_HASH_B));
        System::set_block_number(planted_at + MOTION_DURATION as u64);
        assert_noop!(
            Legislature::close_motion(RuntimeOrigin::signed(2), 1),
            Error::<Test>::ApprovalPending
        );

        // Move past the expiry window and clear the stale token -- called by member 2,
        // who is not the original proposer.
        System::set_block_number(planted_at + APPROVAL_EXPIRY as u64);
        assert_ok!(Legislature::clear_stale_approval(RuntimeOrigin::signed(2)));
        System::assert_last_event(
            Event::PendingApprovalExpired { call_hash: CALL_HASH_A }.into(),
        );
        assert!(PendingLegislatureApproval::<Test>::get().is_none());

        // Motion B, already closed as executed=false path retried: close it again isn't
        // possible (already executed=true was NOT set on the failed attempt above, since
        // `close_motion` only marks executed after the ApprovalPending check -- re-propose
        // a fresh motion C instead to prove the legislature is unblocked).
        assert_ok!(Legislature::propose_motion(RuntimeOrigin::signed(1), CALL_HASH_A));
        System::set_block_number(planted_at + APPROVAL_EXPIRY as u64 + MOTION_DURATION as u64);
        assert_ok!(Legislature::close_motion(RuntimeOrigin::signed(1), 2));

        assert!(PendingLegislatureApproval::<Test>::get().is_some());
    });
}

// ─── Bootstrap lock: Root can seed initial members, but not forever ────────────────────────

#[test]
fn root_can_add_and_remove_members_pre_bootstrap() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        add(2);
        assert_ok!(Legislature::remove_member(RuntimeOrigin::root(), 2));
        assert_eq!(Members::<Test>::get().to_vec(), vec![1]);
        assert!(!Bootstrapped::<Test>::get());
    });
}

#[test]
fn close_bootstrap_requires_root() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_noop!(
            Legislature::close_bootstrap(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn close_bootstrap_requires_at_least_one_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Legislature::close_bootstrap(RuntimeOrigin::root()),
            Error::<Test>::NoMembersToBootstrap
        );
    });
}

#[test]
fn close_bootstrap_cannot_be_called_twice() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::close_bootstrap(RuntimeOrigin::root()));
        assert!(Bootstrapped::<Test>::get());
        System::assert_last_event(Event::BootstrapClosed.into());
        assert_noop!(
            Legislature::close_bootstrap(RuntimeOrigin::root()),
            Error::<Test>::BootstrapClosed
        );
    });
}

/// The core fix this pallet gained: once bootstrapped, `Root` can never again unilaterally
/// add or remove a legislature member — closing the gap where a compromised sudo key could
/// pack or purge the legislature forever.
#[test]
fn root_cannot_add_or_remove_members_after_bootstrap_closed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::close_bootstrap(RuntimeOrigin::root()));

        assert_noop!(
            Legislature::add_member(RuntimeOrigin::root(), 2),
            Error::<Test>::BootstrapClosed
        );
        assert_noop!(
            Legislature::remove_member(RuntimeOrigin::root(), 1),
            Error::<Test>::BootstrapClosed
        );
        assert_eq!(Members::<Test>::get().to_vec(), vec![1]);
    });
}

/// `pallet_elections`'s election-driven seating path must keep working after bootstrap
/// closes -- it does not go through `add_member`/`remove_member` at all.
#[test]
fn election_seating_path_unaffected_by_closed_bootstrap() {
    use pallet_elections::pallet::SeatLegislature;

    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        add(1);
        assert_ok!(Legislature::close_bootstrap(RuntimeOrigin::root()));

        assert_ok!(<Legislature as SeatLegislature<u64>>::replace_members(vec![9, 10]));
        assert_eq!(Members::<Test>::get().to_vec(), vec![9, 10]);
    });
}
