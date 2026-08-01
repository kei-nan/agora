use crate::{
    mock::*, AllowedMerkleRoots, CitizenAnchor, CitizenIndex, CitizenNullifier, CitizenPosition,
    Error, Event, IdentityAnchorRegistry, NullifierRegistry, OprfSchemeVersion,
    ReverificationDeadline, SelfDeclaredSingleDocument, SuspendedNullifiers, TotalCitizens,
};
use frame_support::{assert_noop, assert_ok, traits::ConstU32, BoundedVec};
use sp_runtime::DispatchError;

fn valid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap()
}

fn invalid_proof() -> BoundedVec<u8, ConstU32<4096>> {
    BoundedVec::try_from(vec![INVALID_PROOF_MARKER]).unwrap()
}

/// Builds a well-formed count_4 public_inputs vector (ZKPassport's outer-circuit layout,
/// 9 fields: 8 fixed + 1 disclosure subproof — see `runtime/src/verifier.rs`) with the
/// given nullifier (index 7 = `6 + D`, `scoped_nullifier`) and merkle root (index 0,
/// `certificate_registry_root`); the other slots are left zeroed.
fn public_inputs(nullifier: [u8; 32], merkle_root: [u8; 32]) -> BoundedVec<[u8; 32], ConstU32<18>> {
    let mut v = vec![[0u8; 32]; 9];
    v[0] = merkle_root;
    v[7] = nullifier;
    BoundedVec::try_from(v).unwrap()
}

const ROOT: [u8; 32] = [7u8; 32];
const NULLIFIER_A: [u8; 32] = [1u8; 32];
const NULLIFIER_B: [u8; 32] = [2u8; 32];
const ANCHOR_A: [u8; 32] = [11u8; 32];
const ANCHOR_B: [u8; 32] = [12u8; 32];
const ANCHOR_C: [u8; 32] = [13u8; 32];

fn allow_root() {
    assert_ok!(Identity::add_allowed_merkle_root(RuntimeOrigin::root(), ROOT));
}

fn register(who: u64, nullifier: [u8; 32], anchor: [u8; 32]) {
    assert_ok!(Identity::register_citizen(
        RuntimeOrigin::signed(who),
        valid_proof(),
        public_inputs(nullifier, ROOT),
        anchor,
        valid_proof(),
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
            ANCHOR_A,
            valid_proof(),
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
fn register_citizen_sets_anchor_and_reverification_deadline() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // ReverificationPeriod = 10 in the mock; registered at block 1 -> deadline 11.
        assert_eq!(ReverificationDeadline::<Test>::get(1), Some(11));
        assert_eq!(CitizenAnchor::<Test>::get(1), Some((0, ANCHOR_A)));
        assert_eq!(IdentityAnchorRegistry::<Test>::get((0, ANCHOR_A)), Some(1));
    });
}

#[test]
fn register_citizen_fails_when_already_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_B, ROOT),
                ANCHOR_B,
                valid_proof(),
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
        // One short of count_4's 9-field minimum.
        let short_inputs: BoundedVec<[u8; 32], ConstU32<18>> =
            BoundedVec::try_from(vec![[0u8; 32]; 8]).unwrap();

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                short_inputs,
                ANCHOR_A,
                valid_proof(),
            ),
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
                ANCHOR_A,
                valid_proof(),
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
                ANCHOR_A,
                valid_proof(),
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
        register(1, NULLIFIER_A, ANCHOR_A);

        // A different, not-yet-registered account tries to reuse the same nullifier
        // (with a fresh anchor, to isolate the nullifier check from the anchor check).
        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(2),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT),
                ANCHOR_B,
                valid_proof(),
            ),
            Error::<Test>::NullifierAlreadyUsed
        );
    });
}

#[test]
fn register_citizen_fails_when_anchor_already_used() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // A different account, with a fresh nullifier, tries to reuse the same anchor —
        // this is exactly the case the anchor exclusion registry exists to catch (HANDOFF
        // log #67): a renewed/duplicate passport producing a fresh document-bound nullifier
        // but the same underlying person.
        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(2),
                valid_proof(),
                public_inputs(NULLIFIER_B, ROOT),
                ANCHOR_A,
                valid_proof(),
            ),
            Error::<Test>::AnchorAlreadyUsed
        );
    });
}

