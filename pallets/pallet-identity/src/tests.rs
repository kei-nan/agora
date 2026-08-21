use crate::{
    committee_slot_for, mock::*, AllowedMerkleRoots, CitizenAnchor, CitizenIndex,
    CitizenNullifier, CitizenPosition, CommitteeMembers, Error, Event, IdentityAnchorRegistry,
    NextQueryId, NullifierRegistry, OprfCommitteeKeys, PendingOprfQueryCountBySubmitter,
    OprfRound1Commitments, OprfRound2Responses, OprfSchemeVersion, PendingOprfQueries,
    ReverificationDeadline, SelfDeclaredSingleDocument, SuspendedByJuryReview,
    SuspendedNullifiers, TotalCitizens,
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
fn register_citizen_fails_when_proof_is_future_dated() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        approve_committee_keys(0);

        let mut future_inputs = public_inputs(NULLIFIER_A, ROOT, ANCHOR_A);
        // MaxAnchorProofClockSkew is 300 in the mock (see mock.rs); this is far beyond that.
        future_inputs[2] = current_date_field(TEST_NOW_UNIX_SECS + 999_999);

        assert_noop!(
            Identity::register_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                future_inputs,
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::AnchorProofFuture
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
fn suspend_citizen_extrinsic_is_never_jury_reviewed() {
    // The manual admin-override extrinsic is SuspensionOrigin-gated (EnsureOracleCouncilApproved
    // in the real runtime, requiring the Oracle Council's M-of-N approval) — no jury is ever
    // involved on this path, so it must never be enough on its own to trigger a higher-bar
    // consequence like pallet-executive's office-vacancy sweep.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, None));

        assert!(!Identity::is_suspended_by_jury_reviewed_conviction(&1));
    });
}

#[test]
fn suspend_citizen_internal_records_jury_reviewed_flag() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::suspend_citizen_internal(NULLIFIER_A, None, true));

        assert!(!Identity::is_active_citizen(&1));
        assert!(Identity::is_suspended_by_jury_reviewed_conviction(&1));
    });
}

#[test]
fn suspend_citizen_internal_without_jury_review_is_suspended_but_not_flagged() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::suspend_citizen_internal(NULLIFIER_A, None, false));

        assert!(!Identity::is_active_citizen(&1));
        assert!(!Identity::is_suspended_by_jury_reviewed_conviction(&1));
    });
}

#[test]
fn is_suspended_by_jury_reviewed_conviction_false_once_timed_suspension_expires() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        assert_ok!(Identity::suspend_citizen_internal(NULLIFIER_A, Some(5), true));
        assert!(Identity::is_suspended_by_jury_reviewed_conviction(&1));

        System::set_block_number(6);

        assert!(!Identity::is_suspended_by_jury_reviewed_conviction(&1));
        assert!(Identity::is_active_citizen(&1));
    });
}

