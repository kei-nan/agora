//! Arithmetic mod `BABYJUBJUB_Fr` — BabyJubJub's own prime-order subgroup order, **not**
//! BN254's scalar field (`ark_bn254::Fr`, used everywhere else in this crate for curve
//! coordinates and Poseidon2 hash inputs/outputs).
//!
//! `oprf-nr`'s `babyjubjub/src/lib.nr` (`BABYJUBJUB_Fr` global) gives the decimal value
//! copied verbatim below. Blinding factors (`beta`) and OPRF secret keys are scalars in this
//! field; confusing it with the BN254 scalar field would silently produce wrong curve points
//! (every scalar_mul would reduce mod the wrong modulus).

use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use num_bigint::BigUint;
use num_traits::{One, Zero};
use std::str::FromStr;

/// `BABYJUBJUB_Fr` from `oprf-nr`'s `babyjubjub/src/lib.nr`, decimal, copied verbatim.
pub fn fr_order() -> BigUint {
    BigUint::from_str(
        "2736030358979909402780800718157159386076813972158567259200215660948447373041",
    )
    .expect("valid decimal literal")
}

pub fn reduce(x: &BigUint) -> BigUint {
    x % fr_order()
}

/// Modular inverse via Fermat's little theorem (`fr_order()` is prime): `x^(p-2) mod p`.
pub fn inverse(x: &BigUint) -> BigUint {
    let p = fr_order();
    assert!(!x.is_zero(), "cannot invert zero mod BABYJUBJUB_Fr");
    x.modpow(&(p.clone() - BigUint::from(2u32)), &p)
}

/// A uniformly random nonzero scalar in `[1, fr_order())`. **Dev-only**: uses the OS RNG
/// with no key-ceremony structure whatsoever — see the crate README.
pub fn random_nonzero(rng: &mut impl rand::RngCore) -> BigUint {
    let p = fr_order();
    // Rejection sampling over the byte-width of p; p is ~251 bits, so 32 bytes covers it
    // with a rejection probability well under 1/2^3, negligible for dev use.
    let byte_len = (p.bits() as usize + 7) / 8;
    loop {
        let mut bytes = vec![0u8; byte_len];
        rand::RngCore::fill_bytes(rng, &mut bytes);
        let candidate = BigUint::from_bytes_be(&bytes) % &p;
        if !candidate.is_zero() {
            return candidate;
        }
    }
}

/// Embeds a `BABYJUBJUB_Fr` scalar into `ark_bn254::Fr` (the field Poseidon2 hashing and
/// curve-coordinate arithmetic run over in this crate). Safe and lossless: `fr_order()`
/// (~2^251) is strictly smaller than BN254's scalar-field modulus (~2^254), so every
/// representative reduces to itself.
pub fn to_field(x: &BigUint) -> Fr {
    let bytes = x.to_bytes_be();
    Fr::from_be_bytes_mod_order(&bytes)
}

/// Recovers the canonical `BigUint` representative of an `ark_bn254::Fr` element. Used only
/// where a `Field` result (e.g. `dlog_s`, `dlog_e` challenge/response values that live in
/// `ark_bn254::Fr` per `dlog.nr`'s own hash-based challenge) needs to be treated as an
/// integer.
pub fn from_field(x: &Fr) -> BigUint {
    let bytes = x.into_bigint().to_bytes_be();
    BigUint::from_bytes_be(&bytes)
}

pub fn one() -> BigUint {
    BigUint::one()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_round_trips() {
        let x = BigUint::parse_bytes(
            b"1219447978145766874721695300216900264892278162307712369438981445355514609028",
            10,
        )
        .unwrap();
        let x_inv = inverse(&x);
        let prod = (&x * &x_inv) % fr_order();
        assert_eq!(prod, BigUint::one());
    }

    #[test]
    fn to_field_and_back_round_trips() {
        let x = BigUint::from(987654321u64);
        let f = to_field(&x);
        assert_eq!(from_field(&f), x);
    }
}
