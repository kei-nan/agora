//! # Elections Pallet
//!
//! Liquid Democracy Delegates + Legislature Elections: manages the public delegate registry
//! for vote delegation, and runs periodic elections to seat the top-N delegates (by backing
//! count) into pallet-legislature — entirely automatic, no committee or human certification
//! step anywhere in the flow.
//!
//! This pallet used to also run a separate "Elections Commission" subsystem (commissioners,
//! named "office" elections, candidate registration/certification, result submission).
//! Removed: it certified an election's outcome on nothing but a commissioner's say-so — there
//! was no on-chain tally behind `submit_results` at all — and nothing in this system's actual
//! design turned out to need a citizen-facing "elect one person to a named office" mechanism.
//! Legislature seats fill automatically via the backing mechanism below; the Prime Minister is
//! chosen by the legislature itself via pallet-executive's ranked-choice investiture. See
//! docs/project/changelog/ for the full removal rationale.
//!
//! ### Delegate identity
//! Separate from citizen identity: uses `Poseidon2(national_id || country_code || "delegate")`
//! as nullifier, so the citizen and delegate on-chain identities are cryptographically unlinked.
//! The delegate voluntarily publishes their real name; citizens remain anonymous.
//!
//! ### Backing threshold
//! A delegate becomes Active only when they have ≥ `BackingThreshold` citizen backers.
//! Each citizen may back at most `MaxBackingsPerCitizen` delegates (constitutional parameter,
//! default 5). This makes backing a meaningful signal rather than noise.
//!
//! ### Legislature elections
//! Every `ElectionCycleBlocks` blocks, `on_initialize` ranks all Active delegates by backing
//! count and seats the top `LegislatureSeats` into pallet-legislature via `SeatLegislature`.
//! All three parameters are constitutional (supermajority to change).
//! Defaults: 100 seats, 2-year cycle, max 5 backings per citizen.
//!
//! ### Term limits
//! All parameters are constitutional (supermajority to change):
//! - `TermLengthBlocks`: length of a single term.
//! - `MaxConsecutiveTerms`: max back-to-back terms before a mandatory break.
//! - `MandatoryBreakBlocks`: how long the break must last.
//! - `WarningWindowPct`: what fraction of the final term triggers a warning event (1–50 %).
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
    use codec::DecodeWithMemTracking;
    use frame_support::{
        pallet_prelude::*,
        traits::EnsureOriginWithArg,
    };
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{Saturating, Zero};
    use crate::weights::WeightInfo;

    /// Computes the domain-separated hash a legislature motion's `call_hash` must equal for
    /// `GovernanceOrigin` to authorize `tag`'s call with `params`. See
    /// `pallet_constitution::pallet::legislature_call_hash` for the full rationale.
    pub(crate) fn legislature_call_hash(tag: &'static [u8], params: impl Encode) -> [u8; 32] {
        let mut preimage = alloc::vec::Vec::from(tag);
        preimage.extend(params.encode());
        frame_support::Hashable::blake2_256(&preimage)
    }

    // ── Cross-pallet traits ────────────────────────────────────────────────────

    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
    }

    /// Called by pallet-elections at the end of each election cycle.
    /// The implementation in pallet-legislature replaces the full Members set.
    pub trait SeatLegislature<AccountId> {
        fn replace_members(winners: alloc::vec::Vec<AccountId>) -> DispatchResult;
    }

    /// Benchmark-only hook: makes an account satisfy `CitizenChecker::is_active_citizen` for
    /// extrinsics gated on citizen status (`register_candidate`, `register_as_delegate`,
    /// `back_delegate`). Real citizen registration goes through pallet-identity-zk's full
    /// ZK-proof flow, which a generic pallet-elections benchmark has no way to drive directly —
    /// this hook lets each runtime (or test mock) short-circuit that for benchmarking purposes
    /// only. See `weights.rs`'s module doc comment for which implementations wire this up.
    #[cfg(feature = "runtime-benchmarks")]
    pub trait BenchmarkHelper<AccountId> {
        fn make_active_citizen(who: &AccountId);
    }

    // ── Data types: Delegates ──────────────────────────────────────────────────

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub enum DelegateStatus {
        /// Registered but below the backing threshold — cannot yet receive delegations.
        Pending,
        /// At or above the backing threshold and within term limits — can receive delegations.
        Active,
        /// Served `MaxConsecutiveTerms` back-to-back; must wait until `break_until_block`.
        OnBreak,
    }

    #[derive(Clone, Debug, PartialEq, Eq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct DelegateInfo<BlockNumber> {
        /// Publicly visible real name.
        pub display_name: BoundedVec<u8, ConstU32<64>>,
        /// IPFS hash of the delegate's public profile / policy positions document.
        pub profile_ipfs_hash: [u8; 32],
        pub status: DelegateStatus,
        /// How many consecutive terms this delegate has served.
        pub consecutive_terms: u32,
        /// Block at which the current term started. None when Pending or OnBreak.
        pub term_start_block: Option<BlockNumber>,
        /// Block after which this delegate may return from a mandatory break.
        pub break_until_block: Option<BlockNumber>,
        /// Whether the warning event for the current term has already been emitted.
        pub warning_emitted: bool,
    }

    // ── Pallet struct ──────────────────────────────────────────────────────────

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Config ─────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Hard cap on the number of registered delegates.
        #[pallet::constant]
        type MaxDelegates: Get<u32>;

        /// Maximum number of `Delegates` entries `on_initialize` examines per block for term
        /// warnings/expirations and break-endings. Bounds per-block weight to a constant
        /// regardless of how many delegates are registered (up to `MaxDelegates`), instead of
        /// scanning the whole map every block — see `DelegateSweepCursor`.
        #[pallet::constant]
        type MaxDelegateSweepPerBlock: Get<u32>;

        type CitizenChecker: CitizenChecker<Self::AccountId>;

        /// Origin that can change `BackingThreshold` (ordinary supermajority governance).
        /// `EnsureOriginWithArg` so the call site must pass the domain-separated hash of its
        /// own parameters (see `legislature_call_hash`); the origin then verifies that hash
        /// against the motion's approved `call_hash`, so a motion passed to authorize an
        /// unrelated legislature-gated call elsewhere can never be replayed here.
        type GovernanceOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;

        /// Origin that can change constitutional parameters (supermajority + HRC veto).
        type ConstitutionalOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Cross-pallet hook: called at the end of each election cycle to install winners.
        type LegislatureSeating: SeatLegislature<Self::AccountId>;

        /// Default number of legislature seats (constitutional, default 100).
        #[pallet::constant]
        type DefaultLegislatureSeats: Get<u32>;

        /// Default election cycle length in blocks (constitutional, default 2 years).
        #[pallet::constant]
        type DefaultElectionCycleBlocks: Get<u32>;

        /// Default max number of delegates a citizen may back simultaneously (constitutional, default 5).
        #[pallet::constant]
        type DefaultMaxBackingsPerCitizen: Get<u32>;

        // ── Governance parameter defaults (stored in storage, changeable by governance) ──

        /// Genesis default for the minimum backer count required to hold Active status.
        #[pallet::constant]
        type DefaultBackingThreshold: Get<u32>;

        /// Genesis default for the backing threshold floor (governance may not lower below this).
        #[pallet::constant]
        type DefaultBackingThresholdFloor: Get<u32>;

        /// Genesis default for the backing threshold ceiling (governance may not raise above this).
        #[pallet::constant]
        type DefaultBackingThresholdCeiling: Get<u32>;

        /// Genesis default for a single delegate term length in blocks.
        #[pallet::constant]
        type DefaultTermLengthBlocks: Get<u32>;

        /// Genesis default for the maximum number of consecutive terms before a mandatory break.
        #[pallet::constant]
        type DefaultMaxConsecutiveTerms: Get<u32>;

        /// Genesis default for the mandatory break length in blocks.
        #[pallet::constant]
        type DefaultMandatoryBreakBlocks: Get<u32>;

        /// Genesis default for the warning window as a percentage of the term (1–50 %).
        #[pallet::constant]
        type DefaultWarningWindowPct: Get<u8>;

        /// Weight functions needed for this pallet's extrinsics.
        type WeightInfo: crate::weights::WeightInfo;

        /// See `BenchmarkHelper`'s doc comment.
        #[cfg(feature = "runtime-benchmarks")]
        type BenchmarkHelper: BenchmarkHelper<Self::AccountId>;
    }

    // ── Storage: Delegate registry ─────────────────────────────────────────────

    /// All registered delegates.
    #[pallet::storage]
    pub type Delegates<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, DelegateInfo<BlockNumberFor<T>>>;

    /// Resume point for `on_initialize`'s per-block sweep of `Delegates` (term warnings,
    /// expirations, break-endings): the account *after* which the next block's sweep should
    /// resume, or `None` to start from the beginning of the map. Set to `None` whenever a
    /// sweep reaches the end of the map, so the next block wraps back to the start — this
    /// bounds each block's work to `MaxDelegateSweepPerBlock` entries instead of the whole
    /// map, while still covering every delegate over a bounded number of blocks.
    #[pallet::storage]
    pub type DelegateSweepCursor<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

    /// Number of citizens currently backing each delegate.
    #[pallet::storage]
    pub type BackingCount<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// (backer, delegate) → () — prevents a citizen from backing the same delegate twice.
    #[pallet::storage]
    pub type BackingOf<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat, T::AccountId,
        Blake2_128Concat, T::AccountId,
        (),
    >;

    /// Number of delegates each citizen is currently backing.
    /// Enforced against MaxBackingsPerCitizen on every back_delegate call.
    #[pallet::storage]
    pub type CitizenBackingCount<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    // ── Storage: Governance-controlled parameters ──────────────────────────────

    #[pallet::type_value]
    pub fn DefaultBackingThresholdFn<T: Config>() -> u32 { T::DefaultBackingThreshold::get() }

    /// Minimum citizen backers required to hold Active delegate status.
    #[pallet::storage]
    pub type BackingThreshold<T: Config> = StorageValue<_, u32, ValueQuery, DefaultBackingThresholdFn<T>>;

    #[pallet::type_value]
    pub fn DefaultBackingThresholdFloorFn<T: Config>() -> u32 { T::DefaultBackingThresholdFloor::get() }

    #[pallet::storage]
    pub type BackingThresholdFloor<T: Config> = StorageValue<_, u32, ValueQuery, DefaultBackingThresholdFloorFn<T>>;

    #[pallet::type_value]
    pub fn DefaultBackingThresholdCeilingFn<T: Config>() -> u32 { T::DefaultBackingThresholdCeiling::get() }

    #[pallet::storage]
    pub type BackingThresholdCeiling<T: Config> = StorageValue<_, u32, ValueQuery, DefaultBackingThresholdCeilingFn<T>>;

    #[pallet::type_value]
    pub fn DefaultTermLengthBlocksFn<T: Config>() -> BlockNumberFor<T> {
        T::DefaultTermLengthBlocks::get().into()
    }

    #[pallet::storage]
    pub type TermLengthBlocks<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery, DefaultTermLengthBlocksFn<T>>;

    #[pallet::type_value]
    pub fn DefaultMaxConsecutiveTermsFn<T: Config>() -> u32 { T::DefaultMaxConsecutiveTerms::get() }

    #[pallet::storage]
    pub type MaxConsecutiveTerms<T: Config> = StorageValue<_, u32, ValueQuery, DefaultMaxConsecutiveTermsFn<T>>;

    #[pallet::type_value]
    pub fn DefaultMandatoryBreakBlocksFn<T: Config>() -> BlockNumberFor<T> {
        T::DefaultMandatoryBreakBlocks::get().into()
    }

    #[pallet::storage]
    pub type MandatoryBreakBlocks<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery, DefaultMandatoryBreakBlocksFn<T>>;

    #[pallet::type_value]
    pub fn DefaultWarningWindowPctFn<T: Config>() -> u8 { T::DefaultWarningWindowPct::get() }

    #[pallet::storage]
    pub type WarningWindowPct<T: Config> = StorageValue<_, u8, ValueQuery, DefaultWarningWindowPctFn<T>>;

    // ── Storage: Legislature election parameters (constitutional) ──────────────

    #[pallet::type_value]
    pub fn DefaultSeats<T: Config>() -> u32 { T::DefaultLegislatureSeats::get() }

    /// Number of legislature seats filled at each election. Constitutional.
    #[pallet::storage]
    pub type LegislatureSeats<T: Config> =
        StorageValue<_, u32, ValueQuery, DefaultSeats<T>>;

    #[pallet::type_value]
    pub fn DefaultCycle<T: Config>() -> u32 { T::DefaultElectionCycleBlocks::get() }

    /// Blocks between legislature elections. Constitutional.
    #[pallet::storage]
    pub type ElectionCycleBlocks<T: Config> =
        StorageValue<_, u32, ValueQuery, DefaultCycle<T>>;

    #[pallet::type_value]
    pub fn DefaultMaxBackings<T: Config>() -> u32 { T::DefaultMaxBackingsPerCitizen::get() }

    /// Max delegates a single citizen may back simultaneously. Constitutional.
    #[pallet::storage]
    pub type MaxBackingsPerCitizen<T: Config> =
        StorageValue<_, u32, ValueQuery, DefaultMaxBackings<T>>;

    /// Block at which the last legislature election ran. 0 = no election yet.
    /// First election fires at block ElectionCycleBlocks.
    #[pallet::storage]
    pub type LastElectionBlock<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    // ── Events ─────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        // ── Delegates ──
        DelegateRegistered { delegate: T::AccountId, display_name: BoundedVec<u8, ConstU32<64>> },
        DelegateActivated { delegate: T::AccountId },
        DelegateDeactivated { delegate: T::AccountId },
        DelegateBacked { delegate: T::AccountId, backer: T::AccountId },
        DelegateBackingRemoved { delegate: T::AccountId, backer: T::AccountId },
        DelegateTermWarning { delegate: T::AccountId, blocks_remaining: BlockNumberFor<T> },
        DelegateTermExpired { delegate: T::AccountId },
        DelegateBreakEnded { delegate: T::AccountId },

        // ── Legislature elections ──
        /// Periodic election ran; `seated` delegates installed into the legislature.
        LegislatureElectionRun { at_block: BlockNumberFor<T>, seated: u32 },
        /// Constitutional election parameters were updated.
        ElectionParamsChanged { seats: u32, cycle_blocks: u32, max_backings_per_citizen: u32 },

        // ── Governance parameters ──
        BackingThresholdChanged { new_threshold: u32 },
        TermParamsChanged {
            term_length: BlockNumberFor<T>,
            max_consecutive: u32,
            mandatory_break: BlockNumberFor<T>,
            warning_pct: u8,
        },
        BackingBoundsChanged { floor: u32, ceiling: u32 },
    }

    // ── Errors ─────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        NotActiveCitizen,
        AlreadyRegisteredAsDelegate,
        DelegateNotFound,
        AlreadyBacking,
        NotBacking,
        CannotBackSelf,
        DelegateOnBreak,
        BackingThresholdOutOfBounds,
        WarningPctInvalid,
        FloorExceedsCeiling,
        ThresholdBelowFloor,
        ThresholdAboveCeiling,
        /// Citizen has reached MaxBackingsPerCitizen and cannot back more delegates.
        BackingLimitReached,
        /// Legislature seat count must be at least 1.
        ElectionSeatsZero,
        /// Election cycle length cannot be zero — elections would never run.
        ElectionCycleBlocksZero,
    }

    // ── on_initialize: term warnings, expirations, and legislature elections ───

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_initialize(now: BlockNumberFor<T>) -> Weight {
            let mut weight = Weight::zero();

            // ── Legislature election cycle ──────────────────────────────────────
            let last = LastElectionBlock::<T>::get();
            let cycle: BlockNumberFor<T> = ElectionCycleBlocks::<T>::get().into();
            if !cycle.is_zero() && !now.is_zero() && now >= last.saturating_add(cycle) {
                weight = weight.saturating_add(Self::run_election(now));
            }

            // ── Term warnings and expirations ───────────────────────────────────
            let term_length = TermLengthBlocks::<T>::get();
            let max_consecutive = MaxConsecutiveTerms::<T>::get();
            let break_blocks = MandatoryBreakBlocks::<T>::get();
            let warning_pct = WarningWindowPct::<T>::get();

            // warning_offset = term_length * (100 - warning_pct) / 100
            // Divide-first avoids u32 overflow for large term lengths (saturating_mul at u32::MAX
            // then dividing by 100 would fire the warning months too early for 9+ year terms).
            // Precision loss is at most (complement - 1) blocks — negligible vs. million-block terms.
            let hundred: BlockNumberFor<T> = 100u32.into();
            let complement: BlockNumberFor<T> = (100u32.saturating_sub(warning_pct as u32)).into();
            let warning_offset: BlockNumberFor<T> = (term_length / hundred) * complement;

            // Bounded sweep: examine at most `MaxDelegateSweepPerBlock` delegates this block,
            // resuming from wherever the last block's sweep left off (`DelegateSweepCursor`),
            // instead of iterating every registered delegate unconditionally every block. With
            // `MaxDelegates` in the thousands, an unbounded full-map scan every block is an
            // unbounded-weight griefing vector; this caps it to a constant while still covering
            // every delegate within `MaxDelegates / MaxDelegateSweepPerBlock` blocks.
            let batch_size = T::MaxDelegateSweepPerBlock::get() as usize;
            // A misconfigured zero batch size disables the sweep entirely rather than
            // falling back to an unbounded scan (which is exactly the griefing vector this
            // is meant to close).
            if batch_size == 0 {
                return weight;
            }

            let cursor = DelegateSweepCursor::<T>::get();
            weight = weight.saturating_add(T::DbWeight::get().reads(1));
            let sweep_iter = match &cursor {
                Some(key) => Delegates::<T>::iter_from_key(key.clone()),
                None => Delegates::<T>::iter(),
            };

            let mut examined = 0usize;
            let mut last_key = None;

            for (account, mut info) in sweep_iter {
                if examined >= batch_size {
                    break;
                }
                examined += 1;
                last_key = Some(account.clone());
                weight = weight.saturating_add(T::DbWeight::get().reads(1));

                match info.status {
                    DelegateStatus::Active => {
                        let term_start = match info.term_start_block {
                            Some(b) => b,
                            None => continue,
                        };
                        let blocks_elapsed = now.saturating_sub(term_start);

                        if !info.warning_emitted && blocks_elapsed >= warning_offset {
                            let blocks_remaining = term_length.saturating_sub(blocks_elapsed);
                            Self::deposit_event(Event::DelegateTermWarning {
                                delegate: account.clone(),
                                blocks_remaining,
                            });
                            info.warning_emitted = true;
                            Delegates::<T>::insert(&account, &info);
                            weight = weight.saturating_add(T::DbWeight::get().writes(1));
                        }

                        if blocks_elapsed >= term_length {
                            // Active delegates stay Active for the full term — no interruptions
                            // — so blocks_elapsed equals active time. Always count as full.
                            let half: BlockNumberFor<T> = 2u32.into();
                            let counts_as_full = blocks_elapsed >= term_length / half;

                            if counts_as_full {
                                info.consecutive_terms =
                                    info.consecutive_terms.saturating_add(1);
                            }

                            if info.consecutive_terms >= max_consecutive {
                                info.status = DelegateStatus::OnBreak;
                                info.break_until_block =
                                    Some(now.saturating_add(break_blocks));
                            } else {
                                info.term_start_block = Some(now);
                                info.warning_emitted = false;
                            }

                            Self::deposit_event(Event::DelegateTermExpired {
                                delegate: account.clone(),
                            });
                            Delegates::<T>::insert(&account, &info);
                            weight = weight.saturating_add(T::DbWeight::get().writes(1));
                        }
                        // No write in the steady-state case: elapsed blocks are derived
                        // from (now - term_start_block) on demand, no counter needed.
                    }
                    DelegateStatus::OnBreak => {
                        if let Some(until) = info.break_until_block {
                            if now >= until {
                                info.status = DelegateStatus::Pending;
                                info.consecutive_terms = 0;
                                info.term_start_block = None;
                                info.break_until_block = None;
                                info.warning_emitted = false;
                                Delegates::<T>::insert(&account, &info);
                                Self::deposit_event(Event::DelegateBreakEnded {
                                    delegate: account.clone(),
                                });
                                weight = weight.saturating_add(T::DbWeight::get().writes(1));

                                let count = BackingCount::<T>::get(&account);
                                if count >= BackingThreshold::<T>::get() {
                                    Self::activate_delegate(&account);
                                    weight =
                                        weight.saturating_add(T::DbWeight::get().writes(1));
                                }
                            }
                        }
                    }
                    DelegateStatus::Pending => {}
                }
            }

            // Fewer than a full batch means the sweep reached the end of the map this
            // block -- wrap back to the start next block. Otherwise resume right after the
            // last delegate we examined.
            if examined < batch_size {
                if cursor.is_some() {
                    DelegateSweepCursor::<T>::kill();
                    weight = weight.saturating_add(T::DbWeight::get().writes(1));
                }
            } else {
                DelegateSweepCursor::<T>::put(
                    last_key.expect("examined >= batch_size > 0 implies at least one entry seen"),
                );
                weight = weight.saturating_add(T::DbWeight::get().writes(1));
            }

            weight
        }
    }

    // ── Calls ──────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        // ── Delegate registry ──────────────────────────────────────────────────
        //
        // Removed: the Elections Commission subsystem (commissioners, named "office"
        // elections, candidate registration/certification, result submission/certification —
        // formerly call_index 0-6). It certified election outcomes on nothing but a
        // commissioner's say-so, with no on-chain tally behind it — see
        // docs/project/changelog/ for the removal rationale. Legislature seats are filled
        // entirely by the delegate/backing mechanism below (fully automatic, no committee),
        // and the Prime Minister is chosen by the legislature itself via
        // pallet-executive's ranked-choice investiture — neither needs a citizen-facing
        // "elect one person to a named office" mechanism, so nothing replaces this; it's
        // deleted, not rebuilt. call_index 0-6 are deliberately left unused rather than
        // reassigned (see `#[pallet::call_index]`'s docs — indices need not be contiguous).

        #[pallet::call_index(7)]
        #[pallet::weight(T::WeightInfo::register_as_delegate())]
        pub fn register_as_delegate(
            origin: OriginFor<T>,
            display_name: BoundedVec<u8, ConstU32<64>>,
            profile_ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::NotActiveCitizen);
            ensure!(!Delegates::<T>::contains_key(&who), Error::<T>::AlreadyRegisteredAsDelegate);
            Delegates::<T>::insert(&who, DelegateInfo {
                display_name: display_name.clone(),
                profile_ipfs_hash,
                status: DelegateStatus::Pending,
                consecutive_terms: 0,
                term_start_block: None,
                break_until_block: None,
                warning_emitted: false,
            });
            Self::deposit_event(Event::DelegateRegistered { delegate: who, display_name });
            Ok(())
        }

        /// Back a delegate. Each citizen may back at most `MaxBackingsPerCitizen` delegates.
        /// If this backing pushes the delegate to or above the threshold, they become Active.
        #[pallet::call_index(8)]
        #[pallet::weight(T::WeightInfo::back_delegate())]
        pub fn back_delegate(origin: OriginFor<T>, delegate: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::NotActiveCitizen);
            ensure!(who != delegate, Error::<T>::CannotBackSelf);
            let info = Delegates::<T>::get(&delegate).ok_or(Error::<T>::DelegateNotFound)?;
            ensure!(info.status != DelegateStatus::OnBreak, Error::<T>::DelegateOnBreak);
            ensure!(!BackingOf::<T>::contains_key(&who, &delegate), Error::<T>::AlreadyBacking);

            // Enforce per-citizen backing cap.
            let citizen_count = CitizenBackingCount::<T>::get(&who);
            ensure!(citizen_count < MaxBackingsPerCitizen::<T>::get(), Error::<T>::BackingLimitReached);

            BackingOf::<T>::insert(&who, &delegate, ());
            CitizenBackingCount::<T>::mutate(&who, |c| *c = c.saturating_add(1));
            let new_count = BackingCount::<T>::get(&delegate).saturating_add(1);
            BackingCount::<T>::insert(&delegate, new_count);
            Self::deposit_event(Event::DelegateBacked { delegate: delegate.clone(), backer: who });

            if new_count >= BackingThreshold::<T>::get()
                && Delegates::<T>::get(&delegate)
                    .map_or(false, |d| d.status == DelegateStatus::Pending)
            {
                Self::activate_delegate(&delegate);
            }
            Ok(())
        }

        /// Remove backing from a delegate. Frees one slot in the citizen's backing allowance.
        #[pallet::call_index(9)]
        #[pallet::weight(T::WeightInfo::remove_backing())]
        pub fn remove_backing(origin: OriginFor<T>, delegate: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(BackingOf::<T>::contains_key(&who, &delegate), Error::<T>::NotBacking);

            BackingOf::<T>::remove(&who, &delegate);
            CitizenBackingCount::<T>::mutate(&who, |c| *c = c.saturating_sub(1));
            let new_count = BackingCount::<T>::get(&delegate).saturating_sub(1);
            BackingCount::<T>::insert(&delegate, new_count);
            Self::deposit_event(Event::DelegateBackingRemoved {
                delegate: delegate.clone(), backer: who,
            });

            if new_count < BackingThreshold::<T>::get()
                && Delegates::<T>::get(&delegate)
                    .map_or(false, |d| d.status == DelegateStatus::Active)
            {
                Delegates::<T>::mutate(&delegate, |maybe| {
                    if let Some(d) = maybe { d.status = DelegateStatus::Pending; }
                });
                Self::deposit_event(Event::DelegateDeactivated { delegate });
            }
            Ok(())
        }

        // ── Governance: ordinary supermajority ────────────────────────────────

        #[pallet::call_index(10)]
        #[pallet::weight(T::WeightInfo::set_backing_threshold())]
        pub fn set_backing_threshold(origin: OriginFor<T>, threshold: u32) -> DispatchResult {
            T::GovernanceOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-elections::set_backing_threshold", threshold),
            )?;
            ensure!(threshold >= BackingThresholdFloor::<T>::get(), Error::<T>::ThresholdBelowFloor);
            ensure!(threshold <= BackingThresholdCeiling::<T>::get(), Error::<T>::ThresholdAboveCeiling);
            BackingThreshold::<T>::put(threshold);
            Self::deposit_event(Event::BackingThresholdChanged { new_threshold: threshold });
            Ok(())
        }

        // ── Governance: constitutional supermajority ──────────────────────────

        #[pallet::call_index(11)]
        #[pallet::weight(T::WeightInfo::set_backing_bounds())]
        pub fn set_backing_bounds(origin: OriginFor<T>, floor: u32, ceiling: u32) -> DispatchResult {
            T::ConstitutionalOrigin::ensure_origin(origin)?;
            ensure!(floor <= ceiling, Error::<T>::FloorExceedsCeiling);
            BackingThresholdFloor::<T>::put(floor);
            BackingThresholdCeiling::<T>::put(ceiling);
            let current = BackingThreshold::<T>::get();
            if current < floor { BackingThreshold::<T>::put(floor); }
            if current > ceiling { BackingThreshold::<T>::put(ceiling); }
            Self::deposit_event(Event::BackingBoundsChanged { floor, ceiling });
            Ok(())
        }

        #[pallet::call_index(12)]
        #[pallet::weight(T::WeightInfo::set_term_params())]
        pub fn set_term_params(
            origin: OriginFor<T>,
            term_length: BlockNumberFor<T>,
            max_consecutive: u32,
            mandatory_break: BlockNumberFor<T>,
            warning_pct: u8,
        ) -> DispatchResult {
            T::ConstitutionalOrigin::ensure_origin(origin)?;
            ensure!(warning_pct >= 1 && warning_pct <= 50, Error::<T>::WarningPctInvalid);
            TermLengthBlocks::<T>::put(term_length);
            MaxConsecutiveTerms::<T>::put(max_consecutive);
            MandatoryBreakBlocks::<T>::put(mandatory_break);
            WarningWindowPct::<T>::put(warning_pct);
            Self::deposit_event(Event::TermParamsChanged {
                term_length, max_consecutive, mandatory_break, warning_pct,
            });
            Ok(())
        }

        /// Update constitutional election parameters. Any field left as None is unchanged.
        #[pallet::call_index(13)]
        #[pallet::weight(T::WeightInfo::set_election_params())]
        pub fn set_election_params(
            origin: OriginFor<T>,
            seats: Option<u32>,
            cycle_blocks: Option<u32>,
            max_backings_per_citizen: Option<u32>,
        ) -> DispatchResult {
            T::ConstitutionalOrigin::ensure_origin(origin)?;
            if let Some(s) = seats {
                ensure!(s > 0, Error::<T>::ElectionSeatsZero);
                LegislatureSeats::<T>::put(s);
            }
            if let Some(c) = cycle_blocks {
                ensure!(c > 0, Error::<T>::ElectionCycleBlocksZero);
                ElectionCycleBlocks::<T>::put(c);
            }
            if let Some(m) = max_backings_per_citizen {
                MaxBackingsPerCitizen::<T>::put(m);
            }
            Self::deposit_event(Event::ElectionParamsChanged {
                seats: LegislatureSeats::<T>::get(),
                cycle_blocks: ElectionCycleBlocks::<T>::get(),
                max_backings_per_citizen: MaxBackingsPerCitizen::<T>::get(),
            });
            Ok(())
        }
    }

    // ── Internal helpers ───────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {

        /// Activates `delegate` (Pending/OnBreak -> Active).
        ///
        /// Term-clock handling is the crux of closing the backing-drop-cycling term-limit
        /// evasion: a genuine fresh start (initial registration, or reactivation right after
        /// a completed mandatory break) must start a new term clock, but a transient
        /// backing-drop gap (`remove_backing` flipping Active -> Pending, which never touches
        /// `term_start_block`) must NOT reset it — otherwise a delegate with one cooperating
        /// backer could cycle `remove_backing`/`back_delegate` shortly before each term would
        /// complete and silently restart the elapsed-time clock from zero every time, so
        /// `consecutive_terms` would never reach the cap and the delegate would never be
        /// forced onto a mandatory break.
        ///
        /// `term_start_block` is already the right discriminator for this, in both call
        /// sites: it is only ever `None` on a genuine fresh start (registration leaves it
        /// `None`; `on_initialize`'s OnBreak-ending branch explicitly resets it to `None`
        /// before calling this) and is preserved as `Some` across a `remove_backing`-induced
        /// Pending gap. So: only start a new clock when there isn't one already running.
        fn activate_delegate(delegate: &T::AccountId) {
            let now = frame_system::Pallet::<T>::block_number();
            Delegates::<T>::mutate(delegate, |maybe| {
                if let Some(d) = maybe {
                    d.status = DelegateStatus::Active;
                    if d.term_start_block.is_none() {
                        d.term_start_block = Some(now);
                        d.warning_emitted = false;
                    }
                }
            });
            Self::deposit_event(Event::DelegateActivated { delegate: delegate.clone() });
        }

        /// Run a legislature election: rank Active delegates by backing count, seat the top N.
        fn run_election(now: BlockNumberFor<T>) -> Weight {
            let seats = LegislatureSeats::<T>::get() as usize;

            // Collect all delegates first so we can report exact read counts in the weight.
            let all_delegates: alloc::vec::Vec<_> = Delegates::<T>::iter().collect();
            let total_delegates = all_delegates.len() as u64;

            let mut candidates: alloc::vec::Vec<(T::AccountId, u32)> = all_delegates
                .into_iter()
                .filter_map(|(addr, info)| {
                    // Re-check citizenship now, not just trust Active status from whenever the
                    // backing threshold was last crossed: a delegate can hold Active status for
                    // years, and may have been suspended since (e.g. an Overturned
                    // CitizenConduct court ruling) without ever re-registering. This is the
                    // point power is actually granted, so it's the point that must be checked.
                    if info.status == DelegateStatus::Active
                        && T::CitizenChecker::is_active_citizen(&addr)
                    {
                        Some((addr.clone(), BackingCount::<T>::get(&addr)))
                    } else {
                        None
                    }
                })
                .collect();

            let active_count = candidates.len() as u64;

            // Stable sort by backing count descending — ties broken by storage order.
            candidates.sort_by(|a, b| b.1.cmp(&a.1));

            let winners: alloc::vec::Vec<T::AccountId> = candidates
                .into_iter()
                .take(seats)
                .map(|(addr, _)| addr)
                .collect();

            let seated = winners.len() as u32;
            let _ = T::LegislatureSeating::replace_members(winners);
            LastElectionBlock::<T>::put(now);

            Self::deposit_event(Event::LegislatureElectionRun { at_block: now, seated });

            // Reads: all delegate entries + BackingCount per active delegate + 3 overhead
            //        (LegislatureSeats, ElectionCycleBlocks, LastElectionBlock).
            // Writes: Members (replace_members) + LastElectionBlock.
            T::DbWeight::get().reads_writes(
                total_delegates.saturating_add(active_count).saturating_add(3),
                (seated as u64).saturating_add(2),
            )
        }
    }
}