#[test]
fn restore_citizen_rights_clears_jury_reviewed_flag() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        assert_ok!(Identity::suspend_citizen_internal(NULLIFIER_A, None, true));

        assert_ok!(Identity::restore_citizen_rights(RuntimeOrigin::root(), NULLIFIER_A));

        assert!(!Identity::is_suspended_by_jury_reviewed_conviction(&1));
        // Re-suspending later without an explicit jury_reviewed=true must default to
        // not-jury-reviewed, not silently inherit the cleared record's old value.
        assert_ok!(Identity::suspend_citizen(RuntimeOrigin::root(), NULLIFIER_A, None));
        assert!(!Identity::is_suspended_by_jury_reviewed_conviction(&1));
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
fn reverify_citizen_fails_when_proof_is_future_dated() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        let mut future_inputs = public_inputs(NULLIFIER_A, ROOT, ANCHOR_A);
        // MaxAnchorProofClockSkew is 300 in the mock (see mock.rs); this is far beyond that.
        future_inputs[2] = current_date_field(TEST_NOW_UNIX_SECS + 999_999);

        assert_noop!(
            Identity::reverify_citizen(
                RuntimeOrigin::signed(1),
                valid_proof(),
                future_inputs,
                ANCHOR_A,
                OPRF_PK_HASHES,
            ),
            Error::<Test>::AnchorProofFuture
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

/// Ordering regression test: `migrate_oprf_scheme` must verify the ZK proof *before*
/// consulting `IdentityAnchorRegistry`, exactly matching `register_citizen`'s ordering (see
/// its own doc comment on this point). A bogus proof submitted against an already-used
/// `new_anchor` must fail on proof verification (`InvalidZKProof`), not on the anchor-registry
/// check (`NewAnchorAlreadyUsed`) — the latter would let a caller probe anchor-registry
/// membership with zero real proof-computation cost.
#[test]
fn migrate_oprf_scheme_does_not_leak_anchor_registry_membership_via_bogus_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // Same genuinely-taken (1, ANCHOR_B) setup as
        // `migrate_oprf_scheme_fails_when_new_anchor_already_used`.
        assert_ok!(Identity::rotate_oprf_scheme(RuntimeOrigin::root()));
        register(2, NULLIFIER_B, ANCHOR_B);

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

/// Declares a genuine, council-voted emergency in the mock's wired-in
/// `pallet_emergency_council` instance, via its own real supermajority-vote path — the only
/// way `EnsureActiveEmergency` (bound to `EmergencyRotationOrigin` below) can ever succeed.
fn declare_active_emergency() {
    assert_ok!(EmergencyCouncil::add_council_member(RuntimeOrigin::root(), 1));
    assert_ok!(EmergencyCouncil::add_council_member(RuntimeOrigin::root(), 2));
    assert_ok!(EmergencyCouncil::add_council_member(RuntimeOrigin::root(), 3));
    assert_ok!(EmergencyCouncil::vote_declare_emergency(
        RuntimeOrigin::signed(1),
        [7u8; 32],
        50
    ));
    assert_ok!(EmergencyCouncil::vote_declare_emergency(
        RuntimeOrigin::signed(2),
        [7u8; 32],
        50
    ));
    assert!(pallet_emergency_council::ActiveEmergency::<Test>::get().is_some());
}

#[test]
fn emergency_rotate_oprf_scheme_works_when_emergency_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency();
        assert_eq!(OprfSchemeVersion::<Test>::get(), 0);

        assert_ok!(Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::root()));

        assert_eq!(OprfSchemeVersion::<Test>::get(), 1);
        System::assert_last_event(
            Event::OprfSchemeEmergencyRotated { new_version: 1 }.into(),
        );
    });
}

/// The core security property this wiring exists for: `EmergencyRotationOrigin` is no longer
/// a bare `EnsureRoot` — a root call with *no* active, council-declared emergency must now be
/// rejected, where previously (plain `EnsureRoot`) it would have succeeded.
#[test]
fn emergency_rotate_oprf_scheme_fails_for_root_without_active_emergency() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert!(pallet_emergency_council::ActiveEmergency::<Test>::get().is_none());

        assert_noop!(
            Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::root()),
            DispatchError::BadOrigin
        );
        assert_eq!(OprfSchemeVersion::<Test>::get(), 0);
    });
}

#[test]
fn emergency_rotate_oprf_scheme_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Signed origin fails regardless of emergency state.
        assert_noop!(
            Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );

        declare_active_emergency();
        assert_noop!(
            Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::signed(1)),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn emergency_rotate_oprf_scheme_fails_again_after_emergency_lifted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        declare_active_emergency();
        assert_ok!(Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::root()));

        // Lift the emergency via the real supermajority end-vote path.
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(1)));
        assert_ok!(EmergencyCouncil::vote_end_emergency(RuntimeOrigin::signed(2)));
        assert!(pallet_emergency_council::ActiveEmergency::<Test>::get().is_none());

        // A second emergency rotation is refused now that the emergency has ended, even
        // though the first one succeeded moments earlier under the same root key.
        assert_noop!(
            Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::root()),
            DispatchError::BadOrigin
        );
    });
}

// ─── trigger_voluntary_oprf_rotation ────────────────────────────────────────

#[test]
fn trigger_voluntary_oprf_rotation_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_eq!(OprfSchemeVersion::<Test>::get(), 0);
        let reason = [7u8; 32];

        assert_ok!(Identity::trigger_voluntary_oprf_rotation(RuntimeOrigin::root(), reason));

        assert_eq!(OprfSchemeVersion::<Test>::get(), 1);
        System::assert_last_event(
            Event::OprfSchemeVoluntarilyRotated { new_version: 1, reason }.into(),
        );
    });
}

