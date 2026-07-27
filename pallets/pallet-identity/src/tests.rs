use crate::{
    mock::*, AllowedMerkleRoots, CitizenIndex, CitizenNullifier, CitizenPosition, Error, Event,
    NullifierRegistry, SuspendedNullifiers, TotalCitizens,
};
use frame_support::{assert_noop, assert_ok, traits::ConstU32, BoundedVec};
use sp_runtime::DispatchError;

fn valid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap()
}

fn invalid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![INVALID_PROOF_MARKER]).unwrap()
}

/// Builds a well-formed 5-signal public_inputs vector (matching the Rarimo
/// registerIdentity circuit layout) with the given nullifier (index 2, dg1Commitment)
/// and merkle root (index 4, slaveMerkleRoot); the other slots are left zeroed.
fn public_inputs(nullifier: [u8; 32], merkle_root: [u8; 32]) -> BoundedVec<[u8; 32], ConstU32<16>> {
    let mut v = vec![[0u8; 32]; 5];
    v[2] = nullifier;
    v[4] = merkle_root;
    BoundedVec::try_from(v).unwrap()
}

const ROOT: [u8; 32] = [7u8; 32];
const NULLIFIER_A: [u8; 32] = [1u8; 32];
const NULLIFIER_B: [u8; 32] = [2u8; 32];

fn allow_root() {
    assert_ok!(Identity::add_allowed_merkle_root(RuntimeOrigin::root(), ROOT));
}

fn register(who: u64, nullifier: [u8; 32]) {
    assert_ok!(Identity::register_citizen(
        RuntimeOrigin::signed(who),
        valid_proof(),
        public_inputs(nullifier, ROOT),
    ));
}

// ─── register_citizen ───────────────────────────────────────────────────────

#[test]
fn register_citizen_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();

        assert_ok!(Identity::register_citizen(
            RuntimeOrigin::signed(1),
            valid_proof(),
            public_inputs(NULLIFIER_A, ROOT),
        ));

        assert_eq!(CitizenNullifier::<Test>::get(1), Some(NULLIFIER_A));
        assert_eq!(NullifierRegistry::<Test>::get(NULLIFIER_A), Some(1));
        assert_eq!(CitizenIndex::<Test>::get(0), Some(1));
        assert_eq!(CitizenPosition::<Test>::get(1), Some(0));
        assert_eq!(TotalCitizens::<Test>::get(), 1);
        System::assert_last_event(
            Event::CitizenRegistered { who: 1, nullifier: NULLIFIER_A }.into(),
        );
    });
}

#[test]
fn register_citizen_fails_when_already_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_B, ROOT),
            ),
            Error::<Test>::AlreadyRegistered
        );
    });
}

#[test]
fn register_citizen_fails_with_too_few_public_inputs() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        let short_inputs: BoundedVec<[u8; 32], ConstU32<16>> =
            BoundedVec::try_from(vec![[0u8; 32]; 4]).unwrap();

        assert_noop!(
            Identity::register_citizen(RuntimeOrigin::signed(1), valid_proof(), short_inputs),
            Error::<Test>::InvalidZKProof
        );
    });
}

#[test]
fn register_citizen_fails_when_issuer_not_allowed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Merkle root never added to the allowlist.
        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT),
            ),
            Error::<Test>::IssuerNotAllowed
        );
    });
}

#[test]
fn register_citizen_fails_when_proof_invalid() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                invalid_proof(),
                public_inputs(NULLIFIER_A, ROOT),
            ),
            Error::<Test>::InvalidZKProof
        );
    });
}

#[test]
fn register_citizen_fails_when_nullifier_already_used() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        // A different, not-yet-registered account tries to reuse the same nullifier.
        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(2),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT),
            ),
            Error::<Test>::NullifierAlreadyUsed
        );
    });
}