#[test]
fn register_citizen_fails_when_anchor_proof_invalid() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT),
                ANCHOR_A,
                invalid_proof(),
            ),
            Error::<Test>::InvalidAnchorProof
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
                ANCHOR_A,
                valid_proof(),
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
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::revoke_citizen(RuntimeOrigin::signed(1)));

        assert!(CitizenNullifier::<Test>::get(1).is_none());
        assert!(NullifierRegistry::<Test>::get(NULLIFIER_A).is_none());
        assert!(CitizenPosition::<Test>::get(1).is_none());
        assert!(CitizenAnchor::<Test>::get(1).is_none());
        assert!(IdentityAnchorRegistry::<Test>::get((0, ANCHOR_A)).is_none());
        assert!(ReverificationDeadline::<Test>::get(1).is_none());
        assert_eq!(TotalCitizens::<Test>::get(), 0);
        System::assert_last_event(Event::CitizenRevoked { who: 1 }.into());
    });
}

#[test]
fn revoke_citizen_swap_and_pop_keeps_index_dense() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        register(2, NULLIFIER_B, ANCHOR_B);
        register(3, [3u8; 32], ANCHOR_C);
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
        register(1, NULLIFIER_A, ANCHOR_A);
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
        register(1, NULLIFIER_A, ANCHOR_A);
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
        register(1, NULLIFIER_A, ANCHOR_A);
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
        register(1, NULLIFIER_A, ANCHOR_A);

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
        register(1, NULLIFIER_A, ANCHOR_A);

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
        register(1, NULLIFIER_A, ANCHOR_A);

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
        register(1, NULLIFIER_A, ANCHOR_A);

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
        register(1, NULLIFIER_A, ANCHOR_A);
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
        register(1, NULLIFIER_A, ANCHOR_A);

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
        register(1, NULLIFIER_A, ANCHOR_A);
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

// ─── reverify_citizen ───────────────────────────────────────────────────────

#[test]
fn reverify_citizen_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        assert_eq!(ReverificationDeadline::<Test>::get(1), Some(11));

        System::set_block_number(5);
        assert_ok!(Identity::reverify_citizen(RuntimeOrigin::signed(1), valid_proof()));

        // Deadline is pushed forward from "now" (5), not from the old deadline (11).
        assert_eq!(ReverificationDeadline::<Test>::get(1), Some(15));
        System::assert_last_event(
            Event::CitizenReverified { who: 1, deadline: 15 }.into(),
        );
    });
}

#[test]
fn reverify_citizen_fails_when_not_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::reverify_citizen(RuntimeOrigin::signed(1), valid_proof()),
            Error::<Test>::NotRegistered
        );
    });
}

#[test]
fn reverify_citizen_fails_with_invalid_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_noop!(
            Identity::reverify_citizen(RuntimeOrigin::signed(1), invalid_proof()),
            Error::<Test>::InvalidReverificationProof
        );
    });
}

#[test]
fn is_active_citizen_false_once_reverification_deadline_passes() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        // Deadline is 11; still active at exactly the deadline block.
        System::set_block_number(11);
        assert!(Identity::is_active_citizen(&1));

        // One block past the deadline: lazily treated as inactive.
        System::set_block_number(12);
        assert!(!Identity::is_active_citizen(&1));
    });
}

#[test]
fn reverify_citizen_reactivates_a_lapsed_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        System::set_block_number(12);
        assert!(!Identity::is_active_citizen(&1));

        assert_ok!(Identity::reverify_citizen(RuntimeOrigin::signed(1), valid_proof()));
        assert!(Identity::is_active_citizen(&1));
    });
}

// ─── migrate_oprf_scheme ────────────────────────────────────────────────────