#[test]
fn trigger_voluntary_oprf_rotation_fails_for_unauthorized_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::trigger_voluntary_oprf_rotation(RuntimeOrigin::signed(1), [7u8; 32]),
            DispatchError::BadOrigin
        );
    });
}

#[test]
fn trigger_voluntary_oprf_rotation_is_independent_of_the_other_two_rotation_paths() {
    // Distinctness check: a voluntary rotation advances the same global counter as the
    // scheduled/emergency paths (they share `do_bump_scheme_version`) but emits its own event,
    // and all three can be interleaved without interfering with each other.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_ok!(Identity::rotate_oprf_scheme(RuntimeOrigin::root()));
        assert_eq!(OprfSchemeVersion::<Test>::get(), 1);

        assert_ok!(Identity::trigger_voluntary_oprf_rotation(RuntimeOrigin::root(), [9u8; 32]));
        assert_eq!(OprfSchemeVersion::<Test>::get(), 2);
        System::assert_last_event(
            Event::OprfSchemeVoluntarilyRotated { new_version: 2, reason: [9u8; 32] }.into(),
        );

        // `emergency_rotate_oprf_scheme` is gated by `EmergencyRotationOrigin`
        // (`EnsureActiveEmergency`, wired to pallet-emergency-council's real
        // active-emergency state) — plain root alone is no longer sufficient,
        // see `declare_active_emergency`'s doc comment above.
        declare_active_emergency();
        assert_ok!(Identity::emergency_rotate_oprf_scheme(RuntimeOrigin::root()));
        assert_eq!(OprfSchemeVersion::<Test>::get(), 3);
        System::assert_last_event(Event::OprfSchemeEmergencyRotated { new_version: 3 }.into());
    });
}

#[test]
fn trigger_voluntary_oprf_rotation_fails_on_version_overflow() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        OprfSchemeVersion::<Test>::put(u32::MAX);

        assert_noop!(
            Identity::trigger_voluntary_oprf_rotation(RuntimeOrigin::root(), [1u8; 32]),
            Error::<Test>::OprfSchemeVersionOverflow
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
fn is_active_citizen_lazy_expiry_also_clears_jury_reviewed_flag() {
    // Regression test: is_active_citizen's lazy-expiry branch must clear
    // SuspendedByJuryReview alongside SuspendedNullifiers, not just the latter — otherwise
    // a stale jury-reviewed flag leaks in storage for a nullifier that's no longer
    // suspended at all, once this function (rather than
    // is_suspended_by_jury_reviewed_conviction) is the one that first observes the expiry.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        assert_ok!(Identity::suspend_citizen_internal(NULLIFIER_A, Some(10), true));
        assert!(SuspendedByJuryReview::<Test>::get(NULLIFIER_A));

        // is_active_citizen (not is_suspended_by_jury_reviewed_conviction) is the first call
        // to observe the expiry.
        System::set_block_number(11);
        assert!(Identity::is_active_citizen(&1));

        assert!(SuspendedNullifiers::<Test>::get(NULLIFIER_A).is_none());
        assert!(!SuspendedByJuryReview::<Test>::contains_key(NULLIFIER_A));
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

fn point64(x: [u8; 32], y: [u8; 32]) -> [u8; 64] {
    let mut out = [0u8; 64];
    out[0..32].copy_from_slice(&x);
    out[32..64].copy_from_slice(&y);
    out
}

/// An arbitrary, fixed 64-byte value standing in for a blinded query point. The pallet never
/// validates this is a genuine on-curve point (that's the ZK circuit's job at
/// `register_citizen` time, entirely separate from the mailbox) so any fixed bytes serve the
/// mailbox's own tests just as well as a real KAT vector would.
fn blinded_query() -> [u8; 64] {
    point64([0x11; 32], [0x22; 32])
}

/// A second, distinct fixed query point — used by the query-binding-adjacent tests below to
/// confirm round-1/round-2 state is genuinely keyed per query, not shared.
fn other_query_point() -> [u8; 64] {
    point64([0x33; 32], [0x44; 32])
}

/// Arbitrary but distinguishable round-1 field values for member `seed` — round 1 performs no
/// cryptographic verification (see `OprfRound1Commitment`'s doc comment), so tests only need
/// values that round-trip through storage correctly, not genuinely valid curve points.
fn round1_fields(seed: u8) -> ([u8; 64], [u8; 64], [u8; 64], [u8; 64], [u8; 64]) {
    let p = |b: u8| point64([b; 32], [b.wrapping_add(1); 32]);
    (p(seed), p(seed.wrapping_add(10)), p(seed.wrapping_add(20)), p(seed.wrapping_add(30)), p(seed.wrapping_add(40)))
}

fn submit_round1(who: u64, query_id: u64, slot: u8, seed: u8) -> Result<(), DispatchError> {
    let (r_i, d_g, d_q, e_g, e_q) = round1_fields(seed);
    Identity::submit_oprf_round1(RuntimeOrigin::signed(who), query_id, slot, r_i, d_g, d_q, e_g, e_q)
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

// ─── submit_oprf_round1 / submit_oprf_round2 (Option B, doc 11) ─────────────────────────
//
// Replaces the old single-response-wins `submit_oprf_response` this section used to test
// (changelog entry 82's original mailbox) with the genuine `t`-of-`n` threshold design —
// see `OprfRound1Commitment`'s doc comment in `lib.rs` for why, and
// `oprf-committee-dev/src/threshold.rs` for the full protocol these two calls host the
// on-chain bulletin board for. Neither extrinsic performs cryptographic verification (that
// happens client-side, when a citizen's device combines a locked set's data and the
// resulting ZK proof is checked at `register_citizen`), so these tests only need
// structurally well-formed fixture bytes, not real curve points — see `round1_fields`'s doc
// comment. Mock's `OprfThreshold = 2`, `MaxCommitteeSize = 3`.

/// Registers citizen 1, posts a blinded query as them at the current block, and returns the
/// assigned `query_id` (always 0, since this is the first query in a fresh test ext).
fn setup_pending_query() -> u64 {
    allow_root();
    register(1, NULLIFIER_A, ANCHOR_A);
    assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
    0
}

#[test]
fn submit_oprf_round1_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        let (r_i, d_g, d_q, e_g, e_q) = round1_fields(1);
        assert_ok!(submit_round1(42, query_id, 2, 1));

        let commitments = OprfRound1Commitments::<Test>::get(query_id, 2);
        assert_eq!(commitments.len(), 1);
        assert_eq!(commitments[0].member, 42);
        assert_eq!(commitments[0].r_i, r_i);
        assert_eq!(commitments[0].d_g, d_g);
        assert_eq!(commitments[0].d_q, d_q);
        assert_eq!(commitments[0].e_g, e_g);
        assert_eq!(commitments[0].e_q, e_q);
        System::assert_last_event(
            Event::OprfRound1Submitted { query_id, committee_slot: 2, member: 42 }.into(),
        );
    });
}

