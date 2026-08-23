use crate::{
    mock::*, ActiveEpoch, BudgetBalance, CategoryVotes, CitizenClaimedEpoch, DelegatedWeight,
    DelegatorCount, Delegations, EpochNumber, EpochTokenAllocation, Error, Event, FiscalYearEpoch,
    NextProposalId, NextReferendumId, PendingFinalization, PetitionReferendum, ProposalResults,
    Proposals, ReferendumHasVoted, ReferendumState, ReferendumTally, ReferendumTier, Referenda,
    VoteCommitments,
};
use frame_support::{assert_noop, assert_ok, traits::Hooks, BoundedVec};

fn hash(byte: u8) -> [u8; 32] {
    [byte; 32]
}

fn proof(valid: bool) -> BoundedVec<u8, frame_support::traits::ConstU32<4096>> {
    let bytes = if valid { VALID_TALLY_PROOF.to_vec() } else { INVALID_TALLY_PROOF.to_vec() };
    BoundedVec::try_from(bytes).unwrap()
}

/// Submits an Ordinary-tier proposal from a dedicated submitter account (999) with the given
/// duration and returns its id. Keeping the submitter separate from the accounts under test
/// avoids incidental interaction with citizen-state setup in the test body.
fn make_proposal(duration: u32) -> u32 {
    activate_citizen(999);
    assert_ok!(Voting::submit_proposal(
        RuntimeOrigin::signed(999),
        hash(9),
        ReferendumTier::Ordinary,
        duration,
    ));
    NextProposalId::<Test>::get() - 1
}

/// Creates a referendum directly via the internal petition-pipeline entry point and returns its
/// id. `petition_id` must be unique per call (it's the dedup key).
fn make_referendum(petition_id: u32, tier: ReferendumTier) -> u32 {
    assert_ok!(Voting::create_referendum_internal(petition_id, hash(7), tier));
    NextReferendumId::<Test>::get() - 1
}

fn last_event_matches(pred: impl Fn(&RuntimeEvent) -> bool) -> bool {
    System::events().iter().any(|r| pred(&r.event))
}

// ── submit_proposal ─────────────────────────────────────────────────────────

#[test]
fn submit_proposal_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_ok!(Voting::submit_proposal(
            RuntimeOrigin::signed(1),
            hash(1),
            ReferendumTier::Ordinary,
            MIN_PROPOSAL_DURATION,
        ));
        let (ends_at, topic_hash, tier) = Proposals::<Test>::get(0).unwrap();
        assert_eq!(ends_at, 1 + MIN_PROPOSAL_DURATION as u64);
        assert_eq!(topic_hash, hash(1));
        assert_eq!(tier, ReferendumTier::Ordinary);
        assert_eq!(NextProposalId::<Test>::get(), 1);
        System::assert_last_event(
            Event::ProposalCreated { id: 0, ends_at, topic_hash: hash(1), tier: ReferendumTier::Ordinary }
                .into(),
        );
    });
}

#[test]
fn submit_proposal_fails_for_unregistered_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::submit_proposal(
                RuntimeOrigin::signed(42),
                hash(1),
                ReferendumTier::Ordinary,
                MIN_PROPOSAL_DURATION,
            ),
            Error::<Test>::CitizenNotActive
        );
    });
}

#[test]
fn submit_proposal_fails_for_suspended_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        set_active_citizen(1, false); // simulate a court-ordered suspension
        assert_noop!(
            Voting::submit_proposal(
                RuntimeOrigin::signed(1),
                hash(1),
                ReferendumTier::Ordinary,
                MIN_PROPOSAL_DURATION,
            ),
            Error::<Test>::CitizenNotActive
        );
    });
}

#[test]
fn submit_proposal_fails_duration_too_short() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_noop!(
            Voting::submit_proposal(
                RuntimeOrigin::signed(1),
                hash(1),
                ReferendumTier::Ordinary,
                MIN_PROPOSAL_DURATION - 1,
            ),
            Error::<Test>::InvalidProposalDuration
        );
    });
}

#[test]
fn submit_proposal_fails_duration_too_long() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_noop!(
            Voting::submit_proposal(
                RuntimeOrigin::signed(1),
                hash(1),
                ReferendumTier::Ordinary,
                MAX_PROPOSAL_DURATION + 1,
            ),
            Error::<Test>::InvalidProposalDuration
        );
    });
}

// ── commit_vote ──────────────────────────────────────────────────────────────

#[test]
fn commit_vote_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        activate_citizen(1);
        assert_ok!(Voting::commit_vote(RuntimeOrigin::signed(1), pid, hash(5)));
        let nullifier = nullifier_of(1).unwrap();
        assert_eq!(VoteCommitments::<Test>::get((pid, nullifier)), Some(hash(5)));
        System::assert_last_event(Event::VoteCommitted { proposal_id: pid, nullifier }.into());
    });
}

#[test]
fn commit_vote_fails_for_inactive_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        assert_noop!(
            Voting::commit_vote(RuntimeOrigin::signed(1), pid, hash(5)),
            Error::<Test>::CitizenNotActive
        );
    });
}

#[test]
fn commit_vote_fails_for_citizen_without_nullifier() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        // Active citizen, but pallet-identity never issued them a nullifier.
        set_active_citizen(1, true);
        assert_noop!(
            Voting::commit_vote(RuntimeOrigin::signed(1), pid, hash(5)),
            Error::<Test>::NotRegisteredCitizen
        );
    });
}

#[test]
fn commit_vote_fails_proposal_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_noop!(
            Voting::commit_vote(RuntimeOrigin::signed(1), 999, hash(5)),
            Error::<Test>::ProposalNotFound
        );
    });
}

#[test]
fn commit_vote_fails_after_proposal_ends() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION); // ends_at = 1 + 5 = 6
        activate_citizen(1);
        System::set_block_number(6); // block == ends_at: strictly-less check must fail
        assert_noop!(
            Voting::commit_vote(RuntimeOrigin::signed(1), pid, hash(5)),
            Error::<Test>::ProposalEnded
        );
    });
}

#[test]
fn commit_vote_fails_already_voted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        activate_citizen(1);
        assert_ok!(Voting::commit_vote(RuntimeOrigin::signed(1), pid, hash(5)));
        assert_noop!(
            Voting::commit_vote(RuntimeOrigin::signed(1), pid, hash(6)),
            Error::<Test>::AlreadyVoted
        );
    });
}

// ── delegate_vote ────────────────────────────────────────────────────────────

#[test]
fn delegate_vote_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        let record = Delegations::<Test>::get(0u32, 1u64).unwrap();
        assert_eq!(record.delegate, 2);
        assert_eq!(record.expires_at, 1 + MIN_DELEGATION_DURATION as u64);
        assert_eq!(DelegatorCount::<Test>::get((0u32, 2u64)), 1);
        System::assert_last_event(
            Event::DelegationSet { delegator: 1, delegate: 2, topic_id: 0, expires_at: record.expires_at }
                .into(),
        );
    });
}

#[test]
fn delegate_vote_fails_for_inactive_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(2);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::CitizenNotActive
        );
    });
}

#[test]
fn delegate_vote_fails_duration_too_short() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        activate_citizen(2);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION - 1),
            Error::<Test>::InvalidDelegationDuration
        );
    });
}

#[test]
fn delegate_vote_fails_duration_too_long() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        activate_citizen(2);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION + 1),
            Error::<Test>::InvalidDelegationDuration
        );
    });
}

#[test]
fn delegate_vote_fails_self_delegation() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(1), 1, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCycleDetected
        );
    });
}

#[test]
fn delegate_vote_replacing_same_delegate_keeps_count_unchanged() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatorCount::<Test>::get((0u32, 2u64)), 1);
        // Re-delegate to the SAME delegate with a different duration: net count must not move.
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION + 10));
        assert_eq!(DelegatorCount::<Test>::get((0u32, 2u64)), 1);
        let record = Delegations::<Test>::get(0u32, 1u64).unwrap();
        assert_eq!(record.expires_at, 1 + (MIN_DELEGATION_DURATION + 10) as u64);
    });
}

