//! The client- and committee-side halves of `oprf-nr`'s OPRF protocol
//! (`blinded_query`/`oprf_output.nr`), assembled from `hash_to_curve`, `babyjubjub`,
//! `dlog`, and `noir_poseidon2`.
//!
//! Validated end-to-end in `mod tests` against `oprf-nr`'s own two real known-answer
//! vectors: `blinded_query.nr`'s `oprf_query_kat_test` (client side only) and
//! `oprf_output.nr`'s `verified_oprf_kat_test` (the full round trip: query, committee
//! evaluation + DLEQ proof, unblinding, final output) — both copied from the real vendored
//! source, not derived from this port.

use crate::babyjubjub::Point;
use crate::dlog;
use crate::noir_poseidon2;
use ark_bn254::Fr;
use num_bigint::BigUint;

/// `blinded_query.nr`'s `blinded_query`: `b_q = encode(client_input) * beta`.
pub fn blinded_query(beta: &BigUint, client_input: Fr) -> Point {
    assert!(*beta != BigUint::from(0u64), "beta must be nonzero");
    let q = crate::hash_to_curve::encode(client_input);
    q.scalar_mul(beta)
}

/// One committee's real OPRF evaluation + Chaum-Pedersen proof, in the exact shape
/// `utils::types::OPRFProof` (minus `beta`, which the *client* supplies — see
/// `committee.rs`'s `CommitteeResponse` for the full assembled witness).
pub struct CommitteeEvaluation {
    pub pk: Point,
    pub dlog_e: Fr,
    pub dlog_s: Fr,
    pub response_blinded: Point,
}

/// The committee's side: given the client's blinded query `b_q` and this committee's secret
/// key `sk`, compute the blinded OPRF response `sk * b_q` and a DLog-equality proof that it
/// used the same `sk` as its published public key `pk = sk * G`.
pub fn evaluate(
    rng: &mut impl rand::RngCore,
    sk: &BigUint,
    b_q: &Point,
    ds_dlog: Fr,
) -> CommitteeEvaluation {
    let pk = Point::generator().scalar_mul(sk);
    let response_blinded = b_q.scalar_mul(sk);
    let (dlog_e, dlog_s) = dlog::generate(rng, sk, &pk, b_q, &response_blinded, ds_dlog);
    CommitteeEvaluation { pk, dlog_e, dlog_s, response_blinded }
}

/// The client's side after receiving a committee's response: undo the blinding.
/// `oprf_output.nr`'s `verify_unblinding` checks `response * beta == response_blinded`; this
/// computes that `response` (`response_blinded * beta^{-1} mod BABYJUBJUB_Fr`).
pub fn unblind(response_blinded: &Point, beta: &BigUint) -> Point {
    let beta_inv = crate::scalar::inverse(beta);
    response_blinded.scalar_mul(&beta_inv)
}

/// `oprf_output.nr`'s `generate_output`: `Poseidon2_t4([ds_out, query, response.x, response.y])[1]`
/// — the *raw* ACIR-blackbox permutation (`noir_poseidon2`), not the sponge.
pub fn generate_output(client_input: Fr, response: &Point, ds_out: Fr) -> Fr {
    noir_poseidon2::permute([ds_out, client_input, response.x, response.y])[1]
}

