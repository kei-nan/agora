//! Real, standalone verifier for `circuits/oprf-identity-anchor/backing-nullifier` — a
//! Semaphore/nullifier-scheme proof ("I know a secret whose derived commitment is a leaf of
//! the published backing-commitment tree"), *not* a passport-parsing circuit and *not* a
//! ZKPassport disclosure subproof. See that circuit's `src/main.nr` module docs for the full
//! design and, in particular, for why this module — unlike [`crate::anchor_verifier`] — does
//! perform a genuine pairing check itself rather than only recomputing a commitment: this
//! circuit's proof never rides inside any outer ZKPassport proof, so nothing else on-chain
//! ever verifies it first.
//!
//! # Relationship to [`crate::verifier`]
//!
//! Reuses [`crate::verifier::ultrahonk::verify`] — the same real bb 5.0.0 UltraHonk pairing
//! primitive `ZkPassportUltraHonkVerifier` verifies the outer passport proof with — against
//! this circuit's own verification key instead of ZKPassport's outer-circuit VK. Everything
//! *around* that primitive (the envelope, the public-input shape, the VK) is specific to this
//! circuit and lives here, mirroring how [`crate::verifier`] owns the envelope/shape/VK for the
//! outer proof. The two envelopes are intentionally not interchangeable (different magic byte,
//! no `outer_count` field here — see [`parse_envelope`]) so a proof for one circuit can never be
//! mistaken for the other's.
//!
//! # Public-input layout — `oprf_backing_nullifier`, 4 fields
//!
//! Barretenberg orders UltraHonk public inputs as public parameters in declaration order,
//! followed by return values (same convention `circuits/oprf-identity-anchor/README.md`
//! documents for every other circuit here) — confirmed against a real `bb prove -t evm` run of
//! this circuit's own fixture, not inferred from the source:
//!
//! ```text
//! index   field                       notes
//! -----   -------------------------   -----------------------------------------------------
//! 0       root                        a backing-commitment tree root; caller must check this
//!                                     against `pallet_identity_zk::is_valid_backing_commitment_
//!                                     root` (this module does not — see "What this module does
//!                                     not check" below)
//! 1       delegate_persona_id         the backing's target; caller must check this equals the
//!                                     delegate the surrounding extrinsic actually claims to back
//! 2       max_backings_per_citizen    caller must check this equals the current on-chain
//!                                     `pallet_elections::MaxBackingsPerCitizen` value (see
//!                                     `src/main.nr`'s module docs for why this is a checked
//!                                     public input rather than a compile-time circuit constant)
//! 3       backing_nullifier           the value a caller should record/check for reuse
//! ```
//!
//! # What this module does and does not check
//!
//! Does: envelope parsing, canonical-field-element checking, the exact 4-field shape, and the
//! real UltraHonk pairing check — i.e. "is this a genuinely valid proof of the backing-nullifier
//! statement". Does **not** check `root` against live root history, `delegate_persona_id`
//! against whatever the surrounding extrinsic claims, `max_backings_per_citizen` against live
//! governance state, or `backing_nullifier` for prior reuse — all four are storage-dependent
//! checks a future pallet extrinsic must perform itself, exactly the split
//! `runtime/src/anchor_verifier.rs`'s module docs describe for `oprf_pk_hashes`/`current_date`
//! (mirrors how `register_citizen` already checks `AllowedMerkleRoots`/`OprfCommitteeKeys`
//! itself rather than delegating that to `T::ZkVerifier`). No pallet wires this module yet — see
//! `src/main.nr`'s "Status" section.

#![cfg(not(feature = "dev-mode"))]

extern crate alloc;

use crate::verifier::{ultrahonk, ProofVariant};

/// Envelope magic byte — `'B'`, for backing-nullifier. Deliberately distinct from
/// `crate::verifier`'s `0x5A` (`'Z'`, ZKPassport) so a proof for one circuit can never be
/// mistaken for the other's, even if a caller mixed up which verifier to call.
const ENVELOPE_MAGIC: u8 = 0x42;

/// Envelope format version. Bump on any layout change so an old client's proof is rejected
/// outright instead of being misparsed as the new shape.
const ENVELOPE_VERSION: u8 = 0x01;

