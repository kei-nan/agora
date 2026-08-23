use crate::{
    mock::*, BackingCount, BackingThreshold, BackingThresholdCeiling,
    BackingThresholdFloor, DelegatePersonaIdOf, DelegatePersonaUsed,
    DelegateInfo, DelegateStatus, DelegateSweepCursor, Delegates,
    ElectionCycleBlocks, Error, Event,
    LastElectionBlock, LegislatureSeats, MandatoryBreakBlocks, MaxBackingsPerCitizen,
    MaxConsecutiveTerms, TermLengthBlocks, UsedBackingNullifier, WarningWindowPct,
};
use frame_support::{assert_noop, assert_ok, traits::Hooks, traits::ConstU32, BoundedVec};
use sp_runtime::DispatchError;

// ─── helpers ─────────────────────────────────────────────────────────────────
//
// None of these helpers exercise real cryptography -- see mock.rs's own module doc comment.
// They build deterministic fixtures shaped to satisfy the mock verifiers there, so that the
// large majority of this file's tests (delegate lifecycle, term limits, election seating) can
// keep exercising pallet logic without needing to think about proof internals at all.

fn name() -> BoundedVec<u8, ConstU32<64>> {
    BoundedVec::try_from(b"Alice".to_vec()).unwrap()
}

fn ipfs(byte: u8) -> [u8; 32] {
    [byte; 32]
}

/// Deterministic per-delegate `delegate_persona_id`, tagged in byte 0 so it can never collide
/// with `backing_nullifier_for`'s output space.
fn delegate_persona_id_for(delegate: u64) -> [u8; 32] {
    let mut id = [0u8; 32];
    id[0] = 0xDE;
    id[24..32].copy_from_slice(&delegate.to_be_bytes());
    id
}

/// Must match `TestAccountIdToBytes::to_bytes`.
fn persona_account_bytes_for(who: u64) -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[24..32].copy_from_slice(&who.to_be_bytes());
    bytes
}

/// Deterministic per-(backer, delegate) nullifier, tagged in byte 0. Depending on both `who`
/// and `delegate` (rather than `who` alone) lets a single mock "citizen" back several different
/// delegates simultaneously with distinct nullifiers, matching what `MaxBackingsPerCitizen`
/// distinct real slot indices would produce.
fn backing_nullifier_for(who: u64, delegate: u64) -> [u8; 32] {
    let mut n = [0u8; 32];
    n[0] = 0xBA;
    n[8..16].copy_from_slice(&who.to_be_bytes());
    n[24..32].copy_from_slice(&delegate.to_be_bytes());
    n
}

fn max_backings_field() -> [u8; 32] {
    let mut f = [0u8; 32];
    f[28..32].copy_from_slice(&MaxBackingsPerCitizen::<Test>::get().to_be_bytes());
    f
}

fn backing_root() -> [u8; 32] {
    [0xAAu8; 32]
}

/// A `backing-nullifier` proof/public-input fixture for `who` backing `delegate`, shaped to
/// pass `TestBackingProofVerifier`/`TestBackingRootChecker` and match whatever
/// `DelegatePersonaIdOf::get(delegate)` currently holds (so it only actually verifies once
/// `delegate` has been registered via `register_delegate`).
fn backing_proof(who: u64, delegate: u64) -> (BoundedVec<u8, ConstU32<8192>>, [[u8; 32]; 4]) {
    let proof = BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap();
    let inputs = [
        backing_root(),
        delegate_persona_id_for(delegate),
        max_backings_field(),
        backing_nullifier_for(who, delegate),
    ];
    (proof, inputs)
}

