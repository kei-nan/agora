//! Agora identity-anchor derivation primitives.
//!
//! Shared by every binary in this workspace (`query`/`anchor`/`disclosure`/`migrate`/
//! `migrate-disclosure` for citizen registration, `delegate-query`/`delegate-persona` for
//! delegate-persona creation) so the OPRF client input can never drift between the circuit
//! that *blinds* it and the circuits that *verify the committee's evaluation of it*. If those
//! two derivations disagreed by a single byte the OPRF verification would simply fail, so
//! keeping them in one place is a correctness requirement, not just tidiness.
//!
//! See `circuits/oprf-identity-anchor/README.md` and `docs/project/changelog/069-070.md`
//! entry 69 for the full design rationale, and changelog entry 67 for the architecture
//! decision this implements.

use poseidon::poseidon2::Poseidon2;
use utils::{
    get_array_slice, get_issuing_country_from_mrz, get_mrz_from_dg1, is_id_card,
    pack_be_bytes_into_field,
    types::{DG1Data, SaltedValue},
};

pub mod tests;

/// Offset of the "personal number" / optional-data field within the *flat* 88-character TD3
/// MRZ (i.e. after `DG1_TO_MRZ_OFFSET = 5` has already been removed from DG1).
///
/// ICAO Doc 9303 Part 4, TD3 (passport) MRZ line 2, character positions 29-42 (1-indexed).
/// Line 2 starts at flat index 44, so 44 + (29 - 1) = 72.
///
/// Cross-checked against ZKPassport's own already-verified TD3 offsets in
/// `src/noir/lib/utils/src/constants.nr`, which place the document number at 44,
/// nationality at 54, birthdate at 57, gender at 64 and expiry at 65 — the personal-number
/// field is the 14 characters immediately after the expiry-date check digit at 71, followed
/// by its own check digit at 86 and the composite check digit at 87.
pub global PASSPORT_MRZ_PERSONAL_NUMBER_INDEX: u32 = 72;

/// Length of the TD3 personal-number field, in characters. Positions 29-42 inclusive.
pub global PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH: u32 = 14;

/// ASCII `<`, the ICAO filler character.
pub global ASCII_CODE_FOR_LESS_THAN: u8 = 60;

/// Domain separator for the OPRF *client input*, ASCII "AGORA_IDENTITY_ANCHOR_V1" read as a
/// big-endian integer. Follows the same convention as ZKPassport's `DS_OPRF` ("OPRF") and
/// `DS_DLOG` ("DLOG Equality Proof") in `src/noir/lib/commitment/scoped-nullifier/src/lib.nr`.
pub global DS_IDENTITY_INPUT: Field = 1600625489088573789141910504985227416275133972950615545393;

/// Domain separator for the final OPRF output (`ds_out` argument of `verified_oprf`).
/// ASCII "AGORA_ANCHOR_OPRF_OUT". Purely client-side — the committee never sees it — so it
/// only needs to agree between `anchor` and `migrate`, not with any server configuration.
pub global DS_ANCHOR_OUT: Field = 95404713695560282209143460371751514488667227772244;

/// Domain separator for the Chaum-Pedersen DLog-equality proof (`ds_dlog` argument of
/// `verified_oprf`). ASCII "DLOG Equality Proof".
///
/// UNLIKE `DS_ANCHOR_OUT`, this value MUST match the deployed OPRF committee service's own
/// configuration, because the service produces the DLEQ proof this constant is used to check.
/// It is deliberately kept byte-identical to ZKPassport's `DS_DLOG`, which is the value the
/// TACEO `oprf-service` implementation is already known to be driven with in production.
pub global DS_DLOG: Field = 1523098184080632582082867317389990410064981862;

/// Domain separator for the published anchor itself, ASCII "AGORA_ANCHOR_V1".
pub global DS_ANCHOR: Field = 338945631368262504456167009336383025;

/// Domain separator for the delegate-persona OPRF *client input*, ASCII
/// "AGORA_DELEGATE_INPUT_V1" read as a big-endian integer. See
/// `derive_delegate_identity_input` for why delegate-persona creation uses a **distinctly
/// scoped** client input rather than reusing `identity_input`/`DS_IDENTITY_INPUT` — this is
/// the load-bearing separator that makes the two OPRF evaluations genuinely independent PRF
/// outputs, not merely two differently-tagged hashes of the same secret.
pub global DS_DELEGATE_IDENTITY_INPUT: Field = 6252443316752239663789143805072366868461078375420614193;