#[test]
fn migrate_oprf_scheme_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        assert_eq!(OprfSchemeVersion::<Test>::get(), 0);

        assert_ok!(Identity::migrate_oprf_scheme(
            RuntimeOrigin::signed(1),
            ANCHOR_A,
            ANCHOR_B,
            valid_proof(),
        ));

        assert_eq!(CitizenAnchor::<Test>::get(1), Some((1, ANCHOR_B)));
        assert!(IdentityAnchorRegistry::<Test>::get((0, ANCHOR_A)).is_none());
        assert_eq!(IdentityAnchorRegistry::<Test>::get((1, ANCHOR_B)), Some(1));
        // Eligibility/CitizenIndex bookkeeping is untouched by migration.
        assert_eq!(CitizenIndex::<Test>::get(0), Some(1));
        assert_eq!(TotalCitizens::<Test>::get(), 1);
        System::assert_last_event(
            Event::OprfAnchorMigrated { who: 1, new_scheme_version: 1 }.into(),
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_when_caller_not_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                ANCHOR_A,
                ANCHOR_B,
                valid_proof(),
            ),
            Error::<Test>::NotRegistered
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_when_old_anchor_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // Wrong old anchor — doesn't match what's on file for this citizen.
        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                ANCHOR_B,
                ANCHOR_C,
                valid_proof(),
            ),
            Error::<Test>::OldAnchorNotFound
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_when_new_anchor_already_used() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // Bump the global scheme version, then register a second citizen fresh under the
        // new version with ANCHOR_B — that's now a genuinely taken (1, ANCHOR_B) entry.
        assert_ok!(Identity::rotate_oprf_scheme(RuntimeOrigin::root()));
        register(2, NULLIFIER_B, ANCHOR_B);

        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                ANCHOR_A,
                ANCHOR_B,
                valid_proof(),
            ),
            Error::<Test>::NewAnchorAlreadyUsed
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_with_invalid_migration_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                ANCHOR_A,
                ANCHOR_B,
                invalid_proof(),
            ),
            Error::<Test>::InvalidMigrationProof
        );
    });
}

// ─── rotate_oprf_scheme / emergency_rotate_oprf_scheme ─────────────────────

#[test]
fn rotate_oprf_scheme_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_eq!(OprfSchemeVersion::<Test>::get(), 0);

        assert_ok!(Identity::rotate_oprf_scheme(RuntimeOrigin::root()));

        assert_eq!(OprfSchemeVersion::<Test>::get(), 1);
        System::assert_last_event(Event::OprfSchemeRotated { new_version: 1 }.into());
    });
}

#[test]
fn rotate_oprf_scheme_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::rotate_oprf_scheme(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn emergency_rotate_oprf_scheme_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_eq!(OprfSchemeVersion::<Test>::get(), 0);

        assert_ok!(Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::root()));

        assert_eq!(OprfSchemeVersion::<Test>::get(), 1);
        System::assert_last_event(
            Event::OprfSchemeEmergencyRotated { new_version: 1 }.into(),
        );
    });
}

#[test]
fn emergency_rotate_oprf_scheme_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

// ─── declare_no_other_passport ──────────────────────────────────────────────

#[test]
fn declare_no_other_passport_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::declare_no_other_passport(RuntimeOrigin::signed(1)));

        assert!(SelfDeclaredSingleDocument::<Test>::get(1));
        System::assert_last_event(Event::SelfDeclarationRecorded { who: 1 }.into());
    });
}

#[test]
fn declare_no_other_passport_fails_when_not_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::declare_no_other_passport(RuntimeOrigin::signed(1)),
            Error::<Test>::NotRegistered
        );
    });
}

// ─── helper functions (is_active_citizen / is_citizen / citizen_at) ────────

#[test]
fn is_active_citizen_true_when_registered_and_not_suspended() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

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
        register(1, NULLIFIER_A, ANCHOR_A);
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, Some(10)));

        // Before expiry: suspended.
        assert!(!Identity::is_active_citizen(&1));
        assert!(SuspendedNullifiers::<Test>::get(NULLIFIER_A).is_some());

        // After expiry: active again (still within the reverification deadline of 11), and
        // the stale suspension record is cleaned up as a side effect.
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

        register(1, NULLIFIER_A, ANCHOR_A);
        register(2, NULLIFIER_B, ANCHOR_B);

        assert_eq!(Identity::total_citizens(), 2);
        assert_eq!(Identity::citizen_at(0), Some(1));
        assert_eq!(Identity::citizen_at(1), Some(2));
    });
}
