//! Chaum-Pedersen DLog-equality proof: `oprf-nr`'s `dlog.nr`, both directions.
//!
//! `verify` is a straight Rust mirror of `verify_dlog_equality` — validated in `mod tests`
//! against `dlog.nr`'s own `test_verify_dlog_equality` known-answer vector. `generate` is the
//! **committee's** side (not present in `oprf-nr` at all, since that library only ever
//! verifies a proof — it never had to generate one; that's `TaceoLabs/oprf-service`'s job,
//! the real, closed-source-to-this-repo service this whole crate stands in for). `generate`
//! is a standard Schnorr/Chaum-Pedersen sigma-protocol prover for the same relation `verify`
//! checks, built from first principles (there is no "upstream generator" to port), and its
//! correctness is checked the only way that makes sense for a from-scratch prover: every
//! `generate`d proof must round-trip through `verify` (see `mod tests`), and — more
//! importantly — through `oprf-nr`'s own real, unmodified `verify_dlog_equality` inside
//! `circuits/oprf-identity-anchor`'s `anchor` circuit (see the top-level integration test /
//! binary in this crate, which is the actual point of this whole exercise).

use crate::babyjubjub::Point;
use crate::poseidon2_taceo::t16;
use crate::scalar;
use ark_bn254::Fr;
use num_bigint::BigUint;

/// Recomputes the Fiat-Shamir challenge exactly as `dlog.nr`'s `verify_dlog_equality` does:
/// `Poseidon2_t16([ds_dlog, a.x,a.y, b.x,b.y, c.x,c.y, G.x,G.y, r1.x,r1.y, r2.x,r2.y, 0,0,0])[1]`.
fn challenge(
    ds_dlog: Fr,
    a: &Point,
    b: &Point,
    c: &Point,
    generator: &Point,
    r1: &Point,
    r2: &Point,
) -> Fr {
    let input = [
        ds_dlog, a.x, a.y, b.x, b.y, c.x, c.y, generator.x, generator.y, r1.x, r1.y, r2.x, r2.y,
        Fr::from(0u64), Fr::from(0u64), Fr::from(0u64),
    ];
    t16(input)[1]
}

/// Straight mirror of `dlog.nr`'s `verify_dlog_equality`. `e`/`s` are taken as the raw
/// `Fr` values the circuit's public/private inputs would hold.
pub fn verify(e: Fr, s: Fr, a: &Point, b: &Point, c: &Point, ds_dlog: Fr) -> bool {
    if !(a.is_on_curve() && b.is_on_curve() && c.is_on_curve()) {
        return false;
    }
    if a.is_identity() || b.is_identity() || c.is_identity() {
        return false;
    }
    if !(b.check_sub_group() && c.check_sub_group()) {
        return false;
    }

    let generator = Point::generator();
    let s_scalar = scalar::from_field(&s);
    // `scalar_mul_base_field(e)`: e is used as a raw up-to-254-bit BN254 field value, not
    // pre-reduced mod BABYJUBJUB_Fr — but group-scalar action on a subgroup-order point is
    // inherently periodic mod that subgroup's order, so `e_bn254 mod Fr_order` acts
    // identically. Reducing here is exact, not an approximation.
    let e_scalar = scalar::reduce(&scalar::from_field(&e));

    let gs = generator.scalar_mul(&s_scalar);
    let ae = a.scalar_mul(&e_scalar);
    let r1 = gs.subtract(&ae);

    let bs = b.scalar_mul(&s_scalar);
    let ce = c.scalar_mul(&e_scalar);
    let r2 = bs.subtract(&ce);

    if r1.is_identity() || r2.is_identity() {
        return false;
    }

    let recomputed = challenge(ds_dlog, a, b, c, &generator, &r1, &r2);
    recomputed == e
}

