use crate::{
    mock::*, accountability_call_hash, ApprovedAction, Error, Event,
    EnsureAccountabilityCouncilApproved, Members, PendingAction,
};
use frame_support::{assert_noop, assert_ok, traits::EnsureOriginWithArg};
use sp_runtime::DispatchError;

/// Adds `accounts` to the Council via root (pre-bootstrap path), in order.
fn bootstrap_members(accounts: &[u64]) {
    for a in accounts {
        assert_ok!(AccountabilityCouncil::add_member(RuntimeOrigin::root(), *a));
    }
}

/// Bootstraps a 7-member Council (accounts 1..=7) and closes bootstrap, so every subsequent
/// membership change must go through the supermajority propose/approve flow.
fn bootstrapped_council() {
    bootstrap_members(&[1, 2, 3, 4, 5, 6, 7]);
    assert_ok!(AccountabilityCouncil::close_bootstrap(RuntimeOrigin::root()));
}

/// Has `members` (in order) propose then approve `call_hash`, stopping as soon as the
/// supermajority threshold resolves it (mirrors real client behavior: no point approving after
/// the action already took effect).
fn approve_with(members: &[u64], call_hash: [u8; 32]) {
    let mut iter = members.iter();
    let proposer = *iter.next().expect("at least one approver");
    assert_ok!(AccountabilityCouncil::propose_action(RuntimeOrigin::signed(proposer), call_hash));
    for m in iter {
        if ApprovedAction::<Test>::get(call_hash).is_some() {
            break;
        }
        assert_ok!(AccountabilityCouncil::approve_action(RuntimeOrigin::signed(*m), call_hash));
    }
}

// ─── Bootstrap: Root can seed initial members, but not after close_bootstrap ───────────────

#[test]
fn root_can_add_members_pre_bootstrap() {
    new_test_ext().execute_with(|| {
        assert_ok!(AccountabilityCouncil::add_member(RuntimeOrigin::root(), 1));
        assert_eq!(Members::<Test>::get().into_inner(), vec![1]);
        System::assert_last_event(Event::MemberAdded { who: 1 }.into());
    });
}

#[test]
fn signed_origin_cannot_add_members_pre_bootstrap() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            AccountabilityCouncil::add_member(RuntimeOrigin::signed(1), 2),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn close_bootstrap_requires_root() {
    new_test_ext().execute_with(|| {
        bootstrap_members(&[1]);
        assert_noop!(
            AccountabilityCouncil::close_bootstrap(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn close_bootstrap_requires_at_least_one_member() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            AccountabilityCouncil::close_bootstrap(RuntimeOrigin::root()),
            Error::<Test>::NoMembersToBootstrap
        );
    });
}

#[test]
fn close_bootstrap_cannot_be_called_twice() {
    new_test_ext().execute_with(|| {
        bootstrap_members(&[1]);
        assert_ok!(AccountabilityCouncil::close_bootstrap(RuntimeOrigin::root()));
        assert_noop!(
            AccountabilityCouncil::close_bootstrap(RuntimeOrigin::root()),
            Error::<Test>::AlreadyBootstrapped
        );
    });
}

/// The core self-perpetuation guarantee: once bootstrapped, Root can no longer unilaterally
/// add or remove a Council member — the direct call now requires an
/// `EnsureAccountabilityCouncilApproved` token, which Root's bare origin can never satisfy.
#[test]
fn root_cannot_add_or_remove_members_after_bootstrap() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();

        assert_noop!(
            AccountabilityCouncil::add_member(RuntimeOrigin::root(), 8),
            DispatchError::BadOrigin
        );
        assert_noop!(
            AccountabilityCouncil::remove_member(RuntimeOrigin::root(), 1),
            DispatchError::BadOrigin
        );
        // Membership is unchanged.
        assert_eq!(Members::<Test>::get().len(), 7);
    });
}

/// After bootstrap, the Council can still grow/shrink itself — just via its own supermajority
/// vote instead of Root.
#[test]
fn council_can_add_member_via_own_supermajority_after_bootstrap() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();

        let hash = accountability_call_hash(b"pallet-accountability-council::add_member", &8u64);
        // 5 of 7 is 2/3+ (5*3=15 >= 7*2=14).
        approve_with(&[1, 2, 3, 4, 5], hash);
        assert!(ApprovedAction::<Test>::get(hash).is_some());

        assert_ok!(AccountabilityCouncil::add_member(RuntimeOrigin::signed(1), 8));
        assert!(Members::<Test>::get().contains(&8));
        System::assert_last_event(Event::MemberAdded { who: 8 }.into());
        // The token was consumed — a second attempt to reuse it fails.
        assert!(ApprovedAction::<Test>::get(hash).is_none());
    });
}

#[test]
fn council_can_remove_member_via_own_supermajority_after_bootstrap() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();

        let hash = accountability_call_hash(b"pallet-accountability-council::remove_member", &7u64);
        approve_with(&[1, 2, 3, 4, 5], hash);

        assert_ok!(AccountabilityCouncil::remove_member(RuntimeOrigin::signed(2), 7));
        assert!(!Members::<Test>::get().contains(&7));
        System::assert_last_event(Event::MemberRemoved { who: 7 }.into());
    });
}

