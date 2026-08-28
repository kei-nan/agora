//! # Accountability Council Pallet
//!
//! A dedicated, independent oversight body that governs appointment of `pallet-audit`
//! auditors and `pallet-anticorruption` investigators.
//!
//! ## Why a new, separate council
//! Both `add_auditor`/`remove_auditor` (pallet-audit) and `add_investigator`/`remove_investigator`
//! (pallet-anticorruption) were bare `ensure_root` calls with no configurable `EnsureOrigin` at
//! all. Routing them through `pallet_legislature::EnsureLegislatureMotion` — the obvious existing
//! candidate — would reproduce the exact self-oversight failure real Supreme Audit Institutions
//! are designed to prevent: `pallet-legislature` already controls the treasury budget via
//! `LegislatureOrigin` (see `pallet_treasury_ledger::Config::LegislatureOrigin`), so letting it
//! also pick who audits/investigates that same spending puts the auditors under the thumb of the
//! body they audit. (Concrete cautionary precedent: Indonesia's KPK was weakened when its
//! legislature inserted itself into the anti-corruption commission's appointment structure.)
//!
//! This pallet is that independent body: a small council, explicitly barred from
//! legislature/executive overlap (see `LegislatureChecker`/`ExecutiveChecker` below), governing
//! oversight-role *appointment* only — not operational power. Department-spender designation
//! (`pallet_treasury_ledger::register_department_spender`) is deliberately NOT routed here: it's
//! an operational, Executive-branch-like power, not an oversight one, and stays on
//! `LegislatureOrigin` (already present in that pallet's `Config`).
//!
//! ## Mechanics
//! Membership (`Members`) and the propose/approve/`call_hash`-binding admin-action flow
//! (`propose_action`/`approve_action`/`EnsureAccountabilityCouncilApproved`) mirror
//! `pallet-courts`' Oracle Council admin-action pattern
//! (`propose_admin_action`/`approve_admin_action`/`EnsureOracleCouncilApproved`) — see that
//! pallet's doc comments for the deadlock/replay considerations this design already accounts
//! for (any current member may consume an approved token, not only the original proposer;
//! stale unconsumed tokens can be discarded after `ApprovalExpiryBlocks`). The threshold here is
//! a genuine supermajority (`SupermajorityNumerator`/`Denominator`, evaluated with `>=` — mirrors
//! `pallet-emergency-council`'s `supermajority_reached`), not the Oracle Council's plain `>1/2`
//! majority: an oversight body's own membership and the actions it authorizes should need
//! broader consensus than a simple majority to change.
//!
//! ## Self-perpetuating membership
//! Every other council in this codebase (Oracle Council, AI Model Governance Council, Emergency
//! Council, the legislature itself) is bootstrapped *and* permanently managed by bare `Root`.
//! This one deliberately is not, once bootstrapped: `Root` may freely call `add_member`/
//! `remove_member` only until `close_bootstrap` is called (also `Root`-gated, requires at least
//! one member already seated). After that, `add_member`/`remove_member` route through this same
//! pallet's own `propose_action`/`approve_action`/`EnsureAccountabilityCouncilApproved` flow —
//! the Council governs its own future composition by supermajority vote of its own current
//! members, and `Root` can no longer unilaterally change it. This is the actual fix for the
//! self-oversight problem described above: if `Root` (in practice, however `Root` is reached —
//! sudo today, a stronger origin later) could keep silently swapping auditors/investigators'
//! overseers at will, the independence this pallet exists to provide would be illusory.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use codec::Encode;
    use frame_support::pallet_prelude::*;
    use frame_support::traits::EnsureOriginWithArg;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Saturating;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    // ── Cross-pallet incompatibility checks ─────────────────────────────────────

    /// Implemented by the runtime (reading `pallet-legislature`'s `Members` storage) to check
    /// whether an account currently holds a legislature seat. Mirrors the shape/spirit of
    /// `pallet_legislature::pallet::MinisterChecker` — a small, local trait the runtime wires up
    /// by delegating to the real pallet's storage — kept as its own local trait here (rather than
    /// this crate depending on the `pallet-legislature` crate just for one trait) so this stays
    /// consistent with how every other cross-pallet "Checker" in this codebase is defined
    /// (`CitizenChecker` is independently redefined in `pallet-voting`, `pallet-courts`,
    /// `pallet-constitution`, and `pallet-elections` for the identical reason).
    pub trait LegislatureChecker<AccountId> {
        fn is_legislature_member(who: &AccountId) -> bool;
    }

    /// Implemented by the runtime (reading `pallet-executive`'s minister/Prime-Minister storage)
    /// to check whether an account currently holds executive power. Mirrors
    /// `pallet_legislature::pallet::MinisterChecker::is_active_minister` exactly — same
    /// question, same answer source — defined locally here for the same reason as
    /// `LegislatureChecker` above.
    pub trait ExecutiveChecker<AccountId> {
        fn is_active_minister(who: &AccountId) -> bool;
    }

    // ── call_hash helper ─────────────────────────────────────────────────────────

    /// Computes the domain-separated hash a pending action's `call_hash` must equal for
    /// `EnsureAccountabilityCouncilApproved` to authorize it. `tag` should uniquely identify the
    /// pallet + dispatchable being authorized (e.g. `b"pallet-audit::add_auditor"`) so that
    /// byte-identical parameters passed to a different call never collide. Mirrors
    /// `pallet_constitution::legislature_call_hash` / `pallet_courts`' admin-action hashing.
    /// `pub` (not `pub(crate)`, unlike its pallet-constitution counterpart) because consuming
    /// pallets outside this crate (pallet-audit, pallet-anticorruption, and this pallet's own
    /// post-bootstrap `add_member`/`remove_member`) need to compute the identical hash.
    pub fn accountability_call_hash(tag: &'static [u8], params: impl Encode) -> [u8; 32] {
        let mut preimage = alloc::vec::Vec::from(tag);
        preimage.extend(params.encode());
        frame_support::Hashable::blake2_256(&preimage)
    }

    // ── EnsureOriginWithArg ──────────────────────────────────────────────────────

    /// `EnsureOriginWithArg<RuntimeOrigin, [u8; 32]>` that succeeds only when
    /// `propose_action`/`approve_action` have already driven the given call hash's approvals
    /// past the Accountability Council's supermajority threshold (see `ApprovedAction`). Wire
    /// this as the origin for `pallet_audit::Config::AuditorOrigin`,
    /// `pallet_anticorruption::Config::InvestigatorOrigin`, and any other call this Council is
    /// meant to gate — including this pallet's own `add_member`/`remove_member` once bootstrapped
    /// (see the module doc comment).
    ///
    /// Mirrors `pallet_courts::EnsureOracleCouncilApproved` exactly: any *current* Council member
    /// may consume an approved token (not only the original proposer — the approval vote is what
    /// legitimizes the action, not the proposer's continued availability), and a token is
    /// removed the instant it's consumed so it can never authorize more than the one call it was
    /// approved for.
    pub struct EnsureAccountabilityCouncilApproved<T>(core::marker::PhantomData<T>);

    impl<T: Config> frame_support::traits::EnsureOriginWithArg<T::RuntimeOrigin, [u8; 32]>
        for EnsureAccountabilityCouncilApproved<T>
    {
        type Success = ();

        fn try_origin(
            o: T::RuntimeOrigin,
            call_hash: &[u8; 32],
        ) -> Result<Self::Success, T::RuntimeOrigin> {
            use frame_system::RawOrigin;
            match o.clone().into() {
                Ok(RawOrigin::Signed(who)) if Members::<T>::get().contains(&who) => {
                    if ApprovedAction::<T>::get(call_hash).is_some() {
                        ApprovedAction::<T>::remove(call_hash);
                        Ok(())
                    } else {
                        Err(o)
                    }
                }
                _ => Err(o),
            }
        }

        // NOTE: this cfg can never currently activate — this crate declares no
        // `runtime-benchmarks` feature of its own and has no `benchmarking.rs`. See
        // `pallet_courts::EnsureOracleCouncilApproved`'s identical note for the full accounting;
        // making this real requires adding the feature to this crate's Cargo.toml, wiring it
        // into `runtime/Cargo.toml`, and writing an actual `#[benchmarks]` module.
        #[cfg(feature = "runtime-benchmarks")]
        fn try_successful_origin(call_hash: &[u8; 32]) -> Result<T::RuntimeOrigin, ()> {
            let member = Members::<T>::get().first().cloned().ok_or(())?;
            ApprovedAction::<T>::insert(
                *call_hash,
                (member.clone(), frame_system::Pallet::<T>::block_number()),
            );
            Ok(frame_system::RawOrigin::Signed(member).into())
        }
    }

    // ── Config ──────────────────────────────────────────────────────────────────

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Maximum Council size. Sized at 7-9 in the runtime, similar to the Oracle Council.
        #[pallet::constant]
        type MaxCouncilSize: Get<u32>;
        /// Numerator of the supermajority fraction required for `add_member`/`remove_member`
        /// (post-bootstrap) and `propose_action`/`approve_action` to take effect (e.g. 2 for
        /// 2/3). Evaluated with `>=` (a genuine supermajority, e.g. "at least 2/3"), unlike
        /// `pallet_courts::Config::OracleApprovalNumerator`'s strict `>` (a plain majority means
        /// *more than* half) — see `supermajority_reached`'s doc comment.
        #[pallet::constant]
        type SupermajorityNumerator: Get<u32>;
        /// Denominator of the supermajority fraction (e.g. 3 for 2/3).
        #[pallet::constant]
        type SupermajorityDenominator: Get<u32>;
        /// How many blocks an `ApprovedAction` token may sit unconsumed before any current
        /// Council member can discard it via `clear_stale_action`. Mirrors
        /// `pallet_courts::Config::AdminActionExpiryBlocks`.
        #[pallet::constant]
        type ApprovalExpiryBlocks: Get<u32>;
        /// Checks whether a prospective member currently holds a `pallet-legislature` seat —
        /// enforces the legislature/Council incompatibility rule (see the module doc comment).
        type LegislatureChecker: LegislatureChecker<Self::AccountId>;
        /// Checks whether a prospective member currently holds executive power (minister or
        /// Prime Minister) — enforces the executive/Council incompatibility rule.
        type ExecutiveChecker: ExecutiveChecker<Self::AccountId>;
    }

    // ── Storage ─────────────────────────────────────────────────────────────────

    /// Current Council members.
    #[pallet::storage]
    pub type Members<T: Config> =
        StorageValue<_, BoundedVec<T::AccountId, T::MaxCouncilSize>, ValueQuery>;

    /// Whether the Council has been bootstrapped (see `close_bootstrap`). While `false`, `Root`
    /// may freely call `add_member`/`remove_member`, mirroring every other council in this
    /// codebase. Once `true`, those same two calls require this Council's own supermajority
    /// approval instead (see the module doc comment) — `Root` can never flip this back to
    /// `false`; there is no call that does so.
    #[pallet::storage]
    pub type Bootstrapped<T: Config> = StorageValue<_, bool, ValueQuery>;

    /// call_hash -> (proposer, Council members who have approved) for an in-flight action —
    /// mirrors `pallet_courts::PendingAdminAction`. Covers both external calls this Council
    /// gates for other pallets (e.g. `pallet-audit::add_auditor`) and this pallet's own
    /// post-bootstrap `add_member`/`remove_member`.
    #[pallet::storage]
    pub type PendingAction<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        [u8; 32],
        (T::AccountId, BoundedVec<T::AccountId, T::MaxCouncilSize>),
    >;

    /// call_hash -> (proposer, block approved) once `PendingAction`'s approvals reached the
    /// supermajority threshold — mirrors `pallet_courts::ApprovedAdminAction`. Consumed by
    /// `EnsureAccountabilityCouncilApproved`; `proposer` is retained only as an audit trail.
    #[pallet::storage]
    pub type ApprovedAction<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], (T::AccountId, BlockNumberFor<T>)>;

    // ── Events ──────────────────────────────────────────────────────────────────

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A new member was added to the Council (bootstrap or post-bootstrap).
        MemberAdded { who: T::AccountId },
        /// A member was removed from the Council (bootstrap or post-bootstrap).
        MemberRemoved { who: T::AccountId },
        /// `Root` closed the bootstrap phase; membership changes now require this Council's own
        /// supermajority approval.
        BootstrapClosed,
        /// A Council member proposed an action identified by its domain-separated call hash
        /// (their own approval is recorded immediately).
        ActionProposed { call_hash: [u8; 32], proposer: T::AccountId },
        /// A Council member cast an approval on a pending action.
        ActionApprovalCast { call_hash: [u8; 32], member: T::AccountId },
        /// A pending action reached its supermajority threshold; any current Council member may
        /// now consume it once via `EnsureAccountabilityCouncilApproved`.
        ActionApproved { call_hash: [u8; 32] },
        /// An `ApprovedAction` token expired unconsumed and was discarded via
        /// `clear_stale_action`, freeing the call_hash for a new proposal.
        ActionExpired { call_hash: [u8; 32] },
    }

    // ── Errors ──────────────────────────────────────────────────────────────────

    #[pallet::error]
    pub enum Error<T> {
        /// The account is already a Council member.
        AlreadyMember,
        /// The account is not a Council member.
        MemberNotFound,
        /// Cannot add member: Council (or its per-action approval list) is at maximum capacity.
        CouncilAtCapacity,
        /// The prospective member currently holds a `pallet-legislature` seat or executive
        /// power — the legislature/executive incompatibility rule.
        LegislatureOrExecutiveOverlap,
        /// `close_bootstrap` was called with no members yet seated.
        NoMembersToBootstrap,
        /// `close_bootstrap` was called after bootstrap was already closed.
        AlreadyBootstrapped,
        /// `add_member`/`remove_member` requires `Root` while the Council is still bootstrapping.
        /// Emitted when a signed (non-Root) caller tries to use the direct path pre-bootstrap.
        BootstrapRequiresRoot,
        /// This call_hash already has a pending or approved action awaiting consumption.
        ActionAlreadyProposed,
        /// `approve_action` was called for a call_hash with no pending action.
        NoPendingAction,
        /// This Council member has already approved the current pending action.
        AlreadyApproved,
        /// There is no `ApprovedAction` token for this call_hash to clear.
        NoApprovedAction,
        /// The `ApprovedAction` token has not yet reached `ApprovalExpiryBlocks`.
        ApprovalNotYetStale,
        /// The caller is not a current Council member.
        NotCouncilMember,
    }

    // ── Calls ───────────────────────────────────────────────────────────────────

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Add a member to the Council.
        ///
        /// While the Council is still bootstrapping (`Bootstrapped == false`), only `Root` may
        /// call this directly — mirrors every other council's `add_*_member` in this codebase.
        /// Once bootstrapped, `Root` is no longer accepted: the caller must instead be a current
        /// Council member consuming an `EnsureAccountabilityCouncilApproved` token already
        /// approved for this exact call (`accountability_call_hash(b"pallet-accountability-
        /// council::add_member", &who)`) via `propose_action`/`approve_action` — i.e. the
        /// Council adds its own new members by its own supermajority vote. Either way, `who`
        /// must pass the legislature/executive incompatibility check.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_member(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            Self::authorize_membership_change(origin, b"pallet-accountability-council::add_member", &who)?;
            ensure!(
                !T::LegislatureChecker::is_legislature_member(&who)
                    && !T::ExecutiveChecker::is_active_minister(&who),
                Error::<T>::LegislatureOrExecutiveOverlap
            );
            Members::<T>::try_mutate(|members| {
                ensure!(!members.contains(&who), Error::<T>::AlreadyMember);
                members.try_push(who.clone()).map_err(|_| Error::<T>::CouncilAtCapacity)
            })?;
            Self::deposit_event(Event::MemberAdded { who });
            Ok(())
        }

        /// Remove a member from the Council. Same dual-gate as `add_member`: `Root` pre-
        /// bootstrap, supermajority-approved call post-bootstrap.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_member(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
            Self::authorize_membership_change(origin, b"pallet-accountability-council::remove_member", &who)?;
            Members::<T>::try_mutate(|members| {
                let pos = members.iter().position(|m| m == &who).ok_or(Error::<T>::MemberNotFound)?;
                members.remove(pos);
                Ok::<(), DispatchError>(())
            })?;
            // Purge this member's already-cast approvals from every in-flight PendingAction —
            // mirrors pallet_courts::remove_oracle_member's identical rationale: this Council
            // exists specifically to survive a compromised/departing member, so their vote
            // shouldn't keep counting toward quorum on still-open proposals after removal.
            PendingAction::<T>::translate(
                |_call_hash, (proposer, mut approvers): (T::AccountId, BoundedVec<T::AccountId, T::MaxCouncilSize>)| {
                    if let Some(pos) = approvers.iter().position(|m| m == &who) {
                        approvers.remove(pos);
                    }
                    Some((proposer, approvers))
                },
            );
            Self::deposit_event(Event::MemberRemoved { who });
            Ok(())
        }

        /// `Root`-only, one-time: closes the bootstrap phase. Requires at least one member
        /// already seated. After this, `Root` can never again unilaterally add or remove a
        /// Council member — see the module doc comment and `add_member`/`remove_member`.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn close_bootstrap(origin: OriginFor<T>) -> DispatchResult {
            ensure_root(origin)?;
            ensure!(!Bootstrapped::<T>::get(), Error::<T>::AlreadyBootstrapped);
            ensure!(!Members::<T>::get().is_empty(), Error::<T>::NoMembersToBootstrap);
            Bootstrapped::<T>::put(true);
            Self::deposit_event(Event::BootstrapClosed);
            Ok(())
        }

        /// Propose a Council action identified by the domain-separated hash of the exact call
        /// it will authorize (e.g. `pallet-audit::add_auditor`, or this pallet's own
        /// `add_member`/`remove_member` post-bootstrap — see `accountability_call_hash`). Only
        /// callable by a current Council member; this both proposes the action and casts the
        /// proposer's own approval, exactly like `pallet_courts::propose_admin_action`.
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn propose_action(origin: OriginFor<T>, call_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Members::<T>::get().contains(&who), Error::<T>::NotCouncilMember);
            ensure!(
                PendingAction::<T>::get(call_hash).is_none()
                    && ApprovedAction::<T>::get(call_hash).is_none(),
                Error::<T>::ActionAlreadyProposed
            );
            let mut approvals: BoundedVec<T::AccountId, T::MaxCouncilSize> = BoundedVec::default();
            approvals.try_push(who.clone()).map_err(|_| Error::<T>::CouncilAtCapacity)?;
            PendingAction::<T>::insert(call_hash, (who.clone(), approvals));
            Self::deposit_event(Event::ActionProposed { call_hash, proposer: who });
            Self::try_resolve_action(call_hash);
            Ok(())
        }

        /// Cast a Council approval on `call_hash`'s pending action. See `propose_action`.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn approve_action(origin: OriginFor<T>, call_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Members::<T>::get().contains(&who), Error::<T>::NotCouncilMember);
            PendingAction::<T>::try_mutate(call_hash, |maybe| {
                let (_, approvals) = maybe.as_mut().ok_or(Error::<T>::NoPendingAction)?;
                ensure!(!approvals.contains(&who), Error::<T>::AlreadyApproved);
                approvals.try_push(who.clone()).map_err(|_| Error::<T>::CouncilAtCapacity)
            })?;
            Self::deposit_event(Event::ActionApprovalCast { call_hash, member: who });
            Self::try_resolve_action(call_hash);
            Ok(())
        }

        /// Discard an unconsumed `ApprovedAction` token once `ApprovalExpiryBlocks` have passed
        /// since it was approved. Open to any current Council member. Mirrors
        /// `pallet_courts::clear_stale_admin_action`.
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(5_000, 0))]
        pub fn clear_stale_action(origin: OriginFor<T>, call_hash: [u8; 32]) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Members::<T>::get().contains(&who), Error::<T>::NotCouncilMember);
            let (_, approved_at) =
                ApprovedAction::<T>::get(call_hash).ok_or(Error::<T>::NoApprovedAction)?;
            let now = frame_system::Pallet::<T>::block_number();
            let expiry = BlockNumberFor::<T>::from(T::ApprovalExpiryBlocks::get());
            ensure!(now >= approved_at.saturating_add(expiry), Error::<T>::ApprovalNotYetStale);
            ApprovedAction::<T>::remove(call_hash);
            Self::deposit_event(Event::ActionExpired { call_hash });
            Ok(())
        }
    }

    // ── Helpers ─────────────────────────────────────────────────────────────────

    impl<T: Config> Pallet<T> {
        /// Shared dual-gate for `add_member`/`remove_member`: `Root` pre-bootstrap, an
        /// `EnsureAccountabilityCouncilApproved` token bound to this exact `(tag, who)` call
        /// post-bootstrap. See the module doc comment and those calls' doc comments.
        fn authorize_membership_change(
            origin: OriginFor<T>,
            tag: &'static [u8],
            who: &T::AccountId,
        ) -> DispatchResult {
            if Bootstrapped::<T>::get() {
                let hash = accountability_call_hash(tag, who);
                EnsureAccountabilityCouncilApproved::<T>::ensure_origin(origin, &hash)?;
                Ok(())
            } else {
                Ok(ensure_root(origin)?)
            }
        }

        /// Returns true if `approvals` meets the configured supermajority threshold. Uses
        /// non-strict `>=` — a genuine supermajority (e.g. "at least 2/3") rather than
        /// `pallet_courts::oracle_approval_reached`'s strict `>` (a plain majority means *more*
        /// than half) — mirrors `pallet_emergency_council::supermajority_reached` /
        /// `pallet_courts::ai_model_supermajority_reached`.
        fn supermajority_reached(approvals: u32, council_size: u32) -> bool {
            if council_size == 0 {
                return false;
            }
            approvals.saturating_mul(T::SupermajorityDenominator::get())
                >= council_size.saturating_mul(T::SupermajorityNumerator::get())
        }

        /// If `call_hash`'s pending action has reached the supermajority threshold, move it
        /// from `PendingAction` to `ApprovedAction` so any current member can consume it via
        /// `EnsureAccountabilityCouncilApproved`. Otherwise a no-op. Mirrors
        /// `pallet_courts::try_resolve_admin_action`.
        fn try_resolve_action(call_hash: [u8; 32]) {
            let Some((proposer, approvals)) = PendingAction::<T>::get(call_hash) else {
                return;
            };
            let council_size = Members::<T>::get().len() as u32;
            if Self::supermajority_reached(approvals.len() as u32, council_size) {
                PendingAction::<T>::remove(call_hash);
                let now = frame_system::Pallet::<T>::block_number();
                ApprovedAction::<T>::insert(call_hash, (proposer, now));
                Self::deposit_event(Event::ActionApproved { call_hash });
            }
        }
    }
}