#[test]
fn submit_oprf_round1_fails_for_out_of_range_slot() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();

        assert_noop!(submit_round1(42, query_id, 5, 1), Error::<Test>::InvalidCommitteeSlot);
    });
}

#[test]
fn submit_oprf_round1_fails_for_nonexistent_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_noop!(submit_round1(42, 999, 2, 1), Error::<Test>::QueryNotFound);
    });
}

/// Only a registered committee member for the SAME slot may submit round 1 — a member of a
/// different slot must be rejected even though they are a genuine committee member somewhere.
#[test]
fn submit_oprf_round1_fails_for_member_of_a_different_slot() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 3, 42));

        assert_noop!(submit_round1(42, query_id, 2, 1), Error::<Test>::NotCommitteeMember);
    });
}

#[test]
fn submit_oprf_round1_fails_for_non_member() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();

        assert_noop!(submit_round1(99, query_id, 2, 1), Error::<Test>::NotCommitteeMember);
    });
}

#[test]
fn submit_oprf_round1_fails_for_double_submission() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(submit_round1(42, query_id, 2, 1));

        assert_noop!(submit_round1(42, query_id, 2, 2), Error::<Test>::DuplicateResponse);
    });
}

#[test]
fn submit_oprf_round1_fails_for_expired_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        // Mock's OprfQuerySlaBlocks = 10; posted at block 1, so block 11 is the last valid
        // block and block 12 is past the deadline.
        System::set_block_number(12);
        assert_noop!(submit_round1(42, query_id, 2, 1), Error::<Test>::QueryExpired);
    });
}

#[test]
fn submit_oprf_round1_succeeds_at_the_exact_deadline_block() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        // Deadline is posted_at (1) + OprfQuerySlaBlocks (10) = 11, inclusive.
        System::set_block_number(11);
        assert_ok!(submit_round1(42, query_id, 2, 1));
    });
}

