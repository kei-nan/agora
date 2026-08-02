//! **DEV-ONLY** stand-in for 5 independent OPRF committees (changelog entry 73's governance
//! design: 5 independent ~35-member committees combined by hashed summation).
//!
//! This is emphatically NOT what changelog entry 73 or the `oprf-identity-anchor` README
//! mean by "a committee": there is exactly one process, one machine, one RNG here, holding
//! all 5 secret keys in memory simultaneously. A real committee is dozens of independent
//! people running a real key-generation ceremony none of whom individually knows the full
//! secret, talking to each other and to clients over `TaceoLabs/oprf-service`'s actual
//! network protocol. None of that exists here or is attempted here — see this crate's
//! top-level README for the full list of what is and isn't simulated.
//!
//! What *is* real: each `DevCommittee`'s secret key correctly participates in the same
//! Chaum-Pedersen/OPRF math a genuine committee member would (`oprf.rs::evaluate`,
//! cross-checked against `TaceoLabs/oprf-nr`'s own known-answer vectors), so a proof built
//! from 5 `DevCommittee`s' responses is protocol-correct input to `verified_oprf` — the
//! actual gap this crate closes.

use crate::babyjubjub::Point;
use crate::oprf::{self, CommitteeEvaluation};
use crate::scalar;
use ark_bn254::Fr;
use num_bigint::BigUint;

pub const NUM_COMMITTEES: usize = 5;

/// One committee's in-memory key material. **Not a key share from a real DKG ceremony** —
/// a single freshly-generated scalar, held entirely by this one process.
pub struct DevCommittee {
    pub index: usize,
    pub secret_key: BigUint,
    pub public_key: Point,
}

impl DevCommittee {
    fn generate(index: usize, rng: &mut impl rand::RngCore) -> Self {
        let secret_key = scalar::random_nonzero(rng);
        let public_key = Point::generator().scalar_mul(&secret_key);
        DevCommittee { index, secret_key, public_key }
    }
}

/// The full dev committee set — 5 independent `DevCommittee`s. Every registration/
/// reverification/migration proof this crate produces uses one of these.
pub struct DevCommitteeSet {
    pub committees: [DevCommittee; NUM_COMMITTEES],
}

impl DevCommitteeSet {
    /// Generates 5 fresh, independent secret keys. **Dev-only**: real deployment needs a
    /// governance-run key ceremony per changelog entry 73 — explicitly out of scope here.
    pub fn generate(rng: &mut impl rand::RngCore) -> Self {
        let committees = core::array::from_fn(|i| DevCommittee::generate(i, rng));
        DevCommitteeSet { committees }
    }

    /// Evaluates the same blinded query `b_q` against all 5 committees — mirrors the
    /// README's "symmetric fan-out, not hash-routed" design (changelog entry 73): the exact
    /// same point is sent to every committee, not derived per-committee.
    pub fn evaluate_all(
        &self,
        rng: &mut impl rand::RngCore,
        b_q: &Point,
        ds_dlog: Fr,
    ) -> [CommitteeEvaluation; NUM_COMMITTEES] {
        core::array::from_fn(|i| oprf::evaluate(rng, &self.committees[i].secret_key, b_q, ds_dlog))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dlog;

    #[test]
    fn five_committees_evaluate_the_same_query_independently() {
        let mut rng = rand::rngs::mock::StepRng::new(9, 5);
        let set = DevCommitteeSet::generate(&mut rng);
        assert_eq!(set.committees.len(), NUM_COMMITTEES);

        let ds_dlog = crate::scalar::to_field(
            &BigUint::parse_bytes(b"1523098184080632582082867317389990410064981862", 10).unwrap(),
        );
        let client_input = crate::scalar::to_field(&BigUint::from(42u64));
        let beta = BigUint::from(31415926535u64);
        let b_q = oprf::blinded_query(&beta, client_input);

        let evaluations = set.evaluate_all(&mut rng, &b_q, ds_dlog);

        // Every committee's public key must differ (independent keys) and every proof must
        // verify against that committee's own key and the shared query.
        for i in 0..NUM_COMMITTEES {
            for j in (i + 1)..NUM_COMMITTEES {
                assert_ne!(
                    set.committees[i].public_key, set.committees[j].public_key,
                    "committees {i} and {j} must not share a key"
                );
            }
            let eval = &evaluations[i];
            assert!(dlog::verify(
                eval.dlog_e,
                eval.dlog_s,
                &eval.pk,
                &b_q,
                &eval.response_blinded,
                ds_dlog
            ));
        }
    }
}