#[test]
fn delegate_vote_replacing_different_delegate_updates_counts() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatorCount::<Test>::get((0u32, 2u64)), 1);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 3, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatorCount::<Test>::get((0u32, 2u64)), 0);
        assert_eq!(DelegatorCount::<Test>::get((0u32, 3u64)), 1);
        assert_eq!(Delegations::<Test>::get(0u32, 1u64).unwrap().delegate, 3);
    });
}

#[test]
fn delegate_vote_fails_max_delegations_per_delegate_cap() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Total citizens left at 0 so the percentage cap is skipped entirely — isolates the
        // absolute MaxDelegationsPerDelegate cap (5 in the mock).
        activate_citizen(100);
        for delegator in 1..=MAX_DELEGATIONS_PER_DELEGATE {
            activate_citizen(delegator as u64);
            assert_ok!(Voting::delegate_vote(
                RuntimeOrigin::signed(delegator as u64),
                100,
                0,
                MIN_DELEGATION_DURATION
            ));
        }
        assert_eq!(DelegatorCount::<Test>::get((0u32, 100u64)), MAX_DELEGATIONS_PER_DELEGATE);
        let one_too_many = (MAX_DELEGATIONS_PER_DELEGATE + 1) as u64;
        activate_citizen(one_too_many);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(one_too_many), 100, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCapExceeded
        );
    });
}

#[test]
fn delegate_vote_fails_percentage_cap() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // DelegationCap = 40%, total_citizens = 10 -> at most 4 delegators before the
        // percentage cap binds (tighter than the absolute cap of 5 in this scenario).
        set_total_citizens(10);
        activate_citizen(100);
        for delegator in 1..=4u64 {
            activate_citizen(delegator);
            assert_ok!(Voting::delegate_vote(
                RuntimeOrigin::signed(delegator),
                100,
                0,
                MIN_DELEGATION_DURATION
            ));
        }
        assert_eq!(DelegatorCount::<Test>::get((0u32, 100u64)), 4);
        activate_citizen(5);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(5), 100, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCapExceeded
        );
    });
}

/// Reproduces the exact bug this fix closes: `DelegationCap` bounds *transitively resolved*
/// weight, not just direct fan-in. Mirrors the chain built in
/// `finalize_referendum_resolves_transitive_delegation_chain` (1 -> 2 -> 3) but with
/// `total_citizens` set so the fully-resolved chain weight (2, once citizen 2 forwards their
/// own vote plus citizen 1's) exceeds the 40% cap of a 4-citizen electorate (cap allows at
/// most weight 1 there). The OLD check only looked at citizen 3's direct fan-in from citizen
/// 2 alone (count 1, comfortably under any reasonable cap) and would have let this through,
/// leaving citizen 3 holding 2 of 4 votes (50%) once the chain resolved at finalize time.
#[test]
fn delegate_vote_fails_percentage_cap_on_second_hop_of_transitive_chain() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(4);
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 2u64)), 1);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(2), 3, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCapExceeded
        );
        // Rejected atomically: citizen 2's own delegation state is untouched.
        assert!(Delegations::<Test>::get(0u32, 2u64).is_none());
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 2u64)), 1);
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 3u64)), 0);
    });
}

/// The scenario described in the bug report: several *independently* under-the-cap chains
/// funnel into a shared final delegate, whose combined transitively-resolved weight then
/// exceeds the cap even though no single hop's direct fan-in ever looked large. DelegationCap
/// = 40%, total_citizens = 10 -> cap allows at most weight 4 per delegate. Three branches,
/// each a leaf citizen delegating to their own hub (hub's own resolved weight once it
/// forwards: itself + the one leaf = 2, well under the cap on its own), all then delegate to
/// a shared `Final`. The OLD check only saw Final's direct fan-in (the 3 hubs, count 3) against
/// the cap's count-equivalent and would have let all three through, leaving Final holding
/// 6 of 10 votes (60%) once every chain resolved — well past the 40% cap.
#[test]
fn delegate_vote_fails_percentage_cap_via_combined_transitive_chains() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10);
        for (leaf, hub) in [(201u64, 202u64), (203, 204), (205, 206)] {
            activate_citizen(leaf);
            activate_citizen(hub);
            assert_ok!(Voting::delegate_vote(
                RuntimeOrigin::signed(leaf),
                hub,
                0,
                MIN_DELEGATION_DURATION
            ));
        }
        activate_citizen(300); // shared final delegate

        // First two hubs funnel into Final: 2 + 2 = 4, exactly at the cap.
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(202), 300, 0, MIN_DELEGATION_DURATION));
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(204), 300, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 4);

        // The third hub would push Final to 6/10 = 60%, well past the 40% cap.
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(206), 300, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCapExceeded
        );
        assert!(Delegations::<Test>::get(0u32, 206u64).is_none());
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 4);

        // A brand new citizen delegating straight to Final is equally rejected: Final is
        // already at the cap via the transitive chains alone.
        activate_citizen(400);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(400), 300, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCapExceeded
        );
    });
}

/// A legitimate transitive chain that stays under the cap must still work — the fix must not
/// over-reject. Companion to `finalize_referendum_resolves_transitive_delegation_chain`, which
/// leaves `total_citizens` at 0 (cap skipped entirely); this variant turns the cap on and
/// confirms the same chain still succeeds and still tallies correctly at finalize time.
#[test]
fn delegate_vote_transitive_chain_under_cap_succeeds() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10); // cap allows up to weight 4
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(2), 3, 0, MIN_DELEGATION_DURATION));
        // `DelegatedWeight` counts weight delegated *to* the terminal (citizens 1 and 2's own
        // votes), not the terminal's own vote — mirrors DelegatorCount's pre-existing
        // definition (a delegate's own vote was never counted toward its own cap either).
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 3u64)), 2);
    });
}

/// Revoking a delegation must move its weight back onto the revoking citizen, not just drop
/// it — otherwise repeated delegate/revoke cycles would leak weight out of `DelegatedWeight`
/// and eventually make the cap too permissive (weight vanishing) or impossible to satisfy
/// (weight stuck at a stale terminal forever).
#[test]
fn revoke_delegation_restores_weight_to_the_revoking_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10);
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 2u64)), 1);
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 1u64)), 0);
        assert_ok!(Voting::revoke_delegation(RuntimeOrigin::signed(1), 0));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 2u64)), 0);
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 1u64)), 1);
    });
}

/// Re-delegating to a different delegate must move the weight, not duplicate or drop it.
#[test]
fn delegate_vote_replacing_different_delegate_moves_weight() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10);
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 2u64)), 1);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 3, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 2u64)), 0);
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 3u64)), 1);
    });
}

