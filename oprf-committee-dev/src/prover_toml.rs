//! Renders a `Prover.toml` for the `oprf_identity_anchor` (`anchor`) circuit's
//! `oprf_proofs: [OPRFProof; 5]` parameter (`utils::types::OPRFProof { pk, dlog_e, dlog_s,
//! response_blinded, response, beta }`) from this crate's simulated committee output.
//!
//! TOML array-of-structs-with-a-nested-struct-field syntax was verified empirically before
//! use here (a throwaway `nargo` package in this session's scratchpad, discarded — see the
//! changelog entry this crate ships with) rather than assumed: `[[oprf_proofs]]` starts each
//! array element, and `[oprf_proofs.pk]` / `[oprf_proofs.response_blinded]` /
//! `[oprf_proofs.response]` (singular brackets — they are themselves plain structs, not
//! arrays) attach to the most recently started element.

use crate::babyjubjub::Point;
use crate::committee::DevCommitteeSet;
use crate::oprf::{self, CommitteeEvaluation};
use ark_bn254::Fr;
use ark_ff::{BigInteger, PrimeField};
use num_bigint::BigUint;
use std::fmt::Write as _;

fn field_to_dec(f: &Fr) -> String {
    let bytes = f.into_bigint().to_bytes_be();
    BigUint::from_bytes_be(&bytes).to_string()
}

fn biguint_to_dec(x: &BigUint) -> String {
    x.to_string()
}

fn point_table(out: &mut String, name: &str, p: &Point) {
    let _ = writeln!(out, "[{name}]");
    let _ = writeln!(out, "x = \"{}\"", field_to_dec(&p.x));
    let _ = writeln!(out, "y = \"{}\"", field_to_dec(&p.y));
}

/// One assembled `OPRFProof` witness, ready to render: the committee's evaluation plus the
/// client's own `beta` and unblinded `response` (the client, not the committee, holds
/// these — see `oprf::unblind`).
pub struct OprfProofWitness {
    pub pk: Point,
    pub dlog_e: Fr,
    pub dlog_s: Fr,
    pub response_blinded: Point,
    pub response: Point,
    pub beta: BigUint,
}

impl OprfProofWitness {
    pub fn from_evaluation(eval: &CommitteeEvaluation, response: Point, beta: BigUint) -> Self {
        OprfProofWitness {
            pk: eval.pk,
            dlog_e: eval.dlog_e,
            dlog_s: eval.dlog_s,
            response_blinded: eval.response_blinded,
            response,
            beta,
        }
    }

    fn render(&self) -> String {
        self.render_named("oprf_proofs")
    }

    /// Same as `render`, but under an arbitrary TOML array name — needed by the migration
    /// circuits, which have two separate `[OPRFProof; 5]` parameters (`old_oprf_proofs`/
    /// `new_oprf_proofs`) rather than `anchor`/`disclosure`'s single `oprf_proofs`.
    fn render_named(&self, array_name: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "[[{array_name}]]");
        let _ = writeln!(out, "dlog_e = \"{}\"", field_to_dec(&self.dlog_e));
        let _ = writeln!(out, "dlog_s = \"{}\"", field_to_dec(&self.dlog_s));
        let _ = writeln!(out, "beta = \"{}\"", biguint_to_dec(&self.beta));
        point_table(&mut out, &format!("{array_name}.pk"), &self.pk);
        point_table(&mut out, &format!("{array_name}.response_blinded"), &self.response_blinded);
        point_table(&mut out, &format!("{array_name}.response"), &self.response);
        out
    }
}

/// Runs one full client+committee-generation round — generate 5 fresh committee keys, evaluate
/// the blinded query against all of them, unblind each response — and returns the 5 assembled
/// `OPRFProof` witnesses. This is exactly what `generate_anchor_prover_toml`'s `main` did
/// inline for its single committee generation; factored out here because the migration
/// circuits need to run it twice (once for the outgoing committees, once for the incoming
/// ones) against the same `identity_input`.
pub fn generate_proof_set(
    rng: &mut impl rand::RngCore,
    identity_input: Fr,
    beta: &BigUint,
    ds_dlog: Fr,
) -> [OprfProofWitness; 5] {
    let b_q = oprf::blinded_query(beta, identity_input);
    let committees = DevCommitteeSet::generate(rng);
    let evaluations = committees.evaluate_all(rng, &b_q, ds_dlog);

    let mut proofs: Vec<OprfProofWitness> = Vec::with_capacity(5);
    for eval in evaluations.iter() {
        let response = oprf::unblind(&eval.response_blinded, beta);
        proofs.push(OprfProofWitness::from_evaluation(eval, response, beta.clone()));
    }
    proofs.try_into().unwrap_or_else(|_| panic!("exactly 5"))
}