// ─── Incompatibility: legislature/executive overlap is rejected ───────────────────────────

#[test]
fn add_member_rejects_current_legislature_member() {
    new_test_ext().execute_with(|| {
        set_legislature_member(9);
        assert_noop!(
            AccountabilityCouncil::add_member(RuntimeOrigin::root(), 9),
            Error::<Test>::LegislatureOrExecutiveOverlap
        );
        assert!(!Members::<Test>::get().contains(&9));
    });
}

#[test]
fn add_member_rejects_current_executive_minister() {
    new_test_ext().execute_with(|| {
        set_active_minister(10);
        assert_noop!(
            AccountabilityCouncil::add_member(RuntimeOrigin::root(), 10),
            Error::<Test>::LegislatureOrExecutiveOverlap
        );
        assert!(!Members::<Test>::get().contains(&10));
    });
}

/// The incompatibility check also applies to the post-bootstrap, council-approved path — an
/// approved token cannot smuggle in an otherwise-ineligible account.
#[test]
fn council_approved_add_member_still_rejects_overlap() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        set_legislature_member(11);

        let hash = accountability_call_hash(b"pallet-accountability-council::add_member", &11u64);
        approve_with(&[1, 2, 3, 4, 5], hash);
        assert!(ApprovedAction::<Test>::get(hash).is_some());

        assert_noop!(
            AccountabilityCouncil::add_member(RuntimeOrigin::signed(1), 11),
            Error::<Test>::LegislatureOrExecutiveOverlap
        );
        // `assert_noop!` already confirms the whole call's storage effects were rolled back
        // (FRAME wraps a dispatchable's execution in a storage transaction) — so the approved
        // token survives this failed attempt and could still be consumed once `11` becomes
        // eligible, without the Council needing to re-propose and re-approve from scratch.
        assert!(ApprovedAction::<Test>::get(hash).is_some());
    });
}

// ─── propose_action / approve_action: minority cannot approve, supermajority can ──────────

#[test]
fn lone_vote_cannot_approve_a_generic_action() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        let hash = [7u8; 32];

        assert_ok!(AccountabilityCouncil::propose_action(RuntimeOrigin::signed(1), hash));
        // Only 1 of 7 approved (1*3=3 < 7*2=14) — not resolved.
        assert!(PendingAction::<Test>::get(hash).is_some());
        assert!(ApprovedAction::<Test>::get(hash).is_none());

        // The origin this action would authorize still refuses.
        assert!(EnsureAccountabilityCouncilApproved::<Test>::try_origin(
            RuntimeOrigin::signed(1),
            &hash
        )
        .is_err());
    });
}

#[test]
fn minority_of_four_of_seven_cannot_approve() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        let hash = [8u8; 32];

        // 3 of 7 is short of 2/3 (3*3=9 < 7*2=14).
        assert_ok!(AccountabilityCouncil::propose_action(RuntimeOrigin::signed(1), hash));
        assert_ok!(AccountabilityCouncil::approve_action(RuntimeOrigin::signed(2), hash));
        assert_ok!(AccountabilityCouncil::approve_action(RuntimeOrigin::signed(3), hash));
        assert!(PendingAction::<Test>::get(hash).is_some());
        assert!(ApprovedAction::<Test>::get(hash).is_none());
    });
}

#[test]
fn supermajority_of_five_of_seven_approves_a_generic_action() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        let hash = [9u8; 32];

        // 5 of 7 clears 2/3 (5*3=15 >= 7*2=14).
        approve_with(&[1, 2, 3, 4, 5], hash);
        assert!(PendingAction::<Test>::get(hash).is_none());
        assert!(ApprovedAction::<Test>::get(hash).is_some());
        System::assert_has_event(Event::ActionApproved { call_hash: hash }.into());

        // Any current member (not only the proposer) may consume the token, exactly once.
        assert!(EnsureAccountabilityCouncilApproved::<Test>::try_origin(
            RuntimeOrigin::signed(6),
            &hash
        )
        .is_ok());
        assert!(ApprovedAction::<Test>::get(hash).is_none());
        assert!(EnsureAccountabilityCouncilApproved::<Test>::try_origin(
            RuntimeOrigin::signed(6),
            &hash
        )
        .is_err());
    });
}