/// Reproduces the "hub re-targets to a different delegate" cap-bypass this fix closes. Many
/// citizens delegate to hub B (building real transitive weight at B: 3 leaves + B's own vote =
/// 4), B forwards that full weight on to C via its own first outgoing edge (correct under both
/// old and new code — it's B's first delegation, so `DelegatedWeight[B]` still accurately
/// reflected its real weight at that moment). B then re-targets away from C to a fresh delegate
/// D that already carries weight 1 from an unrelated delegator.
///
/// The OLD code recomputed `who_weight` for B's re-targeted edge as `1 + DelegatedWeight[B]`,
/// which is always `1 + 0 = 1` per `DelegatedWeight`'s documented invariant (B has had an active
/// outgoing delegation the whole time, so its own bucket never accumulates) — making D's
/// projected total look like `1 + 1 = 2` (comfortably under the 40%-of-10 = 4 cap) when B's real
/// weight, exactly what's about to land on D, is 4, for a real total of 5 — over the cap. The fix
/// uses `old_record.resolved_weight` (4, already snapshotted from the B -> C edge, and exactly
/// what the old-edge unwind below it already uses) instead, correctly rejecting the re-target
/// and leaving every count untouched.
#[test]
fn delegate_vote_retargeting_hub_uses_real_transitive_weight_for_cap_check() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10); // cap allows at most weight 4 (40%)

        // Build real weight at hub B (200): three leaves delegate to it.
        activate_citizen(200); // B, the hub
        for leaf in [1u64, 2, 3] {
            activate_citizen(leaf);
            assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(leaf), 200, 0, MIN_DELEGATION_DURATION));
        }
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 200u64)), 3);

        // B forwards its full weight (3 leaves + its own vote = 4) to C (300) — B's first
        // outgoing edge on this topic, correctly handled under both old and new code.
        activate_citizen(300); // C
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(200), 300, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 4);
        assert_eq!(Delegations::<Test>::get(0u32, 200u64).unwrap().resolved_weight, 4);

        // D (400) already carries weight 1 from an unrelated delegator.
        activate_citizen(400); // D
        activate_citizen(500);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(500), 400, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 400u64)), 1);

        // B re-targets from C to D. Real projected total at D is B's real weight (4) + D's
        // existing weight (1) = 5, i.e. 50% of 10 citizens — over the 40% cap. Must reject.
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(200), 400, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCapExceeded
        );

        // Rejected atomically: B's old edge to C, and both C's and D's tracked weight, are
        // untouched — none of B's real weight silently vanished from cap-tracking either.
        assert_eq!(Delegations::<Test>::get(0u32, 200u64).unwrap().delegate, 300);
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 4);
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 400u64)), 1);
    });
}

/// Direct positive-path counterpart to
/// `delegate_vote_retargeting_hub_uses_real_transitive_weight_for_cap_check` above (which only
/// covers the over-cap *rejection* path): a successful hub re-target must move exactly the
/// delegation's snapshotted `resolved_weight` off the old hub and onto the new one — no more,
/// no less — leaving each hub's unrelated, pre-existing weight untouched.
///
/// Both B and C already carry baseline weight from an unrelated delegator each, so this checks
/// genuine hub accounting (additive increment/decrement against a nonzero base), not merely
/// that a value went from 0 to 1.
#[test]
fn delegate_vote_retargeting_hub_successfully_moves_weight_between_hubs() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10); // cap allows at most weight 4 (40%)

        // Hub B (200) starts with baseline weight 1 from an unrelated delegator (101).
        activate_citizen(200); // B
        activate_citizen(101);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(101), 200, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 200u64)), 1);

        // A (1) delegates to hub B: B's weight increases by exactly A's own weight (1).
        activate_citizen(1); // A
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 200, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 200u64)), 2);
        assert_eq!(Delegations::<Test>::get(0u32, 1u64).unwrap().resolved_weight, 1);

        // Hub C (300) also already carries baseline weight 1 from a different unrelated
        // delegator (102).
        activate_citizen(300); // C
        activate_citizen(102);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(102), 300, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 1);

        // A re-targets from B to C (projected total at C: 1 + 1 = 2, comfortably under the
        // cap of 4).
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 300, 0, MIN_DELEGATION_DURATION));

        // B lost exactly A's weight (1): 2 -> 1, leaving its unrelated baseline (from 101)
        // intact. C gained exactly that same amount: 1 -> 2, on top of its own unrelated
        // baseline (from 102).
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 200u64)), 1);
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 2);
        assert_eq!(Delegations::<Test>::get(0u32, 1u64).unwrap().delegate, 300);
    });
}

/// Regression test for the "hub delegates out BEFORE upstream leaves join" ordering — the
/// mirror image of `delegate_vote_retargeting_hub_uses_real_transitive_weight_for_cap_check`
/// above (which covers leaves joining B *before* B delegates onward). Here the order is
/// reversed:
///   1. B delegates to hub C first (B's own first outgoing edge — nothing upstream of B yet,
///      so its own snapshot is correctly 1 at this point under both old and new code).
///   2. A then delegates to B. B already has an outgoing edge, so A's weight resolves
///      transitively straight through to C: `DelegatedWeight[C]` becomes 2 (A + B). *Before
///      this fix*, B's own stored `resolved_weight` (captured back in step 1, when B's only
///      real weight was its own vote) was never updated to reflect that it's now forwarding
///      2, not 1 — a stale snapshot that only `DelegatedWeight[C]` (not B's own record) knew
///      about.
///   3. B re-targets from C to a fresh delegate D — an ordinary, unprivileged `delegate_vote`
///      call touching only B's own edge. *Before this fix*, the code trusted B's stale
///      snapshot (1) to move off C and onto D, leaving a phantom weight-1 stuck at C (nothing
///      resolves there any more) and permanently undercounting D by A's real weight.
///
/// This fix keeps B's own snapshot live via `propagate_weight_along_chain`, so step 2 updates
/// it to 2, and step 3 correctly moves the full 2 off C and onto D.
#[test]
fn delegate_vote_retargeting_hub_after_upstream_join_moves_full_transitive_weight() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10); // cap allows at most weight 4 (40%)

        // Step 1: B (200) delegates to C (300) first — B's own first outgoing edge, nothing
        // upstream of B yet, so this is correctly snapshotted as weight 1 under both old and
        // new code.
        activate_citizen(200); // B
        activate_citizen(300); // C
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(200), 300, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 1);
        assert_eq!(Delegations::<Test>::get(0u32, 200u64).unwrap().resolved_weight, 1);

        // Step 2: A (100) delegates to B — B already has an outgoing edge to C, so A's weight
        // resolves transitively straight through: DelegatedWeight[C] becomes 2.
        activate_citizen(100); // A
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(100), 200, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 2);
        // The fix: B's own stored snapshot must now reflect the full transitive weight (2),
        // not the stale value (1) captured when B's edge to C was first created.
        assert_eq!(Delegations::<Test>::get(0u32, 200u64).unwrap().resolved_weight, 2);
        // B itself still carries 0 in DelegatedWeight — it has an active outgoing edge, so its
        // incoming weight is (correctly) re-attributed to C, not left sitting at B.
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 200u64)), 0);

        // Step 3: B re-targets from C to a fresh delegate D (400) — an ordinary,
        // unprivileged call touching only B's own edge.
        activate_citizen(400); // D
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(200), 400, 0, MIN_DELEGATION_DURATION));

        // C must drop back to 0 — nothing resolves there any more (the bug left a phantom 1
        // permanently stuck at C instead).
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 300u64)), 0);
        // D must reflect the FULL transitive weight of 2 (A + B), not the stale 1 the bug
        // would have moved.
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 400u64)), 2);
        assert_eq!(Delegations::<Test>::get(0u32, 200u64).unwrap().resolved_weight, 2);
        assert_eq!(Delegations::<Test>::get(0u32, 200u64).unwrap().delegate, 400);
    });
}

