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

        /// Maximum council size.
        #[pallet::constant]
        type MaxCouncilSize: Get<u32>;

        /// Numerator of the supermajority fraction needed to act (e.g. 2 for 2/3).
        #[pallet::constant]
        type SupermajorityNumerator: Get<u32>;

        /// Denominator of the supermajority fraction (e.g. 3 for 2/3).
        #[pallet::constant]
        type SupermajorityDenominator: Get<u32>;
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
                    Self::deposit_event(Event::EmergencyExpired { at_block: n });
                    weight = weight.saturating_add(T::DbWeight::get().writes(3));
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
        /// Cannot add member: council is at maximum capacity.
        CouncilAtCapacity,
        /// The account is not in the council list.
        MemberNotFound,
        /// The account is already a council member.
        AlreadyCouncilMember,
    }

    // ── Calls ────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add a member to the Emergency Council. Root only.
        #[pallet::call_index(0)]
        #[pallet::weight(T::WeightInfo::add_council_member())]
        pub fn add_council_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
            Council::<T>::try_mutate(|members| {
                ensure!(!members.contains(&account), Error::<T>::AlreadyCouncilMember);
                members.try_push(account.clone()).map_err(|_| Error::<T>::CouncilAtCapacity)
            })?;
            Self::deposit_event(Event::CouncilMemberAdded { who: account });
            Ok(())
        }

        /// Remove a member from the Emergency Council. Root only.
        #[pallet::call_index(1)]
        #[pallet::weight(T::WeightInfo::remove_council_member())]
        pub fn remove_council_member(origin: OriginFor<T>, account: T::AccountId) -> DispatchResult {
            ensure_root(origin)?;
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
            ensure!(!DeclareVotes::<T>::get(&who), Error::<T>::AlreadyVotedToDeclare);

            // Lock in the proposal terms from the first vote. Subsequent voters' args are
            // ignored so a decisive late voter cannot override the agreed-upon reason or duration.
            if PendingEmergencyProposal::<T>::get().is_none() {
                PendingEmergencyProposal::<T>::put((reason_hash, duration_blocks));
            }
            let (agreed_reason, agreed_duration) =
                PendingEmergencyProposal::<T>::get().unwrap_or((reason_hash, duration_blocks));
            let clamped = agreed_duration.min(T::MaxEmergencyBlocks::get());

            DeclareVotes::<T>::insert(&who, true);

            // Count votes cast so far (including this one, just inserted above).
            let vote_count = council
                .iter()
                .filter(|m| DeclareVotes::<T>::get(m))
                .count() as u32;

            if Self::supermajority_reached(vote_count, council.len() as u32) {
                // Activate emergency using the agreed-upon (first-voter's) terms.
                let now = frame_system::Pallet::<T>::block_number();
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
                Self::deposit_event(Event::EmergencyLifted);
            }
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