fn register_delegate(who: u64) {
    set_active_citizen(who, true);
    // Registration itself doesn't check disclosure currency (only seating does), but every
    // existing seating-focused test in this file assumes a registered delegate is otherwise
    // eligible to be seated, same as it already assumes them an active citizen -- so default
    // registered delegates to a current disclosure here too. Tests that specifically exercise
    // the disclosure gate call `set_current_disclosure(who, false)` afterward to override this.
    set_current_disclosure(who, true);

    let delegate_persona_id = delegate_persona_id_for(who);
    let persona_bytes = persona_account_bytes_for(who);
    let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
        [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
        delegate_persona_id,
        persona_bytes,
        [0u8; 32], [0u8; 32],
    ])
    .unwrap();

    assert_ok!(Elections::register_as_delegate(
        RuntimeOrigin::signed(who),
        who,
        delegate_persona_id,
        BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap(),
        public_inputs,
        1,
        [[0u8; 32]; 5],
        name(),
        ipfs(2),
    ));
}

fn back(who: u64, delegate: u64) {
    set_active_citizen(who, true);
    let (proof, inputs) = backing_proof(who, delegate);
    assert_ok!(Elections::back_delegate(RuntimeOrigin::signed(who), delegate, proof, inputs));
}

fn unback(who: u64, delegate: u64) {
    let (proof, inputs) = backing_proof(who, delegate);
    assert_ok!(Elections::remove_backing(RuntimeOrigin::signed(who), delegate, proof, inputs));
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
        let delegate_persona_id = delegate_persona_id_for(1);
        assert_eq!(DelegatePersonaIdOf::<Test>::get(1), Some(delegate_persona_id));
        assert!(DelegatePersonaUsed::<Test>::contains_key(delegate_persona_id));
        System::assert_last_event(
            Event::DelegateRegistered { delegate: 1, delegate_persona_id, display_name: name() }
                .into(),
        );
    });
}

#[test]
fn register_as_delegate_fails_when_not_active_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let delegate_persona_id = delegate_persona_id_for(1);
        let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            delegate_persona_id,
            persona_account_bytes_for(1),
            [0u8; 32], [0u8; 32],
        ])
        .unwrap();
        assert_noop!(
            Elections::register_as_delegate(
                RuntimeOrigin::signed(1),
                1,
                delegate_persona_id,
                BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap(),
                public_inputs,
                1,
                [[0u8; 32]; 5],
                name(),
                ipfs(2),
            ),
            Error::<Test>::NotActiveCitizen
        );
    });
}

#[test]
fn register_as_delegate_fails_when_already_registered() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        set_active_citizen(1, true);

        let delegate_persona_id = delegate_persona_id_for(1);
        let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            delegate_persona_id,
            persona_account_bytes_for(1),
            [0u8; 32], [0u8; 32],
        ])
        .unwrap();
        assert_noop!(
            Elections::register_as_delegate(
                RuntimeOrigin::signed(1),
                1,
                delegate_persona_id,
                BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap(),
                public_inputs,
                1,
                [[0u8; 32]; 5],
                name(),
                ipfs(2),
            ),
            Error::<Test>::AlreadyRegisteredAsDelegate
        );
    });
}

#[test]
fn register_as_delegate_fails_when_persona_account_does_not_match_signer() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_active_citizen(1, true);
        let delegate_persona_id = delegate_persona_id_for(1);
        let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            delegate_persona_id,
            persona_account_bytes_for(2),
            [0u8; 32], [0u8; 32],
        ])
        .unwrap();
        assert_noop!(
            Elections::register_as_delegate(
                RuntimeOrigin::signed(1),
                2, // persona_account != who
                delegate_persona_id,
                BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap(),
                public_inputs,
                1,
                [[0u8; 32]; 5],
                name(),
                ipfs(2),
            ),
            Error::<Test>::PersonaAccountMismatch
        );
    });
}

