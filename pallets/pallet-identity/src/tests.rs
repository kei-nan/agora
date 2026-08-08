use crate::{
    committee_slot_for, mock::*, AllowedMerkleRoots, CitizenAnchor, CitizenIndex, CitizenNullifier,
    CitizenPosition, CommitteeMembers, Error, Event, IdentityAnchorRegistry, NextQueryId,
    NullifierRegistry, OprfCommitteeKeys, OprfResponses, OprfSchemeVersion, PendingOprfQueries,
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

/// Field-encodes a unix-seconds timestamp the way the outer circuit's `current_date: pub
/// u64` public input is encoded: the low 8 bytes of a 32-byte big-endian field element.
fn current_date_field(secs: u64) -> [u8; 32] {
    let mut b = [0u8; 32];
    b[24..32].copy_from_slice(&secs.to_be_bytes());
    b
}

/// Builds a well-formed count_4 public_inputs vector (ZKPassport's outer-circuit layout,
/// 9 fields: 8 fixed + 1 disclosure subproof — see `runtime/src/verifier.rs`) with the given
/// nullifier (index 7 = `6 + D`, `scoped_nullifier`), merkle root (index 0,
/// `certificate_registry_root`), a fresh `current_date` (index 2, matching
/// `mock::TEST_NOW_UNIX_SECS`), and `anchor` in the sole `param_commitments` slot (index 5)
/// — `TestAnchorVerifier` (see `mock.rs`) treats registration as valid whenever the outer
/// public inputs contain the submitted anchor there. The other slots are left zeroed.
fn public_inputs(
    nullifier: [u8; 32],
    merkle_root: [u8; 32],
    anchor: [u8; 32],
) -> BoundedVec<[u8; 32], ConstU32<18>> {
    let mut v = vec![[0u8; 32]; 9];
    v[0] = merkle_root;
    v[2] = current_date_field(TEST_NOW_UNIX_SECS);
    v[5] = anchor;
    v[7] = nullifier;
    BoundedVec::try_from(v).unwrap()
}

const ROOT: [u8; 32] = [7u8; 32];
const NULLIFIER_A: [u8; 32] = [1u8; 32];
const NULLIFIER_B: [u8; 32] = [2u8; 32];
const ANCHOR_A: [u8; 32] = [11u8; 32];
const ANCHOR_B: [u8; 32] = [12u8; 32];
const ANCHOR_C: [u8; 32] = [13u8; 32];

/// Fixed test committee-key hashes, one per slot. Arbitrary but distinct, so a test that
/// mutates a single slot is unambiguous about which one it changed.
const OPRF_PK_HASHES: [[u8; 32]; 5] =
    [[101u8; 32], [102u8; 32], [103u8; 32], [104u8; 32], [105u8; 32]];

fn allow_root() {
    assert_ok!(Identity::add_allowed_merkle_root(RuntimeOrigin::root(), ROOT));
}

/// Governance-approves `OPRF_PK_HASHES` for all 5 committee slots under `scheme_version`.
/// Idempotent (upsert), so tests can call it freely.
fn approve_committee_keys(scheme_version: u32) {
    for (slot, hash) in OPRF_PK_HASHES.iter().enumerate() {
        assert_ok!(Identity::set_oprf_committee_key(
            RuntimeOrigin::root(),
            scheme_version,
            slot as u8,
            *hash,
        ));
    }
}

fn register(who: u64, nullifier: [u8; 32], anchor: [u8; 32]) {
    approve_committee_keys(OprfSchemeVersion::<Test>::get());
    assert_ok!(Identity::register_citizen(
        RuntimeOrigin::signed(who),
        valid_proof(),
        public_inputs(nullifier, ROOT, anchor),
        anchor,
        OPRF_PK_HASHES,
    ));
}

/// Builds a public_inputs vector shaped for `migrate_oprf_scheme`: `old_anchor` and
/// `new_anchor` each in their own `param_commitments` slot (indices 5 and 6, as if two
/// disclosure subproofs were folded into the outer proof) — `TestAnchorVerifier`'s
/// `verify_migration` (see `mock.rs`) requires the outer public inputs to contain *both*.
fn migration_public_inputs(
    merkle_root: [u8; 32],
    old_anchor: [u8; 32],
    new_anchor: [u8; 32],
) -> BoundedVec<[u8; 32], ConstU32<18>> {
    let mut v = vec![[0u8; 32]; 10];
    v[0] = merkle_root;
    v[2] = current_date_field(TEST_NOW_UNIX_SECS);
    v[5] = old_anchor;
    v[6] = new_anchor;
    BoundedVec::try_from(v).unwrap()
}

// ─── register_citizen ───────────────────────────────────────────────────────

#[test]
fn register_citizen_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        approve_committee_keys(0);

        assert_ok!(Identity::register_citizen(
            RuntimeOrigin::signed(1),
            valid_proof(),
            public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
            ANCHOR_A,
            OPRF_PK_HASHES,
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
                public_inputs(NULLIFIER_B, ROOT, ANCHOR_B),
                ANCHOR_B,
                OPRF_PK_HASHES,
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
                OPRF_PK_HASHES,
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
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
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
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
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
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_B),
                ANCHOR_B,
                OPRF_PK_HASHES,
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
                public_inputs(NULLIFIER_B, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::AnchorAlreadyUsed
        );
    });
}