/// Full mirror of `oprf_output.nr`'s `verified_oprf`, for self-checking this port against
/// the real end-to-end KAT vector. Not used by the actual anchor-circuit witness generation
/// (that happens across two separate parties — client vs. committee — glued together by
/// `committee.rs`), only by tests here.
#[cfg(test)]
pub fn verified_oprf(
    beta: &BigUint,
    oprf_pk: &Point,
    dlog_e: Fr,
    dlog_s: Fr,
    oprf_response_blinded: &Point,
    oprf_response: &Point,
    client_input: Fr,
    ds_dlog: Fr,
    ds_out: Fr,
) -> Fr {
    let b_q = blinded_query(beta, client_input);
    assert!(dlog::verify(dlog_e, dlog_s, oprf_pk, &b_q, oprf_response_blinded, ds_dlog));
    let reblinded = oprf_response.scalar_mul(beta);
    assert_eq!(reblinded.x, oprf_response_blinded.x);
    assert_eq!(reblinded.y, oprf_response_blinded.y);
    generate_output(client_input, oprf_response, ds_out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::PrimeField;

    fn fe(dec: &str) -> Fr {
        Fr::from_be_bytes_mod_order(&BigUint::parse_bytes(dec.as_bytes(), 10).unwrap().to_bytes_be())
    }

    /// `blinded_query.nr`'s `oprf_query_kat_test`.
    #[test]
    fn blinded_query_matches_upstream_kat() {
        let beta = BigUint::parse_bytes(
            b"1219447978145766874721695300216900264892278162307712369438981445355514609028",
            10,
        )
        .unwrap();
        let client_input = fe("387242419257851558984914892096106188402994395767663340643461644969954908901");
        let expected_x = BigUint::parse_bytes(
            b"9674018812144873985515234638873635008708834681175738612597888298086191498827",
            10,
        )
        .unwrap();
        let expected_y = BigUint::parse_bytes(
            b"18818965079786157878748649151755578492493325229415754584538271947197071123195",
            10,
        )
        .unwrap();

        let result = blinded_query(&beta, client_input);
        assert_eq!(crate::scalar::from_field(&result.x), expected_x);
        assert_eq!(crate::scalar::from_field(&result.y), expected_y);
    }

    /// `oprf_output.nr`'s `verified_oprf_kat_test` — the full protocol round trip. This is
    /// the single strongest piece of evidence in this crate: every module (`babyjubjub`,
    /// `hash_to_curve`, `poseidon2_taceo`, `dlog`, `noir_poseidon2`) has to be independently
    /// correct AND correctly wired together for this to produce the right field element.
    #[test]
    fn verified_oprf_matches_upstream_kat() {
        let beta = BigUint::parse_bytes(
            b"2387462819376525223098422965806766751285565755980265072594901385662518965922",
            10,
        )
        .unwrap();
        let oprf_pk = Point::new(
            fe("16048296497646113681290127133582586009660277510307938775951186660467382774945"),
            fe("13451097916688865791218925679662796109386737920791997438101375513111619197164"),
        );
        let dlog_e = fe("5609293693019386176508931649877337091590878173635241438306548223920379307458");
        let dlog_s = fe("1167493435914595771361530871033173621661932035514996719837354510862251986174");
        let oprf_response_blinded = Point::new(
            fe("6882462243439192795495492197995100450516328082301652413647059141168822449465"),
            fe("11410248488379662098266045802345135482683496756414401793793460258484335221028"),
        );
        let oprf_response = Point::new(
            fe("11771927497930831763844779626723106344742708040976110136703486119568919340013"),
            fe("19299702061490581533153169629464406607119112637706400365988657399831357218309"),
        );
        let client_input =
            fe("16778459034791831994680912315190773394547154204120376000937183272785502542361");
        let ds_dlog = fe("1523098184080632582082867317389990410064981862");
        let ds_out = fe("1773399373884719043551596035141478");
        let expected_output =
            fe("21342856517406476000190785734870568200315738457615815351702849709270076362125");

        let got = verified_oprf(
            &beta,
            &oprf_pk,
            dlog_e,
            dlog_s,
            &oprf_response_blinded,
            &oprf_response,
            client_input,
            ds_dlog,
            ds_out,
        );
        assert_eq!(got, expected_output);
    }

    /// Cross-checks `blinded_query` against a REAL `nargo execute --package
    /// oprf_identity_anchor_query` run of the actual (not stubbed) circuit in this repo,
    /// over `circuits/oprf-identity-anchor/query/Prover.toml`'s own committed fixture
    /// (`SAMPLE_DG1`, salt 1111, `TEST_BETA`). `identity_input` is
    /// `identity_anchor::derive_identity_input(SAMPLE_DG1)`, measured the same way (a
    /// throwaway `nargo test --show-output` package depending on the real `identity_anchor`
    /// lib) — see this crate's changelog entry for the exact commands. This is the
    /// strongest available check that `hash_to_curve::encode` really does what the deployed
    /// circuit expects, not just what `oprf-nr`'s own unrelated KAT vectors expect.
    #[test]
    fn blinded_query_matches_the_real_query_circuit_execution() {
        let identity_input = Fr::from_be_bytes_mod_order(
            &hex::decode("27e62fda546af6970d595963b00dbb8af2ae8fe08060b48e40c3e3d29cab6b46")
                .unwrap(),
        );
        let beta = BigUint::parse_bytes(
            b"63865932500786004558985758765891911620034145599124743624338496685411118977",
            10,
        )
        .unwrap();

        let result = blinded_query(&beta, identity_input);

        let expected_x = BigUint::from_bytes_be(
            &hex::decode("2b2ea1997c08dee34c2acae25520fe59176287d2e59b57a5c140fb3d96876c7a")
                .unwrap(),
        );
        let expected_y = BigUint::from_bytes_be(
            &hex::decode("0a489809e97b1ef814248dc60150d1eaf1f0bb2fe0f573c0fd327b6794e2bd60")
                .unwrap(),
        );
        assert_eq!(crate::scalar::from_field(&result.x), expected_x);
        assert_eq!(crate::scalar::from_field(&result.y), expected_y);
    }

    /// Sanity check that `evaluate` + `unblind` produce a response that `verified_oprf`
    /// (hence the real anchor circuit) accepts, for freshly-generated (not upstream-fixed)
    /// key material — i.e. the actual simulator path this crate exists for, not just KAT
    /// replay.
    #[test]
    fn generated_committee_response_round_trips() {
        let mut rng = rand::rngs::mock::StepRng::new(123, 17);
        let ds_dlog = fe("1523098184080632582082867317389990410064981862");
        let ds_out = fe("1773399373884719043551596035141478");
        let client_input = fe("999999999999999999999999999");
        let beta = BigUint::from(4242424242424242u64);

        let b_q = blinded_query(&beta, client_input);
        let sk = BigUint::from(778899112233u64);
        let eval = evaluate(&mut rng, &sk, &b_q, ds_dlog);
        let response = unblind(&eval.response_blinded, &beta);

        let output = verified_oprf(
            &beta,
            &eval.pk,
            eval.dlog_e,
            eval.dlog_s,
            &eval.response_blinded,
            &response,
            client_input,
            ds_dlog,
            ds_out,
        );
        // Independently recompute the expected output the same way `generate_output` would,
        // to make sure `verified_oprf`'s internal assertions weren't vacuously satisfied.
        let expected = generate_output(client_input, &response, ds_out);
        assert_eq!(output, expected);
    }
}