#[test]
fn register_as_delegate_fails_when_zk_proof_invalid() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_active_citizen(1, true);
        let delegate_persona_id = delegate_persona_id_for(1);
        let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            delegate_persona_id,
            persona_account_bytes_for(1),
            [0u8; 32], [0u8; 32],
        ])
        .unwrap();
        assert_noop!(
            Elections::register_as_delegate(
                RuntimeOrigin::signed(1),
                1,
                delegate_persona_id,
                BoundedVec::try_from(vec![INVALID_PROOF_MARKER]).unwrap(),
                public_inputs,
                1,
                [[0u8; 32]; 5],
                name(),
                ipfs(2),
            ),
            Error::<Test>::InvalidZKProof
        );
    });
}

/// A "0xFF"-marked committee key hash is deliberately treated as unapproved by
/// `TestCommitteeKeyChecker` -- this is the pallet-level manifestation of the Sybil-resistance
/// guarantee `CommitteeKeyChecker`'s doc comment describes: a proof cryptographically valid
/// against attacker-chosen "committee" keys must still be rejected if those keys were never
/// governance-approved.
#[test]
fn register_as_delegate_fails_when_committee_key_not_approved() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_active_citizen(1, true);
        let delegate_persona_id = delegate_persona_id_for(1);
        let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            delegate_persona_id,
            persona_account_bytes_for(1),
            [0u8; 32], [0u8; 32],
        ])
        .unwrap();
        let mut oprf_pk_hashes = [[0u8; 32]; 5];
        oprf_pk_hashes[0][0] = UNAPPROVED_COMMITTEE_KEY_MARKER;
        assert_noop!(
            Elections::register_as_delegate(
                RuntimeOrigin::signed(1),
                1,
                delegate_persona_id,
                BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap(),
                public_inputs,
                1,
                oprf_pk_hashes,
                name(),
                ipfs(2),
            ),
            Error::<Test>::CommitteeKeyMismatch
        );
    });
}

#[test]
fn register_as_delegate_fails_when_proof_persona_account_mismatch() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_active_citizen(1, true);
        let delegate_persona_id = delegate_persona_id_for(1);
        // public_inputs is missing account 1's own persona bytes -- TestDelegatePersonaVerifier
        // must reject.
        let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            delegate_persona_id,
            persona_account_bytes_for(99), // wrong account's bytes
            [0u8; 32], [0u8; 32],
        ])
        .unwrap();
        assert_noop!(
            Elections::register_as_delegate(
                RuntimeOrigin::signed(1),
                1,
                delegate_persona_id,
                BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap(),
                public_inputs,
                1,
                [[0u8; 32]; 5],
                name(),
                ipfs(2),
            ),
            Error::<Test>::InvalidDelegatePersonaProof
        );
    });
}

/// The whole point of `DelegatePersonaUsed`: a second registration (even under a different
/// `persona_account`/signer) claiming a `delegate_persona_id` that was already consumed must be
/// rejected.
#[test]
fn register_as_delegate_fails_when_delegate_persona_id_already_used() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        let reused_id = delegate_persona_id_for(1);

        set_active_citizen(2, true);
        let public_inputs: BoundedVec<[u8; 32], ConstU32<18>> = BoundedVec::try_from(vec![
            [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32], [0u8; 32],
            reused_id,
            persona_account_bytes_for(2),
            [0u8; 32], [0u8; 32],
        ])
        .unwrap();
        assert_noop!(
            Elections::register_as_delegate(
                RuntimeOrigin::signed(2),
                2,
                reused_id,
                BoundedVec::try_from(vec![VALID_PROOF_MARKER]).unwrap(),
                public_inputs,
                1,
                [[0u8; 32]; 5],
                name(),
                ipfs(2),
            ),
            Error::<Test>::DelegatePersonaAlreadyUsed
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
        System::assert_last_event(
            Event::DelegateBacked { delegate: 1, backing_nullifier: backing_nullifier_for(2, 1) }
                .into(),
        );
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
        let (proof, inputs) = backing_proof(2, 1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1, proof, inputs),
            Error::<Test>::NotActiveCitizen
        );
    });
}