#[test]
fn register_citizen_fails_when_committee_key_not_approved() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        // No `approve_committee_keys` call — OprfCommitteeKeys is empty for every slot.

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::CommitteeKeyMismatch
        );
    });
}

#[test]
fn register_citizen_fails_when_a_single_committee_key_is_wrong() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        approve_committee_keys(0);
        let mut wrong_hashes = OPRF_PK_HASHES;
        wrong_hashes[2][0] ^= 1; // slot 2 doesn't match what governance approved.

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                wrong_hashes,
            ),
            Error::<Test>::CommitteeKeyMismatch
        );
    });
}

#[test]
fn register_citizen_fails_when_anchor_verification_fails() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        approve_committee_keys(0);
        // public_inputs carries a *different* anchor in its param_commitments slot than the
        // one submitted — TestAnchorVerifier (mock.rs) only accepts when the outer public
        // inputs contain the submitted anchor.
        let mismatched_inputs = public_inputs(NULLIFIER_A, ROOT, ANCHOR_B);

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                mismatched_inputs,
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::InvalidAnchorProof
        );
    });
}

#[test]
fn register_citizen_fails_when_proof_is_stale() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        approve_committee_keys(0);

        let mut stale_inputs = public_inputs(NULLIFIER_A, ROOT, ANCHOR_A);
        // MaxAnchorProofAge is 3600 in the mock (see mock.rs); this is far older than that.
        stale_inputs[2] = current_date_field(TEST_NOW_UNIX_SECS - 999_999);

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                stale_inputs,
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::AnchorProofStale
        );
    });
}

#[test]
fn register_citizen_fails_with_malformed_proof_date() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        approve_committee_keys(0);

        let mut malformed_inputs = public_inputs(NULLIFIER_A, ROOT, ANCHOR_A);
        // A genuine u64 current_date can never set byte 0 of its 32-byte field encoding —
        // only the low 8 bytes are ever populated.
        malformed_inputs[2][0] = 1;

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                malformed_inputs,
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::MalformedProofDate
        );
    });
}

#[test]
fn register_citizen_fails_on_total_citizens_overflow() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        approve_committee_keys(0);
        TotalCitizens::<Test>::put(u32::MAX);

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
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

// ─── OPRF committee key allowlist ───────────────────────────────────────────

#[test]
fn set_oprf_committee_key_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::set_oprf_committee_key(RuntimeOrigin::root(), 0, 2, [9u8; 32]));

        assert_eq!(OprfCommitteeKeys::<Test>::get((0, 2)), Some([9u8; 32]));
        System::assert_last_event(
            Event::OprfCommitteeKeySet { scheme_version: 0, slot: 2 }.into(),
        );
    });
}

