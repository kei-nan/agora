//! # Legislature Pallet
//!
//! On-chain legislative collective for Agora. A fixed set of members propose and vote on
//! motions. When a motion reaches `PassageThreshold`% ayes after `MotionDurationBlocks`,
//! anyone can close it. A closed-passed motion emits `MotionPassed` and marks `executed = true`.
//!
//! `EnsureLegislatureMotion` is the `LegislatureOrigin` type consumed by pallet-constitution
//! and several other legislature-gated pallets (treasury-ledger, voting, executive, elections,
//! identity). It implements `EnsureOriginWithArg<RuntimeOrigin, [u8; 32]>`: the caller must
//! supply the hash of the exact call it is dispatching, and `try_origin` only succeeds if that
//! hash matches the `call_hash` the passed motion actually approved (see `EnsureLegislatureMotion`'s
//! own doc comment below for the full contract). This closes a prior HIGH-severity gap where
//! any passed motion's approval token could be replayed to authorize an unrelated call.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use codec::{Decode, DecodeWithMemTracking, Encode};
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Saturating;

    // ── Motion struct ────────────────────────────────────────────────────────────

    #[derive(Clone, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo, PartialEq, Debug)]
    #[scale_info(skip_type_params(T))]
    pub struct Motion<T: Config> {
        /// Hash of the encoded call being proposed (for reference / auditing).
        pub call_hash: [u8; 32],
        /// Member who created the motion (already counts as an aye).
        pub proposer: T::AccountId,
        /// Current aye count.
        pub ayes: u32,
        /// Current nay count.
        pub nays: u32,
        /// Block after which the motion may be closed.
        pub end_block: BlockNumberFor<T>,
        /// Set to true by `close_motion` — prevents double-execution.
        pub executed: bool,
    }

    // ── Pallet ───────────────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Origin ───────────────────────────────────────────────────────────────────

    /// Origin that passes only when `close_motion` has just passed a motion, *and* the
    /// caller proves — via the `[u8; 32]` argument required by `EnsureOriginWithArg` — that
    /// it is dispatching the exact call the motion approved.
    ///
    /// `close_motion` writes a `PendingLegislatureApproval` token (the passed `call_hash`,
    /// paired with the motion's proposer) to storage. `EnsureLegislatureMotion::try_origin`
    /// consumes that token exactly once, and only if the caller-supplied hash argument
    /// matches it, so a motion that passed to authorize one call can never be replayed to
    /// execute a different one — including a different call in a different
    /// legislature-gated pallet. This is enforced with `EnsureOriginWithArg` (rather than
    /// plain `EnsureOrigin`) so the check lives inside the origin gate itself: a consuming
    /// call site cannot forget to verify the token, because there is no path to a
    /// successful origin without supplying a matching hash. Consuming pallets compute that
    /// hash from their own call's parameters, domain-separated by a pallet+call tag (see
    /// e.g. `pallet_constitution`'s `legislature_call_hash` helper) so byte-identical
    /// parameters can never collide across two different calls.
    pub struct EnsureLegislatureMotion<T>(core::marker::PhantomData<T>);

    impl<T: Config> frame_support::traits::EnsureOriginWithArg<T::RuntimeOrigin, [u8; 32]>
        for EnsureLegislatureMotion<T>
    {
        type Success = ();
        fn try_origin(
            o: T::RuntimeOrigin,
            call_hash: &[u8; 32],
        ) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            match o.clone().into() {
                Ok(RawOrigin::Signed(who)) if Members::<T>::get().contains(&who) => {
                    // Consume the pending approval token only if this member is the
                    // motion's proposer *and* the caller's hash matches the hash the
                    // motion actually passed for. Any other member, or a mismatched
                    // hash, is rejected — preventing both a hostile member from
                    // hijacking the queued action and a passed motion for call A from
                    // being replayed against unrelated call B.
                    if let Some((approved_hash, authorized)) = PendingLegislatureApproval::<T>::get() {
                        if authorized == who && approved_hash == *call_hash {
                            PendingLegislatureApproval::<T>::kill();
                            return Ok(());
                        }
                    }
                    Err(o)
                }
                _ => Err(o),
            }
        }
        #[cfg(feature = "runtime-benchmarks")]
        fn try_successful_origin(call_hash: &[u8; 32]) -> Result<T::RuntimeOrigin, ()> {
            let member = Members::<T>::get().first().cloned().ok_or(())?;
            // Plant a token so the benchmark-generated origin validates.
            PendingLegislatureApproval::<T>::put((*call_hash, member.clone()));
            Ok(frame_system::RawOrigin::Signed(member).into())
        }
    }

    /// Checks whether an account is currently an active executive minister.
    /// Implemented by pallet-executive; called by pallet-legislature to enforce
    /// the incompatibility rule (ministers cannot vote on legislature motions).
    pub trait MinisterChecker<AccountId> {
        fn is_active_minister(who: &AccountId) -> bool;
    }

    // ── Config ───────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Maximum number of legislature seats (e.g. 500).
        #[pallet::constant]
        type MaxMembers: Get<u32>;
        /// How many blocks a motion stays open for voting (e.g. 7 * DAYS).
        #[pallet::constant]
        type MotionDurationBlocks: Get<u32>;
        /// Percentage of *total members* that must vote aye for a motion to pass (e.g. 50).
        /// Evaluated at close time: ayes * 100 >= PassageThreshold * total_members.
        #[pallet::constant]
        type PassageThreshold: Get<u8>;
        /// Checks whether a member is an active executive minister.
        /// Ministers are blocked from voting on motions (incompatibility rule).
        type MinisterChecker: MinisterChecker<Self::AccountId>;
    }

    // ── Storage ──────────────────────────────────────────────────────────────────

    /// Enrolled legislature members.
    #[pallet::storage]
    pub type Members<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxMembers>, ValueQuery>;

    /// Monotonic counter for motion IDs.
    #[pallet::storage]
    pub type NextMotionId<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// motion_id -> Motion.
    #[pallet::storage]
    pub type Motions<T: Config> = StorageMap<_, Blake2_128Concat, u32, Motion<T>>;

    /// (motion_id, member) -> true = aye, false = nay.
    #[pallet::storage]
    pub type MotionVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, T::AccountId), bool>;

    /// Set by `close_motion` when a motion passes; consumed by `EnsureLegislatureMotion`.
    /// Stores `(call_hash, proposer)` — only the motion's proposer can consume the token,
    /// preventing any other member from hijacking a passed motion's approval.
    /// Cleared after it is consumed — each passed motion authorizes exactly one action.
    #[pallet::storage]
    pub type PendingLegislatureApproval<T: Config> =
        StorageValue<_, ([u8; 32], T::AccountId), OptionQuery>;

    // ── Events ───────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A new member was added to the legislature.
        MemberAdded { who: T::AccountId },
        /// A member was removed from the legislature.
        MemberRemoved { who: T::AccountId },
        /// A new motion was proposed.
        MotionProposed { motion_id: u32, call_hash: [u8; 32] },
        /// A member cast a vote on a motion.
        VoteCast { motion_id: u32, member: T::AccountId, approve: bool },
        /// A motion was closed and reached the passage threshold — it passed.
        MotionPassed { motion_id: u32, call_hash: [u8; 32] },
        /// A motion was closed but did not reach the passage threshold — it failed.
        MotionFailed { motion_id: u32 },
    }

    // ── Errors ───────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// The account is not a registered legislature member.
        NotAMember,
        /// The account is already a legislature member.
        AlreadyMember,
        /// No motion exists with the given ID.
        MotionNotFound,
        /// The member has already cast a vote on this motion.
        AlreadyVoted,
        /// The motion's voting window has not yet ended.
        MotionStillOpen,
        /// The motion has already been closed and executed.
        MotionAlreadyExecuted,
        /// Cannot add member: maximum member capacity reached.
        MembersAtCapacity,
        /// The account was not found in the member list.
        MemberNotFound,
        /// Active executive ministers may not vote on legislature motions (incompatibility rule).
        MinisterCannotVote,
        /// The motion's voting window has already closed; votes are no longer accepted.
        VotingWindowClosed,
        /// A previously passed motion's approval token is still pending consumption.
        /// The queued action must be executed before another passed motion can plant a new token.
        ApprovalPending,
    }

    // ── Calls ────────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add a member to the legislature. Root-only (bootstrapping; replaced by elections later).
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_member(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Members::<T>::try_mutate(|members| {
                ensure!(!members.contains(&who), Error::<T>::AlreadyMember);
                members.try_push(who.clone()).map_err(|_| Error::<T>::MembersAtCapacity)
            })?;
            Self::deposit_event(Event::MemberAdded { who });
            Ok(())
        }

        /// Remove a member from the legislature. Root-only.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_member(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Members::<T>::try_mutate(|members| {
                let pos = members.iter().position(|m| m == &who)
                    .ok_or(Error::<T>::MemberNotFound)?;
                members.remove(pos);
                Ok::<(), DispatchError>(())
            })?;
            Self::deposit_event(Event::MemberRemoved { who });
            Ok(())
        }

        /// Propose a motion. Only enrolled members may propose.
        /// The proposer's aye is recorded immediately (ayes starts at 1).
        /// Active executive ministers may not propose motions (incompatibility rule).
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn propose_motion(origin: OriginFor<T>, call_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Members::<T>::get().contains(&who), Error::<T>::NotAMember);
            ensure!(
                !T::MinisterChecker::is_active_minister(&who),
                Error::<T>::MinisterCannotVote
            );

            let motion_id = NextMotionId::<T>::get();
            let end_block = frame_system::Pallet::<T>::block_number()
                .saturating_add(BlockNumberFor::<T>::from(T::MotionDurationBlocks::get()));

            let motion = Motion::<T> {
                call_hash,
                proposer: who.clone(),
                ayes: 1,
                nays: 0,
                end_block,
                executed: false,
            };

            Motions::<T>::insert(motion_id, motion);
            // Record the proposer's automatic aye.
            MotionVotes::<T>::insert((motion_id, who.clone()), true);
            NextMotionId::<T>::put(motion_id.saturating_add(1));

            Self::deposit_event(Event::MotionProposed { motion_id, call_hash });
            Self::deposit_event(Event::VoteCast { motion_id, member: who, approve: true });
            Ok(())
        }

        /// Cast a vote on an open motion. Only enrolled members; one vote per member.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(12_000, 0))]
        pub fn vote_motion(
            origin: OriginFor<T>,
            motion_id: u32,
            approve: bool,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Members::<T>::get().contains(&who), Error::<T>::NotAMember);
            ensure!(
                !T::MinisterChecker::is_active_minister(&who),
                Error::<T>::MinisterCannotVote
            );
            ensure!(
                !MotionVotes::<T>::contains_key((motion_id, who.clone())),
                Error::<T>::AlreadyVoted
            );

            Motions::<T>::try_mutate(motion_id, |maybe_motion| {
                let motion = maybe_motion.as_mut().ok_or(Error::<T>::MotionNotFound)?;
                ensure!(!motion.executed, Error::<T>::MotionAlreadyExecuted);
                let now = frame_system::Pallet::<T>::block_number();
                ensure!(now < motion.end_block, Error::<T>::VotingWindowClosed);

                if approve {
                    motion.ayes = motion.ayes.saturating_add(1);
                } else {
                    motion.nays = motion.nays.saturating_add(1);
                }
                Ok::<(), DispatchError>(())
            })?;

            MotionVotes::<T>::insert((motion_id, who.clone()), approve);
            Self::deposit_event(Event::VoteCast { motion_id, member: who, approve });
            Ok(())
        }

        /// Close a motion after its voting window has ended.
        /// Anyone may call this. If ayes * 100 >= PassageThreshold * total_members,
        /// the motion is marked executed and `MotionPassed` is emitted; otherwise `MotionFailed`.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn close_motion(origin: OriginFor<T>, motion_id: u32) -> DispatchResult {
            let _who = ensure_signed(origin)?;

            let motion = Motions::<T>::get(motion_id).ok_or(Error::<T>::MotionNotFound)?;
            ensure!(!motion.executed, Error::<T>::MotionAlreadyExecuted);

            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now >= motion.end_block, Error::<T>::MotionStillOpen);

            let total_members = Members::<T>::get().len() as u64;
            let threshold = T::PassageThreshold::get() as u64;
            // Use u64 arithmetic to avoid overflow with large member sets (>42M members
            // would overflow u32 when multiplied by 100).
            let passed = (motion.ayes as u64).saturating_mul(100)
                >= threshold.saturating_mul(total_members);

            Motions::<T>::try_mutate(motion_id, |maybe_motion| {
                let m = maybe_motion.as_mut().ok_or(Error::<T>::MotionNotFound)?;
                m.executed = true;
                Ok::<(), DispatchError>(())
            })?;

            if passed {
                // Refuse to overwrite an unconsumed approval token: the pending action must be
                // executed first, otherwise a second motion silently cancels the first.
                ensure!(
                    PendingLegislatureApproval::<T>::get().is_none(),
                    Error::<T>::ApprovalPending
                );
                PendingLegislatureApproval::<T>::put((motion.call_hash, motion.proposer.clone()));
                Self::deposit_event(Event::MotionPassed { motion_id, call_hash: motion.call_hash });
            } else {
                Self::deposit_event(Event::MotionFailed { motion_id });
            }

            Ok(())
        }
    }
}

/// Implement SeatLegislature so pallet-elections can install election winners.
impl<T: pallet::Config> pallet_elections::pallet::SeatLegislature<T::AccountId>
    for pallet::Pallet<T>
{
    fn replace_members(
        winners: alloc::vec::Vec<T::AccountId>,
    ) -> frame_support::pallet_prelude::DispatchResult {
        let bounded = frame_support::BoundedVec::<T::AccountId, T::MaxMembers>::try_from(winners)
            .map_err(|_| frame_support::pallet_prelude::DispatchError::Other(
                "election winners exceed MaxMembers",
            ))?;
        pallet::Members::<T>::put(bounded);
        Ok(())
    }
}