/// `back_delegate` no longer has a `CannotBackSelf` check -- see this pallet's module doc
/// comment for why one would give false assurance under the nullifier-based design (the tx
/// signer is not cryptographically tied to the backing-nullifier's underlying secret, so it
/// cannot actually prevent a delegate from spending one of their own slots on themselves via a
/// cooperating relayer). This test documents the resulting behavior explicitly rather than
/// leaving it untested: a delegate's own account can now submit a `back_delegate` call
/// targeting itself, and it succeeds like any other backing.
#[test]
fn back_delegate_no_longer_rejects_backing_your_own_delegate_account() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        set_active_citizen(1, true);
        let (proof, inputs) = backing_proof(1, 1);

        assert_ok!(Elections::back_delegate(RuntimeOrigin::signed(1), 1, proof, inputs));
        assert_eq!(BackingCount::<Test>::get(1), 1);
    });
}

#[test]
fn back_delegate_fails_when_delegate_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_active_citizen(2, true);
        let (proof, inputs) = backing_proof(2, 1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1, proof, inputs),
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
        let (proof, inputs) = backing_proof(2, 1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1, proof, inputs),
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
        let (proof, inputs) = backing_proof(2, 1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1, proof, inputs),
            Error::<Test>::AlreadyBacking
        );
    });
}

#[test]
fn back_delegate_fails_when_backing_proof_invalid() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        set_active_citizen(2, true);
        let (_, inputs) = backing_proof(2, 1);
        let bad_proof = BoundedVec::try_from(vec![INVALID_PROOF_MARKER]).unwrap();

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1, bad_proof, inputs),
            Error::<Test>::InvalidBackingProof
        );
    });
}

#[test]
fn back_delegate_fails_when_backing_root_invalid() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        set_active_citizen(2, true);
        set_invalid_backing_root(backing_root());
        let (proof, inputs) = backing_proof(2, 1);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1, proof, inputs),
            Error::<Test>::InvalidBackingRoot
        );
    });
}

#[test]
fn back_delegate_fails_when_delegate_persona_id_does_not_match_target() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        register_delegate(2);
        set_active_citizen(3, true);
        // Proof built for delegate 2's persona id, submitted against delegate 1.
        let (proof, mut inputs) = backing_proof(3, 1);
        inputs[1] = delegate_persona_id_for(2);

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(3), 1, proof, inputs),
            Error::<Test>::DelegatePersonaMismatch
        );
    });
}

/// The pallet-level half of the backing-cap enforcement split (see `Error::MaxBackingsMismatch`'s
/// doc comment): a proof whose claimed `max_backings_per_citizen` public input doesn't match the
/// live governance value is rejected outright, regardless of what the (mocked, always-passing)
/// pairing check says. The other half -- that a citizen cannot even construct a valid proof for
/// a `slot_index` beyond the real cap -- is a property of the circuit itself, already covered by
/// `circuits/oprf-identity-anchor/backing-nullifier`'s own `rejects_slot_index_far_beyond_the_cap`
/// test and `runtime/src/backing_nullifier_verifier.rs`'s real-proof suite, not re-provable here
/// without a mock that would defeat the point.
#[test]
fn back_delegate_fails_when_max_backings_public_input_does_not_match_governance_value() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        set_active_citizen(2, true);
        let (proof, mut inputs) = backing_proof(2, 1);
        // Claim a stale cap different from the live MaxBackingsPerCitizen value.
        inputs[2] = {
            let mut f = [0u8; 32];
            f[31] = (MaxBackingsPerCitizen::<Test>::get() + 1) as u8;
            f
        };

        assert_noop!(
            Elections::back_delegate(RuntimeOrigin::signed(2), 1, proof, inputs),
            Error::<Test>::MaxBackingsMismatch
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

        unback(4, 1);

        assert_eq!(BackingCount::<Test>::get(1), 2);
        assert!(!UsedBackingNullifier::<Test>::contains_key(backing_nullifier_for(4, 1)));
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

        unback(5, 1);

        assert_eq!(BackingCount::<Test>::get(1), 3);
        assert_eq!(Delegates::<Test>::get(1).unwrap().status, DelegateStatus::Active);
        System::assert_last_event(
            Event::DelegateBackingRemoved {
                delegate: 1,
                backing_nullifier: backing_nullifier_for(5, 1),
            }
            .into(),
        );
    });
}