#[test]
fn set_oprf_committee_key_upserts_existing_key() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::set_oprf_committee_key(RuntimeOrigin::root(), 0, 2, [9u8; 32]));
        assert_ok!(Identity::set_oprf_committee_key(RuntimeOrigin::root(), 0, 2, [10u8; 32]));

        assert_eq!(OprfCommitteeKeys::<Test>::get((0, 2)), Some([10u8; 32]));
    });
}

#[test]
fn set_oprf_committee_key_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::set_oprf_committee_key(RuntimeOrigin::signed(1), 0, 2, [9u8; 32]),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn set_oprf_committee_key_fails_for_out_of_range_slot() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::set_oprf_committee_key(RuntimeOrigin::root(), 0, 5, [9u8; 32]),
            Error::<Test>::InvalidCommitteeSlot
        );
    });
}

#[test]
fn remove_oprf_committee_key_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::set_oprf_committee_key(RuntimeOrigin::root(), 0, 2, [9u8; 32]));

        assert_ok!(Identity::remove_oprf_committee_key(RuntimeOrigin::root(), 0, 2));

        assert_eq!(OprfCommitteeKeys::<Test>::get((0, 2)), None);
        System::assert_last_event(
            Event::OprfCommitteeKeyRemoved { scheme_version: 0, slot: 2 }.into(),
        );
    });
}

#[test]
fn remove_oprf_committee_key_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::set_oprf_committee_key(RuntimeOrigin::root(), 0, 2, [9u8; 32]));

        assert_noop!(
            Identity::remove_oprf_committee_key(RuntimeOrigin::signed(1), 0, 2),
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
        assert_ok!(Identity::reverify_citizen(
            RuntimeOrigin::signed(1),
            valid_proof(),
            public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
            ANCHOR_A,
            OPRF_PK_HASHES,
        ));

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
            Identity::reverify_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::NotRegistered
        );
    });
}

#[test]
fn reverify_citizen_fails_when_anchor_does_not_match_on_file() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // ANCHOR_B was never registered for this citizen — checked before any proof work.
        assert_noop!(
            Identity::reverify_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_B),
                ANCHOR_B,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::AnchorMismatch
        );
    });
}

#[test]
fn reverify_citizen_fails_with_invalid_zk_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_noop!(
            Identity::reverify_citizen(
                RuntimeOrigin::signed(1),
                invalid_proof(),
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::InvalidZKProof
        );
    });
}

#[test]
fn reverify_citizen_fails_when_outer_proof_does_not_contain_anchor() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // anchor matches what's on file (passes AnchorMismatch), but the outer proof's own
        // param_commitments slot carries a different value — TestAnchorVerifier's
        // verify_reverification (mock.rs) requires the outer public inputs to contain the
        // claimed anchor.
        let mut mismatched = public_inputs(NULLIFIER_A, ROOT, ANCHOR_A);
        mismatched[5] = ANCHOR_B;

        assert_noop!(
            Identity::reverify_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                mismatched,
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::InvalidReverificationProof
        );
    });
}

#[test]
fn reverify_citizen_fails_when_committee_key_not_approved() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        // Un-approve what `register` approved, so the committee-key check fails.
        for slot in 0..5u8 {
            Identity::remove_oprf_committee_key(RuntimeOrigin::root(), 0, slot).unwrap();
        }

        assert_noop!(
            Identity::reverify_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::CommitteeKeyMismatch
        );
    });
}

