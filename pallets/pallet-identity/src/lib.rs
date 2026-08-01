//! # Identity Pallet
//!
//! ZK passport verification and citizen registry.
//! Integrates with Rarimo Freedom Tool for biometric passport NFC + ZK proof.
//! Nullifier = Poseidon2(national_id || country_code) — stable across renewals.
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::Saturating;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Pluggable ZK proof verifier. Implement with the real Rarimo Groth16 verifier.
        /// Use a no-op impl for testing.
        type ZkVerifier: ZkProofVerifier;
        /// The origin permitted to suspend and restore citizen voting rights.
        /// Wired to `pallet_courts::EnsureOracle` in the runtime — only a court ruling
        /// (AI judge or jury) can suspend or restore a citizen.
        type SuspensionOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
        /// The origin permitted to manage the trusted issuer Merkle root allowlist
        /// (add/remove country CA certificate roots). Keeping this separate from
        /// SuspensionOrigin lets governance rotate the allowlist without touching
        /// the court-controlled suspension key. Wired to
        /// `pallet_legislature::EnsureLegislatureMotion` in the runtime.
        type AdminOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
        /// Pluggable OPRF identity-anchor proof verifier (see `AnchorProofVerifier` below):
        /// registration-anchor proofs, reverification/liveness proofs, and cross-scheme
        /// migration-consistency ("dual evaluation") proofs. The real OPRF committee
        /// cryptography itself is out of scope for this pallet (see HANDOFF log #67/#68) —
        /// use a passthrough impl for dev/testing until the OPRF circuit work lands.
        type AnchorVerifier: AnchorProofVerifier;
        /// Number of blocks a registration/reverification remains valid before
        /// `is_active_citizen` lazily starts treating the citizen as inactive for voting
        /// purposes. A governance-tunable parameter (see HANDOFF log #67: "make the rotation
        /// interval itself a governance parameter... not a hardcoded 4 years" — the same
        /// principle applies here, though log #67 leaves this specific cadence as an
        /// explicitly open question distinct from the OPRF rotation cadence).
        #[pallet::constant]
        type ReverificationPeriod: Get<u32>;
        /// The origin permitted to force an out-of-cycle OPRF scheme-version bump ahead of
        /// the normal 4-year schedule (e.g. a suspected OPRF committee compromise — see
        /// HANDOFF log #67's emergency-rotation mechanics). Distinct from the normal
        /// scheduled-rotation path (`rotate_oprf_scheme`, gated by `AdminOrigin`). Intended to
        /// be wired to `pallet_emergency_council`'s emergency-active state in the runtime; see
        /// runtime/src/configs/mod.rs for the current wiring (no dedicated `EnsureOrigin`
        /// exists yet on that pallet, so it's a placeholder there for now).
        type EmergencyRotationOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
    }

    /// Trait for verifying Rarimo-style Groth16 ZK passport proofs.
    /// Implement this in the runtime, plugging in the real Rarimo verifier key and circuit.
    pub trait ZkProofVerifier {
        /// Returns true if the proof is valid for the given nullifier and public inputs.
        /// proof_bytes: serialized Groth16 proof (A, B, C points on BN254).
        /// public_inputs: [nullifier_hash, passport_expiry_timestamp, country_code_hash].
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
    }

    /// Trait for verifying OPRF-derived identity-anchor proofs (see HANDOFF log #67 for the
    /// full Sybil-resistance design this implements). Distinct from `ZkProofVerifier`: that
    /// trait authenticates the passport itself and the per-vote nullifier; this one covers the
    /// separate, one-time registration-uniqueness anchor derived from the passport's stable
    /// personal-number field via an OPRF committee. The real OPRF cryptography is out of scope
    /// for this pallet — implement this once the OPRF circuit work lands, and use a
    /// passthrough impl for dev/testing until then.
    pub trait AnchorProofVerifier {
        /// Verifies that `anchor` was correctly derived — via the OPRF committee, under
        /// `scheme_version` — from the registrant's passport personal-number field, without
        /// revealing that field's value.
        fn verify_registration_anchor(
            proof_bytes: &[u8],
            anchor: [u8; 32],
            scheme_version: u32,
        ) -> bool;
        /// Verifies a reverification/liveness proof: the citizen currently holds a
        /// still-valid, unexpired passport that recomputes to the same anchor already on file.
        fn verify_reverification(proof_bytes: &[u8], anchor: [u8; 32]) -> bool;
        /// Verifies a migration consistency ("dual evaluation") proof: `old_anchor` and
        /// `new_anchor` were both derived from the same underlying personal-number value, just
        /// under different OPRF scheme versions — without revealing that value.
        fn verify_migration(proof_bytes: &[u8], old_anchor: [u8; 32], new_anchor: [u8; 32]) -> bool;
    }

    /// Maps nullifier hash -> registered AccountId. Prevents double-registration.
    #[pallet::storage]
    pub type NullifierRegistry<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], T::AccountId>;

    /// Maps AccountId -> nullifier hash for reverse lookup.
    #[pallet::storage]
    pub type CitizenNullifier<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32]>;

    /// Dense indexed list of citizens for O(1) random selection by courts.
    /// Index 0..TotalCitizens-1 are always occupied.
    #[pallet::storage]
    pub type CitizenIndex<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

    /// Reverse index: AccountId -> position in CitizenIndex. Used for O(1) swap-and-pop.
    #[pallet::storage]
    pub type CitizenPosition<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32>;

    /// Total number of registered citizens.
    #[pallet::storage]
    pub type TotalCitizens<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Court-ordered voting suspensions: nullifier -> optional block when suspension lifts.
    /// None means suspended indefinitely; Some(block) means suspended until that block.
    /// Key absent means not suspended.
    #[pallet::storage]
    pub type SuspendedNullifiers<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], Option<BlockNumberFor<T>>>;

    /// Trusted issuer Merkle roots (certificate_registry_root from ZKPassport's outer
    /// circuit, public_inputs[0]). A proof is only accepted if its certificate_registry_root
    /// matches one of these roots. Roots represent trusted sets of country certificate
    /// authorities.
    #[pallet::storage]
    pub type AllowedMerkleRoots<T: Config> = StorageMap<_, Identity, [u8; 32], bool, ValueQuery>;

    /// Global OPRF anchor-scheme version. Incremented on rotation — scheduled
    /// (`rotate_oprf_scheme`, `AdminOrigin`) or emergency (`emergency_rotate_oprf_scheme`,
    /// `EmergencyRotationOrigin`). New registrations always anchor under this current value.
    /// Bumping this counter does **not** retroactively invalidate anyone: existing citizens
    /// keep whatever version their on-file anchor was registered under until they individually
    /// call `migrate_oprf_scheme` — see HANDOFF log #67's "dual evaluation" window, where both
    /// the outgoing and incoming OPRF committees stay live and trusted side by side for a
    /// transition period.
    #[pallet::storage]
    pub type OprfSchemeVersion<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Identity-anchor exclusion registry, keyed by (scheme_version, anchor) -> the AccountId
    /// registered against it. Keying on scheme version (rather than a single flat
    /// anchor -> AccountId map) is what makes `migrate_oprf_scheme` possible without a global
    /// stop-the-world cutover: a citizen's outgoing-scheme entry can be atomically retired and
    /// a same-citizen incoming-scheme entry inserted, without either colliding with a
    /// *different* citizen's still-valid entry under the other version.
    ///
    /// Deliberately separate from `NullifierRegistry`/`CitizenNullifier` (the per-vote
    /// nullifier used for casting ballots) — this anchor exists purely as a one-time
    /// registration-time uniqueness gate and must never be exposed anywhere that could let it
    /// be correlated with voting activity (see HANDOFF log #67).
    #[pallet::storage]
    pub type IdentityAnchorRegistry<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, [u8; 32]), T::AccountId>;

    /// Reverse lookup: AccountId -> the (scheme_version, anchor) pair currently on file.
    /// `migrate_oprf_scheme` uses this to find and retire the caller's own outgoing-scheme
    /// anchor — checking the caller's own on-file entry (rather than trusting the `old_anchor`
    /// argument on its own) is what stops a citizen from "migrating" using someone else's
    /// anchor value.
    #[pallet::storage]
    pub type CitizenAnchor<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, (u32, [u8; 32])>;

    /// Block at which a citizen's registration lapses for voting purposes unless renewed.
    /// Pushed forward by `reverify_citizen`. Checked lazily inside `is_active_citizen`, the
    /// same point-of-use pattern already used for suspension — no separate background sweep.
    /// Always set at registration; a missing entry for an otherwise-registered citizen is
    /// treated as "past deadline" (inactive) rather than an unbounded grace period.
    #[pallet::storage]
    pub type ReverificationDeadline<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>>;

    /// Self-declared attestation, made at registration, that the citizen holds no other
    /// currently-valid passport from this deployment's country. Purely evidentiary: it exists
    /// so that, if later found false, `pallet-courts`' existing `CitizenConduct` case type has
    /// an on-chain record to point to (no new pallet-courts logic needed — see HANDOFF log
    /// #67). Deliberately not cleared on revocation: the attestation is a historical record of
    /// what was claimed at the time, not a live status flag.
    #[pallet::storage]
    pub type SelfDeclaredSingleDocument<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        CitizenRegistered { who: T::AccountId, nullifier: [u8; 32] },
        CitizenRevoked { who: T::AccountId },
        /// Voting rights suspended by court ruling. `until` = None means indefinite.
        CitizenSuspended { nullifier: [u8; 32], until: Option<BlockNumberFor<T>> },
        /// Voting rights restored (sentence served or conviction overturned).
        CitizenRestored { nullifier: [u8; 32] },
        MerkleRootAdded { merkle_root: [u8; 32] },
        MerkleRootRemoved { merkle_root: [u8; 32] },
        /// A citizen's registration/reverification deadline was pushed forward.
        CitizenReverified { who: T::AccountId, deadline: BlockNumberFor<T> },
        /// A citizen migrated their identity anchor to a new OPRF scheme version. Anchor
        /// values themselves are never included in events (see `IdentityAnchorRegistry`'s
        /// doc comment) — only the resulting scheme version is public.
        OprfAnchorMigrated { who: T::AccountId, new_scheme_version: u32 },
        /// The OPRF scheme version was bumped on the normal governance-scheduled cadence.
        OprfSchemeRotated { new_version: u32 },
        /// The OPRF scheme version was bumped out-of-cycle via `EmergencyRotationOrigin`.
        OprfSchemeEmergencyRotated { new_version: u32 },
        /// A citizen recorded a self-declaration of holding no other valid passport.
        SelfDeclarationRecorded { who: T::AccountId },
    }

    #[pallet::error]
    pub enum Error<T> {
        AlreadyRegistered,
        NullifierAlreadyUsed,
        InvalidZKProof,
        NotRegistered,
        NotSuspended,
        /// The proof's certificate_registry_root is not in the on-chain allowlist of
        /// trusted issuers.
        IssuerNotAllowed,
        /// Citizen registry is full (u32::MAX citizens).
        TotalCitizensOverflow,
        /// A suspended citizen may not self-revoke to escape an active court ruling.
        CannotRevokeWhileSuspended,
        /// The identity-anchor registration proof failed verification.
        InvalidAnchorProof,
        /// This anchor is already registered under the current OPRF scheme version — either a
        /// genuine duplicate registration attempt, or a citizen who should be using
        /// `migrate_oprf_scheme` instead of `register_citizen`.
        AnchorAlreadyUsed,
        /// The reverification/liveness proof failed verification.
        InvalidReverificationProof,
        /// The claimed old anchor is not a genuine, currently-registered anchor belonging to
        /// the caller under their on-file OPRF scheme version.
        OldAnchorNotFound,
        /// The proposed new anchor is already registered under the target scheme version.
        NewAnchorAlreadyUsed,
        /// The migration consistency ("dual evaluation") proof failed verification.
        InvalidMigrationProof,
        /// OPRF scheme version counter would overflow u32::MAX.
        OprfSchemeVersionOverflow,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Register a new citizen using a ZKPassport passport proof.
        ///
        /// `public_inputs` is ZKPassport's `main/outer/count_N` layout — see
        /// `runtime/src/verifier.rs`'s module docs for the authoritative field-by-field
        /// breakdown (confirmed against the circuits repo source, not assumed). In short:
        /// `certificate_registry_root` is index 0, `scoped_nullifier` is index `len - 2`
        /// (`6 + D`, where `D = outer_count - 3` disclosure subproofs, so the exact index
        /// shifts with which outer-circuit variant produced the proof — deriving it from
        /// `len` rather than hardcoding it is what makes this work across variants).
        /// **Not** Rarimo's old 5-signal layout (`dg15PubKeyHash`, `passportHash`,
        /// `dg1Commitment`, `pkIdentityHash`, `slaveMerkleRoot`) this call used to assume.
        ///
        /// `anchor`/`anchor_proof`: the mandatory OPRF identity anchor and its registration
        /// proof (see HANDOFF log #67, `AnchorProofVerifier`). Checked in addition to, and
        /// entirely separately from, the ZK passport proof above — a duplicate anchor is
        /// rejected here even if the passport proof itself is perfectly valid, since the whole
        /// point of the anchor is to catch same-person double registration that a fresh,
        /// renewal-stable-nullifier-free passport proof alone cannot.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn register_citizen(
            origin: OriginFor<T>,
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            // count_N exposes N + 5 public inputs; count_13 (the largest allowlisted
            // variant) is the ceiling at 18.
            public_inputs: BoundedVec<[u8; 32], ConstU32<18>>,
            anchor: [u8; 32],
            anchor_proof: BoundedVec<u8, ConstU32<4096>>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!CitizenNullifier::<T>::contains_key(&who), Error::<T>::AlreadyRegistered);
            // Smallest real outer-circuit variant is count_4 (8 fixed inputs + 1
            // disclosure subproof) => 9 public inputs. Anything shorter cannot be a
            // genuine ZKPassport proof.
            ensure!(public_inputs.len() >= 9, Error::<T>::InvalidZKProof);

            // 1. Check allowlist first (cheap storage lookup, no proof work yet).
            //    certificate_registry_root is public_inputs[0].
            ensure!(AllowedMerkleRoots::<T>::get(public_inputs[0]), Error::<T>::IssuerNotAllowed);

            // 2. Verify the ZK proof (expensive BN254 pairing). Only after confirming the
            //    issuer root is trusted to avoid wasting compute on untrusted roots.
            ensure!(
                T::ZkVerifier::verify(zk_proof.as_slice(), public_inputs.as_slice()),
                Error::<T>::InvalidZKProof
            );

            // 3. Only after the proof is authenticated, extract and check the nullifier.
            //    Checking nullifier uniqueness before proof verification would let an attacker
            //    learn whether a nullifier is registered without submitting a valid proof.
            //    scoped_nullifier sits at index `6 + D` = `len - 2` (oprf_pk_hash is the
            //    last field, at `len - 1`).
            let nullifier = public_inputs[public_inputs.len() - 2];
            ensure!(
                !NullifierRegistry::<T>::contains_key(nullifier),
                Error::<T>::NullifierAlreadyUsed
            );

            // 4. Mandatory identity-anchor check (HANDOFF log #67). Same ordering rationale as
            //    the nullifier above: verify the anchor proof before consulting the exclusion
            //    registry, so an attacker can't probe anchor uniqueness with a bogus proof.
            let scheme_version = OprfSchemeVersion::<T>::get();
            ensure!(
                T::AnchorVerifier::verify_registration_anchor(
                    anchor_proof.as_slice(),
                    anchor,
                    scheme_version
                ),
                Error::<T>::InvalidAnchorProof
            );
            ensure!(
                !IdentityAnchorRegistry::<T>::contains_key((scheme_version, anchor)),
                Error::<T>::AnchorAlreadyUsed
            );

            let pos = TotalCitizens::<T>::get();
            let new_total = pos.checked_add(1).ok_or(Error::<T>::TotalCitizensOverflow)?;
            CitizenIndex::<T>::insert(pos, &who);
            CitizenPosition::<T>::insert(&who, pos);
            TotalCitizens::<T>::put(new_total);
            CitizenNullifier::<T>::insert(&who, nullifier);
            NullifierRegistry::<T>::insert(nullifier, &who);
            IdentityAnchorRegistry::<T>::insert((scheme_version, anchor), &who);
            CitizenAnchor::<T>::insert(&who, (scheme_version, anchor));
            let deadline = frame_system::Pallet::<T>::block_number()
                .saturating_add(BlockNumberFor::<T>::from(T::ReverificationPeriod::get()));
            ReverificationDeadline::<T>::insert(&who, deadline);
            Self::deposit_event(Event::CitizenRegistered { who, nullifier });
            Ok(())
        }

        /// Revoke a citizen registration (e.g. country removed from allowlist).
        /// Blocked while the citizen has an active or unexpired suspension — a suspended
        /// citizen cannot self-revoke to escape an active court ruling.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn revoke_citizen(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let nullifier =
                CitizenNullifier::<T>::get(&who).ok_or(Error::<T>::NotRegistered)?;
            // Block self-revocation if the citizen is currently suspended (including
            // time-limited suspensions whose block hasn't passed yet).
            if let Some(entry) = SuspendedNullifiers::<T>::get(nullifier) {
                let is_active_suspension = match entry {
                    None => true, // indefinite
                    Some(until) => frame_system::Pallet::<T>::block_number() <= until,
                };
                ensure!(!is_active_suspension, Error::<T>::CannotRevokeWhileSuspended);
            }
            CitizenNullifier::<T>::remove(&who);
            NullifierRegistry::<T>::remove(nullifier);
            SuspendedNullifiers::<T>::remove(nullifier);
            ReverificationDeadline::<T>::remove(&who);
            // Retire the citizen's identity anchor too — a fresh registration under a new
            // anchor is expected to go through the normal exclusion check again. Note:
            // `SelfDeclaredSingleDocument` is deliberately NOT cleared here — see its storage
            // doc comment for why (it's a historical attestation record, not a live flag).
            if let Some((version, anchor)) = CitizenAnchor::<T>::take(&who) {
                IdentityAnchorRegistry::<T>::remove((version, anchor));
            }
            // Swap-and-pop: fill the vacated slot with the last citizen to keep the index dense.
            let pos = CitizenPosition::<T>::take(&who).ok_or(Error::<T>::NotRegistered)?;
            let last = TotalCitizens::<T>::get().saturating_sub(1);
            TotalCitizens::<T>::put(last);
            if pos < last {
                if let Some(swapped) = CitizenIndex::<T>::get(last) {
                    CitizenIndex::<T>::insert(pos, &swapped);
                    CitizenPosition::<T>::insert(&swapped, pos);
                }
            }
            CitizenIndex::<T>::remove(last);
            Self::deposit_event(Event::CitizenRevoked { who });
            Ok(())
        }

        /// Suspend a citizen's voting and budget-allocation rights by court order.
        /// `until`: None = indefinite suspension; Some(block) = suspension lifts at that block.
        /// If the citizen is already suspended, the existing record is replaced (allows courts
        /// to extend or modify an active suspension).
        /// Origin: `SuspensionOrigin` (court ruling — see `Config::SuspensionOrigin`).
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn suspend_citizen(
            origin: OriginFor<T>,
            nullifier: [u8; 32],
            until: Option<BlockNumberFor<T>>,
        ) -> DispatchResult {
            T::SuspensionOrigin::ensure_origin(origin)?;
            ensure!(NullifierRegistry::<T>::contains_key(nullifier), Error::<T>::NotRegistered);
            // Upsert: courts may extend or modify an existing suspension.
            SuspendedNullifiers::<T>::insert(nullifier, until);
            Self::deposit_event(Event::CitizenSuspended { nullifier, until });
            Ok(())
        }

        /// Restore suspended voting rights.
        /// Called when a sentence is served, the waiting period passes, or a conviction is
        /// overturned on appeal. Works on both active and expired-but-not-yet-cleaned-up records.
        /// Origin: `SuspensionOrigin` (court ruling — see `Config::SuspensionOrigin`).
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn restore_citizen_rights(
            origin: OriginFor<T>,
            nullifier: [u8; 32],
        ) -> DispatchResult {
            T::SuspensionOrigin::ensure_origin(origin)?;
            ensure!(
                SuspendedNullifiers::<T>::contains_key(nullifier),
                Error::<T>::NotSuspended
            );
            SuspendedNullifiers::<T>::remove(nullifier);
            Self::deposit_event(Event::CitizenRestored { nullifier });
            Ok(())
        }

        /// Add a trusted issuer Merkle root. Proofs with this slaveMerkleRoot will be accepted.
        /// The root represents a specific snapshot of the trusted country CA certificate set.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn add_allowed_merkle_root(
            origin: OriginFor<T>,
            merkle_root: [u8; 32],
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(origin)?;
            AllowedMerkleRoots::<T>::insert(merkle_root, true);
            Self::deposit_event(Event::MerkleRootAdded { merkle_root });
            Ok(())
        }

        /// Remove a trusted issuer Merkle root (e.g. after a CA revocation event).
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn remove_allowed_merkle_root(
            origin: OriginFor<T>,
            merkle_root: [u8; 32],
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(origin)?;
            AllowedMerkleRoots::<T>::remove(merkle_root);
            Self::deposit_event(Event::MerkleRootRemoved { merkle_root });
            Ok(())
        }

        /// Periodic re-verification (HANDOFF log #67): proves the caller still holds a
        /// currently-valid, unexpired passport, and pushes `ReverificationDeadline` forward by
        /// `ReverificationPeriod` blocks from now. Checked lazily inside `is_active_citizen`
        /// rather than via a background sweep, matching this pallet's existing suspension
        /// pattern.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn reverify_citizen(
            origin: OriginFor<T>,
            reverify_proof: BoundedVec<u8, ConstU32<4096>>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let (_version, anchor) =
                CitizenAnchor::<T>::get(&who).ok_or(Error::<T>::NotRegistered)?;
            ensure!(
                T::AnchorVerifier::verify_reverification(reverify_proof.as_slice(), anchor),
                Error::<T>::InvalidReverificationProof
            );
            let deadline = frame_system::Pallet::<T>::block_number()
                .saturating_add(BlockNumberFor::<T>::from(T::ReverificationPeriod::get()));
            ReverificationDeadline::<T>::insert(&who, deadline);
            Self::deposit_event(Event::CitizenReverified { who, deadline });
            Ok(())
        }

        /// OPRF scheme migration (HANDOFF log #67's "dual evaluation"): moves the caller's own
        /// identity anchor from their current on-file scheme version to the next one
        /// (on-file version + 1), given a proof that `old_anchor` and `new_anchor` were both
        /// derived from the same underlying personal-number value. Requires only the citizen's
        /// current passport — no second document, per log #67.
        ///
        /// Note on versioning: the migration target is always the *caller's own* on-file
        /// version plus one, not necessarily the global `OprfSchemeVersion`. This lets citizens
        /// migrate incrementally and in any order during a rotation window rather than all
        /// needing to act atomically with a global cutover; a citizen who has missed more than
        /// one rotation simply needs to call this once per generation to catch up.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(25_000, 0))]
        pub fn migrate_oprf_scheme(
            origin: OriginFor<T>,
            old_anchor: [u8; 32],
            new_anchor: [u8; 32],
            migration_proof: BoundedVec<u8, ConstU32<4096>>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let (old_version, on_file_anchor) =
                CitizenAnchor::<T>::get(&who).ok_or(Error::<T>::NotRegistered)?;
            let new_version =
                old_version.checked_add(1).ok_or(Error::<T>::OprfSchemeVersionOverflow)?;

            // The old anchor must be exactly what's on file for this citizen, genuinely
            // registered under their current scheme version — this single check also catches
            // a caller passing someone else's anchor, since the registry entry would map to a
            // different AccountId (or not exist for this version at all).
            ensure!(on_file_anchor == old_anchor, Error::<T>::OldAnchorNotFound);
            ensure!(
                IdentityAnchorRegistry::<T>::get((old_version, old_anchor)).as_ref() == Some(&who),
                Error::<T>::OldAnchorNotFound
            );
            ensure!(
                !IdentityAnchorRegistry::<T>::contains_key((new_version, new_anchor)),
                Error::<T>::NewAnchorAlreadyUsed
            );
            ensure!(
                T::AnchorVerifier::verify_migration(
                    migration_proof.as_slice(),
                    old_anchor,
                    new_anchor
                ),
                Error::<T>::InvalidMigrationProof
            );

            IdentityAnchorRegistry::<T>::remove((old_version, old_anchor));
            IdentityAnchorRegistry::<T>::insert((new_version, new_anchor), &who);
            CitizenAnchor::<T>::insert(&who, (new_version, new_anchor));
            Self::deposit_event(Event::OprfAnchorMigrated { who, new_scheme_version: new_version });
            Ok(())
        }

        /// Normal, scheduled OPRF scheme-version bump (the 4-year cycle from HANDOFF log #67).
        /// Not explicitly enumerated as a required call in this feature's task description —
        /// added because without *some* non-emergency way to advance `OprfSchemeVersion`, the
        /// scheduled rotation log #67 describes could never actually happen. Gated by the same
        /// `AdminOrigin` already used for this pallet's other governance-controlled action
        /// (the Merkle root allowlist), rather than introducing a new Config field for it.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn rotate_oprf_scheme(origin: OriginFor<T>) -> DispatchResult {
            T::AdminOrigin::ensure_origin(origin)?;
            let new_version = Self::do_bump_scheme_version()?;
            Self::deposit_event(Event::OprfSchemeRotated { new_version });
            Ok(())
        }

        /// Emergency, out-of-cycle OPRF scheme-version bump (HANDOFF log #67): lets
        /// `EmergencyRotationOrigin` (intended to be pallet-emergency-council) force a rotation
        /// ahead of the normal 4-year schedule, e.g. on a suspected OPRF committee compromise.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn emergency_rotate_oprf_scheme(origin: OriginFor<T>) -> DispatchResult {
            T::EmergencyRotationOrigin::ensure_origin(origin)?;
            let new_version = Self::do_bump_scheme_version()?;
            Self::deposit_event(Event::OprfSchemeEmergencyRotated { new_version });
            Ok(())
        }

        /// Self-declaration (HANDOFF log #67): a registered citizen attests, on-chain, that
        /// they hold no other currently-valid passport from this deployment's country.
        /// Deliberately trivial — just an attestation record for later `pallet-courts`
        /// `CitizenConduct` reference if it's ever found false. Idempotent: re-declaring simply
        /// re-affirms the same flag.
        #[pallet::call_index(10)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn declare_no_other_passport(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_citizen(&who), Error::<T>::NotRegistered);
            SelfDeclaredSingleDocument::<T>::insert(&who, true);
            Self::deposit_event(Event::SelfDeclarationRecorded { who });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Called by pallet-courts via CitizenSuspender runtime trait when a conduct ruling
        /// is finalized. Bypasses the extrinsic origin check — courts are pre-authorized.
        /// Upserts: if a suspension record already exists it is replaced, allowing courts to
        /// extend or modify a citizen's suspension without needing to restore first.
        pub fn suspend_citizen_internal(
            nullifier: [u8; 32],
            until: Option<BlockNumberFor<T>>,
        ) -> DispatchResult {
            ensure!(NullifierRegistry::<T>::contains_key(nullifier), Error::<T>::NotRegistered);
            SuspendedNullifiers::<T>::insert(nullifier, until);
            Self::deposit_event(Event::CitizenSuspended { nullifier, until });
            Ok(())
        }

        /// Used by pallet-courts (via a CitizenSelector trait impl in the runtime).
        pub fn citizen_at(index: u32) -> Option<T::AccountId> {
            CitizenIndex::<T>::get(index)
        }

        pub fn total_citizens() -> u32 {
            TotalCitizens::<T>::get()
        }

        /// True if the account is a registered citizen with no active suspension and no missed
        /// reverification deadline. Timed suspensions are lazily removed from storage once
        /// their block has passed, so subsequent is_citizen / suspend calls do not see stale
        /// records — the reverification deadline is checked the same lazy, point-of-use way
        /// (see HANDOFF log #67), though unlike suspension it isn't cleaned up from storage on
        /// expiry: `reverify_citizen` is the only thing that should ever move it forward again.
        pub fn is_active_citizen(who: &T::AccountId) -> bool {
            let Some(nullifier) = CitizenNullifier::<T>::get(who) else { return false; };
            // A missing deadline is treated as "past deadline" rather than an unbounded grace
            // period; register_citizen always sets one, so this should only trip for data that
            // predates this check.
            match ReverificationDeadline::<T>::get(who) {
                Some(deadline) if frame_system::Pallet::<T>::block_number() <= deadline => {}
                _ => return false,
            }
            match SuspendedNullifiers::<T>::get(nullifier) {
                None => true,
                Some(None) => false, // indefinite suspension
                Some(Some(until)) => {
                    if frame_system::Pallet::<T>::block_number() > until {
                        // Lazily clean up the expired record so future suspend/restore
                        // calls don't incorrectly see it as an active suspension.
                        SuspendedNullifiers::<T>::remove(nullifier);
                        true
                    } else {
                        false
                    }
                }
            }
        }

        /// True if the account is registered (regardless of suspension status).
        pub fn is_citizen(who: &T::AccountId) -> bool {
            CitizenNullifier::<T>::contains_key(who)
        }

        /// Shared bump logic for `rotate_oprf_scheme` / `emergency_rotate_oprf_scheme`. Only
        /// advances the global counter — it never touches any citizen's own on-file anchor
        /// version (see `migrate_oprf_scheme`'s doc comment for why that's a separate step).
        fn do_bump_scheme_version() -> Result<u32, DispatchError> {
            let new_version = OprfSchemeVersion::<T>::get()
                .checked_add(1)
                .ok_or(Error::<T>::OprfSchemeVersionOverflow)?;
            OprfSchemeVersion::<T>::put(new_version);
            Ok(new_version)
        }
    }
}