#[test]
fn remove_backing_fails_when_not_backing() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        set_active_citizen(2, true);
        let (proof, inputs) = backing_proof(2, 1);

        assert_noop!(
            Elections::remove_backing(RuntimeOrigin::signed(2), 1, proof, inputs),
            Error::<Test>::NotBacking
        );
    });
}

/// Closes the replay-griefing hole `UsedBackingNullifier`'s doc comment describes: an observer
/// who lifts the exact `(zk_proof, public_inputs)` bytes from `back_delegate`'s own public call
/// data cannot resubmit them as a *different* signer to strip someone else's backing.
#[test]
fn remove_backing_fails_when_submitted_by_a_different_account_than_the_original_backer() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        set_active_citizen(3, true);
        let (proof, inputs) = backing_proof(2, 1); // the same proof account 2 used to back

        assert_noop!(
            Elections::remove_backing(RuntimeOrigin::signed(3), 1, proof, inputs),
            Error::<Test>::NotBacking
        );
        // The backing survives the failed attempt.
        assert_eq!(BackingCount::<Test>::get(1), 1);
    });
}

/// `remove_backing` frees the slot for reuse: after removal, the exact same (deterministic)
/// nullifier can back again.
#[test]
fn remove_backing_frees_the_slot_for_reuse() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        assert_eq!(BackingCount::<Test>::get(1), 1);

        unback(2, 1);
        assert_eq!(BackingCount::<Test>::get(1), 0);
        assert!(!UsedBackingNullifier::<Test>::contains_key(backing_nullifier_for(2, 1)));

        back(2, 1);
        assert_eq!(BackingCount::<Test>::get(1), 1);
        assert_eq!(
            UsedBackingNullifier::<Test>::get(backing_nullifier_for(2, 1)),
            Some((2, delegate_persona_id_for(1)))
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

// ─── term-limit evasion via backing-drop cycling (HIGH-severity fix) ────────
//
// `remove_backing` flips an Active delegate to Pending without touching term_start_block or
// consecutive_terms -- it's a transient gap, not a real break. Previously `activate_delegate`
// unconditionally reset term_start_block = now on every reactivation, so a delegate with one
// cooperating backer could cycle remove_backing -> back_delegate shortly before each term
// would complete and silently restart the elapsed-time clock every time: consecutive_terms
// would never reach MaxConsecutiveTerms, and the delegate would never be forced onto a
// mandatory break. The fix: only reset term_start_block on a genuine fresh start (it's None
// only then); a Pending gap with a term already in progress must preserve it.
#[test]
fn backing_drop_cycling_does_not_evade_term_limit() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1); // activates at threshold (3); term_start_block = Some(1).
        assert_eq!(Delegates::<Test>::get(1).unwrap().term_start_block, Some(1));

        // Shortly before the first term would complete (term_length = 100, so it completes
        // at block 101), backer 4 cooperates: drops their backing, then immediately restores
        // it -- the exact two-transaction exploit described in the audit finding.
        System::set_block_number(95);
        unback(4, 1);
        let mid = Delegates::<Test>::get(1).unwrap();
        assert_eq!(mid.status, DelegateStatus::Pending);
        assert_eq!(mid.term_start_block, Some(1), "a transient gap must not touch the clock");
        assert_eq!(mid.consecutive_terms, 0);

        back(4, 1);
        let reactivated = Delegates::<Test>::get(1).unwrap();
        assert_eq!(reactivated.status, DelegateStatus::Active);
        assert_eq!(
            reactivated.term_start_block,
            Some(1),
            "reactivating from a backing-drop gap must NOT reset the term clock \
             (this is the exploit: it used to reset to Some(95) here)"
        );

        // Real elapsed time still crosses the threshold on schedule -- the cycle bought
        // nothing.
        System::set_block_number(101);
        let _ = Elections::on_initialize(101);
        let after_first_term = Delegates::<Test>::get(1).unwrap();
        assert_eq!(after_first_term.consecutive_terms, 1);
        assert_eq!(after_first_term.status, DelegateStatus::Active);
        assert_eq!(after_first_term.term_start_block, Some(101));

        // Repeat the exact same cooperating-backer cycle right before the second term
        // (which would complete at block 201) finishes.
        System::set_block_number(195);
        unback(4, 1);
        back(4, 1);
        let reactivated_2 = Delegates::<Test>::get(1).unwrap();
        assert_eq!(
            reactivated_2.term_start_block,
            Some(101),
            "second cycle must also preserve the original (renewed) term clock"
        );

        // The delegate is still forced onto the mandatory break once real elapsed time
        // crosses the threshold -- exactly as if backing had never dropped.
        System::set_block_number(201);
        let _ = Elections::on_initialize(201);
        let final_info = Delegates::<Test>::get(1).unwrap();
        assert_eq!(final_info.consecutive_terms, 2, "the cap must still be reached");
        assert_eq!(
            final_info.status,
            DelegateStatus::OnBreak,
            "backing-drop cycling must not evade the mandatory break"
        );
        assert_eq!(final_info.break_until_block, Some(201 + DEFAULT_MANDATORY_BREAK_BLOCKS as u64));
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
        unback(2, 1);
        unback(3, 1);

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