#[test]
fn reverify_citizen_fails_when_proof_is_stale() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        let mut stale_inputs = public_inputs(NULLIFIER_A, ROOT, ANCHOR_A);
        stale_inputs[2] = current_date_field(TEST_NOW_UNIX_SECS - 999_999);

        assert_noop!(
            Identity::reverify_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                stale_inputs,
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::AnchorProofStale
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

        assert_ok!(Identity::reverify_citizen(
            RuntimeOrigin::signed(1),
            valid_proof(),
            public_inputs(NULLIFIER_A, ROOT, ANCHOR_A),
            ANCHOR_A,
            OPRF_PK_HASHES,
        ));
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
        approve_committee_keys(1); // new_version's committee keys

        assert_ok!(Identity::migrate_oprf_scheme(
            RuntimeOrigin::signed(1),
            valid_proof(),
            migration_public_inputs(ROOT, ANCHOR_A, ANCHOR_B),
            ANCHOR_B,
            OPRF_PK_HASHES,
            OPRF_PK_HASHES,
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
                valid_proof(),
                migration_public_inputs(ROOT, ANCHOR_A, ANCHOR_B),
                ANCHOR_B,
                OPRF_PK_HASHES,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::NotRegistered
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
        // new version with ANCHOR_B — that's now a genuinely taken (1, ANCHOR_B) entry
        // (this also approves scheme_version 1's committee keys, via `register`).
        assert_ok!(Identity::rotate_oprf_scheme(RuntimeOrigin::root()));
        register(2, NULLIFIER_B, ANCHOR_B);

        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                valid_proof(),
                migration_public_inputs(ROOT, ANCHOR_A, ANCHOR_B),
                ANCHOR_B,
                OPRF_PK_HASHES,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::NewAnchorAlreadyUsed
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_with_invalid_zk_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        approve_committee_keys(1);

        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                invalid_proof(),
                migration_public_inputs(ROOT, ANCHOR_A, ANCHOR_B),
                ANCHOR_B,
                OPRF_PK_HASHES,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::InvalidZKProof
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_with_invalid_migration_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        approve_committee_keys(1);

        // Wipe new_anchor's slot — TestAnchorVerifier's verify_migration (mock.rs) requires
        // the outer public inputs to contain *both* old_anchor and new_anchor.
        let mut mismatched = migration_public_inputs(ROOT, ANCHOR_A, ANCHOR_B);
        mismatched[6] = [0u8; 32];

        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                valid_proof(),
                mismatched,
                ANCHOR_B,
                OPRF_PK_HASHES,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::InvalidMigrationProof
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_when_new_committee_key_not_approved() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        // No approve_committee_keys(1) call — only the old (version 0) keys are approved.

        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                valid_proof(),
                migration_public_inputs(ROOT, ANCHOR_A, ANCHOR_B),
                ANCHOR_B,
                OPRF_PK_HASHES,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::CommitteeKeyMismatch
        );
    });
}

#[test]
fn migrate_oprf_scheme_fails_when_proof_is_stale() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        approve_committee_keys(1);

        let mut stale_inputs = migration_public_inputs(ROOT, ANCHOR_A, ANCHOR_B);
        stale_inputs[2] = current_date_field(TEST_NOW_UNIX_SECS - 999_999);

        assert_noop!(
            Identity::migrate_oprf_scheme(
                RuntimeOrigin::signed(1),
                valid_proof(),
                stale_inputs,
                ANCHOR_B,
                OPRF_PK_HASHES,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::AnchorProofStale
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

// ─── committee_slot_for ─────────────────────────────────────────────────────

#[test]
fn committee_slot_for_is_deterministic() {
    assert_eq!(committee_slot_for(946_684_800), committee_slot_for(946_684_800));
    assert_eq!(committee_slot_for(0), committee_slot_for(0));
}

#[test]
fn committee_slot_for_always_returns_a_slot_in_range() {
    for dob in 0u64..500 {
        assert!(committee_slot_for(dob) < 5, "slot out of range for dob {dob}");
    }
}

#[test]
fn committee_slot_for_covers_the_full_0_to_5_range() {
    // Poseidon2 output is effectively uniform, so 500 distinct dates of birth should hit
    // every one of the 5 slots — this is not a proof of uniformity, just a sanity check that
    // the mod-5 reduction isn't accidentally collapsing onto a subset of slots.
    let mut seen = [false; 5];
    for dob in 0u64..500 {
        seen[committee_slot_for(dob) as usize] = true;
    }
    assert!(seen.iter().all(|&s| s), "not every committee slot was reached: {seen:?}");
}

#[test]
fn committee_slot_for_differs_across_distinct_inputs_generally() {
    // Not every pair need differ, but the whole 0..500 range collapsing onto a single slot
    // would indicate a broken reduction rather than a legitimate hash collision pattern.
    let distinct: std::collections::BTreeSet<u8> =
        (0u64..20).map(committee_slot_for).collect();
    assert!(distinct.len() > 1);
}

// ─── CommitteeMembers (add/remove) ──────────────────────────────────────────

#[test]
fn add_committee_member_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_eq!(CommitteeMembers::<Test>::get(2).into_inner(), vec![42]);
        System::assert_last_event(Event::CommitteeMemberAdded { slot: 2, who: 42 }.into());
    });
}

