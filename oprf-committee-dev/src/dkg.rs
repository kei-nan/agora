//! Joint-Feldman verifiable secret sharing (Pedersen 1991; the "optimistic path" of
//! Gennaro-Jarecki-Krawczyk-Rabin's DKG — see "What this protocol does NOT handle" below for
//! exactly what's cut relative to full GJKR) over BabyJubJub's prime-order subgroup.
//!
//! This is the pure math layer only: polynomial sampling, Feldman commitments, share
//! generation/verification, share combination, and public-key combination. It has no notion
//! of a process, a file, or a network — every function here takes and returns plain values and
//! is directly unit-testable in a single process (see `mod tests` below), which is exactly the
//! point: the *cryptography* isn't what changes between the old single-process
//! `DevCommitteeSet` simulator and the new ceremony (`src/bin/dkg_party.rs`) — what changes is
//! who calls these functions and what they're allowed to see when they do. `dkg_party.rs` is
//! the module that actually enforces party/process separation by calling these functions from
//! `n` separate OS processes, each with its own memory space, coordinating only through files.
//!
//! ## Where this fits changelog entry 73's design
//!
//! Entry 73 decided the *combination-across-committees* step ("hashed summation": 5
//! independently-keyed committees, each committee's OPRF output hashed and then summed) but
//! left *within one committee* ("each committee's ~35 members... presumably split each
//! committee's key across members") unspecified — `committee.rs`'s own doc comment flags this
//! exact gap: "this crate gives each committee one flat secret scalar" instead of a real
//! threshold split. This module is the threshold split entry 73 presumed but didn't specify:
//! classic Shamir/Feldman `t`-of-`n` secret sharing IS the right primitive for *that* layer.
//! **Update**: the gap this paragraph originally described (`pallet-identity` only ever
//! accepting one flat response per query+slot, so nothing downstream could actually consume a
//! genuine share) is now closed — see `threshold.rs` in this crate for the real `t`-of-`n`
//! evaluation protocol built on top of this module's DKG primitives, and
//! `pallets/pallet-identity`'s `submit_oprf_round1`/`submit_oprf_round2` for the on-chain
//! mailbox that now hosts it (`docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`).
//! It is a
//! *different* mechanism from entry 73's "hashed summation," not a competing one: hashed
//! summation combines 5 different committees' full outputs; this module is how members
//! *within* one of those 5 committees jointly hold that one committee's key in the first
//! place.
//!
//! ## What is real here
//!
//! - **Feldman verifiable secret sharing**, the standard, published construction (Feldman
//!   1987; the DKG built from it is Pedersen 1991 / "Joint-Feldman"): each party samples a
//!   random degree-`(t-1)` polynomial, commits to its coefficients on-curve, evaluates it at
//!   every other party's index, and every recipient can verify a received share against the
//!   sender's public commitments without learning the sender's polynomial or any other
//!   recipient's share. [`verify_share`]'s equation (`s*G == sum_k idx^k * C_k`) is the
//!   textbook Feldman check, not a simplification of it.
//! - **Real threshold behavior**: [`test_only_reconstruct_secret`] (see its own doc — test/
//!   audit code, never part of the live protocol) reconstructs the identical secret from *any*
//!   qualifying size-`t` subset of shares, the actual property that makes this "`t`-of-`n`"
//!   rather than "`n`-of-`n`" — checked directly in `mod tests` from two disjoint subsets.
//! - **Reuses this crate's already-validated group arithmetic unmodified**: every point
//!   operation here is `babyjubjub::Point::add`/`scalar_mul`/`generator`, every scalar
//!   operation is arithmetic mod `scalar::fr_order()` — no new curve or field code, matching
//!   this crate's standing rule (see top-level README) of never reimplementing already-checked
//!   crypto.
//! - **The reconstructed group secret is provably interchangeable with a `DevCommittee`'s flat
//!   secret** for every downstream consumer: `oprf::evaluate` and `dlog::verify` are called
//!   completely unmodified against DKG output in `tests/dkg_ceremony.rs`, and accept it exactly
//!   as they accept `committee.rs`'s single-process key. Nothing downstream needs to know or
//!   care whether a secret came from one RNG call or from combining `t` independent shares.
//!
//! ## What this protocol does NOT handle — real gaps vs. a production ceremony
//!
//! - **No complaint/justification round.** Full GJKR handles a party sending a bad share by
//!   having the *recipient* publicly complain, the *accused sender* publicly justify (reveal
//!   the correct share, which anyone can then check against the commitment), and the group
//!   collectively deciding whether to disqualify the sender and proceed without them. This
//!   module's [`verify_share`] can *detect* a bad share (`dkg_party.rs` does call it, per
//!   party, on every received share) but `dkg_party.rs`'s response to a failure is to abort the
//!   whole ceremony loudly rather than disqualify-and-continue — simpler, safe (never proceeds
//!   on unverified material), but means one misbehaving party can force a full restart rather
//!   than just being excluded. A real ceremony needs the full complaint/justification protocol
//!   (or a re-run with the bad party removed) to avoid that being a trivial denial-of-service.
//! - **No bias-resistance / rushing-adversary defense beyond Feldman's own.** Joint-Feldman is
//!   secure against a *static* adversary controlling fewer than `t` parties from the start; it
//!   is not the stronger "non-interactively biasable" construction some later DKG literature
//!   targets. Fine for a founding ceremony of vetted, named individuals (entry 73's model);
//!   would need re-examination for the eventual sortition-selected 35-member committees, where
//!   an adaptive adversary choosing which parties to corrupt *after* seeing early protocol
//!   messages is a more realistic threat.
//! - **No proactive resharing.** This module produces one set of shares for one committee
//!   generation. Refreshing shares periodically without changing the group public key
//!   (defends against a slow, share-by-share compromise over time) is a related but distinct
//!   protocol (Herzberg et al. 1995's proactive secret sharing; CHURP, cited in changelog entry
//!   73, is the closest real precedent) — not implemented here.
//! - **Reconstruction, when it happens, needs `t` shares in one place.** This is inherent to
//!   Shamir/Feldman, not a bug: [`test_only_reconstruct_secret`] exists ONLY so this crate's
//!   own tests can confirm the ceremony's output is mathematically well-formed. A real
//!   committee must never actually run reconstruction as part of answering a query — a genuine
//!   deployment needs either (a) a non-interactive threshold evaluation scheme (each party
//!   computes a *partial* evaluation from its own share; a public combiner step reconstructs
//!   the final OPRF output from `t` partial evaluations via the exponent, never reconstructing
//!   the scalar secret itself) or (b) a fixed designated combiner party trusted with
//!   less-sensitive material. Neither is built here — see "What a real ceremony still needs"
//!   in `src/bin/dkg_party.rs` and this crate's README for the fuller list.