/// Committee-side proof generation for the relation `verify` checks: given secret scalar
/// `x` (the committee's OPRF secret key) with `a = x*G` and `c = x*b`, produce `(e, s)` such
/// that `verify(e, s, a, b, c, ds_dlog)` holds. Standard Chaum-Pedersen sigma protocol:
/// sample nonce `r`, commit `r1 = r*G`, `r2 = r*b`, hash the same transcript the verifier
/// recomputes to get `e`, then respond `s = r + e*x (mod BABYJUBJUB_Fr)`.
pub fn generate(
    rng: &mut impl rand::RngCore,
    x: &BigUint,
    a: &Point,
    b: &Point,
    c: &Point,
    ds_dlog: Fr,
) -> (Fr, Fr) {
    let generator = Point::generator();
    let r = scalar::random_nonzero(rng);
    let r1 = generator.scalar_mul(&r);
    let r2 = b.scalar_mul(&r);

    let e = challenge(ds_dlog, a, b, c, &generator, &r1, &r2);
    let e_scalar = scalar::reduce(&scalar::from_field(&e));

    let fr_order = scalar::fr_order();
    let s = (&r + (&e_scalar * x) % &fr_order) % &fr_order;

    (e, scalar::to_field(&s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::PrimeField;
    use num_bigint::BigUint;

    fn fe(dec: &str) -> Fr {
        Fr::from_be_bytes_mod_order(&BigUint::parse_bytes(dec.as_bytes(), 10).unwrap().to_bytes_be())
    }

    fn fe_hex(h: &str) -> Fr {
        Fr::from_be_bytes_mod_order(&hex::decode(h).unwrap())
    }

    /// `dlog.nr`'s `test_verify_dlog_equality` — a real, fixed known-answer vector.
    #[test]
    fn verify_matches_upstream_kat() {
        let ds_dlog = fe("1523098184080632582082867317389990410064981862");
        // `oprf_query_key` is given in hex in the upstream test (unlike the other two
        // points, given in decimal) — copied verbatim, not converted by hand.
        let oprf_query_key = Point::new(
            fe_hex("2003f27260a0b5ee81b84f66f8bf2761ea9557262a4bcd16db5ca7abdeee1885"),
            fe_hex("1eb45d38c97f7e65ac1b76d234db3237d2860f2b25c43e020693ef92b5a5f793"),
        );
        let oprf_response_blinded = Point::new(
            fe("6882462243439192795495492197995100450516328082301652413647059141168822449465"),
            fe("11410248488379662098266045802345135482683496756414401793793460258484335221028"),
        );
        let oprf_pk = Point::new(
            fe("16048296497646113681290127133582586009660277510307938775951186660467382774945"),
            fe("13451097916688865791218925679662796109386737920791997438101375513111619197164"),
        );
        let dlog_e = fe("5609293693019386176508931649877337091590878173635241438306548223920379307458");
        let dlog_s = fe("1167493435914595771361530871033173621661932035514996719837354510862251986174");

        assert!(verify(
            dlog_e,
            dlog_s,
            &oprf_pk,
            &oprf_query_key,
            &oprf_response_blinded,
            ds_dlog
        ));
    }

    #[test]
    fn generated_proof_round_trips_through_verify() {
        let mut rng = rand::rngs::mock::StepRng::new(42, 7);
        let ds_dlog = fe("1523098184080632582082867317389990410064981862");
        let x = BigUint::from(999999937u64);
        let a = Point::generator().scalar_mul(&x);
        let b = Point::generator().scalar_mul(&BigUint::from(31337u64));
        let c = b.scalar_mul(&x);

        let (e, s) = generate(&mut rng, &x, &a, &b, &c, ds_dlog);
        assert!(verify(e, s, &a, &b, &c, ds_dlog));
    }

    #[test]
    fn generated_proof_fails_for_wrong_dlog() {
        let mut rng = rand::rngs::mock::StepRng::new(1, 3);
        let ds_dlog = fe("1523098184080632582082867317389990410064981862");
        let x = BigUint::from(123456789u64);
        let a = Point::generator().scalar_mul(&x);
        let b = Point::generator().scalar_mul(&BigUint::from(7777u64));
        // c uses a DIFFERENT exponent than a, so the dlog relation does not hold.
        let c = b.scalar_mul(&BigUint::from(555u64));

        let (e, s) = generate(&mut rng, &x, &a, &b, &c, ds_dlog);
        assert!(!verify(e, s, &a, &b, &c, ds_dlog));
    }
}