#[test]
fn exact_two_thirds_boundary_of_six_members_approves() {
    new_test_ext().execute_with(|| {
        bootstrap_members(&[1, 2, 3, 4, 5, 6]);
        assert_ok!(AccountabilityCouncil::close_bootstrap(RuntimeOrigin::root()));
        let hash = [10u8; 32];

        // 4 of 6 is exactly 2/3 (4*3=12 >= 6*2=12) — the `>=` boundary must include this.
        assert_ok!(AccountabilityCouncil::propose_action(RuntimeOrigin::signed(1), hash));
        assert_ok!(AccountabilityCouncil::approve_action(RuntimeOrigin::signed(2), hash));
        assert_ok!(AccountabilityCouncil::approve_action(RuntimeOrigin::signed(3), hash));
        assert!(ApprovedAction::<Test>::get(hash).is_none());
        assert_ok!(AccountabilityCouncil::approve_action(RuntimeOrigin::signed(4), hash));
        assert!(ApprovedAction::<Test>::get(hash).is_some());
    });
}

#[test]
fn propose_action_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        assert_noop!(
            AccountabilityCouncil::propose_action(RuntimeOrigin::signed(99), [1u8; 32]),
            Error::<Test>::NotCouncilMember
        );
    });
}

#[test]
fn approve_action_rejects_double_approval() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        let hash = [2u8; 32];
        assert_ok!(AccountabilityCouncil::propose_action(RuntimeOrigin::signed(1), hash));
        assert_noop!(
            AccountabilityCouncil::approve_action(RuntimeOrigin::signed(1), hash),
            Error::<Test>::AlreadyApproved
        );
    });
}

#[test]
fn propose_action_rejects_duplicate_call_hash() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        let hash = [3u8; 32];
        assert_ok!(AccountabilityCouncil::propose_action(RuntimeOrigin::signed(1), hash));
        assert_noop!(
            AccountabilityCouncil::propose_action(RuntimeOrigin::signed(2), hash),
            Error::<Test>::ActionAlreadyProposed
        );
    });
}

// ─── remove_member purges in-flight approvals from the removed member ─────────────────────

#[test]
fn remove_member_purges_their_approval_from_pending_actions() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        let target_hash = [4u8; 32];
        // Members 1 and 3 approve a generic action; member 3 will then be removed.
        assert_ok!(AccountabilityCouncil::propose_action(RuntimeOrigin::signed(1), target_hash));
        assert_ok!(AccountabilityCouncil::approve_action(RuntimeOrigin::signed(3), target_hash));
        assert_eq!(PendingAction::<Test>::get(target_hash).unwrap().1.len(), 2);

        let remove_hash =
            accountability_call_hash(b"pallet-accountability-council::remove_member", &3u64);
        approve_with(&[2, 4, 5, 6, 7], remove_hash);
        assert_ok!(AccountabilityCouncil::remove_member(RuntimeOrigin::signed(2), 3));

        // Member 3's approval on the unrelated pending action was purged.
        assert_eq!(PendingAction::<Test>::get(target_hash).unwrap().1.len(), 1);
        assert!(!PendingAction::<Test>::get(target_hash).unwrap().1.contains(&3));
    });
}

// ─── clear_stale_action ─────────────────────────────────────────────────────────────────────

#[test]
fn clear_stale_action_requires_expiry_elapsed() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        let hash = [5u8; 32];
        approve_with(&[1, 2, 3, 4, 5], hash);
        assert!(ApprovedAction::<Test>::get(hash).is_some());

        assert_noop!(
            AccountabilityCouncil::clear_stale_action(RuntimeOrigin::signed(1), hash),
            Error::<Test>::ApprovalNotYetStale
        );

        System::set_block_number(System::block_number() + APPROVAL_EXPIRY as u64);
        assert_ok!(AccountabilityCouncil::clear_stale_action(RuntimeOrigin::signed(1), hash));
        assert!(ApprovedAction::<Test>::get(hash).is_none());
        System::assert_last_event(Event::ActionExpired { call_hash: hash }.into());
    });
}

#[test]
fn clear_stale_action_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        bootstrapped_council();
        assert_noop!(
            AccountabilityCouncil::clear_stale_action(RuntimeOrigin::signed(99), [6u8; 32]),
            Error::<Test>::NotCouncilMember
        );
    });
}

// ─── Capacity / duplicate-member guards ─────────────────────────────────────────────────────

#[test]
fn add_member_fails_when_already_member() {
    new_test_ext().execute_with(|| {
        bootstrap_members(&[1]);
        assert_noop!(
            AccountabilityCouncil::add_member(RuntimeOrigin::root(), 1),
            Error::<Test>::AlreadyMember
        );
    });
}

#[test]
fn add_member_fails_when_at_capacity() {
    new_test_ext().execute_with(|| {
        let full: Vec<u64> = (1..=MAX_COUNCIL_SIZE as u64).collect();
        bootstrap_members(&full);
        assert_noop!(
            AccountabilityCouncil::add_member(RuntimeOrigin::root(), MAX_COUNCIL_SIZE as u64 + 1),
            Error::<Test>::CouncilAtCapacity
        );
    });
}

#[test]
fn remove_member_fails_when_not_found() {
    new_test_ext().execute_with(|| {
        bootstrap_members(&[1]);
        assert_noop!(
            AccountabilityCouncil::remove_member(RuntimeOrigin::root(), 42),
            Error::<Test>::MemberNotFound
        );
    });
}
