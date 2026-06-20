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

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        CitizenRegistered { who: T::AccountId, nullifier: [u8; 32] },
        CitizenRevoked { who: T::AccountId },
        /// Voting rights suspended by court ruling. `until` = None means indefinite.
        CitizenSuspended { nullifier: [u8; 32], until: Option<BlockNumberFor<T>> },
        /// Voting rights restored (sentence served or conviction overturned).
        CitizenRestored { nullifier: [u8; 32] },
    }

    #[pallet::error]
    pub enum Error<T> {
        AlreadyRegistered,
        NullifierAlreadyUsed,
        InvalidZKProof,
        PassportExpired,
        NotRegistered,
        AlreadySuspended,
        NotSuspended,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Register a new citizen using a ZK passport proof and nullifier.
        /// zk_proof: serialized Groth16 proof bytes.
        /// public_inputs: [nullifier_hash, passport_expiry_timestamp, country_code_hash].
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn register_citizen(
            origin: OriginFor<T>,
            nullifier: [u8; 32],
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            // Up to 16 public signals; Rarimo Freedom Tool registration circuit uses 10.
            public_inputs: BoundedVec<[u8; 32], ConstU32<16>>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!CitizenNullifier::<T>::contains_key(&who), Error::<T>::AlreadyRegistered);
            ensure!(
                !NullifierRegistry::<T>::contains_key(nullifier),
                Error::<T>::NullifierAlreadyUsed
            );
            ensure!(
                T::ZkVerifier::verify(zk_proof.as_slice(), public_inputs.as_slice()),
                Error::<T>::InvalidZKProof
            );
            let pos = TotalCitizens::<T>::get();
            CitizenIndex::<T>::insert(pos, &who);
            CitizenPosition::<T>::insert(&who, pos);
            TotalCitizens::<T>::put(pos.saturating_add(1));
            CitizenNullifier::<T>::insert(&who, nullifier);
            NullifierRegistry::<T>::insert(nullifier, &who);
            Self::deposit_event(Event::CitizenRegistered { who, nullifier });
            Ok(())
        }

        /// Revoke a citizen registration (e.g. country removed from allowlist).
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn revoke_citizen(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let nullifier = CitizenNullifier::<T>::take(&who).ok_or(Error::<T>::NotRegistered)?;
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
        /// Origin: root (TODO: replace with court-controlled multisig origin).
        /// Only for offences listed in the constitution — legislature cannot expand this list
        /// without a constitutional amendment.
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn suspend_citizen(
            origin: OriginFor<T>,
            nullifier: [u8; 32],
            until: Option<BlockNumberFor<T>>,
        ) -> DispatchResult {
            T::SuspensionOrigin::ensure_origin(origin)?;
            ensure!(NullifierRegistry::<T>::contains_key(nullifier), Error::<T>::NotRegistered);
            ensure!(
                !SuspendedNullifiers::<T>::contains_key(nullifier),
                Error::<T>::AlreadySuspended
            );
            SuspendedNullifiers::<T>::insert(nullifier, until);
            Self::deposit_event(Event::CitizenSuspended { nullifier, until });
            Ok(())
        }

        /// Restore suspended voting rights.
        /// Called when a sentence is served, the waiting period passes, or a conviction is
        /// overturned on appeal. Restoration is automatic via the same court contract.
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
    }

    impl<T: Config> Pallet<T> {
        /// Used by pallet-courts (via a CitizenSelector trait impl in the runtime).
        pub fn citizen_at(index: u32) -> Option<T::AccountId> {
            CitizenIndex::<T>::get(index)
        }

        pub fn total_citizens() -> u32 {
            TotalCitizens::<T>::get()
        }

        /// True if the account is a registered citizen with no active suspension.
        /// Suspensions with a block expiry are lazily treated as lifted once that block passes.
        pub fn is_active_citizen(who: &T::AccountId) -> bool {
            let Some(nullifier) = CitizenNullifier::<T>::get(who) else { return false; };
            match SuspendedNullifiers::<T>::get(nullifier) {
                None => true,
                Some(None) => false,
                Some(Some(until)) => frame_system::Pallet::<T>::block_number() > until,
            }
        }

        /// True if the account is registered (regardless of suspension status).
        pub fn is_citizen(who: &T::AccountId) -> bool {
            CitizenNullifier::<T>::contains_key(who)
        }
    }
}