/// Companion to the test above: proves the staleness bug wasn't merely a bookkeeping
/// curiosity but a genuine `DelegationCap` bypass. Continues the same B-then-A-then-retarget
/// scenario, then adds three more direct delegators onto D one at a time. With the fix, D's
/// real weight (2 from A+B, correctly tracked after the re-target) plus two more direct
/// delegators reaches exactly the cap (4); a third is correctly rejected, since it would push
/// D's real weight to 5 — over the 40%-of-10 = 4 cap.
///
/// *Before* this fix, D's tracked weight after the re-target would have been the stale 1 (not
/// the real 2). The same three additional delegators would then each individually pass their
/// own cap check against that understated base — 1+1=2, +1=3, +1=4, "under" the cap the whole
/// way — silently letting D end up with real weight 5 (2 real + 3 more), 50% of the
/// electorate, despite every single `delegate_vote` call along the way passing its own
/// `DelegationCapExceeded` check.
#[test]
fn delegate_vote_stale_hub_snapshot_would_have_let_delegation_cap_be_bypassed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10); // cap allows at most weight 4 (40%)

        activate_citizen(200); // B
        activate_citizen(300); // C
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(200), 300, 0, MIN_DELEGATION_DURATION));

        activate_citizen(100); // A
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(100), 200, 0, MIN_DELEGATION_DURATION));

        activate_citizen(400); // D
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(200), 400, 0, MIN_DELEGATION_DURATION));
        // D correctly starts at the real transitive weight of 2 (A + B), not the stale 1 the
        // pre-fix bug would have shown here.
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 400u64)), 2);

        // Two more direct delegators bring D to exactly the cap (2 + 1 + 1 = 4).
        activate_citizen(501);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(501), 400, 0, MIN_DELEGATION_DURATION));
        activate_citizen(502);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(502), 400, 0, MIN_DELEGATION_DURATION));
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 400u64)), 4);

        // A third would push D's real weight to 5 (50% of 10) — over the cap. Correctly
        // rejected by the fix.
        activate_citizen(503);
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(503), 400, 0, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCapExceeded
        );
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 400u64)), 4);
    });
}

/// Lazy expiry cleanup (triggered from within `has_delegation_cycle` when walking a
/// prospective new chain) must restore weight to the account whose expired delegation was
/// removed, exactly like an explicit `revoke_delegation` does — otherwise an expired link
/// would leave its weight stranded at a terminal it no longer actually resolves to.
#[test]
fn delegate_vote_lazy_expiry_cleanup_restores_weight() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        set_total_citizens(10);
        activate_citizen(40); // A
        activate_citizen(41); // B
        activate_citizen(42); // C
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(40), 41, 9, MIN_DELEGATION_DURATION)); // A -> B, expires at 6
        assert_eq!(DelegatedWeight::<Test>::get((9u32, 41u64)), 1);

        System::set_block_number(10); // past expiry (6)
        // C delegates to A. Walking the chain from A finds A's own (expired) record and cleans
        // it up, which must also move the weight it carried back onto A.
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(42), 40, 9, MIN_DELEGATION_DURATION));

        assert_eq!(DelegatedWeight::<Test>::get((9u32, 41u64)), 0);
        // A is a terminal again (own weight 1) plus C's new delegation to A (weight 1) = 2.
        assert_eq!(DelegatedWeight::<Test>::get((9u32, 40u64)), 2);
    });
}

#[test]
fn delegate_vote_fails_cycle_via_transitive_chain() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(10); // A
        activate_citizen(11); // B
        activate_citizen(12); // C
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(10), 11, 7, MIN_DELEGATION_DURATION)); // A -> B
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(11), 12, 7, MIN_DELEGATION_DURATION)); // B -> C
        // C -> A would close the loop A -> B -> C -> A.
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(12), 10, 7, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCycleDetected
        );
    });
}

#[test]
fn delegate_vote_conservative_depth_limit_blocks_non_cycle() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // MaxDelegationDepth = 3. Build a straight (non-cyclic) chain of exactly depth-limit
        // length: X0 -> X1 -> X2 -> X3. Walking from X0 to find `who` takes exactly 3 hops
        // without ever encountering `who` or a None — the pallet conservatively treats this as
        // a cycle to bound on-chain computation, per its own doc comment on has_delegation_cycle.
        activate_citizen(20); // X0
        activate_citizen(21); // X1
        activate_citizen(22); // X2
        activate_citizen(23); // X3
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(20), 21, 3, MIN_DELEGATION_DURATION));
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(21), 22, 3, MIN_DELEGATION_DURATION));
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(22), 23, 3, MIN_DELEGATION_DURATION));

        activate_citizen(30); // unrelated citizen
        assert_noop!(
            Voting::delegate_vote(RuntimeOrigin::signed(30), 20, 3, MIN_DELEGATION_DURATION),
            Error::<Test>::DelegationCycleDetected
        );
    });
}

#[test]
fn delegate_vote_lazily_cleans_up_expired_delegation_in_chain() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(40); // A
        activate_citizen(41); // B
        activate_citizen(42); // C
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(40), 41, 9, MIN_DELEGATION_DURATION)); // A -> B, expires at 6
        assert_eq!(DelegatorCount::<Test>::get((9u32, 41u64)), 1);

        System::set_block_number(10); // past expiry (6)
        // C delegates to A. Walking the chain from A finds A's own (expired) record and cleans
        // it up lazily instead of treating it as a live cycle edge.
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(42), 40, 9, MIN_DELEGATION_DURATION));

        assert!(Delegations::<Test>::get(9u32, 40u64).is_none());
        assert_eq!(DelegatorCount::<Test>::get((9u32, 41u64)), 0);
        assert!(last_event_matches(
            |e| matches!(e, RuntimeEvent::Voting(Event::DelegationExpired { delegator: 40, topic_id: 9 }))
        ));
        assert_eq!(Delegations::<Test>::get(9u32, 42u64).unwrap().delegate, 40);
    });
}

// ── revoke_delegation ────────────────────────────────────────────────────────

#[test]
fn revoke_delegation_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        assert_ok!(Voting::revoke_delegation(RuntimeOrigin::signed(1), 0));
        assert!(Delegations::<Test>::get(0u32, 1u64).is_none());
        assert_eq!(DelegatorCount::<Test>::get((0u32, 2u64)), 0);
        System::assert_last_event(Event::DelegationRevoked { delegator: 1, topic_id: 0 }.into());
    });
}

#[test]
fn revoke_delegation_fails_when_none_exists() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_noop!(
            Voting::revoke_delegation(RuntimeOrigin::signed(1), 0),
            Error::<Test>::NoDelegationOnTopic
        );
    });
}

/// A citizen who delegated while active and was later suspended can no longer revoke the
/// delegation until reinstated — consistent with every other citizen-facing call in this
/// pallet. This doesn't strand the delegation forever: `expires_at` (bounded by
/// `MaxDelegationDurationBlocks`) lapses it on its own regardless.
#[test]
fn revoke_delegation_fails_for_suspended_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MIN_DELEGATION_DURATION));
        set_active_citizen(1, false); // suspend after delegating
        assert_noop!(
            Voting::revoke_delegation(RuntimeOrigin::signed(1), 0),
            Error::<Test>::CitizenNotActive
        );
        // The delegation record itself is untouched — still there, still counted — until it
        // either expires on its own or the citizen is reinstated and revokes it explicitly.
        assert!(Delegations::<Test>::get(0u32, 1u64).is_some());
        assert_eq!(DelegatorCount::<Test>::get((0u32, 2u64)), 1);
    });
}

// ── start_fiscal_year / claim_fiscal_year_tokens ─────────────────────────────

#[test]
fn start_fiscal_year_requires_legislature_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::start_fiscal_year(RuntimeOrigin::signed(1), 100),
            sp_runtime::DispatchError::BadOrigin
        );
    });
}

#[test]
fn start_fiscal_year_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::start_fiscal_year(RuntimeOrigin::root(), 100));
        assert_eq!(FiscalYearEpoch::<Test>::get(), 1);
        assert_eq!(EpochTokenAllocation::<Test>::get(1), Some(100));
        System::assert_last_event(Event::FiscalYearStarted { epoch: 1, tokens_per_citizen: 100 }.into());
    });
}

#[test]
fn claim_fiscal_year_tokens_fails_without_active_fiscal_year() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_noop!(
            Voting::claim_fiscal_year_tokens(RuntimeOrigin::signed(1)),
            Error::<Test>::NoActiveFiscalYear
        );
    });
}

#[test]
fn claim_fiscal_year_tokens_fails_for_inactive_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::start_fiscal_year(RuntimeOrigin::root(), 100));
        assert_noop!(
            Voting::claim_fiscal_year_tokens(RuntimeOrigin::signed(1)),
            Error::<Test>::CitizenNotActive
        );
    });
}