/// Domain separator for the delegate-persona OPRF final output (`ds_out` argument of
/// `verified_oprf`), ASCII "AGORA_DELEGATE_OPRF_OUT". Purely client-side — the committee
/// never sees it — mirrors `DS_ANCHOR_OUT`'s role exactly, just for the delegate-persona
/// evaluation.
pub global DS_DELEGATE_OUT: Field = 6252443316752239663789143805072366868893989024383849812;

/// Domain separator for the published delegate-persona ID itself, ASCII "AGORA_DELEGATE_V1".
/// Mirrors `DS_ANCHOR`'s role in `derive_committee_anchor_term` /
/// `combine_committee_anchors`, but see `derive_delegate_persona_term` for why this alone
/// would not be sufficient domain separation from the anchor even without
/// `DS_DELEGATE_IDENTITY_INPUT`/`DS_DELEGATE_OUT` above.
pub global DS_DELEGATE: Field = 22213140897350455076481323875272149128753;

/// Number of independent OPRF committees a registration/reverification/migration proof must
/// evaluate against. Fixed at 5 per changelog entry 73's governance design: every registration
/// query is sent to all 5 committees symmetrically (not routed by hash, which would leak a
/// low-entropy per-citizen signal), and the final anchor is a hashed-summation combination of
/// all 5 responses. This is `n`-of-`n` combination, not threshold — see entry 73 for the
/// disclosed 48h-SLA availability tradeoff that follows from that.
pub global NUM_COMMITTEES: u32 = 5;

/// The raw TD3 personal-number field, as 14 ASCII characters (right-padded with `<`).
pub type PersonalNumber = [u8; PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH];

/// Extracts the personal-number field from DG1.
///
/// Rejects TD1/ID-card documents outright: their MRZ has a different layout (two separate,
/// differently-sized optional-data slots), and Agora is passport-only for v1 per CLAUDE.md.
/// Silently reading TD3 offsets out of a TD1 MRZ would produce a well-formed but meaningless
/// anchor, so this is an assert rather than a branch.
pub fn get_personal_number_from_dg1(dg1: DG1Data) -> PersonalNumber {
    assert(
        !is_id_card(dg1),
        "Identity anchor requires a TD3 passport; TD1 ID cards are not supported",
    );
    let mrz = get_mrz_from_dg1(dg1);
    get_array_slice::<_, PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH>(
        mrz,
        PASSPORT_MRZ_PERSONAL_NUMBER_INDEX,
    )
}

/// Fails the proof if the personal-number field is entirely filler.
///
/// This matters far more than it looks. The field is optional in ICAO Doc 9303 — a country
/// that does not populate it emits 14 `<` characters for every one of its citizens. Without
/// this check every such passport would derive the *same* OPRF input and therefore the same
/// anchor, so the very first citizen to register would consume the only anchor value the
/// country can produce and permanently lock out everyone else (the pallet rejects a reused
/// anchor with `AnchorAlreadyUsed`). Failing loudly at proof time is the only safe behaviour.
///
/// Changelog entry 67 left "does the deployment country actually populate this field?" as an
/// open question that could not be settled from the ICAO spec. This assert converts that
/// unknown from a silent, population-wide failure into an immediate, diagnosable one.
pub fn assert_personal_number_populated(personal_number: PersonalNumber) {
    let mut populated = false;
    for i in 0..PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH {
        if (personal_number[i] != ASCII_CODE_FOR_LESS_THAN) & (personal_number[i] != 0) {
            populated = true;
        }
    }
    assert(
        populated,
        "MRZ personal-number field is empty; this passport cannot produce an identity anchor",
    );
}

/// Forces the prover to supply DG1's *clear value*, not just its salted hash.
///
/// `SaltedValue<T>` supports a "hash only" mode (`salt == 0`, `value == default`) used
/// elsewhere in the ZKPassport pipeline to keep a value hidden while still binding it into a
/// commitment. That mode is unusable here and actively dangerous: `SaltedValue::get_hash()`
/// returns the supplied hash verbatim, so `comm_in` would still verify while `value` — and
/// therefore the extracted personal number — was all zeroes. Every prover would then derive
/// the same identity input. Requiring clear-value mode is what makes the commitment chain
/// actually constrain the bytes this circuit reads.
pub fn assert_dg1_clear_value(salted_dg1: SaltedValue<DG1Data>) {
    assert(
        salted_dg1.hash == 0,
        "DG1 must be provided as a clear value, not a hash, to derive the identity anchor",
    );
    assert(
        salted_dg1.salt != 0,
        "DG1 salt must be non-zero when providing DG1 as a clear value",
    );
}