#[test]
fn on_initialize_election_skips_delegate_without_current_disclosure_and_falls_through() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // 4 delegates, LegislatureSeats = 3 (default). Backing ranks them 1 > 2 > 3 > 4.
        for d in 1..=4u64 {
            register_delegate(d);
        }
        let backers: [u64; 6] = [5, 6, 7, 8, 9, 10];
        for &b in &backers[0..6] {
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

        // Delegate 1 would win the top seat by backing alone, but their disclosure has lapsed
        // (or was never filed) -- they must be skipped, and delegate 4 (next-highest after the
        // top 3) should fall through into the freed seat instead of the seat simply going unfilled.
        set_current_disclosure(1, false);

        System::set_block_number(DEFAULT_ELECTION_CYCLE_BLOCKS as u64);
        let _ = Elections::on_initialize(System::block_number());

        // 2, 3, 4 seated (3 seats) -- 1 skipped, 4 falls through to fill the freed seat.
        assert_eq!(seat_calls(), vec![vec![2, 3, 4]]);
        System::assert_has_event(Event::SeatingSkippedNoDisclosure { account: 1 }.into());
        // Only the ineligible delegate is skipped -- no spurious skip events for eligible ones.
        assert_eq!(
            System::events()
                .into_iter()
                .filter(|r| matches!(
                    r.event,
                    RuntimeEvent::Elections(Event::SeatingSkippedNoDisclosure { .. })
                ))
                .count(),
            1
        );
    });
}

#[test]
fn on_initialize_election_seats_normally_when_disclosure_current() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        register_delegate(1);
        back(2, 1);
        back(3, 1);
        back(4, 1);

        System::set_block_number(DEFAULT_ELECTION_CYCLE_BLOCKS as u64);
        let _ = Elections::on_initialize(System::block_number());

        assert_eq!(seat_calls(), vec![vec![1]]);
        assert_eq!(
            System::events()
                .into_iter()
                .filter(|r| matches!(
                    r.event,
                    RuntimeEvent::Elections(Event::SeatingSkippedNoDisclosure { .. })
                ))
                .count(),
            0
        );
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
