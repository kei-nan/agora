//! BabyJubJub (EIP-2494 / `oprf-nr`'s `babyjubjub/src/lib.nr`) twisted Edwards curve group,
//! over `ark_bn254::Fr` coordinates (BN254's scalar field — the same field Poseidon2 hashing
//! runs over in this crate, and the field circuit wires live in).
//!
//! Curve equation: `a*x^2 + y^2 = 1 + d*x^2*y^2`, `a = 168700`, `d = 168696` — copied
//! verbatim from `oprf-nr`'s `BABYJUBJUB_A`/`BABYJUBJUB_D`.
//!
//! This is a completely independent implementation from Noir's (ordinary field arithmetic
//! with no constraint system), so it does not need to replicate `oprf-nr`'s
//! unconstrained/assert split — only produce the same *point*, which any correct
//! implementation of the same group law does by definition. Validated below against
//! `oprf-nr`'s own real test vectors (`babyjubjub/src/lib.nr`'s subgroup tests, plus the
//! full `blinded_query`/`verified_oprf` KAT vectors exercised in `oprf.rs`).

use ark_bn254::Fr;
use ark_ff::{Field, PrimeField};
use num_bigint::BigUint;

fn a() -> Fr {
    Fr::from(168700u64)
}
fn d() -> Fr {
    Fr::from(168696u64)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub x: Fr,
    pub y: Fr,
}

impl Point {
    pub fn new(x: Fr, y: Fr) -> Self {
        Point { x, y }
    }

    pub fn identity() -> Self {
        Point { x: Fr::from(0u64), y: Fr::from(1u64) }
    }

    pub fn generator() -> Self {
        // `BABYJUBJUB_GENERATOR_X`/`_Y` from `oprf-nr`'s `babyjubjub/src/lib.nr`, decimal.
        Point {
            x: Fr::from_be_bytes_mod_order(
                &BigUint::parse_bytes(
                    b"5299619240641551281634865583518297030282874472190772894086521144482721001553",
                    10,
                )
                .unwrap()
                .to_bytes_be(),
            ),
            y: Fr::from_be_bytes_mod_order(
                &BigUint::parse_bytes(
                    b"16950150798460657717958625567821834550301663161624707787222815936182638968203",
                    10,
                )
                .unwrap()
                .to_bytes_be(),
            ),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.x == Fr::from(0u64) && self.y == Fr::from(1u64)
    }

    pub fn is_not_identity(&self) -> bool {
        !self.is_identity()
    }

    pub fn is_on_curve(&self) -> bool {
        let lhs = a() * self.x * self.x + self.y * self.y;
        let rhs = Fr::from(1u64) + d() * self.x * self.x * self.y * self.y;
        lhs == rhs
    }

    pub fn negate(&self) -> Self {
        Point { x: -self.x, y: self.y }
    }

    /// Twisted Edwards point addition (complete formula for this curve's parameters).
    pub fn add(&self, other: &Point) -> Point {
        let beta = self.x * other.y;
        let gamma = self.y * other.x;
        let delta = (-a() * self.x + self.y) * (other.x + other.y);
        let tau = beta * gamma;

        let den_x = Fr::from(1u64) + d() * tau;
        let den_y = Fr::from(1u64) - d() * tau;

        let x = (beta + gamma) * den_x.inverse().expect("den_x is nonzero for valid curve points");
        let y = (delta + a() * beta - gamma)
            * den_y.inverse().expect("den_y is nonzero for valid curve points");

        Point { x, y }
    }

    pub fn double(&self) -> Point {
        self.add(self)
    }

    pub fn subtract(&self, other: &Point) -> Point {
        self.add(&other.negate())
    }

    /// Variable-base scalar multiplication, MSB-first double-and-add. `scalar` need not be
    /// reduced mod anything in particular — this is plain integer scalar multiplication in
    /// the group, correct for any nonnegative integer scalar.
    pub fn scalar_mul(&self, scalar: &BigUint) -> Point {
        if scalar == &BigUint::from(0u64) {
            return Point::identity();
        }
        let bytes = scalar.to_bytes_be();
        let mut result = Point::identity();
        for byte in bytes.iter() {
            for i in (0..8).rev() {
                result = result.double();
                if (byte >> i) & 1 == 1 {
                    result = result.add(self);
                }
            }
        }
        result
    }

    /// Whether `self` lies in the prime-order (`BABYJUBJUB_Fr`) subgroup. `oprf-nr`'s
    /// `check_sub_group` multiplies by a 251-bit "characteristic" constant that is, verified
    /// by direct computation from its bit array, exactly `BABYJUBJUB_Fr` itself (not the
    /// full 8x-cofactor curve order) — so this is equivalent to `scalar_mul` by
    /// `crate::scalar::fr_order()` and checking the result is the identity.
    pub fn check_sub_group(&self) -> bool {
        self.scalar_mul(&crate::scalar::fr_order()).is_identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `oprf-nr`'s `test_babyjubjub_in_correct_subgroup`.
    #[test]
    fn accepts_a_real_subgroup_point() {
        let p = Point {
            x: Fr::from_be_bytes_mod_order(
                &BigUint::parse_bytes(
                    b"4597297048474520994314398800947075450541957920804155712178316083765998639288",
                    10,
                )
                .unwrap()
                .to_bytes_be(),
            ),
            y: Fr::from_be_bytes_mod_order(
                &BigUint::parse_bytes(
                    b"5569132826648062501012191259106565336315721760204071234863390487921354852142",
                    10,
                )
                .unwrap()
                .to_bytes_be(),
            ),
        };
        assert!(p.is_on_curve());
        assert!(p.check_sub_group());
    }

    /// `oprf-nr`'s `test_two_torsion_not_correct_subgroup`: (0, -1) is on-curve (2-torsion)
    /// but not in the prime-order subgroup.
    #[test]
    fn rejects_the_two_torsion_point() {
        let p = Point { x: Fr::from(0u64), y: -Fr::from(1u64) };
        assert!(p.is_on_curve());
        assert!(!p.check_sub_group());
    }

    /// `oprf-nr`'s `test_a_plus_two_torsion_not_correct_subgroup`.
    #[test]
    fn rejects_a_plus_two_torsion_point() {
        let p = Point {
            x: Fr::from_be_bytes_mod_order(
                &BigUint::parse_bytes(
                    b"4689731437944306428780974070575571025333825310181802403039288057202868837337",
                    10,
                )
                .unwrap()
                .to_bytes_be(),
            ),
            y: Fr::from_be_bytes_mod_order(
                &BigUint::parse_bytes(
                    b"14792220576807380471707687543579831579477508903129740636298855878394914021491",
                    10,
                )
                .unwrap()
                .to_bytes_be(),
            ),
        };
        assert!(p.is_on_curve());
        assert!(!p.check_sub_group());
    }

    #[test]
    fn generator_is_on_curve_and_in_subgroup() {
        let g = Point::generator();
        assert!(g.is_on_curve());
        assert!(g.check_sub_group());
    }

    #[test]
    fn identity_is_additive_identity() {
        let g = Point::generator();
        let sum = g.add(&Point::identity());
        assert_eq!(sum, g);
    }

    #[test]
    fn scalar_mul_matches_repeated_addition() {
        let g = Point::generator();
        let by5 = g.scalar_mul(&BigUint::from(5u64));
        let manual = g.add(&g).add(&g).add(&g).add(&g);
        assert_eq!(by5, manual);
    }
}