#[test]
fn register_citizen_fails_on_total_citizens_overflow() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        TotalCitizens::<Test>::put(u32::MAX);

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT),
            ),
            Error::<Test>::TotalCitizensOverflow
        );
        // Noop: nothing should have been registered for account 1.
        assert!(CitizenNullifier::<Test>::get(1).is_none());
    });
}

// ─── revoke_citizen ─────────────────────────────────────────────────────────

#[test]
fn revoke_citizen_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert_ok!(Identity::revoke_citizen(RuntimeOrigin::signed(1)));

        assert!(CitizenNullifier::<Test>::get(1).is_none());
        assert!(NullifierRegistry::<Test>::get(NULLIFIER_A).is_none());
        assert!(CitizenPosition::<Test>::get(1).is_none());
        assert_eq!(TotalCitizens::<Test>::get(), 0);
        System::assert_last_event(Event::CitizenRevoked { who: 1 }.into());
    });
}

#[test]
fn revoke_citizen_swap_and_pop_keeps_index_dense() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);
        register(2, NULLIFIER_B);
        register(3, [3u8; 32]);
        // Indices: 0 -> 1, 1 -> 2, 2 -> 3

        // Revoke the first citizen; the last (account 3) should be swapped into slot 0.
        assert_ok!(Identity::revoke_citizen(RuntimeOrigin::signed(1)));

        assert_eq!(TotalCitizens::<Test>::get(), 2);
        assert_eq!(CitizenIndex::<Test>::get(0), Some(3));
        assert_eq!(CitizenPosition::<Test>::get(3), Some(0));
        assert!(CitizenIndex::<Test>::get(2).is_none());
        // Untouched middle entry.
        assert_eq!(CitizenIndex::<Test>::get(1), Some(2));
    });
}

#[test]
fn revoke_citizen_fails_when_not_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::revoke_citizen(RuntimeOrigin::signed(1)),
            Error::<Test>::NotRegistered
        );
    });
}

#[test]
fn revoke_citizen_fails_while_indefinitely_suspended() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, None));

        assert_noop!(
            Identity::revoke_citizen(RuntimeOrigin::signed(1)),
            Error::<Test>::CannotRevokeWhileSuspended
        );
    });
}

#[test]
fn revoke_citizen_fails_while_timed_suspension_still_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, Some(10)));

        // Still at block 1, well before the suspension lifts at block 10.
        assert_noop!(
            Identity::revoke_citizen(RuntimeOrigin::signed(1)),
            Error::<Test>::CannotRevokeWhileSuspended
        );
    });
}

#[test]
fn revoke_citizen_succeeds_after_timed_suspension_expires() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, Some(10)));

        System::set_block_number(11);
        assert_ok!(Identity::revoke_citizen(RuntimeOrigin::signed(1)));
    });
}

// ─── suspend_citizen ────────────────────────────────────────────────────────

#[test]
fn suspend_citizen_works_indefinite() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, None));

        assert_eq!(SuspendedNullifiers::<Test>::get(NULLIFIER_A), Some(None));
        assert!(!Identity::is_active_citizen(&1));
        System::assert_last_event(
            Event::CitizenSuspended { nullifier: NULLIFIER_A, until: None }.into(),
        );
    });
}

#[test]
fn suspend_citizen_works_timed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, Some(5)));

        assert_eq!(SuspendedNullifiers::<Test>::get(NULLIFIER_A), Some(Some(5)));
        System::assert_last_event(
            Event::CitizenSuspended { nullifier: NULLIFIER_A, until: Some(5) }.into(),
        );
    });
}

#[test]
fn suspend_citizen_upserts_existing_suspension() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, Some(5)));
        // Court extends the suspension — this should overwrite, not error.
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, Some(50)));

        assert_eq!(SuspendedNullifiers::<Test>::get(NULLIFIER_A), Some(Some(50)));
        System::assert_last_event(
            Event::CitizenSuspended { nullifier: NULLIFIER_A, until: Some(50) }.into(),
        );
    });
}

