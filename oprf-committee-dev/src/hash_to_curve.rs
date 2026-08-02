//! `oprf-nr`'s `babyjubjub::hash_to_curve::encode` — the Elligator2-style map (RFC 9380
//! flavor) from a BN254 field element to a BabyJubJub curve point, used to turn the client's
//! `identity_input` into the point that gets blinded.
//!
//! Ported using `ark_ff`'s own `Field::sqrt`/`Field::legendre` (both already validated,
//! general-purpose implementations for arbitrary prime fields with known 2-adicity) instead
//! of re-deriving `oprf-nr`'s circuit-friendly constrained sqrt/Legendre witnessing scheme
//! (`hash_to_curve.nr`'s `sqrt`/`legendre`/`is_quadratic_residue_or_zero`, which exist only
//! to make the computation *provable*, not because the underlying math differs) — using a
//! well-tested library square-root routine is strictly lower-risk here than hand-rolling the
//! same Tonelli–Shanks-family algorithm a second time. Whichever square root `ark_ff` returns
//! is then sign-corrected by the same `sgn0` rule the circuit uses, so the result is
//! independent of which of the two roots the library happens to pick.
//!
//! Validated against `oprf-nr`'s own three `encode()` known-answer vectors in `mod tests`
//! below (`hash_to_curve.nr`'s `test_encode_to_curve_kat0/1/2`) — copied from the real
//! vendored source, not derived from this port.

use crate::babyjubjub::Point;
use crate::poseidon2_taceo::t3;
use ark_bn254::Fr;
use ark_ff::{BigInteger, Field, LegendreSymbol, PrimeField};

/// ASCII `"OPRF_HashToField_BabyJubJub"` read as a big-endian integer — `hash_to_curve.nr`'s
/// domain separator for `hash_to_field`.
fn ds_hash_to_field() -> Fr {
    Fr::from_be_bytes_mod_order(
        &num_bigint::BigUint::parse_bytes(b"32627786498498119128812045057993354633158048678109587794777765218", 10)
            .unwrap()
            .to_bytes_be(),
    )
}

fn hash_to_field(input: Fr) -> Fr {
    let state = t3([ds_hash_to_field(), input, Fr::from(0u64)]);
    state[1]
}

fn is_square_or_zero(x: Fr) -> bool {
    x.legendre() != LegendreSymbol::QuadraticNonResidue
}

fn sgn0(x: Fr) -> bool {
    // `Field::sgn0` parity of the canonical representative — same definition as Noir's
    // `Field::sgn0` (`(self as u8) % 2 == 1`, i.e. the low bit of the canonical integer).
    x.into_bigint().to_bytes_le()[0] & 1 == 1
}

fn inverse_or_zero(x: Fr) -> Fr {
    x.inverse().unwrap_or(Fr::from(0u64))
}

/// Elligator2 map to the BabyJubJub Montgomery curve (`t^2 = s^3 + 168698*s^2 + s`), then
/// the standard rational map to twisted-Edwards coordinates.
fn map_to_curve_twisted_edwards(u: Fr) -> (Fr, Fr) {
    let z = Fr::from(5u64);
    let c1 = Fr::from(168698u64);
    let c2 = Fr::from(1u64);

    let tv1_0 = z * u * u;
    let e = tv1_0 + Fr::from(1u64) == Fr::from(0u64);
    let tv1 = if e { Fr::from(0u64) } else { tv1_0 };

    let tv1_plus_1 = tv1 + Fr::from(1u64);
    let x1_inv = inverse_or_zero(tv1_plus_1);
    let x1 = -c1 * x1_inv;

    let gx1_0 = (x1 + c1) * x1;
    let gx1 = (gx1_0 + c2) * x1;

    let x2 = -x1 - c1;
    let gx2 = tv1 * gx1;

    let gx1_is_square = is_square_or_zero(gx1);

    let x = if gx1_is_square { x1 } else { x2 };
    let gx = if gx1_is_square { gx1 } else { gx2 };

    let mut y = gx.sqrt().expect("gx is guaranteed square-or-zero by the elligator2 property");

    let y_sgn = sgn0(y);
    let should_negate = if gx1_is_square { !y_sgn } else { y_sgn };
    if should_negate {
        y = -y;
    }

    // Montgomery (s, t) = (x*k, y*k), k = 1.
    let s = x;
    let t = y;

    // Rational map Montgomery -> twisted Edwards.
    let tv1 = s + Fr::from(1u64);
    let tv2 = inverse_or_zero(tv1 * t);
    let v = tv1 * tv2;
    let w = tv2 * t;
    let tv11 = s - Fr::from(1u64);
    let e2 = tv2 == Fr::from(0u64);
    let out_x = s * v;
    let out_y = if e2 { Fr::from(1u64) } else { w * tv11 };

    (out_x, out_y)
}

/// `hash_to_curve::encode` — hash-to-field, map to twisted Edwards, clear the cofactor (x8).
pub fn encode(input: Fr) -> Point {
    let u = hash_to_field(input);
    let (x, y) = map_to_curve_twisted_edwards(u);
    let point = Point::new(x, y);
    // multiply_by_cofactor = self.double().double().double() (BabyJubJub cofactor = 8).
    point.double().double().double()
}

#[cfg(test)]
mod tests {
    use super::*;
    use num_bigint::BigUint;

    fn fe_hex(h: &str) -> Fr {
        Fr::from_be_bytes_mod_order(&hex::decode(h).unwrap())
    }

    /// `hash_to_curve.nr`'s `test_encode_to_curve_kat0`.
    #[test]
    fn matches_upstream_kat0() {
        let input = fe_hex("03e4070110668921a99c37627dedddb5ab65fae33c19e24d9ee19d7065fdeca8");
        let p = encode(input);
        let expected_x = BigUint::parse_bytes(
            b"10317659717708787122683977912952208883341451354299498236440964928299898571531",
            10,
        )
        .unwrap();
        let expected_y = BigUint::parse_bytes(
            b"2771878628977713302835201233169750856073682825638128695522023521672351725258",
            10,
        )
        .unwrap();
        assert_eq!(crate::scalar::from_field(&p.x), expected_x);
        assert_eq!(crate::scalar::from_field(&p.y), expected_y);
    }

    /// `hash_to_curve.nr`'s `test_encode_to_curve_kat1`.
    #[test]
    fn matches_upstream_kat1() {
        let input = Fr::from(0x42u64);
        let p = encode(input);
        let expected_x = BigUint::parse_bytes(
            b"16453178030699411958341692808730701741568100876455568813278163225032347056514",
            10,
        )
        .unwrap();
        let expected_y = BigUint::parse_bytes(
            b"5447922750205248208490261749483809853022174346498064122782172531486866662376",
            10,
        )
        .unwrap();
        assert_eq!(crate::scalar::from_field(&p.x), expected_x);
        assert_eq!(crate::scalar::from_field(&p.y), expected_y);
    }

    /// `hash_to_curve.nr`'s `test_encode_to_curve_kat2` — the zero input.
    #[test]
    fn matches_upstream_kat2() {
        let p = encode(Fr::from(0u64));
        let expected_x = BigUint::parse_bytes(
            b"16605852874433019712683889710166313607515083375138125349412270828059484170936",
            10,
        )
        .unwrap();
        let expected_y = BigUint::parse_bytes(
            b"12075050546928691602283582412953179086742727007172364313655633055645374686589",
            10,
        )
        .unwrap();
        assert_eq!(crate::scalar::from_field(&p.x), expected_x);
        assert_eq!(crate::scalar::from_field(&p.y), expected_y);
    }
}
