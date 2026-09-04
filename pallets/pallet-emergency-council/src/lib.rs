//! # Emergency Council Pallet
//!
//! The Emergency Council holds time-limited emergency powers with a constitutionally
//! hard-coded sunset clause. This prevents tyranny: emergency powers auto-expire and
//! cannot exceed a maximum duration (`MaxEmergencyBlocks`).
//!
//! ## Flow
//! 1. Root adds council members via `add_council_member`.
//! 2. Any council member calls `vote_declare_emergency(reason_hash, duration_blocks)`.
//! 3. When a supermajority of council members have voted, the emergency activates.
//! 4. The emergency auto-expires at `expires_at` (checked in `on_initialize`).
//! 5. Council members may also vote to end the emergency early via `vote_end_emergency`.
//!
//! ## Constraints
//! - Only one active emergency at a time (`AlreadyActiveEmergency`).
//! - `duration_blocks` is clamped to `MaxEmergencyBlocks` (the constitutional ceiling).
//! - Supermajority threshold: `votes * SupermajorityDenominator >= council_size * SupermajorityNumerator`.
//! - A new emergency cannot be declared within `EmergencyCooldownBlocks` of the previous one
//!   ending (whether by auto-sunset expiry or early `vote_end_emergency`). Without this, the
//!   same supermajority that can declare an emergency could re-declare a fresh one the block
//!   after the previous one ends, chaining into de-facto indefinite emergency powers despite
//!   each individual window being honestly capped by `MaxEmergencyBlocks`.
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

    // ── EmergencyInfo struct ─────────────────────────────────────────────────

    #[derive(Clone, Encode, Decode, MaxEncodedLen, TypeInfo, DecodeWithMemTracking)]
    pub struct EmergencyInfo<BlockNumber, AccountId> {
        /// Block at which the emergency was declared.
        pub declared_at: BlockNumber,
        /// Block at which the emergency expires (auto-cleared by on_initialize).
        pub expires_at: BlockNumber,
        /// IPFS hash of the reason document stored off-chain.
        pub reason_hash: [u8; 32],
        /// Number of council members who voted to declare this emergency.
        pub votes_to_declare: u32,
        /// Running count of council members who have voted to end this emergency early.
        pub votes_to_end: u32,
        /// PhantomData to bind AccountId without storing it directly.
        #[codec(skip)]
        _phantom: core::marker::PhantomData<AccountId>,
    }

    // ── Pallet ───────────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Origin ───────────────────────────────────────────────────────────────

    /// Origin that succeeds only when both hold: the underlying origin is `Root`, *and* an
    /// emergency is currently active (`ActiveEmergency::<T>::get().is_some()`). Mirrors the
    /// structural pattern of `pallet_legislature::EnsureLegislatureMotion` (a marker struct
    /// generic over `T`, implementing the relevant `EnsureOrigin*` trait, with a
    /// `#[cfg(feature = "runtime-benchmarks")] fn try_successful_origin` for benchmarking) but
    /// needs no extra argument via `EnsureOriginWithArg`: unlike a legislature motion, which
    /// must be tied to a specific approved call, "is an emergency active right now" is a
    /// single global fact read directly from storage, not a per-call approval token.
    ///
    /// This is deliberately layered on top of `Root`, not a replacement for it. An origin that
    /// accepted *any* signed (or unsigned) caller as long as `ActiveEmergency` were `Some`
    /// would let arbitrary accounts force an OPRF rotation during a real emergency — a *larger*
    /// attack surface than the bare-root gate it replaces, not a smaller one. Requiring `Root`
    /// as well means the set of accounts that can ever succeed through this origin is a strict
    /// subset of who could succeed before (still gated by however `Root` is reached in this
    /// runtime); what changes is that even `Root` can no longer act unilaterally at will — it
    /// must wait for the Emergency Council to have genuinely declared (via its own
    /// council-membership + supermajority-vote path in `vote_declare_emergency`) an emergency
    /// that has not yet been lifted (`vote_end_emergency`) or auto-sunset-expired
    /// (`on_initialize`). There is no way to make `ActiveEmergency` read `Some` other than
    /// through that real declaration path — this pallet exposes no other writer of that
    /// storage item.
    pub struct EnsureActiveEmergency<T>(core::marker::PhantomData<T>);

    impl<T: Config> frame_support::traits::EnsureOrigin<T::RuntimeOrigin>
        for EnsureActiveEmergency<T>
    {
        type Success = ();

        fn try_origin(o: T::RuntimeOrigin) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            match o.clone().into() {
                Ok(RawOrigin::Root) if ActiveEmergency::<T>::get().is_some() => Ok(()),
                _ => Err(o),
            }
        }

        #[cfg(feature = "runtime-benchmarks")]
        fn try_successful_origin() -> Result<T::RuntimeOrigin, ()> {
            // Plant a minimal active emergency so the benchmark-generated Root origin
            // validates, mirroring how `EnsureLegislatureMotion::try_successful_origin`
            // plants a `PendingLegislatureApproval` token for its own benchmarks.
            let now = frame_system::Pallet::<T>::block_number();
            ActiveEmergency::<T>::put(EmergencyInfo {
                declared_at: now,
                expires_at: now
                    .saturating_add(BlockNumberFor::<T>::from(T::MaxEmergencyBlocks::get())),
                reason_hash: [0u8; 32],
                votes_to_declare: 0,
                votes_to_end: 0,
                _phantom: core::marker::PhantomData,
            });
            Ok(frame_system::RawOrigin::Root.into())
        }
    }

    // ── Cross-pallet cooldown coordination with pallet-executive ────────────────

    /// Lets this pallet coordinate its `CooldownUntil` cooldown with pallet-executive's
    /// independent, cabinet-level emergency mechanism (see this pallet's module doc comment).
    /// Without this, a coalition controlling both bodies could alternate — declare an
    /// emergency here, let it lapse (starting *this* pallet's cooldown), then immediately
    /// declare a fresh one via pallet-executive (whose cooldown was never touched), and vice
    /// versa — producing a near-unbroken declared-emergency state neither pallet's own
    /// intra-pallet cooldown fix anticipated.
    ///
    /// Consumer-defines/provider-implements idiom, as `pallet_elections::DisclosureChecker`
    /// established — with one difference: that relationship is one-directional (only
    /// pallet-elections needs a checker), but this one is symmetric (each pallet needs to both
    /// read and notify the other), so having each pallet implement this trait directly on its
    /// own `Pallet<T>` in both directions would require each pallet's crate to depend on the
    /// other's — a dependency cycle. Instead, `runtime/src/configs/mod.rs` implements this
    /// trait (and pallet-executive's own mirror-image `SiblingEmergencyCooldown` trait)
    /// directly on `Runtime`, delegating into the sibling pallet's own `CooldownUntil` storage
    /// item directly. That storage item is deliberately reused as-is rather than adding a new
    /// "cooldown imposed by the sibling" item: both pallets already treat `CooldownUntil`
    /// purely as "the block before which a fresh declaration is refused," with no other logic
    /// anywhere that distinguishes *why* it was set, so a shared meaning needs no separate
    /// storage slot.
    pub trait SiblingEmergencyCooldown<BlockNumber> {
        /// Whether pallet-executive currently considers itself in cooldown.
        fn is_in_cooldown(now: BlockNumber) -> bool;
        /// Start pallet-executive's own cooldown too. Called at the same point this pallet
        /// starts its own (see `on_initialize` and `vote_end_emergency`), so the two cooldowns
        /// always end together no matter which pallet's emergency actually ran.
        fn notify_emergency_ended(now: BlockNumber);
    }

    /// No-op implementation used by this pallet's own mock (`mock.rs`) so its unit tests don't
    /// need to know pallet-executive exists at all. The real runtime wires the real
    /// `Runtime`-level implementation described above instead.
    impl<BlockNumber> SiblingEmergencyCooldown<BlockNumber> for () {
        fn is_in_cooldown(_now: BlockNumber) -> bool {
            false
        }
        fn notify_emergency_ended(_now: BlockNumber) {}
    }

    // ── Config ───────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Hard constitutional maximum for an emergency duration in blocks.
        /// Set to 216_000 (30 days at this chain's actual 12s/block time) in runtime — not
        /// 432_000 (which would be 30 days at 6s/block, this pallet's own stale prior claim).
        /// See `runtime/src/configs/mod.rs`'s `MaxEmergencyBlocks` doc comment for the full
        /// correction.
        #[pallet::constant]
        type MaxEmergencyBlocks: Get<u32>;

        /// Minimum blocks that must pass after an emergency ends (sunset expiry, or early
        /// `vote_end_emergency`) before the council can declare another one. Closes the gap
        /// where the same supermajority could otherwise chain back-to-back emergencies into
        /// indefinite emergency powers.
        #[pallet::constant]
        type EmergencyCooldownBlocks: Get<u32>;

        /// Maximum council size.
        #[pallet::constant]
        type MaxCouncilSize: Get<u32>;

        /// Numerator of the supermajority fraction needed to act (e.g. 2 for 2/3).
        #[pallet::constant]
        type SupermajorityNumerator: Get<u32>;

        /// Denominator of the supermajority fraction (e.g. 3 for 2/3).
        #[pallet::constant]
        type SupermajorityDenominator: Get<u32>;
        /// See `SiblingEmergencyCooldown`'s doc comment above.
        type SiblingEmergencyCooldown: SiblingEmergencyCooldown<BlockNumberFor<Self>>;
        /// Weight functions needed for this pallet's extrinsics.
        type WeightInfo: crate::weights::WeightInfo;
    }

    // ── Storage ──────────────────────────────────────────────────────────────

    /// Current council members.
    #[pallet::storage]
    pub type Council<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxCouncilSize>, ValueQuery>;

    /// Active emergency, if any.
    #[pallet::storage]
    pub type ActiveEmergency<T: Config> =
        StorageValue<_, EmergencyInfo<BlockNumberFor<T>, T::AccountId>, OptionQuery>;

    /// Which council members have voted to declare the current emergency (reset each new emergency).
    #[pallet::storage]
    pub type DeclareVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    /// Which council members have voted to end the current emergency early.
    #[pallet::storage]
    pub type EndVotes<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    /// Proposal terms locked in by the first council member to vote for an emergency.
    /// Prevents a decisive late voter from overriding the agreed-upon reason or duration.
    #[pallet::storage]
    pub type PendingEmergencyProposal<T: Config> = StorageValue<_, ([u8; 32], u32), OptionQuery>;

    /// Whether the Emergency Council's bootstrap phase has been closed (see `close_bootstrap`).
    /// While `false`, `Root` may freely call `add_council_member`/`remove_council_member` — the
    /// bootstrapping path used to seed the initial council. Once `true`, both calls are refused
    /// unconditionally (`Error::BootstrapClosed`), including for `Root`: this closes a gap where
    /// a compromised sudo key could otherwise unilaterally pack or purge the Emergency Council
    /// forever, not just during genesis bootstrap. Unlike
    /// `pallet_accountability_council::Bootstrapped`, there is deliberately no alternate
    /// `Root`-free path to change membership after this point — this pallet has no self-
    /// governance mechanism of its own (its supermajority votes govern declaring/ending an
    /// emergency, not council composition), so once closed, this council's membership is frozen
    /// for good. `Root` can never flip this back to `false`; there is no call that does so.
    #[pallet::storage]
    pub type Bootstrapped<T: Config> = StorageValue<_, bool, ValueQuery>;

    /// Block number before which a new emergency cannot be declared. Set to
    /// `now + EmergencyCooldownBlocks` whenever an emergency ends, by any path (sunset
    /// expiry in `on_initialize`, or early `vote_end_emergency`). Defaults to zero, so the
    /// very first emergency this pallet ever sees is never blocked by it.
    #[pallet::storage]
    pub type CooldownUntil<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    // ── Hooks ────────────────────────────────────────────────────────────────

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        /// Auto-expire an active emergency when `expires_at <= n`.
        fn on_initialize(n: BlockNumberFor<T>) -> Weight {
            // Always charge at minimum for reading ActiveEmergency.
            let mut weight = T::DbWeight::get().reads(1);
            if let Some(info) = ActiveEmergency::<T>::get() {
                if info.expires_at <= n {
                    ActiveEmergency::<T>::kill();
                    // Clear both vote maps so they don't linger into the next emergency.
                    // Without clearing DeclareVotes here, members who voted for the expired
                    // emergency would receive AlreadyVotedToDeclare on the next declaration.
                    let _ = DeclareVotes::<T>::clear(u32::MAX, None);
                    let _ = EndVotes::<T>::clear(u32::MAX, None);
                    PendingEmergencyProposal::<T>::kill();
                    CooldownUntil::<T>::put(
                        n.saturating_add(BlockNumberFor::<T>::from(T::EmergencyCooldownBlocks::get())),
                    );
                    // Also start pallet-executive's cooldown — see `SiblingEmergencyCooldown`'s
                    // doc comment for why.
                    T::SiblingEmergencyCooldown::notify_emergency_ended(n);
                    Self::deposit_event(Event::EmergencyExpired { at_block: n });
                    weight = weight.saturating_add(T::DbWeight::get().writes(4));
                }
            }
            weight
        }
    }

    // ── Events ───────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// An emergency was declared by supermajority vote.
        EmergencyDeclared { expires_at: BlockNumberFor<T>, reason_hash: [u8; 32] },
        /// An emergency was lifted early by supermajority vote.
        EmergencyLifted,
        /// An emergency expired naturally at the sunset block.
        EmergencyExpired { at_block: BlockNumberFor<T> },
        /// A new member was added to the Emergency Council.
        CouncilMemberAdded { who: T::AccountId },
        /// A member was removed from the Emergency Council.
        CouncilMemberRemoved { who: T::AccountId },
        /// `Root` closed the bootstrap phase; `add_council_member`/`remove_council_member` can
        /// never be called successfully again (see `Bootstrapped`'s doc comment).
        BootstrapClosed,
    }

    // ── Errors ───────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// The caller is not a member of the Emergency Council.
        NotCouncilMember,
        /// This council member has already voted to declare the current emergency.
        AlreadyVotedToDeclare,
        /// This council member has already voted to end the current emergency.
        AlreadyVotedToEnd,
        /// There is no active emergency to act on.
        NoActiveEmergency,
        /// Cannot declare a new emergency while one is already active.
        AlreadyActiveEmergency,
        /// Cannot declare a new emergency until `EmergencyCooldownBlocks` have passed since
        /// the previous one ended.
        EmergencyCooldownActive,
        /// `vote_declare_emergency`: a `PendingEmergencyProposal` already exists (from an
        /// earlier voter) and this caller's own `reason_hash`/`duration_blocks` arguments
        /// don't match its locked-in terms. A vote only counts toward the specific terms the
        /// voter actually submitted — see `vote_declare_emergency`'s doc comment.
        EmergencyProposalMismatch,
        /// Cannot add member: council is at maximum capacity.
        CouncilAtCapacity,
        /// The account is not in the council list.
        MemberNotFound,
        /// The account is already a council member.
        AlreadyCouncilMember,
        /// `add_council_member`/`remove_council_member` was called after `close_bootstrap` — or
        /// `close_bootstrap` itself was called a second time. Once closed, the bootstrap phase
        /// can never reopen; see `Bootstrapped`'s doc comment.
        BootstrapClosed,
        /// `close_bootstrap` was called with no members yet seated.
        NoMembersToBootstrap,
    }

    // ── Calls ────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add a member to the Emergency Council. Root only, and only while the bootstrap phase
        /// is still open (`Bootstrapped == false`) — see `Bootstrapped`'s doc comment and
        /// `close_bootstrap`. Once bootstrap is closed this call always fails, even for `Root`.
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::add_council_member())]
        pub fn add_council_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(!Bootstrapped::<T>::get(), Error::<T>::BootstrapClosed);
            Council::<T>::try_mutate(|members| {
                ensure!(!members.contains(&account), Error::<T>::AlreadyCouncilMember);
                members.try_push(account.clone()).map_err(|_| Error::<T>::CouncilAtCapacity)
            })?;
            Self::deposit_event(Event::CouncilMemberAdded { who: account });
            Ok(())
        }

        /// Remove a member from the Emergency Council. Root only, same bootstrap gate as
        /// `add_council_member`.
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::remove_council_member())]
        pub fn remove_council_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(!Bootstrapped::<T>::get(), Error::<T>::BootstrapClosed);
            Council::<T>::try_mutate(|members| {
                let pos = members
                    .iter()
                    .position(|m| m == &account)
                    .ok_or(Error::<T>::MemberNotFound)?;
                members.remove(pos);
                Ok::<(), DispatchError>(())
            })?;
            // Clear any pending declare/end votes from this member to keep state clean.
            DeclareVotes::<T>::remove(&account);
            EndVotes::<T>::remove(&account);
            Self::deposit_event(Event::CouncilMemberRemoved { who: account });
            Ok(())
        }

        /// Vote to declare an emergency. Any council member may call.
        ///
        /// `duration_blocks` is clamped to `MaxEmergencyBlocks` before use.
        /// When a supermajority of council members have voted, the emergency activates
        /// and all DeclareVotes / EndVotes are reset.
        #[pallet::call_index(2)]
        #[pallet::weight(T::WeightInfo::vote_declare_emergency())]
        pub fn vote_declare_emergency(
            origin: OriginFor<T>,
            reason_hash: [u8; 32],
            duration_blocks: u32,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let council = Council::<T>::get();
            ensure!(council.contains(&who), Error::<T>::NotCouncilMember);
            ensure!(ActiveEmergency::<T>::get().is_none(), Error::<T>::AlreadyActiveEmergency);
            let now = frame_system::Pallet::<T>::block_number();
            ensure!(now >= CooldownUntil::<T>::get(), Error::<T>::EmergencyCooldownActive);
            ensure!(
                !T::SiblingEmergencyCooldown::is_in_cooldown(now),
                Error::<T>::EmergencyCooldownActive
            );
            ensure!(!DeclareVotes::<T>::get(&who), Error::<T>::AlreadyVotedToDeclare);

            // Lock in the proposal terms from the first vote. A subsequent voter's own
            // `reason_hash`/`duration_blocks` must exactly match those locked-in terms — their
            // vote only counts toward what they actually submitted, not silently toward
            // whatever the first caller happened to propose (which they may never have seen).
            // A mismatch is rejected outright rather than silently discarding their arguments.
            let (agreed_reason, agreed_duration) = match PendingEmergencyProposal::<T>::get() {
                None => {
                    PendingEmergencyProposal::<T>::put((reason_hash, duration_blocks));
                    (reason_hash, duration_blocks)
                }
                Some((pending_reason, pending_duration)) => {
                    ensure!(
                        pending_reason == reason_hash && pending_duration == duration_blocks,
                        Error::<T>::EmergencyProposalMismatch
                    );
                    (pending_reason, pending_duration)
                }
            };
            let clamped = agreed_duration.min(T::MaxEmergencyBlocks::get());

            DeclareVotes::<T>::insert(&who, true);

            // Count votes cast so far (including this one, just inserted above).
            let vote_count = council
                .iter()
                .filter(|m| DeclareVotes::<T>::get(m))
                .count() as u32;

            if Self::supermajority_reached(vote_count, council.len() as u32) {
                // Activate emergency using the agreed-upon (first-voter's) terms.
                let expires_at = now.saturating_add(BlockNumberFor::<T>::from(clamped));

                let info = EmergencyInfo {
                    declared_at: now,
                    expires_at,
                    reason_hash: agreed_reason,
                    votes_to_declare: vote_count,
                    votes_to_end: 0,
                    _phantom: core::marker::PhantomData,
                };
                ActiveEmergency::<T>::put(info);

                // Proposal consumed; clear it. Reset both vote maps.
                PendingEmergencyProposal::<T>::kill();
                let _ = DeclareVotes::<T>::clear(u32::MAX, None);
                let _ = EndVotes::<T>::clear(u32::MAX, None);

                Self::deposit_event(Event::EmergencyDeclared { expires_at, reason_hash: agreed_reason });
            }
            Ok(())
        }

        /// Vote to end the current emergency early. Any council member may call.
        ///
        /// When a supermajority is reached, the emergency is cleared immediately.
        #[pallet::call_index(3)]
        #[pallet::weight(T::WeightInfo::vote_end_emergency())]
        pub fn vote_end_emergency(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let council = Council::<T>::get();
            ensure!(council.contains(&who), Error::<T>::NotCouncilMember);
            ensure!(ActiveEmergency::<T>::get().is_some(), Error::<T>::NoActiveEmergency);
            ensure!(!EndVotes::<T>::get(&who), Error::<T>::AlreadyVotedToEnd);

            EndVotes::<T>::insert(&who, true);

            let vote_count = council
                .iter()
                .filter(|m| EndVotes::<T>::get(m))
                .count() as u32;

            if Self::supermajority_reached(vote_count, council.len() as u32) {
                ActiveEmergency::<T>::kill();
                let _ = EndVotes::<T>::clear(u32::MAX, None);
                PendingEmergencyProposal::<T>::kill();
                let now = frame_system::Pallet::<T>::block_number();
                CooldownUntil::<T>::put(
                    now.saturating_add(BlockNumberFor::<T>::from(T::EmergencyCooldownBlocks::get())),
                );
                // Also start pallet-executive's cooldown — see `SiblingEmergencyCooldown`'s
                // doc comment for why.
                T::SiblingEmergencyCooldown::notify_emergency_ended(now);
                Self::deposit_event(Event::EmergencyLifted);
            }
            Ok(())
        }

        /// `Root`-only, one-time: closes the Emergency Council's bootstrap phase. Requires at
        /// least one member already seated. After this, `add_council_member`/
        /// `remove_council_member` can never be called successfully again by anyone, including
        /// `Root` — see `Bootstrapped`'s doc comment. This pallet has no post-bootstrap
        /// self-governance path for its own membership (unlike
        /// `pallet_accountability_council`): the Council's composition is frozen once closed.
        #[pallet::call_index(4)]
        #[pallet::weight(T::WeightInfo::close_bootstrap())]
        pub fn close_bootstrap(origin: OriginFor<T>) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(!Bootstrapped::<T>::get(), Error::<T>::BootstrapClosed);
            ensure!(!Council::<T>::get().is_empty(), Error::<T>::NoMembersToBootstrap);
            Bootstrapped::<T>::put(true);
            Self::deposit_event(Event::BootstrapClosed);
            Ok(())
        }
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Returns true if `votes` meets the configured supermajority threshold.
        ///
        /// Formula: `votes * SupermajorityDenominator >= council_size * SupermajorityNumerator`
        /// Default config: 2/3 supermajority → `votes * 3 >= council_size * 2`.
        fn supermajority_reached(votes: u32, council_size: u32) -> bool {
            if council_size == 0 {
                return false;
            }
            votes.saturating_mul(T::SupermajorityDenominator::get())
                >= council_size.saturating_mul(T::SupermajorityNumerator::get())
        }
    }
}