// ── AccountabilityCouncilChecker implementations (reverse-direction overlap gate) ──────────
//
// `add_member` (above) already blocks the "current legislature/executive member joins the
// Council" direction. These implement the other direction: `pallet-elections`' automatic
// legislature seating and `pallet-executive`'s minister/PM appointment path both need to ask
// "is this account currently a Council member?" before installing them into legislature or
// executive power. Following the same consumer-defines/provider-implements idiom this codebase
// already uses for `DisclosureChecker` (defined in pallet-elections, implemented directly on
// `pallet_anticorruption::Pallet<T>`): each consuming pallet defines its own local
// `AccountabilityCouncilChecker<AccountId>` trait, and this pallet (the provider, reading its
// own `Members` storage) implements each one directly on its own `Pallet<T>` — no `Runtime`-
// level delegating impl needed, and no risk of the two checks drifting apart since both read
// the same `Members` storage item via the same method body shape.
impl<T: Config> pallet_elections::AccountabilityCouncilChecker<T::AccountId> for Pallet<T> {
    fn is_current_member(who: &T::AccountId) -> bool {
        Members::<T>::get().contains(who)
    }
}

impl<T: Config> pallet_executive::AccountabilityCouncilChecker<T::AccountId> for Pallet<T> {
    fn is_current_member(who: &T::AccountId) -> bool {
        Members::<T>::get().contains(who)
    }
}
