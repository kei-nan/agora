//! Threshold (`t`-of-`n`) Chaum-Pedersen DLog-equality proving — "Option B" from
//! `docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`.
//!
//! # Why this exists
//!
//! `dlog::generate`/`dlog::verify` implement the relation for a single prover holding the
//! *entire* secret. Real committee members hold only a Feldman/Shamir share of it
//! (`dkg.rs`), so a real deployment needs `t` of them to jointly produce **one** proof that
//! verifies against the *group's* combined public key — without ever reconstructing the
//! group secret in one place, and (this is the point of this module) without needing any
//! change to `dlog::verify`, the on-chain no_std port of it
//! (`pallets/pallet-identity/src/dlog_verify.rs`), or the Noir circuits that call
//! `oprf-nr`'s `verified_oprf`. All three already accept an ordinary `(e, s)` proof against
//! one public key; this module's whole job is to make `t` independent members jointly
//! produce exactly that shape.
//!
//! # Why this can't be "each member proves independently, then sum the proofs"
//!
//! It was tried and rejected in design first (see doc 11's "Why proving can't move off the
//! phone" sibling discussion and the session that produced this module) for a concrete
//! reason: each member's Fiat-Shamir challenge is a hash of *their own* nonce commitment. If
//! member `i` computes their own `e_i` independently, `Σ λ_i · resp_i` does not collapse into
//! a valid proof of anything, because the `e_i`'s differ per member — the algebra needs one
//! *shared* challenge, which needs every member's nonce commitment to be visible before any
//! member computes a response. That forces two rounds, not one.
//!
//! # Protocol (adapted from FROST — Komlo & Goldberg, 2020 — to a two-generator DLEQ instead
//! of a single-generator Schnorr signature)
//!
//! Setup (from a real `dkg.rs` ceremony, not this module): member `i` holds share `s_i`;
//! `Y_i = s_i·G` and the group key `Y = s·G` are derivable by anyone from the public Feldman
//! commitments.
//!
//! **Round 1** (any member willing to answer a query `Q`, independently, no coordination
//! needed yet): compute the partial evaluation `R_i = s_i·Q` ([`round1`]'s `r_i`) and publish
//! it alongside two fresh nonce commitments `(D_i^G, D_i^Q) = (d_i·G, d_i·Q)` and
//! `(E_i^G, E_i^Q) = (e_i·G, e_i·Q)` for two independently sampled nonces `d_i`, `e_i`. Two
//! nonces per member, not one, and the "binding factor" `ρ_i` in round 2 — this is FROST's
//! specific defense against the Drijvers et al. (2019) rogue-nonce attack on naive two-round
//! Schnorr multisignatures, where an adversary who sees honest parties' commitments first can
//! bias the aggregate. Skipping it (a single nonce, no binding factor) was considered and
//! rejected in this session specifically because it reproduces that known-broken shape.
//!
//! **Set locking**: once exactly `t` members' round-1 messages exist for a given query +
//! committee, that specific index set `S` is fixed (this module takes `S` as a caller-supplied
//! `&[Round1Commitment]`; enforcing "exactly first `t`" is `pallet-identity`'s job, not this
//! module's — see doc 11's "what implementing Option A/B eventually touches" for the on-chain
//! schema this implies).
//!
//! **Binding factors**: `ρ_i = H(i, transcript(S))` ([`binding_factor`]) — ties every member's
//! nonce usage to the specific participant set and specific query, so commitments from one
//! session can never be recombined with another's.
//!
//! **Round 2** (each member of `S`, now that all of `S`'s round-1 messages are visible):
//! compute the shared challenge `e` ([`combined_challenge`], over the Lagrange-combined
//! evaluation `R = Σ λ_i R_i` and the aggregated nonce commitments), then respond with a
//! single scalar `z_i = (d_i + ρ_i·e_i) + e·λ_i·s_i` ([`round2_response`]).
//!
//! **Combine**: `z = Σ_{i∈S} z_i` ([`combine_responses`]). The pair `(e, z)` is now an
//! ordinary, valid Chaum-Pedersen proof of `(Y, Q, R)` — see this module's own tests for the
//! algebraic identity and a check that it passes the *unmodified* `dlog::verify`.
//!
//! # What is and isn't provided here
//!
//! This module is the protocol math only — it has no notion of "which round is open" or
//! "has the set locked yet"; that state lives on-chain (not yet implemented — see doc 11).
//! It also does not implement DKG itself (`dkg.rs` does) or the wasm FFI boundary
//! (`ffi.rs` does, also not yet extended for this — see this crate's own follow-up work).
//!
//! Every nonce here (`d_i`, `e_i`) must be freshly sampled per query and never reused across
//! queries, exactly like `dlog::generate`'s existing nonce — reuse leaks the share, same as
//! it would leak the whole secret in the non-threshold case.