/// Writes the `salted_dg1`/`salted_expiry_date`/`salted_dg2_hash`/`salted_dg2_hash_type`/
/// `salted_private_nullifier` block every circuit in this workspace shares (`anchor`,
/// `disclosure`, `migrate`, `migrate-disclosure` all take the same `SaltedValue<...>`
/// witnesses ahead of their circuit-specific parameters) — the exact fixture
/// `query/src/main.nr`'s own `tests::fixture()` and `query/Prover.toml` already commit to, so
/// every Prover.toml this crate renders derives the identical `identity_input` the query
/// circuit's own committed fixture does.
fn write_salted_fixture(out: &mut String) {
    let _ = writeln!(out, "[salted_dg1]");
    let _ = writeln!(out, "salt = \"1111\"");
    let _ = writeln!(out, "hash = \"0\"");
    let _ = writeln!(out, "value = [\"97\", \"91\", \"95\", \"31\", \"88\", \"80\", \"60\", \"65\", \"85\", \"83\", \"83\", \"73\", \"76\", \"86\", \"69\", \"82\", \"72\", \"65\", \"78\", \"68\", \"60\", \"60\", \"74\", \"79\", \"72\", \"78\", \"78\", \"89\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"80\", \"65\", \"49\", \"50\", \"51\", \"52\", \"53\", \"54\", \"55\", \"95\", \"65\", \"85\", \"83\", \"56\", \"56\", \"49\", \"49\", \"49\", \"50\", \"95\", \"77\", \"51\", \"48\", \"48\", \"49\", \"48\", \"49\", \"95\", \"60\", \"67\", \"89\", \"66\", \"69\", \"82\", \"67\", \"73\", \"84\", \"89\", \"60\", \"60\", \"60\", \"60\", \"60\", \"60\", \"0\", \"0\"]");
    let _ = writeln!(out);
    let _ = writeln!(out, "[salted_expiry_date]");
    let _ = writeln!(out, "salt = \"2222\"");
    let _ = writeln!(out, "hash = \"0\"");
    let _ = writeln!(out, "value = [\"51\", \"48\", \"48\", \"49\", \"48\", \"49\"]");
    let _ = writeln!(out);
    let _ = writeln!(out, "[salted_dg2_hash]");
    let _ = writeln!(out, "salt = \"3333\"");
    let _ = writeln!(out, "hash = \"0\"");
    let _ = writeln!(out, "value = \"4444\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "[salted_dg2_hash_type]");
    let _ = writeln!(out, "salt = \"5555\"");
    let _ = writeln!(out, "hash = \"0\"");
    let _ = writeln!(out, "value = \"3\"");
    let _ = writeln!(out);
    let _ = writeln!(out, "[salted_private_nullifier]");
    let _ = writeln!(out, "salt = \"0\"");
    let _ = writeln!(out, "hash = \"6666\"");
    let _ = writeln!(out, "value = \"0\"");
    let _ = writeln!(out);
}

/// The `anchor` circuit's non-OPRF inputs, reusing exactly the same fixture
/// `query/src/main.nr`'s own `tests::fixture()` and `query/Prover.toml` already use, so the
/// `comm_in`/`identity_input` this crate derives its blinded query from is the one those
/// files already document (not a fresh, undocumented fixture).
pub struct AnchorFixture {
    pub comm_in: String,
    pub scheme_version: String,
}

/// Renders the full `Prover.toml` for `oprf_identity_anchor` given 5 assembled proofs.
pub fn render_anchor_prover_toml(fixture: &AnchorFixture, proofs: &[OprfProofWitness; 5]) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Generated by oprf-committee-dev (DEV-ONLY simulator).");
    let _ = writeln!(out, "# See oprf-committee-dev/README.md before trusting this for anything beyond");
    let _ = writeln!(out, "# local dev/test proving. Salted-value fixture matches");
    let _ = writeln!(out, "# circuits/oprf-identity-anchor/query/Prover.toml and query/src/main.nr's");
    let _ = writeln!(out, "# tests::fixture() exactly (same SAMPLE_DG1/salt/comm_in), so this Prover.toml");
    let _ = writeln!(out, "# derives the identical identity_input the query circuit's own committed fixture does.");
    let _ = writeln!(out);
    let _ = writeln!(out, "comm_in = \"{}\"", fixture.comm_in);
    let _ = writeln!(out, "scheme_version = \"{}\"", fixture.scheme_version);
    let _ = writeln!(out);
    write_salted_fixture(&mut out);

    for proof in proofs {
        out.push_str(&proof.render());
        out.push('\n');
    }

    out
}