#[test]
fn add_committee_member_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::add_committee_member(RuntimeOrigin::signed(1), 2, 42),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn add_committee_member_fails_for_out_of_range_slot() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::add_committee_member(RuntimeOrigin::root(), 5, 42),
            Error::<Test>::InvalidCommitteeSlot
        );
    });
}

#[test]
fn add_committee_member_fails_when_already_present() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_noop!(
            Identity::add_committee_member(RuntimeOrigin::root(), 2, 42),
            Error::<Test>::CommitteeMemberAlreadyPresent
        );
    });
}

#[test]
fn add_committee_member_fails_when_roster_full() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Mock's MaxCommitteeSize = 3.
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 1));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 2));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 3));

        assert_noop!(
            Identity::add_committee_member(RuntimeOrigin::root(), 2, 4),
            Error::<Test>::CommitteeRosterFull
        );
    });
}

#[test]
fn remove_committee_member_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_ok!(Identity::remove_committee_member(RuntimeOrigin::root(), 2, 42));

        assert!(CommitteeMembers::<Test>::get(2).is_empty());
        System::assert_last_event(Event::CommitteeMemberRemoved { slot: 2, who: 42 }.into());
    });
}

#[test]
fn remove_committee_member_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_noop!(
            Identity::remove_committee_member(RuntimeOrigin::signed(1), 2, 42),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn remove_committee_member_fails_when_not_present() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::remove_committee_member(RuntimeOrigin::root(), 2, 42),
            Error::<Test>::CommitteeMemberNotPresent
        );
    });
}

// ─── submit_oprf_query ───────────────────────────────────────────────────────

fn blinded_query() -> [u8; 64] {
    [7u8; 64]
}

#[test]
fn submit_oprf_query_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));

        let record = PendingOprfQueries::<Test>::get(0).expect("query 0 should exist");
        assert_eq!(record.submitter, 1);
        assert_eq!(record.blinded_query, blinded_query());
        assert_eq!(record.posted_at, 1);
        assert_eq!(NextQueryId::<Test>::get(), 1);
        System::assert_last_event(
            Event::OprfQuerySubmitted { query_id: 0, submitter: 1 }.into(),
        );
    });
}

#[test]
fn submit_oprf_query_fails_when_not_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()),
            Error::<Test>::NotRegistered
        );
    });
}

#[test]
fn submit_oprf_query_increments_query_id_across_submissions() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));

        assert!(PendingOprfQueries::<Test>::get(0).is_some());
        assert!(PendingOprfQueries::<Test>::get(1).is_some());
        assert_eq!(NextQueryId::<Test>::get(), 2);
    });
}

// ─── submit_oprf_response ───────────────────────────────────────────────────

fn evaluation() -> [u8; 64] {
    [8u8; 64]
}

fn dlog_proof_64() -> BoundedVec<u8, ConstU32<64>> {
    BoundedVec::try_from(vec![9u8; 64]).unwrap()
}

/// Registers citizen 1, posts a blinded query as them at the current block, and returns the
/// assigned `query_id` (always 0, since this is the first query in a fresh test ext).
fn setup_pending_query() -> u64 {
    allow_root();
    register(1, NULLIFIER_A, ANCHOR_A);
    assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
    0
}