/// Shared body of `derive_identity_input`/`derive_delegate_identity_input`:
/// `Poseidon2(ds, packed(personal_number[14]), packed(issuing_country[3]))`.
///
/// The issuing country is folded in even though Agora deploys one chain per country (so it is
/// effectively constant): it costs one extra Poseidon2 input, and it means the anchor space
/// stays collision-free by construction if the country allowlist is ever widened.
///
/// Kept private: callers pick a domain separator via the two `pub` wrappers below, never a
/// caller-chosen `Field`, so it is not possible to accidentally derive an identity input under
/// an ad hoc separator that neither `anchor`/`disclosure`/`migrate*` nor
/// `delegate-query`/`delegate-persona` actually use.
fn derive_scoped_identity_input(dg1: DG1Data, ds: Field) -> Field {
    let personal_number = get_personal_number_from_dg1(dg1);
    assert_personal_number_populated(personal_number);
    let issuing_country = get_issuing_country_from_mrz(dg1);

    Poseidon2::hash(
        [
            ds,
            pack_be_bytes_into_field::<PASSPORT_MRZ_PERSONAL_NUMBER_LENGTH, 31>(personal_number),
            pack_be_bytes_into_field::<3, 31>(issuing_country),
        ],
        3,
    )
}

/// Derives the citizen-registration OPRF client input from an authenticated DG1:
/// `derive_scoped_identity_input(dg1, DS_IDENTITY_INPUT)`.
///
/// This is the value that replaces `calculate_private_nullifier(dg1, e_content, sod_signature)`
/// in the upstream `oprf-auth` flow. Both inputs are byte ranges of DG1, which is already
/// bound into `comm_in` by the caller, so switching between them requires no change anywhere
/// upstream in the commitment chain.
pub fn derive_identity_input(dg1: DG1Data) -> Field {
    derive_scoped_identity_input(dg1, DS_IDENTITY_INPUT)
}

/// Derives the delegate-persona OPRF client input from an authenticated DG1:
/// `derive_scoped_identity_input(dg1, DS_DELEGATE_IDENTITY_INPUT)`.
///
/// # Why this is a *distinct* OPRF query, not a reuse of `identity_input`
///
/// An earlier design considered reusing the citizen's existing anchor `oprf_output_i` values
/// (cached from registration) and only changing the *outer* combiner separator —
/// `Poseidon2(DS_DELEGATE, i, oprf_output_i, scheme_version)` in place of
/// `Poseidon2(DS_ANCHOR, i, oprf_output_i, scheme_version)`. That would have been cheaper (no
/// extra committee round-trip) and is plausibly safe under a random-oracle argument alone: the
/// committee never sees `ds_out`/the outer separator (both are applied client-side, after
/// unblinding — see `TaceoLabs/oprf-nr`'s `generate_output`), so on-chain `anchor` and
/// `delegate_persona_id` would be two independent-looking hash evaluations of the same secret
/// `oprf_output_i`, unlinkable to an observer who never learns `oprf_output_i` itself. That
/// argument leans on exactly one thing: that Poseidon2 provides real domain separation between
/// `H(DS_ANCHOR, i, x, v)` and `H(DS_DELEGATE, i, x, v)` for a shared secret `x`.
///
/// This module deliberately does not rely on that alone. Scoping the *client input* itself —
/// so `verified_oprf` evaluates each committee's actual PRF (not just a downstream hash) at a
/// different point for the anchor versus the delegate persona — means unlinkability instead
/// rests on the OPRF's own PRF security (the same assumption `identity_input` vs. any other
/// hypothetical future per-purpose input would already need), the same class of guarantee this
/// codebase already leans on for `DS_IDENTITY_INPUT` itself, rather than adding a new reliance
/// on hash-domain-separation-under-a-shared-preimage holding perfectly. It also removes an
/// operational hazard the reuse design would otherwise introduce: reusing cached `oprf_output_i`
/// values would require the client to persist raw per-committee OPRF response material
/// (`OPRFProof`) indefinitely after registration, which is not otherwise required anywhere in
/// this pipeline (`disclosure`/`migrate-disclosure` both run a fresh committee round-trip on
/// every invocation) and would widen the at-rest secrets an attacker could exfiltrate from a
/// compromised device. Since delegate-persona creation is already specified as "a fresh,
/// dedicated full proof event" requiring its own passport scan, the fresh-committee-round-trip
/// cost of a genuinely distinct query is not incremental UX friction on top of that — it is the
/// natural cost of the event, not an extra one.
pub fn derive_delegate_identity_input(dg1: DG1Data) -> Field {
    derive_scoped_identity_input(dg1, DS_DELEGATE_IDENTITY_INPUT)
}