#[test]
fn suspend_citizen_fails_when_not_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, None),
            Error::<Test>::NotRegistered
        );
    });
}

#[test]
fn suspend_citizen_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert_noop!(
            Identity::suspend_citizen(RuntimeOrigin::signed(1), NULLIFIER_A, None),
            DispatchError::BadOrigin
        );
    });
}

// ─── restore_citizen_rights ─────────────────────────────────────────────────

#[test]
fn restore_citizen_rights_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, None));

        assert_ok!(Identity::restore_citizen_rights(RuntimeOrigin::root(), NULLIFIER_A));

        assert!(SuspendedNullifiers::<Test>::get(NULLIFIER_A).is_none());
        assert!(Identity::is_active_citizen(&1));
        System::assert_last_event(Event::CitizenRestored { nullifier: NULLIFIER_A }.into());
    });
}

#[test]
fn restore_citizen_rights_fails_when_not_suspended() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert_noop!(
            Identity::restore_citizen_rights(RuntimeOrigin::root(), NULLIFIER_A),
            Error::<Test>::NotSuspended
        );
    });
}

#[test]
fn restore_citizen_rights_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, None));

        assert_noop!(
            Identity::restore_citizen_rights(RuntimeOrigin::signed(1), NULLIFIER_A),
            DispatchError::BadOrigin
        );
    });
}

// ─── trusted issuer Merkle root allowlist ───────────────────────────────────

#[test]
fn add_allowed_merkle_root_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::add_allowed_merkle_root(RuntimeOrigin::root(), ROOT));

        assert!(AllowedMerkleRoots::<Test>::get(ROOT));
        System::assert_last_event(Event::MerkleRootAdded { merkle_root: ROOT }.into());
    });
}

#[test]
fn add_allowed_merkle_root_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::add_allowed_merkle_root(RuntimeOrigin::signed(1), ROOT),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn remove_allowed_merkle_root_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();

        assert_ok!(Identity::remove_allowed_merkle_root(RuntimeOrigin::root(), ROOT));

        assert!(!AllowedMerkleRoots::<Test>::get(ROOT));
        System::assert_last_event(Event::MerkleRootRemoved { merkle_root: ROOT }.into());
    });
}

#[test]
fn remove_allowed_merkle_root_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();

        assert_noop!(
            Identity::remove_allowed_merkle_root(RuntimeOrigin::signed(1), ROOT),
            DispatchError::BadOrigin
        );
    });
}

// ─── helper functions (is_active_citizen / is_citizen / citizen_at) ────────

#[test]
fn is_active_citizen_true_when_registered_and_not_suspended() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);

        assert!(Identity::is_active_citizen(&1));
        assert!(Identity::is_citizen(&1));
    });
}

#[test]
fn is_active_citizen_false_when_not_registered() {
    new_test_ext().execute_with(|| {
        assert!(!Identity::is_active_citizen(&1));
        assert!(!Identity::is_citizen(&1));
    });
}

#[test]
fn is_active_citizen_lazily_clears_expired_timed_suspension() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A);
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, Some(10)));

        // Before expiry: suspended.
        assert!(!Identity::is_active_citizen(&1));
        assert!(SuspendedNullifiers::<Test>::get(NULLIFIER_A).is_some());

        // After expiry: active again, and the stale record is cleaned up as a side effect.
        System::set_block_number(11);
        assert!(Identity::is_active_citizen(&1));
        assert!(SuspendedNullifiers::<Test>::get(NULLIFIER_A).is_none());
    });
}

#[test]
fn citizen_at_and_total_citizens_track_registrations() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        assert_eq!(Identity::total_citizens(), 0);
        assert!(Identity::citizen_at(0).is_none());

        register(1, NULLIFIER_A);
        register(2, NULLIFIER_B);

        assert_eq!(Identity::total_citizens(), 2);
        assert_eq!(Identity::citizen_at(0), Some(1));
        assert_eq!(Identity::citizen_at(1), Some(2));
    });
}