/// The `migrate` circuit's non-OPRF inputs: same shared salted fixture, plus the two scheme
/// versions being rotated between (`old_scheme_version` must be nonzero; `new_scheme_version`
/// must differ from it, per `migrate/src/main.nr`'s own asserts).
pub struct MigrateFixture {
    pub comm_in: String,
    pub old_scheme_version: String,
    pub new_scheme_version: String,
}

/// Renders the full `Prover.toml` for `oprf_identity_anchor_migrate` given 5 old-committee
/// proofs and 5 new-committee proofs (`old_oprf_proofs`/`new_oprf_proofs` — the circuit's own
/// parameter names).
pub fn render_migrate_prover_toml(
    fixture: &MigrateFixture,
    old_proofs: &[OprfProofWitness; 5],
    new_proofs: &[OprfProofWitness; 5],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Generated by oprf-committee-dev (DEV-ONLY simulator).");
    let _ = writeln!(out, "# See oprf-committee-dev/README.md before trusting this for anything beyond");
    let _ = writeln!(out, "# local dev/test proving. `old_oprf_proofs` reuses the exact same committee");
    let _ = writeln!(out, "# generation + query as anchor/Prover.toml (same RNG seed, same beta, same");
    let _ = writeln!(out, "# identity_input), so `old_anchor` this circuit computes should equal the");
    let _ = writeln!(out, "# `anchor` value anchor/Prover.toml's proof already produced. `new_oprf_proofs`");
    let _ = writeln!(out, "# is an independently-generated committee set standing in for the post-rotation");
    let _ = writeln!(out, "# scheme.");
    let _ = writeln!(out);
    let _ = writeln!(out, "comm_in = \"{}\"", fixture.comm_in);
    let _ = writeln!(out, "old_scheme_version = \"{}\"", fixture.old_scheme_version);
    let _ = writeln!(out, "new_scheme_version = \"{}\"", fixture.new_scheme_version);
    let _ = writeln!(out);
    write_salted_fixture(&mut out);

    for proof in old_proofs {
        out.push_str(&proof.render_named("old_oprf_proofs"));
        out.push('\n');
    }
    for proof in new_proofs {
        out.push_str(&proof.render_named("new_oprf_proofs"));
        out.push('\n');
    }

    out
}

/// The `migrate-disclosure` circuit's non-OPRF inputs: `migrate`'s fixture plus the same
/// `current_date`/`service_scope`/`service_subscope` triple `disclosure/Prover.toml` already
/// adds over `anchor`'s fixture.
pub struct MigrateDisclosureFixture {
    pub comm_in: String,
    pub old_scheme_version: String,
    pub new_scheme_version: String,
    pub current_date: String,
    pub service_scope: String,
    pub service_subscope: String,
}