/// Derives one committee's contribution to the published identity anchor.
///
/// `Poseidon2(DS_ANCHOR, committee_index, oprf_output, scheme_version)`
///
/// `committee_index` (0..`NUM_COMMITTEES`) is a circuit constant supplied by the caller's loop
/// position, not prover-controlled data — it exists purely for domain separation between the
/// 5 committees' terms before they are summed by `combine_committee_anchors`. Changelog entry
/// 73 requires each term be hashed before combination rather than summed as raw OPRF output:
/// real hybrid-cryptography combiners never sum raw group elements directly, since they carry
/// algebraic structure a combiner attack can exploit (RFC 9497's vOPRF spec has a dedicated
/// `Finalize`-hash step for exactly this reason). Hashing per-committee first, with a distinct
/// separator per committee, is what gives the combined anchor its security property: it stays
/// unpredictable as long as even one of the 5 committees is honest, even if the other 4 are
/// fully compromised (a standard one-time-pad argument, sound in the random-oracle model).
///
/// `scheme_version` is mixed in so that the same personal number under two different
/// committee generations yields unrelated terms, matching `pallet-identity`'s
/// `IdentityAnchorRegistry` key of `(scheme_version, anchor)`. It does not, on its own, prove
/// which committee key produced `oprf_output` — that is what the separately exposed
/// `oprf_pk_hash` public output is for, and the on-chain verifier must check the
/// `scheme_version -> oprf_pk_hash` binding itself, once per committee slot.
pub fn derive_committee_anchor_term(
    committee_index: u32,
    oprf_output: Field,
    scheme_version: Field,
) -> Field {
    Poseidon2::hash([DS_ANCHOR, committee_index as Field, oprf_output, scheme_version], 4)
}

/// Derives one committee's contribution to the published delegate-persona ID.
///
/// `Poseidon2(DS_DELEGATE, committee_index, oprf_output, scheme_version)` — structurally
/// identical to [`derive_committee_anchor_term`], but `oprf_output` here comes from evaluating
/// `derive_delegate_identity_input`'s output, not `derive_identity_input`'s, through a
/// **separate** committee round-trip (see that function's doc comment for why: the
/// distinctness has to live at the OPRF-query level, not only in this outer separator). Fed
/// into the same [`combine_committee_anchors`] combiner as the anchor — that function only
/// sums whatever 5 terms it is given, so no delegate-specific combiner is needed.
pub fn derive_delegate_persona_term(
    committee_index: u32,
    oprf_output: Field,
    scheme_version: Field,
) -> Field {
    Poseidon2::hash([DS_DELEGATE, committee_index as Field, oprf_output, scheme_version], 4)
}

/// Combines all 5 committees' hashed terms into the final published anchor by field addition.
///
/// `final_anchor = term_0 + term_1 + ... + term_4` (BN254 scalar field addition).
///
/// This is the "hashed summation" combiner from changelog entry 73: addition, not another
/// hash, because addition is what makes the one-time-pad security argument go through
/// (an adversary holding 4 of the 5 committees' secret terms can compute their sum and
/// subtract it from any observed `final_anchor`, but that only recovers the 5th, honest
/// committee's term — which is exactly as unpredictable to them as before, since it was
/// produced by a PRF they don't hold the key for). Combining by another hash instead would
/// not preserve this property.
pub fn combine_committee_anchors(terms: [Field; NUM_COMMITTEES]) -> Field {
    let mut sum: Field = 0;
    for i in 0..NUM_COMMITTEES {
        sum += terms[i];
    }
    sum
}