/// Fixed envelope header length, in bytes: `magic(1) + version(1) + proof_variant(1) +
/// proof_len(4)`. One byte shorter than `crate::verifier`'s 8-byte header — there is no
/// `outer_count` field here, because this circuit has no ZKPassport-style family of variants;
/// exactly one circuit, exactly one VK.
const ENVELOPE_HEADER_LEN: usize = 7;

/// Number of public inputs this circuit exposes — see this module's public-input-layout docs.
const PUBLIC_INPUT_COUNT: usize = 4;

/// Index of `root` in the public-input array.
pub const ROOT_INDEX: usize = 0;
/// Index of `delegate_persona_id` in the public-input array.
pub const DELEGATE_PERSONA_ID_INDEX: usize = 1;
/// Index of `max_backings_per_citizen` in the public-input array.
pub const MAX_BACKINGS_PER_CITIZEN_INDEX: usize = 2;
/// Index of `backing_nullifier` in the public-input array.
pub const BACKING_NULLIFIER_INDEX: usize = 3;

/// BN254 scalar field modulus `r`, big-endian — duplicated from `crate::verifier` rather than
/// imported, since that constant is private to that module and this is a three-line check not
/// worth widening its visibility for (same reasoning `crate::anchor_verifier` already gives for
/// its own copy).
const BN254_FR_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

fn is_canonical_fr(value: &[u8; 32]) -> bool {
    *value < BN254_FR_MODULUS_BE
}

/// This circuit's verification key: the real `bb write_vk -t evm` output (1888 bytes,
/// `28*64 + 3*32` — the standard bb 5.0.0 UltraHonk `-t evm` VK size, same as
/// `crate::verifier`'s `count_4` asset) for `circuits/oprf-identity-anchor/backing-nullifier`,
/// compiled with `nargo 1.0.0-beta.22`. Produced and installed for real in this session —
/// see `circuits/oprf-identity-anchor/backing-nullifier/src/main.nr`'s "Status" section for the
/// full `nargo execute` -> `bb write_vk` -> `bb prove` -> `bb verify` round trip this asset and
/// this module's own test fixtures came from.
const BACKING_NULLIFIER_VK: &[u8] = include_bytes!("../assets/vk_backing_nullifier.bin");

/// Which `bb` proving mode produced the proof — mirrors `crate::verifier::ProofVariant`
/// exactly; re-exported rather than duplicated so the two envelopes stay byte-compatible in
/// spirit without actually sharing a header shape.
pub use crate::verifier::ProofVariant as BackingNullifierProofVariant;

/// The parsed envelope: everything the header pins down, plus the raw bb proof.
struct Envelope<'a> {
    variant: ProofVariant,
    proof: &'a [u8],
}

/// Parses and validates the proof envelope. `None` on any malformed input. Structurally
/// identical to `crate::verifier::parse_envelope` minus the `outer_count` byte — see this
/// module's docs for why that field doesn't exist here.
fn parse_envelope(proof_bytes: &[u8]) -> Option<Envelope<'_>> {
    if proof_bytes.len() < ENVELOPE_HEADER_LEN {
        return None;
    }
    if proof_bytes[0] != ENVELOPE_MAGIC || proof_bytes[1] != ENVELOPE_VERSION {
        return None;
    }

    let variant = match proof_bytes[2] {
        0 => ProofVariant::Zk,
        1 => ProofVariant::Plain,
        _ => return None,
    };

    let proof_len = u32::from_be_bytes([
        proof_bytes[3],
        proof_bytes[4],
        proof_bytes[5],
        proof_bytes[6],
    ]) as usize;

    // Exact-length match: reject both truncation and trailing padding, so a proof can never be
    // silently extended with attacker-chosen bytes the verifier ignores — same rationale as
    // `crate::verifier::parse_envelope`.
    if proof_bytes.len() != ENVELOPE_HEADER_LEN.saturating_add(proof_len) {
        return None;
    }

    // bb's UltraHonk proof is a flat array of 32-byte words; a length that isn't a multiple of
    // 32 cannot be one, whatever the circuit.
    if proof_len == 0 || proof_len % 32 != 0 {
        return None;
    }

    Some(Envelope { variant, proof: &proof_bytes[ENVELOPE_HEADER_LEN..] })
}