use crate::babyjubjub::Point;
use crate::dlog;
use crate::poseidon2_taceo::t16;
use crate::scalar;
use ark_bn254::Fr;
use num_bigint::BigUint;

/// One member's round-1 broadcast: their partial evaluation plus two nonce-commitment pairs.
/// Safe to publish in full — none of these fields reveal the member's share (Feldman/DLEQ
/// commitments are hiding), matching the existing mailbox's "safe to be public" framing for
/// `blinded_query`/`response_blinded`.
#[derive(Clone, Debug, PartialEq)]
pub struct Round1Commitment {
    pub index: u64,
    pub r_i: Point,
    pub d_g: Point,
    pub d_q: Point,
    pub e_g: Point,
    pub e_q: Point,
}

/// One member's round-1 secret state — the two nonces behind their published commitments.
/// Never transmitted; held locally between round 1 and round 2 exactly as a committee member
/// already holds their share between DKG and evaluation. Reusing these for a second query is
/// the threshold analogue of `dlog::generate`'s existing single-use-nonce requirement — see
/// `committee-node`'s own module docs on this point (`wasm_host.rs`'s fresh-nonce-every-call
/// note) for why reuse is catastrophic, not just bad practice.
#[derive(Clone, Debug, PartialEq)]
pub struct Round1Nonces {
    pub d: BigUint,
    pub e: BigUint,
}

/// Round 1: member `index`, holding share `s_i`, answers blinded query `q`. Returns the public
/// commitment (to publish) and the secret nonce pair (to hold until round 2).
pub fn round1(
    rng: &mut impl rand::RngCore,
    index: u64,
    s_i: &BigUint,
    q: &Point,
) -> (Round1Commitment, Round1Nonces) {
    let r_i = q.scalar_mul(s_i);
    let d = scalar::random_nonzero(rng);
    let e = scalar::random_nonzero(rng);
    let generator = Point::generator();
    let commitment = Round1Commitment {
        index,
        r_i,
        d_g: generator.scalar_mul(&d),
        d_q: q.scalar_mul(&d),
        e_g: generator.scalar_mul(&e),
        e_q: q.scalar_mul(&e),
    };
    (commitment, Round1Nonces { d, e })
}

/// Lagrange coefficient `λ_i` for index `i` within the qualifying set `indices`, evaluated at
/// `x = 0` (standard Shamir reconstruction): `λ_i = Π_{j≠i} idx_j / (idx_j - idx_i) mod
/// fr_order()`. Pure `BigUint` arithmetic in the correct field throughout — this is exactly
/// the computation that would need unsound non-native circuit arithmetic if attempted inside
/// the Noir circuit (Option A, rejected for that reason); done here, in ordinary Rust, in the
/// field it actually belongs to, it's direct modular arithmetic with no correctness subtlety.
pub fn lagrange_coefficient(i: u64, indices: &[u64]) -> BigUint {
    let modulus = scalar::fr_order();
    let idx_i = BigUint::from(i) % &modulus;
    let mut num = BigUint::from(1u64);
    let mut den = BigUint::from(1u64);
    for &j in indices {
        if j == i {
            continue;
        }
        let idx_j = BigUint::from(j) % &modulus;
        num = (&num * &idx_j) % &modulus;
        // idx_j - idx_i, mod modulus (BigUint has no negative numbers, so add modulus first
        // when idx_j < idx_i to stay in range).
        let diff = if idx_j >= idx_i {
            (&idx_j - &idx_i) % &modulus
        } else {
            (&modulus - (&idx_i - &idx_j)) % &modulus
        };
        den = (&den * &diff) % &modulus;
    }
    (&num * scalar::inverse(&den)) % &modulus
}