#[test]
fn claim_fiscal_year_tokens_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::start_fiscal_year(RuntimeOrigin::root(), 100));
        activate_citizen(1);
        assert_ok!(Voting::claim_fiscal_year_tokens(RuntimeOrigin::signed(1)));
        assert_eq!(BudgetBalance::<Test>::get(1), 100);
        assert_eq!(CitizenClaimedEpoch::<Test>::get(1), Some(1));
        System::assert_last_event(Event::BudgetTokensClaimed { who: 1, epoch: 1, tokens: 100 }.into());
    });
}

#[test]
fn claim_fiscal_year_tokens_fails_when_already_claimed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::start_fiscal_year(RuntimeOrigin::root(), 100));
        activate_citizen(1);
        assert_ok!(Voting::claim_fiscal_year_tokens(RuntimeOrigin::signed(1)));
        assert_noop!(
            Voting::claim_fiscal_year_tokens(RuntimeOrigin::signed(1)),
            Error::<Test>::BudgetAlreadyClaimed
        );
    });
}

// ── allocate_budget ──────────────────────────────────────────────────────────

fn setup_claimed_citizen(who: u64, tokens: u64) {
    activate_citizen(who);
    if FiscalYearEpoch::<Test>::get() == 0 {
        assert_ok!(Voting::start_fiscal_year(RuntimeOrigin::root(), tokens));
    }
    assert_ok!(Voting::claim_fiscal_year_tokens(RuntimeOrigin::signed(who)));
}

#[test]
fn allocate_budget_fails_for_inactive_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::allocate_budget(RuntimeOrigin::signed(1), 0, 1),
            Error::<Test>::CitizenNotActive
        );
    });
}

#[test]
fn allocate_budget_fails_invalid_category() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        setup_claimed_citizen(1, 100);
        assert_noop!(
            Voting::allocate_budget(RuntimeOrigin::signed(1), BUDGET_CATEGORY_COUNT, 1),
            Error::<Test>::InvalidCategoryId
        );
    });
}

#[test]
fn allocate_budget_fails_no_active_fiscal_year() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        activate_citizen(1);
        assert_noop!(
            Voting::allocate_budget(RuntimeOrigin::signed(1), 0, 1),
            Error::<Test>::NoActiveFiscalYear
        );
    });
}

#[test]
fn allocate_budget_fails_budget_not_claimed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::start_fiscal_year(RuntimeOrigin::root(), 100));
        activate_citizen(1);
        assert_noop!(
            Voting::allocate_budget(RuntimeOrigin::signed(1), 0, 1),
            Error::<Test>::BudgetNotClaimed
        );
    });
}

#[test]
fn allocate_budget_works_and_charges_quadratic_cost() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        setup_claimed_citizen(1, 100);
        assert_ok!(Voting::allocate_budget(RuntimeOrigin::signed(1), 0, 3));
        assert_eq!(BudgetBalance::<Test>::get(1), 100 - 9);
        assert_eq!(CategoryVotes::<Test>::get((1u64, 1u32, 0u32)), 3);
        System::assert_last_event(
            Event::BudgetAllocated { who: 1, epoch: 1, category_id: 0, vote_count: 3 }.into(),
        );
    });
}

#[test]
fn allocate_budget_fails_insufficient_tokens() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        setup_claimed_citizen(1, 10);
        // vote_count = 4 costs 16 > balance of 10.
        assert_noop!(
            Voting::allocate_budget(RuntimeOrigin::signed(1), 0, 4),
            Error::<Test>::InsufficientBudgetTokens
        );
        assert_eq!(BudgetBalance::<Test>::get(1), 10);
    });
}

#[test]
fn allocate_budget_refunds_on_decrease() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        setup_claimed_citizen(1, 100);
        assert_ok!(Voting::allocate_budget(RuntimeOrigin::signed(1), 0, 3)); // cost 9, balance 91
        assert_ok!(Voting::allocate_budget(RuntimeOrigin::signed(1), 0, 1)); // cost 1, refund 8
        assert_eq!(BudgetBalance::<Test>::get(1), 99);
        assert_eq!(CategoryVotes::<Test>::get((1u64, 1u32, 0u32)), 1);
    });
}

// ── vote_referendum ──────────────────────────────────────────────────────────

#[test]
fn vote_referendum_fails_without_active_epoch() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        activate_citizen(1);
        assert_noop!(
            Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true),
            Error::<Test>::VotingEpochNotActive
        );
    });
}

#[test]
fn vote_referendum_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        activate_citizen(1);
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert!(ReferendumHasVoted::<Test>::get((rid, 1u64)));
        assert_eq!(ReferendumTally::<Test>::get(rid), (1, 0));
        System::assert_last_event(
            Event::ReferendumVoteCast { referendum_id: rid, voter: 1, in_favor: true }.into(),
        );
    });
}

#[test]
fn vote_referendum_fails_for_inactive_citizen() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        assert_noop!(
            Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true),
            Error::<Test>::CitizenNotActive
        );
    });
}

#[test]
fn vote_referendum_fails_referendum_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        activate_citizen(1);
        assert_noop!(
            Voting::vote_referendum(RuntimeOrigin::signed(1), 999, true),
            Error::<Test>::ReferendumNotFound
        );
    });
}

#[test]
fn vote_referendum_fails_already_voted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        activate_citizen(1);
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_noop!(
            Voting::vote_referendum(RuntimeOrigin::signed(1), rid, false),
            Error::<Test>::AlreadyVotedInReferendum
        );
    });
}

#[test]
fn vote_referendum_fails_after_referendum_end_block_even_with_epoch_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        // Epoch spans well past the referendum's own end_block (1 + REFERENDUM_DURATION).
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        activate_citizen(1);
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1); // past referendum end_block
        assert_noop!(
            Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true),
            Error::<Test>::ReferendumNotActive
        );
    });
}

#[test]
fn vote_referendum_fails_once_epoch_window_has_passed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION)); // [1, 6]
        let rid = make_referendum(1, ReferendumTier::Ordinary); // still active until block 21
        activate_citizen(1);
        System::set_block_number(1 + MIN_EPOCH_DURATION as u64 + 1); // 7, past epoch end but referendum still open
        assert_noop!(
            Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true),
            Error::<Test>::VotingEpochNotActive
        );
    });
}

// ── finalize_referendum ──────────────────────────────────────────────────────

#[test]
fn finalize_referendum_fails_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::finalize_referendum(RuntimeOrigin::signed(1), 999),
            Error::<Test>::ReferendumNotFound
        );
    });
}

#[test]
fn finalize_referendum_fails_while_still_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        assert_noop!(
            Voting::finalize_referendum(RuntimeOrigin::signed(1), rid),
            Error::<Test>::ReferendumStillActive
        );
    });
}

#[test]
fn finalize_referendum_ordinary_passes_at_exact_threshold_boundary() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, false));
        // 1 yes / 1 no = exactly 50% = PassageThreshold boundary (inclusive).
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(3), rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Passed);
        assert!(enacted_laws().contains(&(ReferendumTier::Ordinary, hash(7))));
        System::assert_last_event(Event::ReferendumPassed { referendum_id: rid, topic_hash: hash(7) }.into());
    });
}

#[test]
fn finalize_referendum_ordinary_fails_below_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, false));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(3), rid, false));
        // 1 yes / 2 no = 33% < 50%.
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(4), rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Failed);
        assert!(enacted_laws().is_empty());
        System::assert_last_event(Event::ReferendumFailed { referendum_id: rid }.into());
    });
}

#[test]
fn finalize_referendum_fails_when_no_votes_cast() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let rid = make_referendum(1, ReferendumTier::Ordinary);
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(1), rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Failed);
    });
}

#[test]
fn finalize_referendum_constitutional_tier_uses_supermajority_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::create_constitutional_referendum(RuntimeOrigin::root(), hash(1)));
        let rid = 0u32;
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        // 2 yes / 1 no = 66.7% < ConstitutionalPassageThreshold (67%) -> must fail.
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(3), rid, false));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(4), rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Failed);
        assert!(enacted_laws().is_empty());
    });
}