/// The core threshold property: the `OprfThreshold`-th (here: 2nd, mock threshold = 2)
/// distinct member's round-1 submission locks the set and emits `OprfRound1SetLocked`; a
/// third genuinely-registered member (roster capacity is 3, strictly above the threshold) is
/// then rejected even though they are a real committee member who simply arrived too late.
#[test]
fn submit_oprf_round1_locks_the_set_at_threshold_and_rejects_further_members() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 43));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 44));

        assert_ok!(submit_round1(42, query_id, 2, 1));
        assert_eq!(OprfRound1Commitments::<Test>::get(query_id, 2).len(), 1);

        assert_ok!(submit_round1(43, query_id, 2, 2));
        assert_eq!(OprfRound1Commitments::<Test>::get(query_id, 2).len(), 2);
        System::assert_has_event(Event::OprfRound1SetLocked { query_id, committee_slot: 2 }.into());

        // A third, genuinely registered member for the same slot is rejected: the set is full.
        assert_noop!(submit_round1(44, query_id, 2, 3), Error::<Test>::OprfRound1SetLocked);
        assert_eq!(OprfRound1Commitments::<Test>::get(query_id, 2).len(), 2);
    });
}

/// Round-1 state is genuinely per-query: a second, distinct query in the same slot starts
/// its own independent qualifying set rather than sharing the first query's.
#[test]
fn submit_oprf_round1_state_is_independent_per_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_a = setup_pending_query();
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), other_query_point()));
        let query_b = query_a + 1;
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));

        assert_ok!(submit_round1(42, query_a, 2, 1));
        assert_ok!(submit_round1(42, query_b, 2, 1));

        assert_eq!(OprfRound1Commitments::<Test>::get(query_a, 2).len(), 1);
        assert_eq!(OprfRound1Commitments::<Test>::get(query_b, 2).len(), 1);
    });
}

// ─── submit_oprf_round2 ──────────────────────────────────────────────────────────────

#[test]
fn submit_oprf_round2_fails_before_round1_locked() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(submit_round1(42, query_id, 2, 1));
        // Threshold is 2; only 1 round-1 submission exists, so the set isn't locked yet.

        assert_noop!(
            Identity::submit_oprf_round2(RuntimeOrigin::signed(42), query_id, 2, [7u8; 32]),
            Error::<Test>::OprfRound1NotLocked
        );
    });
}

#[test]
fn submit_oprf_round2_fails_for_out_of_range_slot() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();

        assert_noop!(
            Identity::submit_oprf_round2(RuntimeOrigin::signed(42), query_id, 5, [7u8; 32]),
            Error::<Test>::InvalidCommitteeSlot
        );
    });
}

/// A committee member who never submitted round 1 for this query cannot skip straight to
/// round 2, even after the real locked set exists.
#[test]
fn submit_oprf_round2_fails_for_non_participant() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 43));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 44));
        assert_ok!(submit_round1(42, query_id, 2, 1));
        assert_ok!(submit_round1(43, query_id, 2, 2));
        // Set is now locked at {42, 43}; 44 never participated in round 1.

        assert_noop!(
            Identity::submit_oprf_round2(RuntimeOrigin::signed(44), query_id, 2, [7u8; 32]),
            Error::<Test>::NotInLockedSet
        );
    });
}

#[test]
fn submit_oprf_round2_works_after_lock() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 43));
        assert_ok!(submit_round1(42, query_id, 2, 1));
        assert_ok!(submit_round1(43, query_id, 2, 2));

        assert_ok!(Identity::submit_oprf_round2(RuntimeOrigin::signed(42), query_id, 2, [7u8; 32]));
        assert_ok!(Identity::submit_oprf_round2(RuntimeOrigin::signed(43), query_id, 2, [8u8; 32]));

        let responses = OprfRound2Responses::<Test>::get(query_id, 2);
        assert_eq!(responses.len(), 2);
        assert!(responses.iter().any(|r| r.member == 42 && r.z_i == [7u8; 32]));
        assert!(responses.iter().any(|r| r.member == 43 && r.z_i == [8u8; 32]));
        System::assert_last_event(
            Event::OprfRound2Submitted { query_id, committee_slot: 2, member: 43 }.into(),
        );
    });
}