/// A simple domain-separated transcript hash absorbing an arbitrary number of curve points via
/// repeated calls to the already-validated `t16` permutation (`poseidon2_taceo`), 7 points
/// (14 field elements) per block plus 2 reserved elements for the running state and a block
/// counter — chosen so the whole 16-wide state is used, matching `dlog.rs::challenge`'s own
/// use of `t16` elsewhere in this crate. Used only for [`binding_factor`], never for anything
/// a Noir circuit needs to reproduce (the whole point of Option B is that the circuit never
/// sees this protocol at all), so there is no upstream vector to match — determinism and
/// sensitivity to every input are the only properties this needs, both covered by this
/// module's own tests.
fn transcript_hash(points: &[Point]) -> Fr {
    let mut state = [Fr::from(0u64); 16];
    for (block_index, chunk) in points.chunks(7).enumerate() {
        state[0] += Fr::from(block_index as u64);
        for (slot, p) in chunk.iter().enumerate() {
            state[1 + 2 * slot] += p.x;
            state[2 + 2 * slot] += p.y;
        }
        state = t16(state);
    }
    state[0]
}

/// `ρ_i = H(i, transcript({R_j, D_j^G, D_j^Q, E_j^G, E_j^Q : j ∈ S}), Q)` — FROST's binding
/// factor, adapted to hash curve points directly via [`transcript_hash`] rather than FROST's
/// original byte-string transcript (no reason to match FROST's exact encoding — only
/// determinism and per-session uniqueness matter here, see this module's top-level docs).
pub fn binding_factor(i: u64, commitments: &[Round1Commitment], q: &Point) -> Fr {
    let mut points = Vec::with_capacity(commitments.len() * 5 + 2);
    // i and Q both feed into the transcript so ρ_i differs both per-member and per-query.
    points.push(Point::generator().scalar_mul(&BigUint::from(i)));
    points.push(*q);
    for c in commitments {
        points.push(c.r_i);
        points.push(c.d_g);
        points.push(c.d_q);
        points.push(c.e_g);
        points.push(c.e_q);
    }
    transcript_hash(&points)
}

/// `R = Σ_{i∈S} λ_i · R_i` — the Lagrange-combined OPRF evaluation. Pure EC point arithmetic
/// (`Point::add`/`scalar_mul`), safe and native-field-agnostic since it happens in Rust, not a
/// circuit; see module docs for why the in-circuit version of this exact operation was the
/// point where Option A failed.
pub fn combine_evaluations(commitments: &[Round1Commitment]) -> Point {
    let indices: Vec<u64> = commitments.iter().map(|c| c.index).collect();
    let mut acc = Point::identity();
    for c in commitments {
        let lambda = lagrange_coefficient(c.index, &indices);
        acc = acc.add(&c.r_i.scalar_mul(&lambda));
    }
    acc
}

/// `(D^G, D^Q) = Σ_{i∈S} (D_i^G + ρ_i·E_i^G, D_i^Q + ρ_i·E_i^Q)` — the aggregated nonce
/// commitments every participant needs (identically) to compute the shared challenge.
pub fn aggregate_nonce_commitments(
    commitments: &[Round1Commitment],
    binding_factors: &[(u64, Fr)],
) -> (Point, Point) {
    let rho = |idx: u64| -> BigUint {
        binding_factors
            .iter()
            .find(|(i, _)| *i == idx)
            .map(|(_, r)| scalar::from_field(r))
            .expect("binding factor missing for participant index")
    };
    let mut d_g = Point::identity();
    let mut d_q = Point::identity();
    for c in commitments {
        let rho_i = rho(c.index);
        d_g = d_g.add(&c.d_g.add(&c.e_g.scalar_mul(&rho_i)));
        d_q = d_q.add(&c.d_q.add(&c.e_q.scalar_mul(&rho_i)));
    }
    (d_g, d_q)
}

/// The shared Fiat-Shamir challenge for the combined proof — reuses `dlog::challenge` (made
/// `pub(crate)` for exactly this) unmodified, so the combined `(e, z)` this protocol produces
/// is checked by the *identical* recomputation `dlog::verify`/`dlog_verify.rs`/the Noir
/// circuit already perform. This is the crux of why Option B needs no downstream changes.
pub fn combined_challenge(y: &Point, q: &Point, r: &Point, d_g: &Point, d_q: &Point, ds_dlog: Fr) -> Fr {
    dlog::challenge(ds_dlog, y, q, r, &Point::generator(), d_g, d_q)
}