/// Validates the public-input array's shape: exactly [`PUBLIC_INPUT_COUNT`] canonical `Fr`
/// elements. Unlike `crate::verifier::check_public_inputs`, there is no per-variant field count
/// to branch on (this circuit has exactly one shape) and no semantic checks belong here either
/// — `root`/`delegate_persona_id`/`max_backings_per_citizen`/`backing_nullifier` are all values
/// a *caller* must additionally check against chain storage (see this module's top-of-file
/// docs); this function only rejects a structurally malformed array before it ever reaches the
/// pairing check.
fn check_public_inputs(public_inputs: &[[u8; 32]]) -> bool {
    public_inputs.len() == PUBLIC_INPUT_COUNT && public_inputs.iter().all(is_canonical_fr)
}

/// Full verification: envelope, shape, then the real UltraHonk pairing check against
/// [`BACKING_NULLIFIER_VK`]. `Some(())` only for a genuinely valid proof; `None` for every
/// failure mode alike, so a caller cannot mistake "could not check" for "checked and passed" —
/// same posture as `crate::verifier::verify_inner`.
fn verify_inner(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> Option<()> {
    if BACKING_NULLIFIER_VK.is_empty() {
        return None;
    }
    let envelope = parse_envelope(proof_bytes)?;
    if !check_public_inputs(public_inputs) {
        return None;
    }
    ultrahonk::verify(BACKING_NULLIFIER_VK, envelope.variant, envelope.proof, public_inputs)
}

/// The real `backing-nullifier` proof verifier. Not yet wired to any pallet `Config` — see this
/// module's top-of-file docs — so this is a plain function, not a trait impl, until a consumer
/// exists to define the trait shape against.
pub struct BackingNullifierVerifier;

impl BackingNullifierVerifier {
    /// `true` only for a proof that is genuinely valid against
    /// [`BACKING_NULLIFIER_VK`] and structurally well-shaped — see this module's top-of-file
    /// docs for what a caller must additionally check afterward.
    pub fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool {
        verify_inner(proof_bytes, public_inputs).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    /// A well-formed envelope around `proof_len` bytes of filler.
    fn envelope(magic: u8, version: u8, variant: u8, proof_len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(ENVELOPE_HEADER_LEN + proof_len);
        out.extend_from_slice(&[magic, version, variant]);
        out.extend_from_slice(&(proof_len as u32).to_be_bytes());
        out.extend(core::iter::repeat(0xAB).take(proof_len));
        out
    }

    fn valid_public_inputs() -> [[u8; 32]; PUBLIC_INPUT_COUNT] {
        let mut inputs = [[0u8; 32]; PUBLIC_INPUT_COUNT];
        inputs[BACKING_NULLIFIER_INDEX][31] = 1;
        inputs
    }

    #[test]
    fn parses_a_well_formed_envelope() {
        let bytes = envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 0, 64);
        let parsed = parse_envelope(&bytes).expect("well-formed envelope should parse");
        assert_eq!(parsed.variant, ProofVariant::Zk);
        assert_eq!(parsed.proof.len(), 64);
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        assert!(parse_envelope(&envelope(0x00, ENVELOPE_VERSION, 0, 64)).is_none());
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, 0x02, 0, 64)).is_none());
    }

    #[test]
    fn rejects_the_zkpassport_magic_byte() {
        // The whole point of a distinct magic byte: a ZKPassport-envelope proof must not be
        // mistaken for a backing-nullifier one, even if a caller passed the wrong bytes in.
        assert!(parse_envelope(&envelope(0x5A, ENVELOPE_VERSION, 0, 64)).is_none());
    }

    #[test]
    fn rejects_unknown_proof_variant() {
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 2, 64)).is_none());
    }

    #[test]
    fn rejects_truncated_and_padded_envelopes() {
        let bytes = envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 0, 64);
        assert!(parse_envelope(&bytes[..bytes.len() - 1]).is_none());

        let mut padded = bytes;
        padded.push(0x00);
        assert!(parse_envelope(&padded).is_none());
    }

    #[test]
    fn rejects_proof_length_that_is_not_a_multiple_of_a_field_element() {
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 0, 100)).is_none());
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 0, 0)).is_none());
    }

    #[test]
    fn accepts_a_well_shaped_public_input_array() {
        assert!(check_public_inputs(&valid_public_inputs()));
    }

    #[test]
    fn rejects_the_wrong_number_of_public_inputs() {
        assert!(!check_public_inputs(&[[0u8; 32]; 3]));
        assert!(!check_public_inputs(&[[0u8; 32]; 5]));
    }

    #[test]
    fn rejects_a_non_canonical_public_input() {
        let mut inputs = valid_public_inputs();
        inputs[0] = BN254_FR_MODULUS_BE;
        assert!(!check_public_inputs(&inputs));
    }

    #[test]
    fn backing_nullifier_vk_asset_is_the_real_bb_evm_vk() {
        assert_eq!(
            BACKING_NULLIFIER_VK.len(),
            1888,
            "the backing-nullifier VK must be the 1888-byte bb -t evm UltraHonk VK",
        );
    }

    // ---------------------------------------------------------------------------
    // Real bb 5.0.0 fixtures — a genuine `nargo execute` -> `bb write_vk -t evm` ->
    // `bb prove -t evm`/`-t evm-no-zk` -> `bb verify` round trip against the real
    // `oprf_backing_nullifier` circuit and the exact fixture in
    // `circuits/oprf-identity-anchor/backing-nullifier/Prover.toml` (whose header explains
    // where those numbers come from). See `src/main.nr`'s "Status" section.
    // ---------------------------------------------------------------------------

    const ZK_PROOF: &[u8] = include_bytes!("../assets/test/ultrahonk_backing_nullifier_zk/proof");
    const ZK_PUBS: &[u8] = include_bytes!("../assets/test/ultrahonk_backing_nullifier_zk/pubs");

    const PLAIN_PROOF: &[u8] =
        include_bytes!("../assets/test/ultrahonk_backing_nullifier_plain/proof");
    const PLAIN_PUBS: &[u8] =
        include_bytes!("../assets/test/ultrahonk_backing_nullifier_plain/pubs");

    fn split_pubs(raw: &[u8]) -> Vec<[u8; 32]> {
        assert_eq!(raw.len() % 32, 0, "a pubs file is a whole number of words");
        raw.chunks_exact(32)
            .map(|chunk| <[u8; 32]>::try_from(chunk).expect("chunks_exact(32) yields 32 bytes"))
            .collect()
    }

    fn wrap(variant_byte: u8, proof: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(ENVELOPE_HEADER_LEN + proof.len());
        bytes.extend_from_slice(&[ENVELOPE_MAGIC, ENVELOPE_VERSION, variant_byte]);
        bytes.extend_from_slice(&(proof.len() as u32).to_be_bytes());
        bytes.extend_from_slice(proof);
        bytes
    }

    /// The real cross-check vector: these public inputs must be exactly the tuple
    /// `circuits/oprf-identity-anchor/backing-nullifier/Prover.toml` and
    /// `lib/identity-anchor/src/tests.nr`'s
    /// `backing_commitment_and_nullifier_match_the_real_rust_side_vector`/
    /// `backing_tree_root_matches_the_real_rust_side_vector_for_an_empty_tree_first_leaf`
    /// independently reproduce from `pallets/pallet-identity`'s own Poseidon2 port.
    #[test]
    fn zk_fixture_public_inputs_match_the_documented_layout() {
        let pubs = split_pubs(ZK_PUBS);
        assert_eq!(pubs.len(), PUBLIC_INPUT_COUNT);

        let mut root = [0u8; 32];
        root.copy_from_slice(
            &hex_bytes("09efbc7edc789c162ded75ed12f2d9e84fb8822535552d9d2f80ce535762f46e"),
        );
        assert_eq!(pubs[ROOT_INDEX], root);

        let mut delegate_persona_id = [0u8; 32];
        delegate_persona_id[31] = 0x09;
        delegate_persona_id[30] = 0x03;
        assert_eq!(pubs[DELEGATE_PERSONA_ID_INDEX], delegate_persona_id);

        let mut max_backings = [0u8; 32];
        max_backings[31] = 5;
        assert_eq!(pubs[MAX_BACKINGS_PER_CITIZEN_INDEX], max_backings);

        let mut nullifier = [0u8; 32];
        nullifier.copy_from_slice(
            &hex_bytes("22b020294a8f8b82c34471b0c189da23fdacbbeaa2fec1427c327efa2b4d56e4"),
        );
        assert_eq!(pubs[BACKING_NULLIFIER_INDEX], nullifier);
    }

    fn hex_bytes(hex: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for i in 0..32 {
            out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    #[test]
    fn verifies_a_real_zk_proof_end_to_end() {
        let bytes = wrap(0, ZK_PROOF);
        assert!(
            BackingNullifierVerifier::verify(&bytes, &split_pubs(ZK_PUBS)),
            "a genuine bb 5.0.0 `-t evm` proof of the real circuit must verify",
        );
    }

    #[test]
    fn verifies_a_real_plain_proof_end_to_end() {
        let bytes = wrap(1, PLAIN_PROOF);
        assert!(
            BackingNullifierVerifier::verify(&bytes, &split_pubs(PLAIN_PUBS)),
            "a genuine bb 5.0.0 `-t evm-no-zk` proof of the real circuit must verify",
        );
    }

    #[test]
    fn rejects_a_zk_proof_declared_as_plain_and_vice_versa() {
        assert!(!BackingNullifierVerifier::verify(&wrap(1, ZK_PROOF), &split_pubs(ZK_PUBS)));
        assert!(!BackingNullifierVerifier::verify(&wrap(0, PLAIN_PROOF), &split_pubs(PLAIN_PUBS)));
    }

    #[test]
    fn rejects_single_bit_mutations_anywhere_in_a_real_proof() {
        let pubs = split_pubs(ZK_PUBS);
        let step = ZK_PROOF.len() / 40;
        let mut checked = 0;
        for offset in (0..ZK_PROOF.len()).step_by(step.max(1)) {
            let mut mutated = ZK_PROOF.to_vec();
            mutated[offset] ^= 0x01;
            let bytes = wrap(0, &mutated);
            assert!(
                !BackingNullifierVerifier::verify(&bytes, &pubs),
                "flipping bit 0 of proof byte {offset} must be rejected",
            );
            checked += 1;
        }
        assert!(checked >= 20, "expected a meaningful spread of offsets");
    }

    /// The load-bearing claim behind `src/main.nr`'s design decision to bind
    /// `delegate_persona_id` as a plain public input rather than folding it into a hash: this
    /// mutates *only* that one field (index 1) of a real, otherwise-untouched proof/public-input
    /// pair and confirms the proof is rejected — exactly the "resubmit against a different
    /// delegate" attack the circuit's module docs argue this prevents, demonstrated here against
    /// the real production verifier, not just asserted in prose.
    #[test]
    fn rejects_a_real_proof_resubmitted_against_a_different_delegate_persona_id() {
        let bytes = wrap(0, ZK_PROOF);
        let mut pubs = split_pubs(ZK_PUBS);
        pubs[DELEGATE_PERSONA_ID_INDEX][31] ^= 0x01;
        assert!(
            !BackingNullifierVerifier::verify(&bytes, &pubs),
            "a proof must not verify against a delegate_persona_id it was not generated for",
        );
    }

    /// Same claim, exercised across *every* public input, mirroring
    /// `crate::verifier`'s own `rejects_mutated_public_inputs_for_a_real_proof` — confirms `root`
    /// and `max_backings_per_citizen` are just as tamper-evident as `delegate_persona_id`.
    #[test]
    fn rejects_mutated_public_inputs_for_a_real_proof() {
        let bytes = wrap(0, ZK_PROOF);
        for index in 0..ZK_PUBS.len() {
            let mut raw = ZK_PUBS.to_vec();
            raw[index] ^= 0x01;
            assert!(
                !BackingNullifierVerifier::verify(&bytes, &split_pubs(&raw)),
                "flipping bit 0 of public-input byte {index} must be rejected",
            );
        }
    }

    #[test]
    fn rejects_a_real_proof_against_the_wrong_verification_key() {
        // The recursive fixtures in `crate::verifier`'s own test module are a real, well-formed,
        // correctly-sized VK for a different circuit entirely — the sharper negative case than a
        // corrupted VK.
        let foreign_vk = include_bytes!("../assets/test/ultrahonk_bb5_recursive_zk/vk");
        let pubs = split_pubs(ZK_PUBS);
        assert!(ultrahonk::verify(foreign_vk, ProofVariant::Zk, ZK_PROOF, &pubs).is_none());
    }

    #[test]
    fn rejects_malformed_proof_bytes_under_a_well_formed_envelope() {
        let bogus = wrap(0, &[0xABu8; 4544]);
        assert!(!BackingNullifierVerifier::verify(&bogus, &valid_public_inputs()));
    }

    #[test]
    fn rejects_wrong_shaped_public_inputs_before_reaching_the_pairing_check() {
        let bytes = wrap(0, ZK_PROOF);
        let mut pubs = split_pubs(ZK_PUBS);
        pubs.pop();
        assert!(!BackingNullifierVerifier::verify(&bytes, &pubs));
    }
}