use crate::babyjubjub::Point;
use crate::scalar;
use num_bigint::BigUint;
use num_traits::Zero;

/// One party's random degree-`(t-1)` secret polynomial, `f(x) = a_0 + a_1 x + ... +
/// a_{t-1} x^{t-1}`, coefficients uniform in `[1, fr_order())` via
/// [`scalar::random_nonzero`]. `a_0` (`coeffs[0]`) is this party's own secret contribution to
/// the eventual group secret — never itself transmitted anywhere; only evaluations of the
/// whole polynomial at other parties' indices are.
pub struct Polynomial {
    pub coeffs: Vec<BigUint>,
}

impl Polynomial {
    /// Samples a fresh random polynomial of degree `t - 1` (so `t` coefficients). `t` is the
    /// reconstruction threshold: any `t` of the `n` shares this polynomial (summed with every
    /// other party's) eventually produces can reconstruct the group secret; any `t - 1` reveal
    /// nothing (information-theoretic, the standard Shamir guarantee).
    pub fn random(t: usize, rng: &mut impl rand::RngCore) -> Self {
        assert!(t >= 1, "threshold must be at least 1");
        let coeffs = (0..t).map(|_| scalar::random_nonzero(rng)).collect();
        Polynomial { coeffs }
    }

    /// `f(x) mod fr_order()` via Horner's method. `x` is a 1-based party index; `x = 0` is
    /// reserved for the secret itself (`f(0) = coeffs[0]`) and is never evaluated as an actual
    /// party's share in this protocol — only [`test_only_reconstruct_secret`] ever computes
    /// (indirectly, via interpolation, not direct evaluation) the value at `0`.
    pub fn eval(&self, x: u64) -> BigUint {
        let modulus = scalar::fr_order();
        let x = BigUint::from(x) % &modulus;
        let mut acc = BigUint::zero();
        for c in self.coeffs.iter().rev() {
            acc = (&acc * &x + c) % &modulus;
        }
        acc
    }
}

