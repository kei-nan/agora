//! # Legislature Pallet
//!
//! On-chain legislative collective for Agora. A fixed set of members propose and vote on
//! motions. When a motion reaches `PassageThreshold`% ayes after `MotionDurationBlocks`,
//! anyone can close it. A closed-passed motion emits `MotionPassed` and marks `executed = true`.
//! `PassageThreshold` is the *floor*: the minimum support (51%, matching the referendum path's
//! Ordinary tier) a motion must clear before its approval token is even planted. `close_motion`
//! freezes the tally (`ayes`, `total_members` at close time) into that token alongside the
//! call hash, so a consuming pallet can later demand more than the floor for calls that need it.
//!
//! `EnsureLegislatureMotion` is the `LegislatureOrigin` type consumed by pallet-constitution
//! and several other legislature-gated pallets (treasury-ledger, voting, executive, elections,
//! identity). It implements `EnsureOriginWithArg<RuntimeOrigin, [u8; 32]>`: the caller must
//! supply the hash of the exact call it is dispatching, and `try_origin` only succeeds if that
//! hash matches the `call_hash` the passed motion actually approved (see `EnsureLegislatureMotion`'s
//! own doc comment below for the full contract). This closes a prior HIGH-severity gap where
//! any passed motion's approval token could be replayed to authorize an unrelated call.
//!
//! It *also* implements a second overload, `EnsureOriginWithArg<RuntimeOrigin, ([u8; 32], u8)>`,
//! for pallets whose calls need more than the floor threshold depending on what they're actually
//! authorizing (tier-aware law enactment, in pallet-constitution's case). The `u8` is the
//! required passage percentage; the caller-pallet computes it from its own authentic, unspoofable
//! data (a call parameter that is itself bound into `call_hash`, or a direct on-chain storage
//! lookup) — never from a free-form value an untrusted party can pick. `try_origin` checks the
//! *frozen* `ayes`/`total_members` tally against that required percentage, so the threshold
//! actually enforced always matches the real thing being authorized, not whatever a proposer
//! claimed at propose time. See `pallet_constitution`'s call sites for a concrete example, and
//! that pallet's module doc comment for why this can't be gamed.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::WeightInfo;

#[frame_support::pallet]
pub mod pallet {

    use codec::{Decode, DecodeWithMemTracking, Encode};
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Saturating;
    use crate::weights::WeightInfo;

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
    /// the motion's proposer, the frozen `ayes`/`total_members` tally, and the block it was
    /// planted at) to storage. `EnsureLegislatureMotion::try_origin` consumes that token
    /// exactly once, and only if the caller-supplied hash argument matches it, so a motion
    /// that passed to authorize one call can never be replayed to execute a different one —
    /// including a different call in a different legislature-gated pallet. Any current
    /// legislature member may consume the token, not only the original proposer: the vote
    /// that passed the motion is what legitimizes the action, not the proposer's continued
    /// availability, and requiring the exact proposer created a permanent-deadlock risk if
    /// they went offline or were removed before executing it. If no member consumes it
    /// before `PendingApprovalExpiryBlocks` elapses, `clear_stale_approval` lets any member
    /// discard it so a new motion can pass. This is enforced with
    /// `EnsureOriginWithArg` (rather than plain `EnsureOrigin`) so the check lives inside
    /// the origin gate itself: a consuming call site cannot forget to verify the token,
    /// because there is no path to a successful origin without supplying a matching hash.
    /// Consuming pallets compute that hash from their own call's parameters,
    /// domain-separated by a pallet+call tag (see e.g. `pallet_constitution`'s
    /// `legislature_call_hash` helper) so byte-identical parameters can never collide
    /// across two different calls.
    ///
    /// Two `EnsureOriginWithArg` overloads are implemented below:
    ///   - `Arg = [u8; 32]` — the original, hash-only check. A token is usable here as long
    ///     as it exists at all (i.e. the motion cleared the pallet-wide `PassageThreshold`
    ///     floor at close time). Used by pallets whose calls don't need more than that floor
    ///     (treasury-ledger, executive, elections, identity, voting).
    ///   - `Arg = ([u8; 32], u8)` — hash *and* a required percentage. The token must also
    ///     meet that higher bar, checked against the tally frozen at close time. Used by
    ///     pallet-constitution to enforce tier-aware supermajorities (Structural/Foundational
    ///     laws need more than a bare Ordinary majority) — see that pallet's doc comments for
    ///     why the required-percentage argument can't be gamed by a proposer.
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
                    // Consume the pending approval token if the caller's hash matches
                    // the hash the motion actually passed for. Any current member may
                    // consume it — not only the original proposer, since `who` is
                    // already verified to be a current member above and it's the vote
                    // that legitimizes the action. A mismatched hash is rejected,
                    // preventing a passed motion for call A from being replayed against
                    // unrelated call B. The frozen tally is not re-checked here — a
                    // planted token already cleared the floor threshold at close time.
                    if let Some((approved_hash, _proposer, _ayes, _total, _planted_at)) =
                        PendingLegislatureApproval::<T>::get()
                    {
                        if approved_hash == *call_hash {
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
            // Plant a token so the benchmark-generated origin validates (100% tally, so
            // both overloads accept it regardless of required percentage).
            PendingLegislatureApproval::<T>::put((
                *call_hash,
                member.clone(),
                1u32,
                1u32,
                frame_system::Pallet::<T>::block_number(),
            ));
            Ok(frame_system::RawOrigin::Signed(member).into())
        }
    }