#[test]
fn submit_oprf_round2_fails_for_double_submission() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 43));
        assert_ok!(submit_round1(42, query_id, 2, 1));
        assert_ok!(submit_round1(43, query_id, 2, 2));
        assert_ok!(Identity::submit_oprf_round2(RuntimeOrigin::signed(42), query_id, 2, [7u8; 32]));

        assert_noop!(
            Identity::submit_oprf_round2(RuntimeOrigin::signed(42), query_id, 2, [9u8; 32]),
            Error::<Test>::DuplicateResponse
        );
    });
}

// ─── prune_oprf_query ──────────────────────────────────────────────────────────────────
//
// Mock's OprfQuerySlaBlocks = 10, OprfThreshold = 2, MaxPendingOprfQueriesPerCitizen = 3.

/// Drives round-1 and round-2 to completion (`OprfThreshold` = 2 responses each) on every one
/// of the `NUM_COMMITTEES` slots for `query_id`, using fresh committee-member accounts per
/// slot (`100 + slot*10 + {0,1}`) well clear of the citizen accounts (1, 2) used elsewhere in
/// this file. Mirrors what a real query needs before a citizen's off-chain client can combine
/// a genuine anchor (`register_citizen` checks all `NUM_COMMITTEES` `oprf_pk_hashes`) — see
/// `prune_oprf_query`'s "fully answered" condition.
fn fully_answer_all_slots(query_id: u64) {
    for slot in 0..crate::NUM_COMMITTEES {
        let base = 100 + (slot as u64) * 10;
        let (m1, m2) = (base, base + 1);
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), slot, m1));
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), slot, m2));
        assert_ok!(submit_round1(m1, query_id, slot, 1));
        assert_ok!(submit_round1(m2, query_id, slot, 2));
        assert_ok!(Identity::submit_oprf_round2(RuntimeOrigin::signed(m1), query_id, slot, [slot; 32]));
        assert_ok!(Identity::submit_oprf_round2(
            RuntimeOrigin::signed(m2),
            query_id,
            slot,
            [slot.wrapping_add(1); 32]
        ));
    }
}

#[test]
fn prune_oprf_query_fails_for_nonexistent_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Identity::prune_oprf_query(RuntimeOrigin::signed(1), 999),
            Error::<Test>::QueryNotFound
        );
    });
}

#[test]
fn prune_oprf_query_fails_when_neither_expired_nor_fully_answered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();

        // Still within the SLA window (deadline is block 11) and nothing has been answered.
        assert_noop!(
            Identity::prune_oprf_query(RuntimeOrigin::signed(1), query_id),
            Error::<Test>::QueryNotPrunable
        );
    });
}

#[test]
fn prune_oprf_query_removes_an_expired_dead_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 1);

        // Past the deadline (posted_at 1 + OprfQuerySlaBlocks 10 = 11).
        System::set_block_number(12);
        assert_ok!(Identity::prune_oprf_query(RuntimeOrigin::signed(7), query_id));

        assert!(PendingOprfQueries::<Test>::get(query_id).is_none());
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 0);
        System::assert_last_event(Event::OprfQueryPruned { query_id, submitter: 1 }.into());
    });
}

/// An expired query with partial (unlocked) round-1 state is prunable, and pruning clears
/// that partial state too, not just `PendingOprfQueries` itself.
#[test]
fn prune_oprf_query_clears_partial_round1_state_on_an_expired_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        assert_ok!(Identity::add_committee_member(RuntimeOrigin::root(), 2, 42));
        assert_ok!(submit_round1(42, query_id, 2, 1));
        assert_eq!(OprfRound1Commitments::<Test>::get(query_id, 2).len(), 1);

        System::set_block_number(12);
        assert_ok!(Identity::prune_oprf_query(RuntimeOrigin::signed(7), query_id));

        assert_eq!(OprfRound1Commitments::<Test>::get(query_id, 2).len(), 0);
    });
}

/// A query with every one of the `NUM_COMMITTEES` slots fully answered (each at
/// `OprfThreshold` round-2 responses) is prunable even before its SLA deadline passes.
#[test]
fn prune_oprf_query_removes_a_fully_answered_query_before_expiry() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let query_id = setup_pending_query();
        fully_answer_all_slots(query_id);

        // Still well within the SLA window.
        System::set_block_number(5);
        assert_ok!(Identity::prune_oprf_query(RuntimeOrigin::signed(1), query_id));

        assert!(PendingOprfQueries::<Test>::get(query_id).is_none());
        for slot in 0..crate::NUM_COMMITTEES {
            assert_eq!(OprfRound1Commitments::<Test>::get(query_id, slot).len(), 0);
            assert_eq!(OprfRound2Responses::<Test>::get(query_id, slot).len(), 0);
        }
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 0);
    });
}