/// Feldman commitments to a polynomial's coefficients: `C_k = a_k * G`. Safe to broadcast
/// publicly — this is exactly what lets [`verify_share`] check a received share without either
/// party learning anything the other didn't already intend to reveal.
pub fn commit(poly: &Polynomial) -> Vec<Point> {
    poly.coeffs.iter().map(|c| Point::generator().scalar_mul(c)).collect()
}

/// Verifies a share `s` (claimed to be `f(party_index)` for the polynomial `commitments` was
/// computed from) against the sender's public Feldman commitments: checks
/// `s*G == sum_k (party_index^k) * C_k`. This is the textbook Feldman VSS verification
/// equation — it holds if and only if `s` really is `f(party_index)` for *some* polynomial
/// with those exact commitments, without the verifier ever seeing `f` itself.
pub fn verify_share(share: &BigUint, party_index: u64, commitments: &[Point]) -> bool {
    let lhs = Point::generator().scalar_mul(share);
    let modulus = scalar::fr_order();
    let idx = BigUint::from(party_index) % &modulus;
    let mut rhs = Point::identity();
    let mut power = BigUint::from(1u64);
    for c in commitments {
        rhs = rhs.add(&c.scalar_mul(&power));
        power = (&power * &idx) % &modulus;
    }
    lhs == rhs
}

/// Sums a party's own verified incoming shares into that party's final DKG output share,
/// `x_i = sum_j f_j(i) mod fr_order()`. Deliberately takes only plain `BigUint`s, not any
/// notion of "which process called this" — the actual isolation guarantee comes from
/// `dkg_party.rs` only ever calling this once per OS process, over shares that process alone
/// received, never from combining two different parties' shares in one call.
pub fn combine_received_shares(shares: &[BigUint]) -> BigUint {
    let modulus = scalar::fr_order();
    shares.iter().fold(BigUint::zero(), |acc, s| (&acc + s) % &modulus)
}

/// The group public key: `Y = sum_i (a_i0 * G) = (sum_i a_i0) * G`, i.e. `G` times the sum of
/// every party's own secret contribution. Takes only the *first* commitment
/// (`commitments[0]`) from each party — purely a function of public values, computable by
/// anyone (not just a committee member) once every party has posted round-1 commitments, which
/// is exactly why `dkg_party.rs` has every party compute and publish its own copy independently
/// rather than routing this through a single trusted combiner.
pub fn group_public_key(first_commitments: &[Point]) -> Point {
    first_commitments.iter().fold(Point::identity(), |acc, c| acc.add(c))
}