#[test]
fn finalize_referendum_constitutional_tier_passes_above_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::create_constitutional_referendum(RuntimeOrigin::root(), hash(2)));
        let rid = 0u32;
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        activate_citizen(4);
        // 3 yes / 1 no = 75% >= 67%.
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(3), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(4), rid, false));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(5), rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Passed);
        assert!(enacted_laws().contains(&(ReferendumTier::Constitutional, hash(2))));
    });
}

#[test]
fn finalize_referendum_foundational_tier_uses_highest_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::create_foundational_referendum(RuntimeOrigin::root(), hash(3)));
        let rid = 0u32;
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        // 2 yes / 1 no = 66.7% < FoundationalPassageThreshold (75%) -> must fail.
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(3), rid, false));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(4), rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Failed);
        assert!(enacted_laws().is_empty());
    });
}

#[test]
fn finalize_referendum_foundational_tier_passes_above_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::create_foundational_referendum(RuntimeOrigin::root(), hash(4)));
        let rid = 0u32;
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        activate_citizen(4);
        activate_citizen(5);
        // 4 yes / 1 no = 80% >= 75%.
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(3), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(4), rid, true));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(5), rid, false));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(6), rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Passed);
        assert!(enacted_laws().contains(&(ReferendumTier::Foundational, hash(4))));
    });
}

// ── liquid democracy delegation resolution (finalize_referendum) ────────────
//
// `hash(0)` derives `topic_id` 0 (`u32::from_le_bytes([0, 0, 0, 0])`) via `topic_id_of`, which
// happens to match the `topic_id` literal (`0`) every `delegate_vote` test above already uses —
// no coincidence, chosen so delegations set up the usual way line up with these referenda
// without a separate topic-id computation in every test.

/// Creates a referendum with an explicit `topic_hash` (unlike `make_referendum`, which always
/// uses `hash(7)`) so delegation topic_id can be made to line up with it deliberately.
fn make_referendum_with_hash(petition_id: u32, tier: ReferendumTier, topic_hash: [u8; 32]) -> u32 {
    assert_ok!(Voting::create_referendum_internal(petition_id, topic_hash, tier));
    NextReferendumId::<Test>::get() - 1
}

#[test]
fn finalize_referendum_counts_delegated_weight_for_the_delegate() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0));
        activate_citizen(1); // delegator, never votes directly
        activate_citizen(2); // delegate, votes directly
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        // Before finalize, the raw tally only reflects citizen 2's own direct vote.
        assert_eq!(ReferendumTally::<Test>::get(rid), (1, 0));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        // Citizen 1 never voted, but delegated to citizen 2 — their weight now counts too.
        assert_eq!(ReferendumTally::<Test>::get(rid), (2, 0));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Passed);
    });
}

#[test]
fn finalize_referendum_direct_vote_overrides_delegation_no_double_count() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0));
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION));
        // Citizen 1 delegated to 2, but still casts their own (opposite) vote directly.
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, false));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        // 1 yes (citizen 2) / 1 no (citizen 1's own direct vote) — no delegated weight added on
        // top, since citizen 1 voted directly.
        assert_eq!(ReferendumTally::<Test>::get(rid), (1, 1));
    });
}

#[test]
fn finalize_referendum_resolves_transitive_delegation_chain() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0));
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        // 1 -> 2 -> 3; only 3 votes directly. MAX_DELEGATION_DEPTH (3) comfortably covers this.
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION));
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(2), 3, 0, MAX_DELEGATION_DURATION));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(3), rid, true));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        // Citizen 3's own vote plus both citizen 1 and citizen 2's transitively-resolved weight.
        assert_eq!(ReferendumTally::<Test>::get(rid), (3, 0));
    });
}

/// Same 1 -> 2 -> 3 chain as `finalize_referendum_resolves_transitive_delegation_chain`, but
/// with `total_citizens` set so `DelegationCap` is actually in force and the chain's fully
/// resolved weight (3) sits comfortably under it (cap allows up to weight 4 out of 10
/// citizens). Confirms the cap fix doesn't regress a legitimate transitive chain: both hops
/// are accepted and the final tally still resolves the whole chain onto citizen 3's vote.
#[test]
fn finalize_referendum_resolves_transitive_delegation_chain_within_cap() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0));
        set_total_citizens(10);
        activate_citizen(1);
        activate_citizen(2);
        activate_citizen(3);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION));
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(2), 3, 0, MAX_DELEGATION_DURATION));
        // Delegated-in weight only (citizens 1 and 2's own votes) — citizen 3's own vote is
        // added separately by vote_referendum/finalize_referendum below.
        assert_eq!(DelegatedWeight::<Test>::get((0u32, 3u64)), 2);
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(3), rid, true));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        assert_eq!(ReferendumTally::<Test>::get(rid), (3, 0));
    });
}

#[test]
fn finalize_referendum_ignores_delegated_weight_when_delegate_never_votes() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0));
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION));
        // Citizen 2 never calls vote_referendum.
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        assert_eq!(ReferendumTally::<Test>::get(rid), (0, 0));
        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Failed);
    });
}

#[test]
fn finalize_referendum_excludes_delegated_weight_from_suspended_delegator() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0));
        activate_citizen(1);
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION));
        // Citizen 1 is suspended after delegating but before the referendum closes.
        set_active_citizen(1, false);
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        // Only citizen 2's own vote — a suspended citizen doesn't gain representation by proxy.
        assert_eq!(ReferendumTally::<Test>::get(rid), (1, 0));
    });
}

#[test]
fn finalize_referendum_counts_delegation_active_through_referendum_close_even_if_expired_by_finalize_time() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0));
        // end_block = 1 + REFERENDUM_DURATION = 21.
        activate_citizen(1);
        activate_citizen(2);
        // Delegation expires exactly at the referendum's close block (expires_at == end_block),
        // per DelegationRecord's documented "valid for referenda whose close block <= expires_at".
        assert_ok!(Voting::delegate_vote(
            RuntimeOrigin::signed(1),
            2,
            0,
            REFERENDUM_DURATION,
        ));
        let expires_at = Delegations::<Test>::get(0u32, 1u64).unwrap().expires_at;
        assert_eq!(expires_at, 1 + REFERENDUM_DURATION as u64);
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        // Call finalize well after the delegation's expires_at has passed (block "now" > 21).
        System::set_block_number(expires_at + 10);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        // Still counted: validity is judged against the referendum's end_block, not the block
        // finalize_referendum happens to be called at.
        assert_eq!(ReferendumTally::<Test>::get(rid), (2, 0));
    });
}

#[test]
fn finalize_referendum_delegation_scan_is_bounded_to_the_referendums_own_topic() {
    // Regression test for the DoS finding: `apply_delegated_weight` must only scan delegators
    // on the referendum's own topic (`Delegations::<T>::iter_prefix(topic_id)`), not every
    // delegation on every topic. Set up a delegation on an unrelated topic whose delegate votes
    // the same way citizen 2 does, and confirm it never contributes to this referendum's tally
    // -- proving the double-map is genuinely keyed (and iterated) topic-first, not just filtered
    // after a full scan.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0)); // topic_id 0
        activate_citizen(1); // delegator on topic 0
        activate_citizen(2); // delegate on topic 0, votes directly
        activate_citizen(3); // delegator on a DIFFERENT topic (hash(1) -> topic_id != 0)
        activate_citizen(4); // that other delegation's delegate, also votes directly
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(1), 2, 0, MAX_DELEGATION_DURATION));
        let other_topic = Voting::topic_id_of(&hash(1));
        assert_ne!(other_topic, 0);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(3), 4, other_topic, MAX_DELEGATION_DURATION));
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(2), rid, true));
        // Citizen 4 never votes on `rid` (there's no referendum on `other_topic` here), but even
        // if they had, their delegator (3) must not be pulled into this referendum's tally.
        System::set_block_number(1 + REFERENDUM_DURATION as u64 + 1);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(9), rid));
        // Only citizen 1's delegated weight plus citizen 2's own vote -- citizen 3's
        // other-topic delegation contributes nothing.
        assert_eq!(ReferendumTally::<Test>::get(rid), (2, 0));
    });
}