/// Pruning one citizen's expired query must not touch a different, unrelated citizen's still-
/// live query, nor a different (also live) query from the same citizen.
#[test]
fn prune_oprf_query_does_not_affect_unrelated_citizen_or_query() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);
        register(2, NULLIFIER_B, ANCHOR_B);

        // Citizen 1's query (to be pruned) and citizen 2's query (must survive).
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        let query_a = 0u64;
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(2), other_query_point()));
        let query_b = 1u64;
        // A second, still-live query from citizen 1 too.
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        let query_c = 2u64;

        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 2);
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(2), 1);

        System::set_block_number(12); // past every query's deadline
        assert_ok!(Identity::prune_oprf_query(RuntimeOrigin::signed(1), query_a));

        assert!(PendingOprfQueries::<Test>::get(query_a).is_none());
        assert!(PendingOprfQueries::<Test>::get(query_b).is_some());
        assert!(PendingOprfQueries::<Test>::get(query_c).is_some());
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 1);
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(2), 1);
    });
}

#[test]
fn submit_oprf_query_fails_once_per_citizen_cap_is_reached() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        // Mock's MaxPendingOprfQueriesPerCitizen = 3.
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 3);

        assert_noop!(
            Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()),
            Error::<Test>::TooManyPendingOprfQueries
        );
    });
}

/// A citizen at the cap can submit a fresh query again once pruning frees up headroom —
/// confirms the cap tracks currently-open queries, not a lifetime count.
#[test]
fn submit_oprf_query_succeeds_again_after_pruning_frees_the_cap() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        allow_root();
        register(1, NULLIFIER_A, ANCHOR_A);

        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_noop!(
            Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()),
            Error::<Test>::TooManyPendingOprfQueries
        );

        System::set_block_number(12); // past every query's deadline
        assert_ok!(Identity::prune_oprf_query(RuntimeOrigin::signed(1), 0));
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 2);

        assert_ok!(Identity::submit_oprf_query(RuntimeOrigin::signed(1), blinded_query()));
        assert_eq!(PendingOprfQueryCountBySubmitter::<Test>::get(1), 3);
    });
}

// ── legislature_call_hash (HIGH-severity motion-hijack fix) ────────────────────
//
// See the equivalent block in pallet-constitution's tests for the full rationale. The
// binding invariant itself is proven against the real `EnsureLegislatureMotion` origin in
// pallet-legislature's own suite; here we confirm this pallet's `AdminOrigin`-gated calls
// never hash to the same value for overlapping raw parameters -- in particular that a
// motion approved to add a Merkle root can't double as authorization to set an OPRF
// committee key, or vice versa.
#[test]
fn legislature_call_hash_differs_between_merkle_root_and_committee_key_calls() {
    let merkle_root =
        crate::pallet::legislature_call_hash(b"pallet-identity::add_allowed_merkle_root", NULLIFIER_A);
    let committee_key = crate::pallet::legislature_call_hash(
        b"pallet-identity::set_oprf_committee_key",
        (1u32, 0u8, NULLIFIER_A),
    );
    assert_ne!(merkle_root, committee_key);
}

#[test]
fn legislature_call_hash_differs_between_add_and_remove_merkle_root() {
    let add =
        crate::pallet::legislature_call_hash(b"pallet-identity::add_allowed_merkle_root", NULLIFIER_A);
    let remove =
        crate::pallet::legislature_call_hash(b"pallet-identity::remove_allowed_merkle_root", NULLIFIER_A);
    assert_ne!(add, remove);
}

#[test]
fn legislature_call_hash_differs_for_different_committee_slots() {
    let a = crate::pallet::legislature_call_hash(
        b"pallet-identity::set_oprf_committee_key",
        (1u32, 0u8, NULLIFIER_A),
    );
    let b = crate::pallet::legislature_call_hash(
        b"pallet-identity::set_oprf_committee_key",
        (1u32, 1u8, NULLIFIER_A),
    );
    assert_ne!(a, b);
}