/// Renders the full `Prover.toml` for `oprf_identity_anchor_migrate_disclosure`.
pub fn render_migrate_disclosure_prover_toml(
    fixture: &MigrateDisclosureFixture,
    old_proofs: &[OprfProofWitness; 5],
    new_proofs: &[OprfProofWitness; 5],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Generated by oprf-committee-dev (DEV-ONLY simulator). Reuses the SAME");
    let _ = writeln!(out, "# old/new committee proofs as migrate/Prover.toml, adding migrate-disclosure's");
    let _ = writeln!(out, "# extra public inputs (current_date, service_scope, service_subscope). See");
    let _ = writeln!(out, "# oprf-committee-dev/README.md before trusting this for anything beyond local");
    let _ = writeln!(out, "# dev/test proving.");
    let _ = writeln!(out);
    let _ = writeln!(out, "comm_in = \"{}\"", fixture.comm_in);
    let _ = writeln!(out, "current_date = \"{}\"", fixture.current_date);
    let _ = writeln!(out, "old_scheme_version = \"{}\"", fixture.old_scheme_version);
    let _ = writeln!(out, "new_scheme_version = \"{}\"", fixture.new_scheme_version);
    let _ = writeln!(out, "service_scope = \"{}\"", fixture.service_scope);
    let _ = writeln!(out, "service_subscope = \"{}\"", fixture.service_subscope);
    let _ = writeln!(out);
    write_salted_fixture(&mut out);

    for proof in old_proofs {
        out.push_str(&proof.render_named("old_oprf_proofs"));
        out.push('\n');
    }
    for proof in new_proofs {
        out.push_str(&proof.render_named("new_oprf_proofs"));
        out.push('\n');
    }

    out
}

/// The `delegate-persona` circuit's non-OPRF inputs: `disclosure`'s
/// `comm_in`/`scheme_version`/`current_date`/`service_scope`/`service_subscope` fixture, plus
/// the `persona_account` witness (a raw 32-byte AccountId) `delegate-persona/src/main.nr`
/// additionally takes.
pub struct DelegatePersonaFixture {
    pub comm_in: String,
    pub scheme_version: String,
    pub current_date: String,
    pub service_scope: String,
    pub service_subscope: String,
    pub persona_account: [u8; 32],
}

/// Renders the full `Prover.toml` for `oprf_delegate_persona`. Unlike `render_anchor_prover_toml`
/// and friends, the OPRF proofs here are **not** expected to reuse `anchor`/`disclosure`'s
/// committee-evaluation output — see `identity_anchor::derive_delegate_identity_input`'s doc
/// comment for why delegate-persona creation runs its own, separately-scoped OPRF query against
/// the same 5 committees rather than reusing the registration query's responses.
pub fn render_delegate_persona_prover_toml(
    fixture: &DelegatePersonaFixture,
    proofs: &[OprfProofWitness; 5],
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Generated by oprf-committee-dev (DEV-ONLY simulator).");
    let _ = writeln!(out, "# See oprf-committee-dev/README.md before trusting this for anything beyond");
    let _ = writeln!(out, "# local dev/test proving. Salted-value fixture matches");
    let _ = writeln!(out, "# circuits/oprf-identity-anchor/delegate-query/Prover.toml exactly (same");
    let _ = writeln!(out, "# SAMPLE_DG1/salt/comm_in as the registration fixture — comm_in does not depend");
    let _ = writeln!(out, "# on which client input is being blinded). The 5 oprf_proofs below are a FRESH");
    let _ = writeln!(out, "# evaluation against delegate_identity_input (identity_anchor::");
    let _ = writeln!(out, "# derive_delegate_identity_input(SAMPLE_DG1)), using the SAME simulated 5-committee");
    let _ = writeln!(out, "# key set (RNG seed 0xA6012) as anchor/Prover.toml — standing in for the same real");
    let _ = writeln!(out, "# 5 committees answering a second, distinctly-scoped query, not a different committee");
    let _ = writeln!(out, "# set. Deliberately NOT copy-pasted from anchor/Prover.toml's oprf_proofs block, unlike");
    let _ = writeln!(out, "# disclosure/Prover.toml's relationship to anchor/Prover.toml.");
    let _ = writeln!(out);
    let _ = writeln!(out, "comm_in = \"{}\"", fixture.comm_in);
    let _ = writeln!(out, "current_date = \"{}\"", fixture.current_date);
    let _ = writeln!(out, "scheme_version = \"{}\"", fixture.scheme_version);
    let _ = writeln!(out, "service_scope = \"{}\"", fixture.service_scope);
    let _ = writeln!(out, "service_subscope = \"{}\"", fixture.service_subscope);
    let persona_account_strs: Vec<String> =
        fixture.persona_account.iter().map(|b| format!("\"{b}\"")).collect();
    let _ = writeln!(out, "persona_account = [{}]", persona_account_strs.join(", "));
    let _ = writeln!(out);
    write_salted_fixture(&mut out);

    for proof in proofs {
        out.push_str(&proof.render());
        out.push('\n');
    }

    out
}