#[test]
fn submit_oprf_response_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_ok!(Identity::submit_oprf_response(
            RuntimeOrigin::signed(42),
            query_id,
            2,
            evaluation(),
            dlog_proof_64(),
        ));

        let record = OprfResponses::<Test>::get(query_id, 2).expect("response should exist");
        assert_eq!(record.responder, 42);
        assert_eq!(record.evaluation, evaluation());
        assert_eq!(record.dlog_proof.into_inner(), vec![9u8; 64]);
        System::assert_last_event(
            Event::OprfResponseSubmitted { query_id, committee_slot: 2, responder: 42 }.into(),
        );
    });
}

#[test]
fn submit_oprf_response_fails_for_out_of_range_slot() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();

        assert_noop!(
            Identity::submit_oprf_response(
                RuntimeOrigin::signed(42),
                query_id,
                5,
                evaluation(),
                dlog_proof_64(),
            ),
            Error::<Test>::InvalidCommitteeSlot
        );
    });
}

#[test]
fn submit_oprf_response_fails_for_wrong_dlog_proof_length() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        let short_proof: BoundedVec<u8, ConstU32<64>> =
            BoundedVec::try_from(vec![9u8; 32]).unwrap();
        assert_noop!(
            Identity::submit_oprf_response(
                RuntimeOrigin::signed(42),
                query_id,
                2,
                evaluation(),
                short_proof,
            ),
            Error::<Test>::InvalidDlogProofLength
        );
    });
}

#[test]
fn submit_oprf_response_fails_for_nonexistent_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_noop!(
            Identity::submit_oprf_response(
                RuntimeOrigin::signed(42),
                999,
                2,
                evaluation(),
                dlog_proof_64(),
            ),
            Error::<Test>::QueryNotFound
        );
    });
}

/// Only a registered committee member for the SAME slot may respond — a member of a
/// different slot must be rejected even though they are a genuine committee member somewhere.
#[test]
fn submit_oprf_response_fails_for_member_of_a_different_slot() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 3, 42));

        assert_noop!(
            Identity::submit_oprf_response(
                RuntimeOrigin::signed(42),
                query_id,
                2,
                evaluation(),
                dlog_proof_64(),
            ),
            Error::<Test>::NotCommitteeMember
        );
    });
}

#[test]
fn submit_oprf_response_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();

        assert_noop!(
            Identity::submit_oprf_response(
                RuntimeOrigin::signed(99),
                query_id,
                2,
                evaluation(),
                dlog_proof_64(),
            ),
            Error::<Test>::NotCommitteeMember
        );
    });
}

#[test]
fn submit_oprf_response_fails_for_double_response() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(Identity::submit_oprf_response(
            RuntimeOrigin::signed(42),
            query_id,
            2,
            evaluation(),
            dlog_proof_64(),
        ));

        assert_noop!(
            Identity::submit_oprf_response(
                RuntimeOrigin::signed(42),
                query_id,
                2,
                evaluation(),
                dlog_proof_64(),
            ),
            Error::<Test>::DuplicateResponse
        );
    });
}

#[test]
fn submit_oprf_response_fails_for_expired_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        // Mock's OprfQuerySlaBlocks = 10; posted at block 1, so block 11 is the last valid
        // block and block 12 is past the deadline.
        System::set_block_number(12);
        assert_noop!(
            Identity::submit_oprf_response(
                RuntimeOrigin::signed(42),
                query_id,
                2,
                evaluation(),
                dlog_proof_64(),
            ),
            Error::<Test>::QueryExpired
        );
    });
}

#[test]
fn submit_oprf_response_succeeds_at_the_exact_deadline_block() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        // Deadline is posted_at (1) + OprfQuerySlaBlocks (10) = 11, inclusive.
        System::set_block_number(11);
        assert_ok!(Identity::submit_oprf_response(
            RuntimeOrigin::signed(42),
            query_id,
            2,
            evaluation(),
            dlog_proof_64(),
        ));
    });
}