/// **Test/audit-only — never part of the live protocol, and `dkg_party.rs` never calls this.**
/// Reconstructs the full group secret from `t` `(party_index, share)` pairs via Lagrange
/// interpolation at `x = 0`: `secret = sum_i share_i * prod_{j != i} (x_j / (x_j - x_i))`.
///
/// A real deployment must never run this outside a test/audit harness verifying the ceremony's
/// math after the fact — reconstructing the full secret in one place, even transiently, is
/// exactly the single point of failure `t`-of-`n` sharing exists to avoid. It exists here
/// solely so this crate's own tests (and any future auditor re-checking a real ceremony's
/// transcript) can confirm a completed DKG run produced a secret consistent with
/// [`group_public_key`]'s output, using the same `oprf::evaluate`/`dlog::verify` this crate
/// already validates for a flat single-process secret — see `tests/dkg_ceremony.rs`.
pub fn test_only_reconstruct_secret(indices_and_shares: &[(u64, BigUint)]) -> BigUint {
    let modulus = scalar::fr_order();
    let mut secret = BigUint::zero();
    for &(xi, ref yi) in indices_and_shares {
        let mut num = BigUint::from(1u64);
        let mut den = BigUint::from(1u64);
        for &(xj, _) in indices_and_shares {
            if xj == xi {
                continue;
            }
            num = (&num * BigUint::from(xj)) % &modulus;
            let diff = if xj > xi {
                BigUint::from(xj - xi) % &modulus
            } else {
                (&modulus - (BigUint::from(xi - xj) % &modulus)) % &modulus
            };
            den = (&den * diff) % &modulus;
        }
        let den_inv = scalar::inverse(&den);
        let lagrange_coeff = (&num * den_inv) % &modulus;
        secret = (&secret + (yi * &lagrange_coeff) % &modulus) % &modulus;
    }
    secret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dlog;
    use crate::oprf;
    use ark_bn254::Fr;
    use ark_ff::PrimeField;

    fn fe(dec: &str) -> Fr {
        Fr::from_be_bytes_mod_order(&BigUint::parse_bytes(dec.as_bytes(), 10).unwrap().to_bytes_be())
    }

    /// Single-process simulation of the whole `n`-party protocol (no files, no processes —
    /// `dkg_party.rs`/`tests/dkg_ceremony.rs` are what add the real process boundary). This
    /// test exists to validate the *math* in isolation before trusting the process-separated
    /// version built on top of it: every party's polynomial, every commitment, every pairwise
    /// share and its Feldman verification, all in one place where a test can inspect
    /// everything (which a real ceremony's own participants individually never can).
    #[test]
    fn feldman_dkg_produces_a_threshold_consistent_secret() {
        let mut rng = rand::rngs::mock::StepRng::new(0xF00D, 0xBEEF);
        let n = 5usize;
        let t = 3usize;

        let polys: Vec<Polynomial> = (0..n).map(|_| Polynomial::random(t, &mut rng)).collect();
        let commitments: Vec<Vec<Point>> = polys.iter().map(commit).collect();

        // Every party i (1-based) sums shares from every party j, verifying each against j's
        // published commitments first — exactly what `dkg_party.rs` does per-process.
        let mut final_shares: Vec<BigUint> = Vec::with_capacity(n);
        for i in 1..=n as u64 {
            let mut received = Vec::with_capacity(n);
            for j in 0..n {
                let s = polys[j].eval(i);
                assert!(
                    verify_share(&s, i, &commitments[j]),
                    "party {i}'s share from sender {j} must verify against sender's commitments"
                );
                received.push(s);
            }
            final_shares.push(combine_received_shares(&received));
        }

        let first_commitments: Vec<Point> = commitments.iter().map(|c| c[0]).collect();
        let y = group_public_key(&first_commitments);

        // Reconstruct from the first `t` shares...
        let subset_a: Vec<(u64, BigUint)> =
            (1..=t as u64).map(|i| (i, final_shares[(i - 1) as usize].clone())).collect();
        let secret_a = test_only_reconstruct_secret(&subset_a);
        assert_eq!(
            Point::generator().scalar_mul(&secret_a),
            y,
            "secret reconstructed from shares 1..=t must match the group public key"
        );

        // ...and from a DISJOINT (where n allows) set of t shares — the actual "any qualifying
        // subset reconstructs the same secret" threshold property, not just "reconstruction
        // works for one arbitrarily chosen subset."
        let subset_b: Vec<(u64, BigUint)> = ((n - t + 1)..=n)
            .map(|i| (i as u64, final_shares[i - 1].clone()))
            .collect();
        let secret_b = test_only_reconstruct_secret(&subset_b);
        assert_eq!(secret_a, secret_b, "every qualifying t-subset must reconstruct the identical secret");

        // The reconstructed secret must be usable by this crate's EXISTING, unmodified
        // committee-evaluation math exactly as a `DevCommittee`'s flat secret is — the concrete
        // claim that a threshold-shared secret is interchangeable with a flat one for every
        // downstream consumer.
        let ds_dlog = fe("1523098184080632582082867317389990410064981862");
        let client_input = fe("13131313131313131313");
        let beta = BigUint::from(2468013579u64);
        let b_q = oprf::blinded_query(&beta, client_input);
        let eval = oprf::evaluate(&mut rng, &secret_a, &b_q, ds_dlog);
        assert!(dlog::verify(eval.dlog_e, eval.dlog_s, &eval.pk, &b_q, &eval.response_blinded, ds_dlog));
        assert_eq!(eval.pk, y, "the evaluation's own pk = sk*G must equal the DKG's published group key");
    }

    #[test]
    fn verify_share_rejects_a_tampered_share() {
        let mut rng = rand::rngs::mock::StepRng::new(7, 11);
        let poly = Polynomial::random(3, &mut rng);
        let commitments = commit(&poly);
        let real_share = poly.eval(4);
        let tampered = (&real_share + BigUint::from(1u64)) % scalar::fr_order();
        assert!(verify_share(&real_share, 4, &commitments));
        assert!(!verify_share(&tampered, 4, &commitments));
    }
}