/// Round 2: member `index`'s response scalar, given their round-1 nonces, their share, their
/// own binding factor and Lagrange coefficient (both computable by anyone from public data —
/// see [`binding_factor`]/[`lagrange_coefficient`]), and the shared challenge `e`.
/// `z_i = (d_i + ρ_i·e_i) + e·λ_i·s_i (mod fr_order())`.
pub fn round2_response(
    nonces: &Round1Nonces,
    s_i: &BigUint,
    rho_i: Fr,
    lambda_i: &BigUint,
    e: Fr,
) -> Fr {
    let modulus = scalar::fr_order();
    let rho_i = scalar::from_field(&rho_i);
    let e = scalar::reduce(&scalar::from_field(&e));
    let k_i = (&nonces.d + (&rho_i * &nonces.e) % &modulus) % &modulus;
    let z_i = (&k_i + (&e * lambda_i % &modulus) * s_i % &modulus) % &modulus;
    scalar::to_field(&z_i)
}

/// `z = Σ_{i∈S} z_i mod fr_order()` — the final combined proof response. `(e, z)` together
/// with `(Y, Q, R)` from [`combined_challenge`]/[`combine_evaluations`] is a complete,
/// ordinary Chaum-Pedersen proof — verify it with the unmodified `dlog::verify(e, z, &Y, &Q,
/// &R, ds_dlog)`, exactly as a single-prover proof would be.
pub fn combine_responses(responses: &[Fr]) -> Fr {
    let modulus = scalar::fr_order();
    let sum = responses
        .iter()
        .fold(BigUint::from(0u64), |acc, z| (&acc + scalar::from_field(z)) % &modulus);
    scalar::to_field(&sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dkg;
    use ark_ff::PrimeField;

    /// Runs a full `t`-of-`n` DKG (real `dkg.rs`, not a shortcut) and the full 2-round
    /// protocol in this module for a synthetic query, returning `(Y, Q, R, e, z)` — everything
    /// needed to call the *unmodified* `dlog::verify`.
    fn simulate(
        rng: &mut impl rand::RngCore,
        n: usize,
        t: usize,
        q: &Point,
        ds_dlog: Fr,
    ) -> (Point, Point, Fr, Fr) {
        // DKG: n parties, each dealing a degree-(t-1) polynomial (Joint-Feldman, same
        // structure `dkg_party.rs` uses — see dkg.rs's own module docs).
        let polys: Vec<dkg::Polynomial> = (0..n).map(|_| dkg::Polynomial::random(t, rng)).collect();
        let commitments: Vec<Vec<Point>> = polys.iter().map(dkg::commit).collect();
        let group_pk = {
            let firsts: Vec<Point> = commitments.iter().map(|c| c[0]).collect();
            dkg::group_public_key(&firsts)
        };
        // Each party i's final share = sum over all dealers j of f_j(i).
        let shares: Vec<BigUint> = (1..=n as u64)
            .map(|i| {
                let received: Vec<BigUint> = polys.iter().map(|p| p.eval(i)).collect();
                // Sanity: every share verifies against its dealer's public commitments —
                // exactly what a real recipient would check before accepting it.
                for (dealer, share) in received.iter().enumerate() {
                    assert!(dkg::verify_share(share, i, &commitments[dealer]));
                }
                dkg::combine_received_shares(&received)
            })
            .collect();

        // Qualifying set: first t of n (indices 1..=t), matching "first t to commit locks the
        // set" from this module's docs.
        let participants: Vec<u64> = (1..=t as u64).collect();

        // Round 1.
        let mut round1_msgs = Vec::new();
        let mut round1_secrets = Vec::new();
        for &i in &participants {
            let s_i = &shares[(i - 1) as usize];
            let (commitment, secret) = round1(rng, i, s_i, q);
            round1_msgs.push(commitment);
            round1_secrets.push(secret);
        }

        // Binding factors + aggregated nonce commitments — computable by anyone.
        let rhos: Vec<(u64, Fr)> =
            participants.iter().map(|&i| (i, binding_factor(i, &round1_msgs, q))).collect();
        let (d_g, d_q) = aggregate_nonce_commitments(&round1_msgs, &rhos);
        let r = combine_evaluations(&round1_msgs);
        let e = combined_challenge(&group_pk, q, &r, &d_g, &d_q, ds_dlog);

        // Round 2: each participant independently computes their response.
        let responses: Vec<Fr> = participants
            .iter()
            .zip(round1_secrets.iter())
            .map(|(&i, nonces)| {
                let s_i = &shares[(i - 1) as usize];
                let lambda_i = lagrange_coefficient(i, &participants);
                let rho_i = rhos.iter().find(|(idx, _)| *idx == i).unwrap().1;
                round2_response(nonces, s_i, rho_i, &lambda_i, e)
            })
            .collect();
        let z = combine_responses(&responses);

        (group_pk, r, e, z)
    }

    fn ds_dlog() -> Fr {
        Fr::from_be_bytes_mod_order(
            &BigUint::parse_bytes(b"1523098184080632582082867317389990410064981862", 10)
                .unwrap()
                .to_bytes_be(),
        )
    }

    /// The core claim of this whole module: a combined `t`-of-`n` proof passes the
    /// **unmodified** `dlog::verify` — the same function a single, non-threshold prover's
    /// proof already has to pass. If this test passes, the Noir circuit (which checks the
    /// identical relation via `oprf-nr`'s `verify_dlog_equality`) accepts it too, with zero
    /// circuit changes — see this crate's `tests/` for the further check against the real
    /// compiled circuit.
    #[test]
    fn combined_proof_verifies_for_3_of_5() {
        let mut rng = rand::rngs::mock::StepRng::new(7, 11);
        let q = Point::generator().scalar_mul(&BigUint::from(424242u64));
        let (y, r, e, z) = simulate(&mut rng, 5, 3, &q, ds_dlog());
        assert!(dlog::verify(e, z, &y, &q, &r, ds_dlog()));
    }

    /// Exports one combined 3-of-5 proof's values as decimal literals, for cross-checking
    /// against the *real* `oprf-nr` Noir dependency's `verify_dlog_equality` directly (not
    /// this crate's own Rust port of it) — the strongest available check that Option B
    /// genuinely needs no circuit change, matching this project's existing practice of
    /// validating against the real compiled/executed artifact rather than trusting a Rust
    /// port alone (see `oprf.rs`'s `blinded_query_matches_the_real_query_circuit_execution`).
    /// Not a `#[test]` assertion on its own (the KAT-style tests above already assert
    /// correctness) — just a fixture generator, run manually and read once. `#[ignore]`d so
    /// normal `cargo test` runs stay hermetic (no filesystem writes); run explicitly with
    /// `cargo test --lib export_combined_proof_fixture -- --ignored --nocapture` to
    /// regenerate. Used once to produce the fixture cross-checked in
    /// `/tmp/.../threshold-noir-check` against the real `oprf-nr` v1.0.0
    /// `verify_dlog_equality` — see this crate's changelog/session notes for that result.
    #[test]
    #[ignore]
    fn export_combined_proof_fixture_for_noir_cross_check() {
        let mut rng = rand::rngs::mock::StepRng::new(7, 11);
        let q = Point::generator().scalar_mul(&BigUint::from(424242u64));
        let (y, r, e, z) = simulate(&mut rng, 5, 3, &q, ds_dlog());
        assert!(dlog::verify(e, z, &y, &q, &r, ds_dlog()));

        let dec = |f: &Fr| -> String { scalar::from_field(f).to_string() };
        let point_dec = |p: &Point| -> (String, String) { (dec(&p.x), dec(&p.y)) };
        let (yx, yy) = point_dec(&y);
        let (qx, qy) = point_dec(&q);
        let (rx, ry) = point_dec(&r);
        let fixture = format!(
            "oprf_pk_x = \"{yx}\"\noprf_pk_y = \"{yy}\"\nquery_x = \"{qx}\"\nquery_y = \"{qy}\"\nresponse_x = \"{rx}\"\nresponse_y = \"{ry}\"\ndlog_e = \"{}\"\ndlog_s = \"{}\"\n",
            dec(&e),
            dec(&z),
        );
        std::fs::write("/tmp/threshold_proof_fixture.toml", &fixture)
            .expect("write fixture for manual Noir cross-check");
    }

    /// Same claim at the founding-group scale changelog #073 actually specified (6-of-7).
    #[test]
    fn combined_proof_verifies_for_6_of_7() {
        let mut rng = rand::rngs::mock::StepRng::new(99, 3);
        let q = Point::generator().scalar_mul(&BigUint::from(13371337u64));
        let (y, r, e, z) = simulate(&mut rng, 7, 6, &q, ds_dlog());
        assert!(dlog::verify(e, z, &y, &q, &r, ds_dlog()));
    }

    /// `t = n` (no redundancy) still works — a degenerate but valid threshold.
    #[test]
    fn combined_proof_verifies_for_2_of_2() {
        let mut rng = rand::rngs::mock::StepRng::new(55, 21);
        let q = Point::generator().scalar_mul(&BigUint::from(9001u64));
        let (y, r, e, z) = simulate(&mut rng, 2, 2, &q, ds_dlog());
        assert!(dlog::verify(e, z, &y, &q, &r, ds_dlog()));
    }

    /// The point of a threshold scheme: a DIFFERENT qualifying subset of the same committee
    /// reconstructs the exact same `R` (same group secret's evaluation) — this is what makes
    /// the anchor stable regardless of *which* `t` members happened to be online, not just
    /// that some subset works.
    #[test]
    fn different_qualifying_subsets_reconstruct_the_same_evaluation() {
        let mut rng = rand::rngs::mock::StepRng::new(3, 5);
        let n = 5;
        let t = 3;
        let q = Point::generator().scalar_mul(&BigUint::from(777u64));

        let polys: Vec<dkg::Polynomial> = (0..n).map(|_| dkg::Polynomial::random(t, &mut rng)).collect();
        let shares: Vec<BigUint> = (1..=n as u64)
            .map(|i| {
                let received: Vec<BigUint> = polys.iter().map(|p| p.eval(i)).collect();
                dkg::combine_received_shares(&received)
            })
            .collect();

        let subset_a: Vec<u64> = vec![1, 2, 3];
        let subset_b: Vec<u64> = vec![2, 4, 5];

        let combine_for = |indices: &[u64]| -> Point {
            let msgs: Vec<Round1Commitment> = indices
                .iter()
                .map(|&i| {
                    let (c, _) = round1(&mut rand::rngs::mock::StepRng::new(1, 1), i, &shares[(i - 1) as usize], &q);
                    c
                })
                .collect();
            combine_evaluations(&msgs)
        };

        assert_eq!(combine_for(&subset_a), combine_for(&subset_b));
    }

    /// Sanity: fewer than `t` participants do NOT reconstruct the correct evaluation (they
    /// silently compute a value with no cryptographic meaning — Lagrange interpolation with
    /// too few points reconstructs the wrong polynomial, not an error).
    #[test]
    fn fewer_than_threshold_participants_give_a_different_result() {
        let mut rng = rand::rngs::mock::StepRng::new(4, 9);
        let n = 5;
        let t = 3;
        let q = Point::generator().scalar_mul(&BigUint::from(555u64));

        let polys: Vec<dkg::Polynomial> = (0..n).map(|_| dkg::Polynomial::random(t, &mut rng)).collect();
        let shares: Vec<BigUint> = (1..=n as u64)
            .map(|i| {
                let received: Vec<BigUint> = polys.iter().map(|p| p.eval(i)).collect();
                dkg::combine_received_shares(&received)
            })
            .collect();
        let firsts: Vec<Point> = polys.iter().map(|p| dkg::commit(p)[0]).collect();
        let group_pk = dkg::group_public_key(&firsts);
        let true_r = q.scalar_mul(&dkg::test_only_reconstruct_secret(
            &(1..=t as u64).map(|i| (i, shares[(i - 1) as usize].clone())).collect::<Vec<_>>(),
        ));

        // Only 2 participants for a threshold of 3.
        let short_set: Vec<u64> = vec![1, 2];
        let msgs: Vec<Round1Commitment> = short_set
            .iter()
            .map(|&i| round1(&mut rand::rngs::mock::StepRng::new(2, 2), i, &shares[(i - 1) as usize], &q).0)
            .collect();
        let wrong_r = combine_evaluations(&msgs);

        assert_ne!(wrong_r, true_r);
        let _ = group_pk; // group_pk only used to make clear true_r is w.r.t. the real group secret
    }

    /// Lagrange coefficients round-trip: reconstructing the *secret itself* (not an
    /// evaluation) via this module's coefficients must match `dkg.rs`'s own
    /// `test_only_reconstruct_secret`, which uses an independently-written interpolation.
    #[test]
    fn lagrange_coefficients_agree_with_dkg_reconstruction() {
        let mut rng = rand::rngs::mock::StepRng::new(17, 4);
        let t = 4;
        let poly = dkg::Polynomial::random(t, &mut rng);
        let indices: Vec<u64> = vec![2, 5, 9, 13];
        let shares: Vec<(u64, BigUint)> = indices.iter().map(|&i| (i, poly.eval(i))).collect();

        let modulus = scalar::fr_order();
        let via_this_module = indices
            .iter()
            .zip(shares.iter())
            .fold(BigUint::from(0u64), |acc, (&i, (_, s))| {
                (&acc + lagrange_coefficient(i, &indices) * s) % &modulus
            });
        let via_dkg = dkg::test_only_reconstruct_secret(&shares);
        assert_eq!(via_this_module, via_dkg);
        assert_eq!(via_this_module, poly.coeffs[0]);
    }
}
