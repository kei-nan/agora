use crate::{
    mock::*, ActiveEpoch, BudgetBalance, CategoryVotes, CitizenClaimedEpoch, DelegatorCount,
    Delegations, EpochNumber, EpochTokenAllocation, Error, Event, FiscalYearEpoch,
    NextProposalId, NextReferendumId, PetitionReferendum, ProposalResults, Proposals,
    ReferendumHasVoted, ReferendumState, ReferendumTally, ReferendumTier, Referenda,
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
        let record = Delegations::<Test>::get((1u64, 0u32)).unwrap();
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
        let record = Delegations::<Test>::get((1u64, 0u32)).unwrap();
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
        assert_eq!(Delegations::<Test>::get((1u64, 0u32)).unwrap().delegate, 3);
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

        assert!(Delegations::<Test>::get((40u64, 9u32)).is_none());
        assert_eq!(DelegatorCount::<Test>::get((9u32, 41u64)), 0);
        assert!(last_event_matches(
            |e| matches!(e, RuntimeEvent::Voting(Event::DelegationExpired { delegator: 40, topic_id: 9 }))
        ));
        assert_eq!(Delegations::<Test>::get((42u64, 9u32)).unwrap().delegate, 40);
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
        assert!(Delegations::<Test>::get((1u64, 0u32)).is_none());
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
        assert!(Delegations::<Test>::get((1u64, 0u32)).is_some());
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
