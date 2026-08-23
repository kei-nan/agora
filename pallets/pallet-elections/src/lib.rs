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
//! ### Delegate identity — now cryptographically separated via ZK personas
//! `register_as_delegate` no longer trusts `ensure_signed`'s caller identity directly as the
//! delegate's public persona. Instead it requires a real ZK proof of the `delegate-persona`
//! circuit (`circuits/oprf-identity-anchor/delegate-persona`, commit 2e07f68) riding inside a
//! fresh outer ZKPassport proof: the citizen's wallet runs a genuinely separate 5-committee OPRF
//! round-trip (distinct from registration's own anchor evaluation) to derive a stable
//! `delegate_persona_id`, and binds a chosen `persona_account` into that proof's
//! `param_commitment` (anti-front-running — an observer cannot resubmit the proof against a
//! different account). `T::ZkVerifier` performs the real bb 5.0.0 pairing check, then
//! `T::CommitteeKeyChecker` confirms the 5 committee key hashes are governance-approved (closing
//! the same self-minted-committee gap `pallet_identity_zk::register_citizen` guards against —
//! without it a prover could fabricate arbitrary "committee" keys and mint unlimited personas),
//! then `T::DelegatePersonaVerifier` recomputes and checks the `param_commitment`
//! (`runtime/src/anchor_verifier.rs::check_delegate_persona`). `DelegatePersonaUsed` is an
//! insert-once nullifier map on `delegate_persona_id` itself, so the same citizen cannot mint a
//! second, different persona. `persona_account` is still an ordinary `T::AccountId` — the same
//! type `Delegates`/`SeatLegislature`/`DisclosureChecker` already key on — so pallet-legislature's
//! seating and pallet-anticorruption's disclosure gate need no changes to work with it.
//!
//! Backing is unlinkable too: `back_delegate`/`remove_backing` require a real
//! `backing-nullifier` circuit proof (`circuits/oprf-identity-anchor/backing-nullifier`) proving
//! Merkle-path membership of the citizen's `backing_commitment` in pallet-identity's published
//! tree, at a slot index range-checked *in-circuit* against the live `MaxBackingsPerCitizen`
//! value (a checked public input, not a plaintext per-citizen counter this pallet maintains
//! itself — see `UsedBackingNullifier`'s doc comment). No account ever appears in
//! `back_delegate`/`remove_backing`'s public state keyed by "who backs whom" — only a nullifier.
//!
//! **Residual gap that survives this (2026-08-23):** an unlinkable proof only anonymizes the
//! *derivation*, not the *transaction* that reveals it. That transaction is still a signed
//! extrinsic with a signer `AccountId`, a fee-payment source, and a block timestamp — funding
//! the account from the citizen's own known account, or submitting close in time to other
//! citizen-linked activity, deanonymizes it via ordinary chain analysis, not cryptanalysis. Same
//! class of gap as `pallet-voting`'s `commit_vote` (see its doc comment); no relayer/mixnet/
//! unsigned-ZK-gated submission path or faucet-like funding mechanism exists anywhere in this
//! repo to close it — see `docs/project/pallets/elections.md` for the full writeup.
//!
//! A second, narrower residual gap is specific to backing: the `backing-nullifier` circuit
//! (deliberately, by its own design — see its module docs) does not bind any `AccountId` into
//! its proof, so the exact `(zk_proof, public_inputs)` bytes a citizen submits to `back_delegate`
//! are public call data afterward, and would let *anyone* who observed them resubmit the
//! identical proof to `remove_backing` and strip that backing without the citizen's consent.
//! `UsedBackingNullifier` closes this by recording the *submitting* `AccountId` alongside each
//! nullifier and requiring the same signer to reverse their own action — this does not leak any
//! privacy beyond what `back_delegate`'s own signer already publicly exposed. One consequence:
//! `back_delegate` no longer rejects backing your own delegate account as a self-evident
//! `CannotBackSelf` check — the tx signer (`who`) is no longer cryptographically tied to the
//! backing-nullifier's underlying secret, so that check would give false assurance without
//! actually preventing anything (a delegate wanting to spend one of their own
//! `MaxBackingsPerCitizen` slots on themselves already could, via a cooperating relayer). The
//! practical exposure is bounded: self-backing can win a delegate at most one of the
//! `BackingThreshold` backers they need, the same as any single legitimate citizen's backing
//! power.
//!
//! ### Backing threshold
//! A delegate becomes Active only when they have ≥ `BackingThreshold` citizen backers.
//! Each citizen may back at most `MaxBackingsPerCitizen` delegates (constitutional parameter,
//! default 5) — enforced by the `backing-nullifier` circuit's own in-circuit range check on
//! `slot_index`, not by any plaintext per-citizen counter on-chain (there is deliberately no
//! such counter any more: knowing "how many delegates has citizen X backed" would itself be a
//! privacy leak). This makes backing a meaningful signal rather than noise.
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

    /// Number of independent OPRF committees — must match
    /// `circuits/oprf-identity-anchor/lib/identity-anchor`'s `NUM_COMMITTEES` and
    /// `runtime/src/anchor_verifier.rs`'s own copy of the same constant.
    pub const NUM_COMMITTEES: usize = 5;

    pub trait CitizenChecker<AccountId> {
        fn is_active_citizen(who: &AccountId) -> bool;
    }

    /// Converts a `T::AccountId` to its raw 32-byte representation, for binding
    /// `persona_account` into a delegate-persona proof's `param_commitment` (see
    /// `runtime/src/anchor_verifier.rs`'s `account_to_field_limbs`) exactly the way the
    /// citizen's own wallet packed it when building the proof. A pluggable Config item —
    /// rather than a bare `T::AccountId: Into<[u8; 32]>` bound threaded through every `impl`
    /// block in this file — so a test mock's `u64` `AccountId` doesn't need to satisfy a
    /// genuine 32-byte encoding: the real runtime's `AccountId32` is genuinely 32 raw bytes
    /// end-to-end, but a mock only needs *some* deterministic mapping to exercise the logic.
    pub trait AccountIdToBytes<AccountId> {
        fn to_bytes(who: &AccountId) -> [u8; 32];
    }

    /// Verifies the outer ZKPassport proof a `delegate-persona` creation proof rides inside —
    /// the same real bb 5.0.0 UltraHonk pairing check `pallet_identity_zk::Config::ZkVerifier`
    /// performs (see `runtime/src/verifier.rs`), reused here rather than duplicated because a
    /// delegate-persona proof is, cryptographically, just another outer ZKPassport proof
    /// exposing a `disclosure`-shaped subproof (see
    /// `circuits/oprf-identity-anchor/delegate-persona`).
    pub trait ZkProofVerifier {
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
    }

    /// Recomputes and checks the `delegate-persona` circuit's `param_commitment` against an
    /// already-`T::ZkVerifier`-verified outer proof's public inputs — mirrors
    /// `pallet_identity_zk::AnchorProofVerifier`'s split between the pairing check (above) and
    /// this pure recomputation. Backed in production by
    /// `runtime/src/anchor_verifier.rs::check_delegate_persona` (commit 2e07f68).
    pub trait DelegatePersonaVerifier {
        fn check_delegate_persona(
            outer_public_inputs: &[[u8; 32]],
            delegate_persona_id: [u8; 32],
            persona_account: [u8; 32],
            scheme_version: u32,
            oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
        ) -> bool;
    }

    /// Checks a set of `NUM_COMMITTEES` committee-key hashes against the governance-approved
    /// keys on file for the given OPRF scheme version — the same Sybil-resistance guarantee
    /// `pallet_identity_zk::register_citizen` enforces via its own (private)
    /// `check_committee_keys`, applied here so a delegate persona cannot be self-minted against
    /// attacker-chosen "committee" keys the real OPRF committee never signed off on (without
    /// this check, `verified_oprf`'s DLEQ proof only shows internal consistency with
    /// *whatever* key the prover supplied, not that the key belongs to the real committee).
    /// Implemented by pallet-identity-zk directly on its own `Pallet<T>`
    /// (`are_committee_keys_approved`), mirroring `DisclosureChecker`'s
    /// consumer-defines/provider-implements idiom below.
    pub trait CommitteeKeyChecker {
        fn are_committee_keys_approved(
            scheme_version: u32,
            oprf_pk_hashes: &[[u8; 32]; NUM_COMMITTEES],
        ) -> bool;
    }

    /// Real standalone UltraHonk pairing check for a `backing-nullifier` circuit proof — see
    /// `circuits/oprf-identity-anchor/backing-nullifier` and
    /// `runtime/src/backing_nullifier_verifier.rs`. Unlike `ZkProofVerifier` above, this proof
    /// never rides inside any outer ZKPassport proof, so nothing else verifies it first.
    pub trait BackingProofVerifier {
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
    }

    /// Checks a backing-commitment tree root against pallet-identity's own root history —
    /// backed by `pallet_identity_zk::Pallet::<T>::is_valid_backing_commitment_root`, which
    /// accepts any root that was current within that pallet's retention window, not only the
    /// current one.
    pub trait BackingRootChecker {
        fn is_valid_backing_commitment_root(root: [u8; 32]) -> bool;
    }

    /// Called by pallet-elections at the end of each election cycle.
    /// The implementation in pallet-legislature replaces the full Members set.
    pub trait SeatLegislature<AccountId> {
        fn replace_members(winners: alloc::vec::Vec<AccountId>) -> DispatchResult;
    }

    /// Gate: returns `false` if `who` does not have a *current* asset disclosure on file
    /// (never filed, or filed but past its `AssetDisclosureRenewalBlocks` renewal deadline).
    /// Checked in `run_election`, at the moment a delegate would actually be seated into
    /// pallet-legislature — not at `register_as_delegate`/`back_delegate` time — for the same
    /// reason `CitizenChecker` is re-checked there rather than trusted from whenever `Active`
    /// status was last crossed: a delegate can hold backing for years, and their disclosure
    /// can lapse at any point in between, so seating time is the only point that reflects
    /// current reality. Implemented directly on `pallet_anticorruption::Pallet<T>` (wrapping
    /// its own `has_current_disclosure`), following the same idiom this pallet already uses
    /// for `SeatLegislature` above: the *consumer* (this pallet) defines the trait, the
    /// *provider* implements it directly on its own `Pallet<T>`, and the runtime just aliases
    /// `Config::DisclosureChecker` to the provider's pallet type — no `Runtime`-level
    /// delegating impl needed.
    pub trait DisclosureChecker<AccountId> {
        fn has_current_disclosure(who: &AccountId) -> bool;
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

        /// Cross-pallet gate: checked per-candidate during seating (see `run_election`) so an
        /// account without a current asset disclosure is skipped rather than seated.
        type DisclosureChecker: DisclosureChecker<Self::AccountId>;

        /// See `AccountIdToBytes`'s doc comment above.
        type AccountIdToBytes: AccountIdToBytes<Self::AccountId>;

        /// See `ZkProofVerifier`'s doc comment above — verifies the outer ZKPassport proof a
        /// `register_as_delegate` call submits.
        type ZkVerifier: ZkProofVerifier;

        /// See `DelegatePersonaVerifier`'s doc comment above.
        type DelegatePersonaVerifier: DelegatePersonaVerifier;

        /// See `CommitteeKeyChecker`'s doc comment above.
        type CommitteeKeyChecker: CommitteeKeyChecker;

        /// See `BackingProofVerifier`'s doc comment above — verifies the standalone
        /// `backing-nullifier` proof `back_delegate`/`remove_backing` submit.
        type BackingProofVerifier: BackingProofVerifier;

        /// See `BackingRootChecker`'s doc comment above.
        type BackingRootChecker: BackingRootChecker;

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

    /// Number of citizens currently backing each delegate. Incrementally maintained by
    /// `back_delegate`/`remove_backing` (not derived by counting `UsedBackingNullifier` entries
    /// at read time, which would make `run_election` scan unboundedly many nullifiers) — this is
    /// the only place a running backing count lives; it is deliberately not decomposable back
    /// into "which citizens" contributed to it.
    #[pallet::storage]
    pub type BackingCount<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// `delegate_persona_id` → `()`. Insert-once nullifier set: prevents the same citizen (whose
    /// `delegate_persona_id` is a deterministic function of their passport-derived identity,
    /// evaluated fresh per delegate-persona creation — see `derive_delegate_identity_input`'s
    /// doc comment) from minting a second, different delegate persona. Keyed on the id itself,
    /// not on `persona_account`, since the id is the value that is actually derived once per
    /// citizen; `persona_account` is merely whichever account they bound to it.
    #[pallet::storage]
    pub type DelegatePersonaUsed<T: Config> = StorageMap<_, Blake2_128Concat, [u8; 32], ()>;

    /// `persona_account` → `delegate_persona_id`, set once at `register_as_delegate` alongside
    /// `DelegatePersonaUsed` and never changed afterward. The reverse lookup
    /// `back_delegate`/`remove_backing` use to check a `backing-nullifier` proof's public
    /// `delegate_persona_id` input actually targets the `delegate: T::AccountId` the caller
    /// named.
    #[pallet::storage]
    pub type DelegatePersonaIdOf<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32]>;

    /// `backing_nullifier` → `(submitter, delegate_persona_id)`. Replaces the old plaintext
    /// `BackingOf`/`CitizenBackingCount` maps entirely: a citizen's backing is now represented
    /// only by a nullifier derived from a private secret this pallet never sees, so there is no
    /// on-chain record of *which citizen* backs *which delegate* — only that some valid slot
    /// nullifier currently backs `delegate_persona_id`.
    ///
    /// `submitter` is the `AccountId` that called `back_delegate` to create this entry, required
    /// again by `remove_backing` before it will free the slot. This is not a privacy regression:
    /// the `backing-nullifier` circuit deliberately does not bind any `AccountId` (see its
    /// module docs), so the raw `(zk_proof, public_inputs)` bytes are otherwise replayable by
    /// *anyone* who observed them in `back_delegate`'s own public call data — recording the
    /// signer who already publicly submitted them and requiring a match closes that replay
    /// without exposing anything the original `back_delegate` call didn't already.
    #[pallet::storage]
    pub type UsedBackingNullifier<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], (T::AccountId, [u8; 32])>;

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
        DelegateRegistered {
            delegate: T::AccountId,
            delegate_persona_id: [u8; 32],
            display_name: BoundedVec<u8, ConstU32<64>>,
        },
        DelegateActivated { delegate: T::AccountId },
        DelegateDeactivated { delegate: T::AccountId },
        /// `backing_nullifier` is the only public trace of this backing action — deliberately
        /// not a `backer: T::AccountId` field, since the whole point of the nullifier-based
        /// backing scheme is that no on-chain event links a specific citizen to a specific
        /// delegate (see this pallet's module doc comment).
        DelegateBacked { delegate: T::AccountId, backing_nullifier: [u8; 32] },
        DelegateBackingRemoved { delegate: T::AccountId, backing_nullifier: [u8; 32] },
        DelegateTermWarning { delegate: T::AccountId, blocks_remaining: BlockNumberFor<T> },
        DelegateTermExpired { delegate: T::AccountId },
        DelegateBreakEnded { delegate: T::AccountId },

        // ── Legislature elections ──
        /// Periodic election ran; `seated` delegates installed into the legislature.
        LegislatureElectionRun { at_block: BlockNumberFor<T>, seated: u32 },
        /// A delegate would otherwise have been seated this cycle (Active, active citizen, and
        /// ranked within the top `LegislatureSeats` by backing) but was skipped because they do
        /// not have a current asset disclosure on file. The next-highest-backed eligible
        /// delegate takes the seat instead — see `run_election`'s doc comment for why this is a
        /// skip-and-fall-through rather than a hard error.
        SeatingSkippedNoDisclosure { account: T::AccountId },
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
        /// `back_delegate`: this proof's `backing_nullifier` is already backing some delegate.
        AlreadyBacking,
        /// `remove_backing`: this proof's `backing_nullifier` is not currently recorded as
        /// backing `delegate` under the calling account — either it was never used, or it was
        /// last backed by a different account (see `UsedBackingNullifier`'s doc comment for why
        /// the submitting account must match), or it currently backs a different delegate.
        NotBacking,
        DelegateOnBreak,
        BackingThresholdOutOfBounds,
        WarningPctInvalid,
        FloorExceedsCeiling,
        ThresholdBelowFloor,
        ThresholdAboveCeiling,
        /// Legislature seat count must be at least 1.
        ElectionSeatsZero,
        /// Election cycle length cannot be zero — elections would never run.
        ElectionCycleBlocksZero,
        /// `persona_account` must equal the calling account — see `register_as_delegate`'s doc
        /// comment.
        PersonaAccountMismatch,
        /// This `delegate_persona_id` was already used by a (possibly different) persona
        /// registration.
        DelegatePersonaAlreadyUsed,
        /// The outer ZKPassport proof failed `T::ZkVerifier`'s pairing check, or had too few
        /// public inputs to plausibly carry a `disclosure`-shaped subproof.
        InvalidZKProof,
        /// One or more of the 5 submitted committee-key hashes does not match the
        /// governance-approved key for its slot under the given OPRF scheme version.
        CommitteeKeyMismatch,
        /// `T::DelegatePersonaVerifier::check_delegate_persona` rejected the recomputed
        /// `param_commitment` — the proof does not attest to this `delegate_persona_id`/
        /// `persona_account`/`scheme_version`/`oprf_pk_hashes` tuple.
        InvalidDelegatePersonaProof,
        /// `T::BackingProofVerifier` rejected the `backing-nullifier` proof outright (bad
        /// envelope, wrong shape, or a failed UltraHonk pairing check).
        InvalidBackingProof,
        /// The proof's `root` public input is not a backing-commitment tree root that pallet-
        /// identity currently recognizes as valid (too old, or never existed).
        InvalidBackingRoot,
        /// The proof's `delegate_persona_id` public input does not match the `delegate` argument
        /// the caller named.
        DelegatePersonaMismatch,
        /// The proof's `max_backings_per_citizen` public input does not equal the live
        /// `MaxBackingsPerCitizen` governance value — see that circuit's own module docs for why
        /// this is a checked public input rather than a compile-time constant. This is the
        /// on-chain half of the backing-cap enforcement; the other half (that a citizen cannot
        /// construct a proof for a `slot_index` outside that bound at all) is enforced entirely
        /// inside the circuit itself, not by any counter this pallet maintains.
        MaxBackingsMismatch,
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

        /// Register the caller as a delegate under a fresh, ZK-derived persona.
        ///
        /// `zk_proof`/`public_inputs` is a fresh outer ZKPassport proof (same envelope/shape as
        /// `pallet_identity_zk::register_citizen`'s own — see that call's doc comment) carrying
        /// a `delegate-persona` disclosure subproof (`circuits/oprf-identity-anchor/
        /// delegate-persona`), *not* the citizen's original registration proof: delegate-persona
        /// creation is a genuinely separate, on-demand proof event with its own 5-committee OPRF
        /// round-trip, deliberately not folded into registration (see that circuit's module
        /// docs). `delegate_persona_id` is the resulting stable per-citizen identifier;
        /// `persona_account` (== the caller, enforced below) is the account it gets bound to.
        /// `scheme_version`/`oprf_pk_hashes` are the same per-committee key material
        /// `register_citizen` takes, checked against the governance-approved keys on file for
        /// that scheme version before the persona commitment is trusted.
        #[pallet::call_index(7)]
        #[pallet::weight(T::WeightInfo::register_as_delegate())]
        pub fn register_as_delegate(
            origin: OriginFor<T>,
            persona_account: T::AccountId,
            delegate_persona_id: [u8; 32],
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            public_inputs: BoundedVec<[u8; 32], ConstU32<18>>,
            scheme_version: u32,
            oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES],
            display_name: BoundedVec<u8, ConstU32<64>>,
            profile_ipfs_hash: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            // The proof binds `persona_account`, not `who`, into `param_commitment` -- so the
            // front-running protection only holds if the two are forced equal here (otherwise
            // an observer could front-run someone else's valid proof by resubmitting it with
            // themselves as `who` and pointing `persona_account` at the original prover's
            // account, though that would still only register *that* account, not steal
            // anything -- requiring equality closes even that).
            ensure!(who == persona_account, Error::<T>::PersonaAccountMismatch);
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::NotActiveCitizen);
            ensure!(!Delegates::<T>::contains_key(&who), Error::<T>::AlreadyRegisteredAsDelegate);
            ensure!(
                !DelegatePersonaUsed::<T>::contains_key(delegate_persona_id),
                Error::<T>::DelegatePersonaAlreadyUsed
            );

            // Outer proof pairing check first (expensive, and the gate before trusting anything
            // else in `public_inputs`) -- same ordering `pallet_identity_zk::register_citizen`
            // uses. `> 8` mirrors that call's own `>= 9`: 8 fixed fields plus at least one
            // `param_commitment` slot.
            ensure!(public_inputs.len() > 8, Error::<T>::InvalidZKProof);
            ensure!(
                T::ZkVerifier::verify(zk_proof.as_slice(), public_inputs.as_slice()),
                Error::<T>::InvalidZKProof
            );

            // Committee keys must be governance-approved for this scheme version -- otherwise a
            // prover could self-mint an unlimited number of delegate personas against keys the
            // real OPRF committee never signed off on (see `CommitteeKeyChecker`'s doc comment).
            ensure!(
                T::CommitteeKeyChecker::are_committee_keys_approved(scheme_version, &oprf_pk_hashes),
                Error::<T>::CommitteeKeyMismatch
            );

            let persona_account_bytes = T::AccountIdToBytes::to_bytes(&persona_account);
            ensure!(
                T::DelegatePersonaVerifier::check_delegate_persona(
                    public_inputs.as_slice(),
                    delegate_persona_id,
                    persona_account_bytes,
                    scheme_version,
                    oprf_pk_hashes,
                ),
                Error::<T>::InvalidDelegatePersonaProof
            );

            DelegatePersonaUsed::<T>::insert(delegate_persona_id, ());
            DelegatePersonaIdOf::<T>::insert(&who, delegate_persona_id);

            Delegates::<T>::insert(&who, DelegateInfo {
                display_name: display_name.clone(),
                profile_ipfs_hash,
                status: DelegateStatus::Pending,
                consecutive_terms: 0,
                term_start_block: None,
                break_until_block: None,
                warning_emitted: false,
            });
            Self::deposit_event(Event::DelegateRegistered {
                delegate: who,
                delegate_persona_id,
                display_name,
            });
            Ok(())
        }

        /// Back `delegate` using a `backing-nullifier` proof. Each citizen may back at most
        /// `MaxBackingsPerCitizen` delegates simultaneously -- enforced entirely inside the
        /// circuit (see `Error::MaxBackingsMismatch`'s doc comment), not by any per-citizen
        /// counter here. If this backing pushes the delegate to or above the threshold, they
        /// become Active.
        ///
        /// There is deliberately no `CannotBackSelf` check any more -- see this pallet's module
        /// doc comment for why one would give false assurance under the nullifier-based design.
        #[pallet::call_index(8)]
        #[pallet::weight(T::WeightInfo::back_delegate())]
        pub fn back_delegate(
            origin: OriginFor<T>,
            delegate: T::AccountId,
            zk_proof: BoundedVec<u8, ConstU32<8192>>,
            public_inputs: [[u8; 32]; 4],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::NotActiveCitizen);
            let info = Delegates::<T>::get(&delegate).ok_or(Error::<T>::DelegateNotFound)?;
            ensure!(info.status != DelegateStatus::OnBreak, Error::<T>::DelegateOnBreak);

            let (nullifier, delegate_persona_id) =
                Self::verify_backing_proof(&delegate, zk_proof.as_slice(), &public_inputs)?;
            ensure!(!UsedBackingNullifier::<T>::contains_key(nullifier), Error::<T>::AlreadyBacking);

            UsedBackingNullifier::<T>::insert(nullifier, (who, delegate_persona_id));
            let new_count = BackingCount::<T>::get(&delegate).saturating_add(1);
            BackingCount::<T>::insert(&delegate, new_count);
            Self::deposit_event(Event::DelegateBacked {
                delegate: delegate.clone(),
                backing_nullifier: nullifier,
            });

            if new_count >= BackingThreshold::<T>::get()
                && Delegates::<T>::get(&delegate)
                    .map_or(false, |d| d.status == DelegateStatus::Pending)
            {
                Self::activate_delegate(&delegate);
            }
            Ok(())
        }

        /// Remove backing from `delegate`, freeing the slot for reuse. Requires the *same*
        /// account that originally called `back_delegate` to resubmit the *same*
        /// `backing-nullifier` proof (recomputing the same `backing_nullifier`, since it depends
        /// only on the citizen's fixed secret and slot index, not on `delegate_persona_id`) --
        /// see `UsedBackingNullifier`'s doc comment for why.
        #[pallet::call_index(9)]
        #[pallet::weight(T::WeightInfo::remove_backing())]
        pub fn remove_backing(
            origin: OriginFor<T>,
            delegate: T::AccountId,
            zk_proof: BoundedVec<u8, ConstU32<8192>>,
            public_inputs: [[u8; 32]; 4],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let (nullifier, delegate_persona_id) =
                Self::verify_backing_proof(&delegate, zk_proof.as_slice(), &public_inputs)?;

            let (submitter, recorded_persona_id) =
                UsedBackingNullifier::<T>::get(nullifier).ok_or(Error::<T>::NotBacking)?;
            ensure!(submitter == who, Error::<T>::NotBacking);
            ensure!(recorded_persona_id == delegate_persona_id, Error::<T>::NotBacking);

            UsedBackingNullifier::<T>::remove(nullifier);
            let new_count = BackingCount::<T>::get(&delegate).saturating_sub(1);
            BackingCount::<T>::insert(&delegate, new_count);
            Self::deposit_event(Event::DelegateBackingRemoved {
                delegate: delegate.clone(),
                backing_nullifier: nullifier,
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

    /// Big-endian-packs `value` into the low 4 bytes of a 32-byte field element — the same
    /// encoding `runtime/src/anchor_verifier.rs`'s `u32_to_field_bytes` and the
    /// `backing-nullifier` circuit's own `max_backings_per_citizen` public input use. Duplicated
    /// here rather than imported since pallet-elections deliberately has no dependency on the
    /// runtime crate.
    fn u32_to_field_bytes(value: u32) -> [u8; 32] {
        let mut bytes = [0u8; 32];
        bytes[28..32].copy_from_slice(&value.to_be_bytes());
        bytes
    }

    impl<T: Config> Pallet<T> {

        /// Verifies a `backing-nullifier` proof claiming to target `delegate`, returning
        /// `(backing_nullifier, delegate_persona_id)` on success. Shared by
        /// `back_delegate`/`remove_backing` — see `BackingProofVerifier`/`BackingRootChecker`'s
        /// doc comments for what each individual check covers; this only performs the
        /// storage-dependent half `runtime/src/backing_nullifier_verifier.rs`'s module docs say
        /// a caller must do (root validity, `delegate_persona_id`/`max_backings_per_citizen`
        /// matching live state).
        fn verify_backing_proof(
            delegate: &T::AccountId,
            zk_proof: &[u8],
            public_inputs: &[[u8; 32]; 4],
        ) -> Result<([u8; 32], [u8; 32]), DispatchError> {
            ensure!(
                T::BackingProofVerifier::verify(zk_proof, public_inputs.as_slice()),
                Error::<T>::InvalidBackingProof
            );

            let root = public_inputs[0];
            ensure!(
                T::BackingRootChecker::is_valid_backing_commitment_root(root),
                Error::<T>::InvalidBackingRoot
            );

            let delegate_persona_id = DelegatePersonaIdOf::<T>::get(delegate)
                .ok_or(Error::<T>::DelegateNotFound)?;
            ensure!(public_inputs[1] == delegate_persona_id, Error::<T>::DelegatePersonaMismatch);

            let max_backings = MaxBackingsPerCitizen::<T>::get();
            ensure!(
                public_inputs[2] == u32_to_field_bytes(max_backings),
                Error::<T>::MaxBackingsMismatch
            );

            Ok((public_inputs[3], delegate_persona_id))
        }

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
        ///
        /// ## Asset-disclosure gate: skip-and-fall-through, not hard-error
        /// A delegate without a *current* asset disclosure (see `DisclosureChecker`) is
        /// excluded from the candidate pool entirely — not seated, and not counted against the
        /// `LegislatureSeats` cap — so the next-highest-backed eligible delegate fills the seat
        /// instead. This deliberately mirrors the existing `CitizenChecker` re-check just below
        /// rather than making the whole `run_election` call (an `on_initialize` hook, not even
        /// a fallible extrinsic a citizen could retry) fail outright: `on_initialize` runs
        /// unconditionally every block past the cycle boundary, so an error here would mean the
        /// election *never* runs again until someone manually intervenes — one official's
        /// lapsed paperwork should not be able to freeze legislature seating for everyone else.
        /// It also matches the real-world remedy: the affected delegate files an up-to-date
        /// disclosure and is eligible again next cycle, no governance action required. The skip
        /// is not silent, though — `Event::SeatingSkippedNoDisclosure` is emitted per skipped
        /// account so it is visible on-chain.
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
                    if info.status != DelegateStatus::Active
                        || !T::CitizenChecker::is_active_citizen(&addr)
                    {
                        return None;
                    }
                    // Same reasoning, for asset-disclosure currency — see this function's doc
                    // comment for why this is a skip (excluded from the pool, next-highest
                    // eligible delegate takes the seat) rather than a hard error.
                    if !T::DisclosureChecker::has_current_disclosure(&addr) {
                        Self::deposit_event(Event::SeatingSkippedNoDisclosure {
                            account: addr.clone(),
                        });
                        return None;
                    }
                    Some((addr.clone(), BackingCount::<T>::get(&addr)))
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
