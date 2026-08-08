//! **DEV/TEST-ONLY** simulator for the `TaceoLabs/oprf-nr` OPRF protocol.
//!
//! See the top-level `README.md` in this crate before using or trusting anything here. In
//! one sentence: this plays the role of 5 independent OPRF committees using in-memory,
//! freshly-generated key material, so `circuits/oprf-identity-anchor`'s `anchor`/
//! `disclosure`/`migrate` circuits can be proven and verified against protocol-correct
//! committee output instead of the `oprf_proof.beta` passthrough stub every prior session
//! used. It is not, and must never be mistaken for, a production OPRF committee: no real
//! distinct parties, no key ceremony, no network protocol, no threshold secret sharing.

pub mod scalar;
pub mod babyjubjub;
pub mod poseidon2_taceo;
pub mod hash_to_curve;
pub mod noir_poseidon2;
pub mod dlog;
pub mod oprf;
pub mod committee;
pub mod prover_toml;
pub mod ffi;
pub mod dkg;