#[test]
fn finalize_referendum_weight_scales_with_total_citizens() {
    // Regression test for the DoS finding: `finalize_referendum` is permissionless
    // (`ensure_signed`), and its declared weight must track the real cost of the delegation
    // scan it triggers instead of staying flat as the delegation graph (bounded above by
    // `CitizenChecker::total_citizens()`) grows.
    use frame_support::dispatch::GetDispatchInfo;
    let weight_at = |total_citizens: u32| {
        set_total_citizens(total_citizens);
        let call = crate::Call::<Test>::finalize_referendum { referendum_id: 0 };
        call.get_dispatch_info().call_weight
    };
    let empty = weight_at(0);
    let small = weight_at(10);
    let large = weight_at(1_000_000);
    // Strictly increasing in the citizen count, not a flat estimate.
    assert!(small.ref_time() > empty.ref_time());
    assert!(large.ref_time() > small.ref_time());
    // Sanity bound: the weight for a huge citizenry must be meaningfully larger than the base
    // cost charged when there are no citizens at all -- not just a rounding-noise difference.
    assert!(large.ref_time() > empty.ref_time().saturating_mul(1000));
}

// ── automatic finalization (on_initialize / PendingFinalization) ────────────

/// Fix for the "finalize-then-react" window: before this fix, `finalize_referendum` was
/// permissionless but never called automatically, so a referendum could sit past its `end_block`
/// for an unbounded number of blocks, during which the (already publicly visible) tally could be
/// manipulated via a late delegation to the winning side. This proves the window is closed:
/// `on_initialize` finalizes the referendum automatically at `end_block + 1` (scheduled via
/// `PendingFinalization` at referendum creation), so a delegation created afterward — even to
/// the referendum's own winning voter, on the referendum's own topic — has zero effect on the
/// already-locked-in tally, because finalization has already run by the time it lands.
#[test]
fn referendum_auto_finalizes_on_initialize_closing_the_late_delegation_window() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));
        let rid = make_referendum_with_hash(1, ReferendumTier::Ordinary, hash(0)); // topic_id 0
        activate_citizen(1); // V, the sole direct voter
        assert_ok!(Voting::vote_referendum(RuntimeOrigin::signed(1), rid, true));
        assert_eq!(ReferendumTally::<Test>::get(rid), (1, 0));

        let end_block = 1 + REFERENDUM_DURATION as u64; // 21
        let finalize_block = end_block + 1; // 22
        assert!(PendingFinalization::<Test>::get(finalize_block).contains(&rid));

        // Advance to the block on_initialize runs the scheduled auto-finalization at, WITHOUT
        // ever calling the finalize_referendum extrinsic — exactly what happens automatically
        // every block on a real chain.
        System::set_block_number(finalize_block);
        let _ = Voting::on_initialize(finalize_block);

        let (_, _, _, state, _) = Referenda::<Test>::get(rid).unwrap();
        assert_eq!(state, ReferendumState::Passed);
        assert_eq!(ReferendumTally::<Test>::get(rid), (1, 0));
        assert!(PendingFinalization::<Test>::get(finalize_block).is_empty());

        // An attacker who saw the (already public) tally now delegates a fresh citizen to the
        // winning voter, on the referendum's own topic — exactly the reaction this fix must
        // render inert.
        activate_citizen(2);
        assert_ok!(Voting::delegate_vote(RuntimeOrigin::signed(2), 1, 0, MIN_DELEGATION_DURATION));

        // The referendum is already finalized: the late delegation cannot be pulled into its
        // tally, and re-running finalization on it is rejected outright.
        assert_eq!(ReferendumTally::<Test>::get(rid), (1, 0));
        assert_noop!(
            Voting::finalize_referendum(RuntimeOrigin::signed(99), rid),
            Error::<Test>::ReferendumNotActive
        );
    });
}

/// If a block's `PendingFinalization` list is already at `MaxReferendaPerBlock` capacity, a
/// referendum scheduled to finalize in that same block is simply left unscheduled rather than
/// failing creation — it must still finalize correctly via the permissionless
/// `finalize_referendum` extrinsic fallback.
#[test]
fn referendum_finalization_scheduling_overflow_falls_back_to_the_manual_extrinsic() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION));

        // Fill this block's PendingFinalization list to capacity with unrelated referenda that
        // all share the same end_block (created in the same block, same ReferendumDurationBlocks).
        let mut ids = Vec::new();
        for i in 0..MAX_REFERENDA_PER_BLOCK {
            ids.push(make_referendum_with_hash(100 + i, ReferendumTier::Ordinary, hash(1)));
        }
        let finalize_block = 1 + REFERENDUM_DURATION as u64 + 1;
        assert_eq!(
            PendingFinalization::<Test>::get(finalize_block).len(),
            MAX_REFERENDA_PER_BLOCK as usize
        );

        // One more referendum, same finalization block: the schedule is full, so it's left
        // unscheduled — but creation itself still succeeds.
        let overflow_rid =
            make_referendum_with_hash(999, ReferendumTier::Ordinary, hash(1));
        assert_eq!(
            PendingFinalization::<Test>::get(finalize_block).len(),
            MAX_REFERENDA_PER_BLOCK as usize
        );
        assert!(!PendingFinalization::<Test>::get(finalize_block).contains(&overflow_rid));

        // The hook still finalizes everything it did schedule...
        System::set_block_number(finalize_block);
        let _ = Voting::on_initialize(finalize_block);
        for id in ids {
            let (_, _, _, state, _) = Referenda::<Test>::get(id).unwrap();
            assert_eq!(state, ReferendumState::Failed); // no votes cast
        }

        // ...and the overflow referendum, never auto-scheduled, is still stuck in `Voting` until
        // the permissionless fallback extrinsic is called for it explicitly.
        let (_, _, _, state, _) = Referenda::<Test>::get(overflow_rid).unwrap();
        assert_eq!(state, ReferendumState::Voting);
        assert_ok!(Voting::finalize_referendum(RuntimeOrigin::signed(1), overflow_rid));
        let (_, _, _, state, _) = Referenda::<Test>::get(overflow_rid).unwrap();
        assert_eq!(state, ReferendumState::Failed);
    });
}

// ── submit_maci_tally ────────────────────────────────────────────────────────

#[test]
fn submit_maci_tally_requires_legislature_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        assert_noop!(
            Voting::submit_maci_tally(RuntimeOrigin::signed(1), pid, 10, 5, hash(1), proof(true)),
            sp_runtime::DispatchError::BadOrigin
        );
    });
}

#[test]
fn submit_maci_tally_fails_proposal_not_found() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::submit_maci_tally(RuntimeOrigin::root(), 999, 10, 5, hash(1), proof(true)),
            Error::<Test>::ProposalNotFound
        );
    });
}

#[test]
fn submit_maci_tally_fails_while_proposal_still_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        assert_noop!(
            Voting::submit_maci_tally(RuntimeOrigin::root(), pid, 10, 5, hash(1), proof(true)),
            Error::<Test>::ProposalStillActive
        );
    });
}

#[test]
fn submit_maci_tally_fails_invalid_proof() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        System::set_block_number(1 + MIN_PROPOSAL_DURATION as u64 + 1);
        assert_noop!(
            Voting::submit_maci_tally(RuntimeOrigin::root(), pid, 10, 5, hash(1), proof(false)),
            Error::<Test>::InvalidTallyProof
        );
    });
}

