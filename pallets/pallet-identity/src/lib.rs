//! # Identity Pallet
//!
//! ZK passport verification and citizen registry.
//! Integrates with Rarimo Freedom Tool for biometric passport NFC + ZK proof.
//! Nullifier = Poseidon2(national_id || country_code) — stable across renewals.
#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {

    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Pluggable ZK proof verifier. Implement with the real Rarimo Groth16 verifier.
        /// Use a no-op impl for testing.
        type ZkVerifier: ZkProofVerifier;
        /// The origin permitted to suspend and restore citizen voting rights.
        /// Currently wired to EnsureRoot in the runtime.
        /// TODO: replace with a court-controlled multisig or collective origin.
        type SuspensionOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
        /// The origin permitted to manage the trusted issuer Merkle root allowlist
        /// (add/remove country CA certificate roots). Keeping this separate from
        /// SuspensionOrigin lets governance rotate the allowlist without touching
        /// the court-controlled suspension key. Use EnsureRoot for now.
        type AdminOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
    }

    /// Trait for verifying Rarimo-style Groth16 ZK passport proofs.
    /// Implement this in the runtime, plugging in the real Rarimo verifier key and circuit.
    pub trait ZkProofVerifier {
        /// Returns true if the proof is valid for the given nullifier and public inputs.
        /// proof_bytes: serialized Groth16 proof (A, B, C points on BN254).
        /// public_inputs: [nullifier_hash, passport_expiry_timestamp, country_code_hash].
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
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

    /// Trusted issuer Merkle roots (slaveMerkleRoot from Rarimo circuit, public_inputs[4]).
    /// A proof is only accepted if its slaveMerkleRoot matches one of these roots.
    /// Roots represent trusted sets of country certificate authorities.
    #[pallet::storage]
    pub type AllowedMerkleRoots<T: Config> = StorageMap<_, Identity, [u8; 32], bool, ValueQuery>;

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
    }

    #[pallet::error]
    pub enum Error<T> {
        AlreadyRegistered,
        NullifierAlreadyUsed,
        InvalidZKProof,
        NotRegistered,
        NotSuspended,
        /// The proof's slaveMerkleRoot is not in the on-chain allowlist of trusted issuers.
        IssuerNotAllowed,
        /// Citizen registry is full (u32::MAX citizens).
        TotalCitizensOverflow,
        /// A suspended citizen may not self-revoke to escape an active court ruling.
        CannotRevokeWhileSuspended,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Register a new citizen using a Rarimo ZK passport proof.
        ///
        /// Rarimo registerIdentity circuit public signals (nPublic = 5):
        ///   [0] dg15PubKeyHash  — Poseidon hash of DG15 active-auth public key (0 if NA)
        ///   [1] passportHash    — PoseidonHash(SHA-256(signedAttributes)[252:])
        ///   [2] dg1Commitment   — PoseidonHash(DG1_chunks..., skIdentity)  ← used as nullifier
        ///   [3] pkIdentityHash  — PoseidonHash(babyJubJub_pubkey.X, .Y)
        ///   [4] slaveMerkleRoot — root of trusted issuer CA certificate tree (public INPUT)
        ///
        /// The nullifier is derived from public_inputs[2] (dg1Commitment). No need to pass it
        /// separately — it binds the passport's MRZ data to the user's on-device identity key.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn register_citizen(
            origin: OriginFor<T>,
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            // Rarimo registerIdentity produces exactly 5 public signals.
            public_inputs: BoundedVec<[u8; 32], ConstU32<16>>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!CitizenNullifier::<T>::contains_key(&who), Error::<T>::AlreadyRegistered);
            ensure!(public_inputs.len() >= 5, Error::<T>::InvalidZKProof);

            // 1. Check allowlist first (cheap storage lookup, no proof work yet).
            //    slaveMerkleRoot is public_inputs[4].
            ensure!(AllowedMerkleRoots::<T>::get(public_inputs[4]), Error::<T>::IssuerNotAllowed);

            // 2. Verify the ZK proof (expensive BN254 pairing). Only after confirming the
            //    issuer root is trusted to avoid wasting compute on untrusted roots.
            ensure!(
                T::ZkVerifier::verify(zk_proof.as_slice(), public_inputs.as_slice()),
                Error::<T>::InvalidZKProof
            );

            // 3. Only after the proof is authenticated, extract and check the nullifier.
            //    Checking nullifier uniqueness before proof verification would let an attacker
            //    learn whether a nullifier is registered without submitting a valid proof.
            let nullifier = public_inputs[2];
            ensure!(
                !NullifierRegistry::<T>::contains_key(nullifier),
                Error::<T>::NullifierAlreadyUsed
            );

            let pos = TotalCitizens::<T>::get();
            let new_total = pos.checked_add(1).ok_or(Error::<T>::TotalCitizensOverflow)?;
            CitizenIndex::<T>::insert(pos, &who);
            CitizenPosition::<T>::insert(&who, pos);
            TotalCitizens::<T>::put(new_total);
            CitizenNullifier::<T>::insert(&who, nullifier);
            NullifierRegistry::<T>::insert(nullifier, &who);
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
        /// Origin: root (TODO: replace with court-controlled multisig origin).
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
        /// Origin: root (TODO: replace with court-controlled multisig origin).
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

        /// True if the account is a registered citizen with no active suspension.
        /// Timed suspensions are lazily removed from storage once their block has passed,
        /// so subsequent is_citizen / suspend calls do not see stale records.
        pub fn is_active_citizen(who: &T::AccountId) -> bool {
            let Some(nullifier) = CitizenNullifier::<T>::get(who) else { return false; };
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
    }
}