    impl<T: Config>
        frame_support::traits::EnsureOriginWithArg<T::RuntimeOrigin, ([u8; 32], u8)>
        for EnsureLegislatureMotion<T>
    {
        type Success = ();
        fn try_origin(
            o: T::RuntimeOrigin,
            arg: &([u8; 32], u8),
        ) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            let (call_hash, required_pct) = *arg;
            match o.clone().into() {
                Ok(RawOrigin::Signed(who)) if Members::<T>::get().contains(&who) => {
                    if let Some((approved_hash, _proposer, ayes, total, _planted_at)) =
                        PendingLegislatureApproval::<T>::get()
                    {
                        let meets_required = (ayes as u64).saturating_mul(100)
                            >= (required_pct as u64).saturating_mul(total as u64);
                        if approved_hash == call_hash && meets_required {
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
        fn try_successful_origin(arg: &([u8; 32], u8)) -> Result<T::RuntimeOrigin, ()> {
            let (call_hash, _required_pct) = *arg;
            let member = Members::<T>::get().first().cloned().ok_or(())?;
            // 100/100 satisfies any required percentage up to 100.
            PendingLegislatureApproval::<T>::put((
                call_hash,
                member.clone(),
                100u32,
                100u32,
                frame_system::Pallet::<T>::block_number(),
            ));
            Ok(frame_system::RawOrigin::Signed(member).into())
        }
    }

    /// Checks whether an account is currently an active executive minister.
    /// Implemented by pallet-executive; called by pallet-legislature to enforce
    /// the incompatibility rule (ministers cannot vote on legislature motions).
    pub trait MinisterChecker<AccountId> {
        fn is_active_minister(who: &AccountId) -> bool;
    }

    /// Checks whether an account currently sits on the Accountability Council.
    /// Implemented by pallet-accountability-council; called by pallet-legislature's
    /// bootstrap-phase `add_member` to enforce the same legislature/Council overlap bar
    /// that `pallet_elections::AccountabilityCouncilChecker` already enforces for
    /// post-bootstrap automatic seating (see `pallet_elections::SeatLegislature` and
    /// `add_member`'s doc comment). Same consumer-defines/provider-implements idiom as
    /// `MinisterChecker` above and `pallet_elections`/`pallet_executive`'s own
    /// `AccountabilityCouncilChecker` traits.
    pub trait AccountabilityCouncilChecker<AccountId> {
        fn is_current_member(who: &AccountId) -> bool;
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
        /// Floor percentage of *total members* that must vote aye for a motion to pass at all
        /// (e.g. 51, matching the referendum path's Ordinary tier). Evaluated at close time:
        /// ayes * 100 >= PassageThreshold * total_members. This is only the minimum a motion's
        /// approval token needs to be planted — a consuming pallet may demand more via the
        /// `([u8; 32], u8)` `EnsureOriginWithArg` overload on `EnsureLegislatureMotion` (see
        /// that type's doc comment); the frozen tally is re-checked there against whatever
        /// higher bar the specific call being authorized actually requires.
        #[pallet::constant]
        type PassageThreshold: Get<u8>;
        /// How many blocks an unconsumed `PendingLegislatureApproval` token may sit before any
        /// member can discard it via `clear_stale_approval`, unblocking the legislature from a
        /// proposer who never executes it (offline, lost key, or removed via `remove_member`).
        #[pallet::constant]
        type PendingApprovalExpiryBlocks: Get<u32>;
        /// Checks whether a member is an active executive minister.
        /// Ministers are blocked from voting on motions (incompatibility rule).
        type MinisterChecker: MinisterChecker<Self::AccountId>;
        /// Checks whether an account currently sits on the Accountability Council. Consulted
        /// by bootstrap-phase `add_member` so Root can't seat a sitting Council member into
        /// the legislature during the one-time bootstrap window — see
        /// `AccountabilityCouncilChecker`'s doc comment.
        type AccountabilityCouncilChecker: AccountabilityCouncilChecker<Self::AccountId>;
        /// Weight functions needed for this pallet's extrinsics.
        type WeightInfo: crate::weights::WeightInfo;
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

    /// Whether the legislature's bootstrap phase has been closed (see `close_bootstrap`). While
    /// `false`, `Root` may freely call `add_member`/`remove_member` — the bootstrapping path
    /// used to seed the initial legislature. Once `true`, both calls are refused unconditionally
    /// (`Error::BootstrapClosed`), including for `Root`: this closes a gap where a compromised
    /// sudo key could otherwise unilaterally pack or purge the legislature forever, not just
    /// during genesis bootstrap. Unlike `pallet_accountability_council::Bootstrapped` (which
    /// hands post-bootstrap membership control to the Council's own supermajority vote), there
    /// is deliberately no alternate `Root`-free path wired up here: the legislature's real
    /// ongoing membership mechanism is `pallet_elections`'s automatic, backing-driven seating via
    /// `SeatLegislature::replace_members` (see the impl at the bottom of this file), which does
    /// not read this flag at all and keeps working unaffected after bootstrap closes. `Root` can
    /// never flip this back to `false`; there is no call that does so.
    #[pallet::storage]
    pub type Bootstrapped<T: Config> = StorageValue<_, bool, ValueQuery>;

    /// Set by `close_motion` when a motion passes; consumed by `EnsureLegislatureMotion`.
    /// Stores `(call_hash, proposer, ayes, total_members, planted_at)` — any current
    /// legislature member may consume the token (see `EnsureLegislatureMotion`'s doc comment
    /// for why it's no longer restricted to the original proposer). `ayes`/`total_members` are
    /// the tally frozen at close time, so a consuming pallet's tier-aware
    /// `EnsureOriginWithArg<_, ([u8; 32], u8)>` check (see `EnsureLegislatureMotion`'s doc
    /// comment) can verify the *real* support a motion received meets whatever higher
    /// threshold the call being authorized actually requires. `planted_at` is the block the
    /// token was written, used by `clear_stale_approval` to discard a token nobody ever
    /// consumed. Cleared after it is consumed — each passed motion authorizes exactly one
    /// action.
    #[pallet::storage]
    pub type PendingLegislatureApproval<T: Config> = StorageValue<
        _,
        ([u8; 32], T::AccountId, u32, u32, BlockNumberFor<T>),
        OptionQuery,
    >;

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
        /// A `PendingLegislatureApproval` token expired unconsumed and was discarded via
        /// `clear_stale_approval`, freeing the legislature to pass a new motion.
        PendingApprovalExpired { call_hash: [u8; 32] },
        /// `Root` closed the bootstrap phase; `add_member`/`remove_member` can never be called
        /// successfully again (see `Bootstrapped`'s doc comment).
        BootstrapClosed,
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
        /// There is no pending approval token to clear.
        NoPendingApproval,
        /// The pending approval token has not yet reached `PendingApprovalExpiryBlocks`.
        ApprovalNotYetStale,
        /// `add_member`/`remove_member` was called after `close_bootstrap` — or `close_bootstrap`
        /// itself was called a second time. Once closed, the bootstrap phase can never reopen;
        /// see `Bootstrapped`'s doc comment.
        BootstrapClosed,
        /// `close_bootstrap` was called with no members yet seated.
        NoMembersToBootstrap,
        /// The account currently sits on the Accountability Council — barred from bootstrap-
        /// phase legislature membership (the reverse of the join-time check
        /// `pallet_accountability_council::add_member` already performs). Mirrors
        /// `pallet_elections`'s `AccountabilityCouncilMember` skip at post-bootstrap automatic
        /// seating. See `AccountabilityCouncilChecker`'s doc comment.
        AccountabilityCouncilMember,
    }

    // ── Calls ────────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add a member to the legislature. Root-only, and only while the bootstrap phase is
        /// still open (`Bootstrapped == false`) — see `Bootstrapped`'s doc comment and
        /// `close_bootstrap`. Once bootstrap is closed this call always fails, even for `Root`.
        /// Also refuses a sitting Accountability Council member, matching the overlap bar
        /// `pallet_elections`'s post-bootstrap automatic seating already enforces — see
        /// `AccountabilityCouncilChecker`'s doc comment.
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::add_member())]
        pub fn add_member(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(!Bootstrapped::<T>::get(), Error::<T>::BootstrapClosed);
            ensure!(
                !T::AccountabilityCouncilChecker::is_current_member(&who),
                Error::<T>::AccountabilityCouncilMember
            );
            Members::<T>::try_mutate(|members| {
                ensure!(!members.contains(&who), Error::<T>::AlreadyMember);
                members.try_push(who.clone()).map_err(|_| Error::<T>::MembersAtCapacity)
            })?;
            Self::deposit_event(Event::MemberAdded { who });
            Ok(())
        }

        /// Remove a member from the legislature. Root-only, same bootstrap gate as `add_member`.
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::remove_member())]
        pub fn remove_member(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(!Bootstrapped::<T>::get(), Error::<T>::BootstrapClosed);
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
        #[pallet::weight(T::WeightInfo::propose_motion())]
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
        #[pallet::weight(T::WeightInfo::vote_motion())]
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
        /// `MotionPassed` only means the motion cleared the floor (`PassageThreshold`) — the
        /// `ayes`/`total_members` tally is frozen into the approval token, and a consuming
        /// pallet's call may additionally require a higher, call-specific percentage of that
        /// same tally at authorization time (see `EnsureLegislatureMotion`'s doc comment).
        #[pallet::call_index(4)]
        #[pallet::weight(T::WeightInfo::close_motion())]
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
                PendingLegislatureApproval::<T>::put((
                    motion.call_hash,
                    motion.proposer.clone(),
                    motion.ayes,
                    total_members as u32,
                    now,
                ));
                Self::deposit_event(Event::MotionPassed { motion_id, call_hash: motion.call_hash });
            } else {
                Self::deposit_event(Event::MotionFailed { motion_id });
            }