#[test]
fn submit_maci_tally_works_and_enacts_law_when_passed() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION); // topic_hash(9), tier Ordinary
        System::set_block_number(1 + MIN_PROPOSAL_DURATION as u64 + 1);
        assert_ok!(Voting::submit_maci_tally(RuntimeOrigin::root(), pid, 60, 40, hash(1), proof(true)));
        assert_eq!(ProposalResults::<Test>::get(pid), Some((60, 40, hash(1))));
        System::assert_last_event(
            Event::TallySubmitted { proposal_id: pid, yes_votes: 60, no_votes: 40, commitment_root: hash(1) }
                .into(),
        );
        assert!(enacted_laws().contains(&(ReferendumTier::Ordinary, hash(9))));
    });
}

#[test]
fn submit_maci_tally_does_not_enact_law_below_threshold() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        System::set_block_number(1 + MIN_PROPOSAL_DURATION as u64 + 1);
        assert_ok!(Voting::submit_maci_tally(RuntimeOrigin::root(), pid, 10, 90, hash(1), proof(true)));
        assert!(enacted_laws().is_empty());
    });
}

#[test]
fn submit_maci_tally_fails_when_already_submitted() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let pid = make_proposal(MIN_PROPOSAL_DURATION);
        System::set_block_number(1 + MIN_PROPOSAL_DURATION as u64 + 1);
        assert_ok!(Voting::submit_maci_tally(RuntimeOrigin::root(), pid, 60, 40, hash(1), proof(true)));
        assert_noop!(
            Voting::submit_maci_tally(RuntimeOrigin::root(), pid, 60, 40, hash(1), proof(true)),
            Error::<Test>::TallyAlreadySubmitted
        );
    });
}

// ── open_voting_epoch / close_voting_epoch ───────────────────────────────────

#[test]
fn open_voting_epoch_requires_legislature_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::open_voting_epoch(RuntimeOrigin::signed(1), MIN_EPOCH_DURATION),
            sp_runtime::DispatchError::BadOrigin
        );
    });
}

#[test]
fn open_voting_epoch_fails_duration_too_short() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION - 1),
            Error::<Test>::InvalidEpochDuration
        );
    });
}

#[test]
fn open_voting_epoch_fails_duration_too_long() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::open_voting_epoch(RuntimeOrigin::root(), MAX_EPOCH_DURATION + 1),
            Error::<Test>::InvalidEpochDuration
        );
    });
}

#[test]
fn open_voting_epoch_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        assert_eq!(ActiveEpoch::<Test>::get(), Some((1, 1 + MIN_EPOCH_DURATION as u64)));
        assert_eq!(EpochNumber::<Test>::get(), 1);
        System::assert_last_event(
            Event::VotingEpochOpened { epoch: 1, start: 1, end: 1 + MIN_EPOCH_DURATION as u64 }.into(),
        );
    });
}

#[test]
fn open_voting_epoch_fails_when_already_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        assert_noop!(
            Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION),
            Error::<Test>::EpochAlreadyActive
        );
    });
}

#[test]
fn close_voting_epoch_fails_when_not_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::close_voting_epoch(RuntimeOrigin::signed(1)),
            Error::<Test>::VotingEpochNotActive
        );
    });
}

#[test]
fn close_voting_epoch_fails_while_still_active() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        assert_noop!(
            Voting::close_voting_epoch(RuntimeOrigin::signed(1)),
            Error::<Test>::EpochStillActive
        );
    });
}

#[test]
fn close_voting_epoch_works_after_end_block() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION));
        System::set_block_number(1 + MIN_EPOCH_DURATION as u64 + 1);
        assert_ok!(Voting::close_voting_epoch(RuntimeOrigin::signed(1)));
        assert!(ActiveEpoch::<Test>::get().is_none());
        System::assert_last_event(Event::VotingEpochClosed { epoch: 1 }.into());
    });
}

#[test]
fn on_initialize_auto_closes_expired_epoch() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::open_voting_epoch(RuntimeOrigin::root(), MIN_EPOCH_DURATION)); // ends at 6
        let past_end = 1 + MIN_EPOCH_DURATION as u64 + 1;
        System::set_block_number(past_end);
        let _ = Voting::on_initialize(past_end);
        assert!(ActiveEpoch::<Test>::get().is_none());
        System::assert_last_event(Event::VotingEpochClosed { epoch: 1 }.into());
    });
}

// ── create_constitutional_referendum / create_foundational_referendum ───────

#[test]
fn create_constitutional_referendum_requires_legislature_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::create_constitutional_referendum(RuntimeOrigin::signed(1), hash(1)),
            sp_runtime::DispatchError::BadOrigin
        );
    });
}

#[test]
fn create_constitutional_referendum_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::create_constitutional_referendum(RuntimeOrigin::root(), hash(1)));
        let (petition_id, topic_hash, ends_at, state, tier) = Referenda::<Test>::get(0).unwrap();
        assert_eq!(petition_id, u32::MAX);
        assert_eq!(topic_hash, hash(1));
        assert_eq!(ends_at, 1 + REFERENDUM_DURATION as u64);
        assert_eq!(state, ReferendumState::Voting);
        assert_eq!(tier, ReferendumTier::Constitutional);
        System::assert_last_event(
            Event::ReferendumCreated {
                referendum_id: 0,
                petition_id: u32::MAX,
                topic_hash: hash(1),
                ends_at,
                tier: ReferendumTier::Constitutional,
            }
            .into(),
        );
    });
}

#[test]
fn create_foundational_referendum_requires_legislature_origin() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_noop!(
            Voting::create_foundational_referendum(RuntimeOrigin::signed(1), hash(1)),
            sp_runtime::DispatchError::BadOrigin
        );
    });
}

#[test]
fn create_foundational_referendum_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::create_foundational_referendum(RuntimeOrigin::root(), hash(2)));
        let (petition_id, topic_hash, _, state, tier) = Referenda::<Test>::get(0).unwrap();
        assert_eq!(petition_id, u32::MAX);
        assert_eq!(topic_hash, hash(2));
        assert_eq!(state, ReferendumState::Voting);
        assert_eq!(tier, ReferendumTier::Foundational);
    });
}

// ── create_referendum_internal (petition pipeline) ──────────────────────────

#[test]
fn create_referendum_internal_fails_on_duplicate_petition() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        assert_ok!(Voting::create_referendum_internal(5, hash(1), ReferendumTier::Ordinary));
        assert_eq!(PetitionReferendum::<Test>::get(5), Some(0));
        assert_noop!(
            Voting::create_referendum_internal(5, hash(2), ReferendumTier::Ordinary),
            Error::<Test>::ReferendumAlreadyExists
        );
    });
}

// ── legislature_call_hash (HIGH-severity motion-hijack fix) ────────────────────
//
// See the equivalent block in pallet-constitution's tests for the full rationale. The
// binding invariant itself is proven against the real `EnsureLegislatureMotion` origin in
// pallet-legislature's own suite; here we confirm this pallet's five
// `LegislatureOrigin`-gated calls never hash to the same value for overlapping raw
// parameters.
#[test]
fn legislature_call_hash_differs_across_constitutional_and_foundational_referenda() {
    // Both calls take exactly one `[u8; 32]` argument -- the case most likely to collide
    // without the call-tag domain separator.
    let constitutional =
        crate::pallet::legislature_call_hash(b"pallet-voting::create_constitutional_referendum", hash(1));
    let foundational =
        crate::pallet::legislature_call_hash(b"pallet-voting::create_foundational_referendum", hash(1));
    assert_ne!(constitutional, foundational);
}

#[test]
fn legislature_call_hash_differs_for_different_topic_hashes() {
    let a =
        crate::pallet::legislature_call_hash(b"pallet-voting::create_constitutional_referendum", hash(1));
    let b =
        crate::pallet::legislature_call_hash(b"pallet-voting::create_constitutional_referendum", hash(2));
    assert_ne!(a, b);
}