            Ok(())
        }

        /// Discard an unconsumed `PendingLegislatureApproval` token once
        /// `PendingApprovalExpiryBlocks` have passed since it was planted. Open to any current
        /// legislature member — recovers the legislature from a proposer (or every consuming
        /// member) never executing the queued action, which would otherwise block every future
        /// motion from passing (`close_motion` refuses to overwrite a pending token).
        #[pallet::call_index(5)]
        #[pallet::weight(T::WeightInfo::clear_stale_approval())]
        pub fn clear_stale_approval(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Members::<T>::get().contains(&who), Error::<T>::NotAMember);

            let (call_hash, _proposer, _ayes, _total, planted_at) =
                PendingLegislatureApproval::<T>::get().ok_or(Error::<T>::NoPendingApproval)?;
            let now = frame_system::Pallet::<T>::block_number();
            let expiry = BlockNumberFor::<T>::from(T::PendingApprovalExpiryBlocks::get());
            ensure!(
                now >= planted_at.saturating_add(expiry),
                Error::<T>::ApprovalNotYetStale
            );

            PendingLegislatureApproval::<T>::kill();
            Self::deposit_event(Event::PendingApprovalExpired { call_hash });
            Ok(())
        }

        /// `Root`-only, one-time: closes the legislature's bootstrap phase. Requires at least
        /// one member already seated. After this, `add_member`/`remove_member` can never be
        /// called successfully again by anyone, including `Root` — see `Bootstrapped`'s doc
        /// comment. `pallet_elections`'s automatic seating (`SeatLegislature::replace_members`,
        /// implemented at the bottom of this file) is unaffected and remains the ongoing
        /// membership mechanism after this point.
        #[pallet::call_index(6)]
        #[pallet::weight(T::WeightInfo::close_bootstrap())]
        pub fn close_bootstrap(origin: OriginFor<T>) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(!Bootstrapped::<T>::get(), Error::<T>::BootstrapClosed);
            ensure!(!Members::<T>::get().is_empty(), Error::<T>::NoMembersToBootstrap);
            Bootstrapped::<T>::put(true);
            Self::deposit_event(Event::BootstrapClosed);
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
