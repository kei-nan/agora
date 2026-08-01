//! ZKPassport UltraHonk passport-proof verifier (BN254 / Barretenberg).
//!
//! Replaces the former `RarimoGroth16Verifier`. Rarimo was dropped as the passport-ZK
//! vendor (see `docs/project/changelog/065-068.md` entry 65); ZKPassport's circuits
//! (<https://github.com/zkpassport/circuits>) are Noir/UltraHonk, not circom/Groth16,
//! so both the proof bytes and the public-input layout are completely different.
//!
//! Only compiled when the `dev-mode` feature is absent. In dev-mode, `configs/mod.rs`
//! uses `PassthroughZkVerifier` instead. Note `dev-mode` is a *default* feature of the
//! runtime crate, so a plain `cargo check --workspace` does not compile this file — use
//! `cargo check -p agora-runtime --no-default-features --features std`.
//!
//! # Current status: real pairing check, backed by `ultrahonk-no-std`
//!
//! The UltraHonk pairing check is now performed for real by `ultrahonk-no-std`
//! (<https://github.com/kei-nan/ultrahonk_verifier>, branch `bb-5.0.0-port`), a fork of
//! zkVerify's verifier ported from bb 3.0.3 to bb 5.0.0 — the version ZKPassport pins.
//! See [`ultrahonk`] for the integration and for what the port had to change.
//!
//! # Proof envelope (what `zk_proof` must contain)
//!
//! Produced by `mobile/src/chain/proofEncoding.ts::encodeUltraHonkProof`. The two files
//! are one contract; change neither alone.
//!
//! ```text
//! [ 0]      magic           0x5A ('Z' for ZKPassport)
//! [ 1]      format_version  0x01
//! [ 2]      outer_count     N, the ZKPassport `main/outer/count_N` variant (4..=13)
//! [ 3]      proof_variant   0 = ZK (`bb prove -t evm`), 1 = Plain (`bb prove -t evm-no-zk`)
//! [ 4.. 8]  proof_len       u32, big-endian
//! [ 8..8+L] proof           the raw bytes of bb's `proof` output file, verbatim
//! ```
//!
//! Total length must be exactly `8 + proof_len` — no padding, no truncation.
//!
//! The proof bytes are passed through byte-for-byte rather than re-encoded: bb's
//! UltraHonk proof is already a flat array of 32-byte big-endian words, and any
//! re-serialization on the mobile side would just be a chance to corrupt it. The
//! header exists to pin down *which* circuit and *which* proving mode produced it,
//! neither of which is recoverable from the proof bytes alone.
//!
//! # Public inputs — ZKPassport `main/outer/count_N`
//!
//! Passed separately (`public_inputs`), as bb's `public_inputs` output file chunked
//! into 32-byte big-endian canonical BN254 `Fr` elements. Confirmed against
//! `src/noir/bin/main/outer/count_N/src/main.nr` in the circuits repo at `d3a75ac`:
//!
//! ```text
//! index          field
//! ------------   -----------------------------------------------------------
//! 0              certificate_registry_root
//! 1              circuit_registry_root
//! 2              current_date              (unix seconds, u64)
//! 3              service_scope
//! 4              service_subscope
//! 5 .. 5+D       param_commitments[D]      D = N - 3 (the disclosure-subproof count)
//! 5+D            nullifier_type
//! 6+D            scoped_nullifier
//! 7+D            oprf_pk_hash
//! ```
//!
//! So `public_inputs.len() == N + 5` exactly; for the `count_4` variant that is 9.
//!
//! This is **not** Rarimo's old 5-signal layout
//! (`dg15PubKeyHash`, `passportHash`, `dg1Commitment`, `pkIdentityHash`, `slaveMerkleRoot`).
//!
//! ## Where the pairing-point (aggregation) object lives — and why it is *not* here
//!
//! ZKPassport's outer circuit recursively verifies its subproofs (`verify_proof_with_type`),
//! so bb attaches an 8-word **pairing-point object** to it. The obvious worry is that those
//! 8 words are spliced into the public-input array, silently shifting every index above —
//! which would make index `0` a pairing limb rather than `certificate_registry_root`. They
//! are not. **bb 5.0.0 carries the pairing-point object as the first 8 words of the `proof`
//! file, never in the `public_inputs` file**, and this holds for *every* circuit, recursive
//! or not; recursion changes only the values (non-trivial vs. eight zero words), not the
//! location. The verifier backend parses those 8 words straight off the front of the proof
//! (`ZKProof::from_bytes`/`PlainProof::from_bytes`), so nothing here has to know about them.
//!
//! This was established from real bytes, by two independent routes that agree:
//!
//! * **bb 5.0.0 fixtures.** Across all six of the backend's fixtures
//!   (`simple`/`rich`/`recursive` × `plain`/`zk`), the VK header's `combined_input_size`
//!   is exactly `len(public_inputs file)/32 + 8` — `10 = 2 + 8`, `11 = 3 + 8`, `10 = 2 + 8`.
//!   The `recursive_*` proofs (a circuit that genuinely recursion-verifies) begin with eight
//!   non-zero 136/118-bit limb words; the non-recursive `simple_*` proofs begin with eight
//!   zero words. The pairing object is in the proof in both cases, and in the public inputs
//!   in neither.
//! * **ZKPassport's own compiled VK.** `assets/vk_zkpassport_outer_count_4.bin` — the real
//!   `bb write_vk -t evm` output for `main/outer/count_4` — has `log_circuit_size = 22`,
//!   `pub_inputs_offset = 5` and `combined_input_size = 17`. That is `9 + 8`: nine semantic
//!   public inputs, exactly the `N + 5` table above, plus the eight pairing words bb counts
//!   separately. Cross-checked against `main/outer/count_4/src/main.nr` at the pinned commit,
//!   whose `fn main` declares exactly those nine `pub` parameters in exactly that order and
//!   returns nothing.
//!
//! `assert_count_4_vk_matches_the_documented_public_input_layout` pins this down in CI: it
//! parses the production VK asset with the backend's own parser and asserts
//! `combined_input_size == (4 + 5) + 8`. If ZKPassport ever changes the circuit's public
//! interface, that test fails loudly rather than this file silently misreading field indices.
//!
//! The backend independently enforces the same invariant on every call
//! (`combined_input_size - 8 == public_inputs.len()`), so a `count_N`-shaped array that
//! disagreed with the VK would be rejected there even if [`check_public_inputs`] let it past.
//!
//! `pallet_identity_zk::register_citizen` reads this layout correctly: index `0`
//! (`certificate_registry_root`) for the allowlist check, index `len - 2` (`6 + D`,
//! `scoped_nullifier`) for the nullifier, and its `public_inputs` bound is
//! `ConstU32<18>` (the `count_13` ceiling, `13 + 5`). This was previously mismatched
//! against Rarimo's old indices; fixed once this module pinned the real layout down. Those
//! indices were re-checked against the pairing-point finding above and need no adjustment —
//! the array the pallet sees is the semantic `N + 5` one, unshifted.

#![cfg(not(feature = "dev-mode"))]

extern crate alloc;

/// Envelope magic byte — `'Z'`, for ZKPassport.
const ENVELOPE_MAGIC: u8 = 0x5A;

/// Envelope format version. Bump on any layout change so an old client's proof is
/// rejected outright instead of being misparsed as the new shape.
const ENVELOPE_VERSION: u8 = 0x01;

/// Fixed envelope header length, in bytes.
const ENVELOPE_HEADER_LEN: usize = 8;

/// The three non-disclosure subproofs every ZKPassport outer circuit wraps
/// (`sig-check/dsc`, `sig-check/id-data`, `data-check/integrity`). The number of
/// disclosure subproofs — and hence of `param_commitments` — is `outer_count - 3`.
const BASE_SUBPROOF_COUNT: u8 = 3;

/// The number of public inputs an outer circuit exposes, excluding `param_commitments`:
/// `certificate_registry_root`, `circuit_registry_root`, `current_date`, `service_scope`,
/// `service_subscope`, `nullifier_type`, `scoped_nullifier`, `oprf_pk_hash`.
const FIXED_PUBLIC_INPUT_COUNT: usize = 8;

/// BN254 scalar field modulus `r`, big-endian. Public inputs are canonical field
/// elements, so every one must be strictly below this.
const BN254_FR_MODULUS_BE: [u8; 32] = [
    0x30, 0x64, 0x4e, 0x72, 0xe1, 0x31, 0xa0, 0x29, 0xb8, 0x50, 0x45, 0xb6, 0x81, 0x81, 0x58, 0x5d,
    0x28, 0x33, 0xe8, 0x48, 0x79, 0xb9, 0x70, 0x91, 0x43, 0xe1, 0xf5, 0x93, 0xf0, 0x00, 0x00, 0x01,
];

/// Which `bb` proving mode produced the proof. bb's ZK and non-ZK UltraHonk proofs have
/// different lengths *and* different transcript layouts, and nothing in the proof bytes
/// distinguishes them, so the envelope has to say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProofVariant {
    /// `bb prove -t evm` — keccak transcript, zero-knowledge.
    Zk,
    /// `bb prove -t evm-no-zk` — keccak transcript, no witness privacy.
    Plain,
}

/// Verification keys for the ZKPassport outer circuits this chain accepts, keyed by
/// `outer_count`.
///
/// Deliberately a short allowlist rather than "any `count_N`": each entry is a distinct
/// circuit this chain has to trust, and trusting ten of them when one is used just widens
/// the surface. `count_4` (3 base subproofs + 1 disclosure) is the registration shape.
///
/// Each blob is the raw output of
/// `bb write_vk -t evm -b main_outer_count_4.json -o . ` under the bb version ZKPassport
/// pins (5.0.0) — 1888 bytes for UltraHonk on BN254. An **empty** asset means "not
/// installed yet", and every proof for that variant is rejected; it is never a reason to
/// skip verification.
///
/// `count_4`'s asset is now the real VK: compiled `main/outer/count_4` from
/// `zkpassport/circuits` at the pinned commit (`d3a75ac`, tag `bb-v5.0.0`) with
/// `nargo 1.0.0-beta.22`, then ran `bb write_vk -t evm` under bb 5.0.0 against that
/// ACIR — see `docs/project/changelog` for the exact commands. This does not by itself
/// let any proof verify — [`ultrahonk::verify`] is still a stub — it only means
/// `lookup_vk(4)` now succeeds instead of failing on a missing asset.
const OUTER_CIRCUIT_VKS: &[(u8, &[u8])] = &[(
    4,
    include_bytes!("../assets/vk_zkpassport_outer_count_4.bin"),
)];

/// Implements `ZkProofVerifier` against ZKPassport's `main/outer/count_N` circuits.
///
/// Performs a real UltraHonk pairing check via [`ultrahonk`]. Every failure mode —
/// malformed envelope, unknown circuit variant, wrong public-input shape, and a proof that
/// simply does not verify — collapses to `false`, so there is no path by which a caller can
/// mistake "could not check" for "checked and passed".
pub struct ZkPassportUltraHonkVerifier;

impl pallet_identity_zk::ZkProofVerifier for ZkPassportUltraHonkVerifier {
    fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool {
        verify_inner(proof_bytes, public_inputs).is_some()
    }
}

/// The parsed envelope: everything the header pins down, plus the raw bb proof.
struct Envelope<'a> {
    outer_count: u8,
    variant: ProofVariant,
    proof: &'a [u8],
}

/// Parses and validates the proof envelope. `None` on any malformed input.
fn parse_envelope(proof_bytes: &[u8]) -> Option<Envelope<'_>> {
    if proof_bytes.len() < ENVELOPE_HEADER_LEN {
        return None;
    }
    if proof_bytes[0] != ENVELOPE_MAGIC || proof_bytes[1] != ENVELOPE_VERSION {
        return None;
    }

    let outer_count = proof_bytes[2];
    let variant = match proof_bytes[3] {
        0 => ProofVariant::Zk,
        1 => ProofVariant::Plain,
        _ => return None,
    };

    let proof_len = u32::from_be_bytes([
        proof_bytes[4],
        proof_bytes[5],
        proof_bytes[6],
        proof_bytes[7],
    ]) as usize;

    // Exact-length match: reject both truncation and trailing padding, so a proof can
    // never be silently extended with attacker-chosen bytes the verifier ignores.
    if proof_bytes.len() != ENVELOPE_HEADER_LEN.saturating_add(proof_len) {
        return None;
    }

    // bb's UltraHonk proof is a flat array of 32-byte words; a length that isn't a
    // multiple of 32 cannot be one, whatever the circuit.
    if proof_len == 0 || proof_len % 32 != 0 {
        return None;
    }

    Some(Envelope {
        outer_count,
        variant,
        proof: &proof_bytes[ENVELOPE_HEADER_LEN..],
    })
}

/// Looks up the verification key for an outer-circuit variant. `None` if the variant is
/// not on the allowlist, or if its VK asset has not been populated yet.
fn lookup_vk(outer_count: u8) -> Option<&'static [u8]> {
    OUTER_CIRCUIT_VKS
        .iter()
        .find(|(count, _)| *count == outer_count)
        .map(|(_, vk)| *vk)
        .filter(|vk| !vk.is_empty())
}

/// True when `value` is a canonical BN254 `Fr` element, i.e. strictly less than the
/// field modulus when read big-endian. bb emits canonical elements; anything else is a
/// malformed or maliciously-crafted public input.
fn is_canonical_fr(value: &[u8; 32]) -> bool {
    *value < BN254_FR_MODULUS_BE
}

/// Validates the public inputs against the `count_N` layout documented at the top of
/// this module. Returns the number of disclosure subproofs `D` on success.
fn check_public_inputs(outer_count: u8, public_inputs: &[[u8; 32]]) -> Option<usize> {
    // A `count_N` circuit wraps 3 base subproofs plus `N - 3` disclosure subproofs, so
    // anything at or below 3 is not a real outer circuit.
    if outer_count <= BASE_SUBPROOF_COUNT {
        return None;
    }
    let disclosure_count = (outer_count - BASE_SUBPROOF_COUNT) as usize;

    // certificate_registry_root, circuit_registry_root, current_date, service_scope,
    // service_subscope, param_commitments[D], nullifier_type, scoped_nullifier,
    // oprf_pk_hash.
    if public_inputs.len() != FIXED_PUBLIC_INPUT_COUNT + disclosure_count {
        return None;
    }

    if !public_inputs.iter().all(is_canonical_fr) {
        return None;
    }

    // `current_date` is a `u64` in the circuit, so its field element must fit in the low
    // 8 bytes. A value outside that range means the caller assembled the input array
    // wrongly (most likely against the old Rarimo ordering).
    let current_date = &public_inputs[2];
    if current_date[..24].iter().any(|byte| *byte != 0) {
        return None;
    }

    // The circuit asserts `scoped_nullifier != 0`; enforce it here too so a malformed
    // input array is rejected before it ever reaches the pairing check.
    let scoped_nullifier = &public_inputs[6 + disclosure_count];
    if scoped_nullifier.iter().all(|byte| *byte == 0) {
        return None;
    }

    Some(disclosure_count)
}

/// Full verification. `Some(())` only if the proof is genuinely valid; `None` for every
/// failure mode alike, so the caller cannot accidentally treat a structural rejection as
/// anything other than a rejection.
///
/// Order matters: the cheap structural checks run before the pairing check, so a malformed
/// envelope or a wrong-shaped public-input array costs nothing rather than a full
/// verification.
fn verify_inner(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> Option<()> {
    let envelope = parse_envelope(proof_bytes)?;
    let vk = lookup_vk(envelope.outer_count)?;
    check_public_inputs(envelope.outer_count, public_inputs)?;

    ultrahonk::verify(vk, envelope.variant, envelope.proof, public_inputs)
}

/// The UltraHonk backend: `ultrahonk-no-std`, forked and ported to bb 5.0.0.
///
/// # Which crate, and why a fork was needed
///
/// The natural candidate was `ultrahonk-no-std`
/// (<https://github.com/zkVerify/ultrahonk_verifier>) — genuinely `no_std`, already in
/// production inside zkVerify's own Substrate WASM runtime, and with a
/// `verify(vk, proof, pubs)` signature that lines up with `ZkProofVerifier::verify`. Its
/// newest tag (`v0.3.2`) targets **bb 3.0.3**, whereas ZKPassport pins **bb 5.0.0**, and
/// that gap turned out to be cryptographic rather than cosmetic: for the same `log_n = 5`
/// circuit the crate expected a 4800-byte proof where bb 5.0.0 emits 4544. The 256-byte
/// (8-word) difference is the pairing-point object, which shrank from 16 words to 8 —
/// but simply patching that constant made the length checks pass and then failed inside
/// sumcheck ("Total Sum differs from Round Target Sum"), i.e. the transcript had changed
/// too.
///
/// Nothing upstream targeted bb 5.x (no tag, branch or open PR), no equivalent crate exists
/// on crates.io, and the nearest Polkadot-adjacent alternative —
/// `zkemail/polkavm-noir-verifier` — generates PolkaVM *contracts* from
/// `bb write_solidity_verifier` output and leans on EVM pairing precompiles, so it cannot be
/// linked into a runtime as a library. Hence the fork:
/// <https://github.com/kei-nan/ultrahonk_verifier>, branch `bb-5.0.0-port`, which ports the
/// pairing-point encoding (16 → 8 words, 136/118-bit limbs), the VK precomputed-commitment
/// reordering to bb 5.0.0's `EntityId` layout, the sumcheck relation changes, and
/// `CONST_PROOF_SIZE_LOG_N` (28 → 25). It carries regression fixtures generated by real bb
/// 5.0.0 for three circuit shapes × both proving modes, including a genuinely recursive
/// circuit whose pairing-point object is not the point at infinity.
///
/// # What this wrapper does and does not check
///
/// It maps [`ProofVariant`] onto the crate's `ProofType` and hands over the bytes. The
/// variant is **not** inferred from the proof length: bb's ZK and non-ZK proofs are
/// different lengths for a given `log_n`, but each length is valid for *some* `log_n`, so
/// guessing would let a caller steer the parse. The envelope declares it; the crate then
/// checks the declared variant's expected length against the VK's `log_circuit_size` and
/// rejects a mismatch. Declaring the wrong variant therefore fails rather than silently
/// reinterpreting the transcript.
///
/// The crate also re-checks the public-input count against the VK
/// (`combined_input_size - 8 == pubs.len()`), which is an independent confirmation of the
/// layout conclusion in this module's docs.
///
/// Panic posture: the crate performs its length checks before any indexing, and its
/// internal `expect`s sit behind those checks. The VK is a compile-time `include_bytes!`
/// asset rather than caller data, so the one arithmetic edge in the crate's own
/// public-input check (`combined_input_size - 8` underflowing for a nonsense VK) is not
/// reachable from an extrinsic.
mod ultrahonk {
    use super::ProofVariant;
    use alloc::boxed::Box;
    use ultrahonk_no_std::{errors::VerifyError, ProofType};

    /// Verifies an UltraHonk proof. `Some(())` only on a genuinely valid proof.
    ///
    /// Every error the backend can report — bad VK, unparseable proof, wrong length, wrong
    /// public-input count, failed sumcheck, failed pairing — collapses to `None`. The
    /// distinction matters for debugging but not for the decision, and keeping it out of the
    /// return type means no caller can pattern-match its way into treating a soft failure as
    /// success.
    pub fn verify(
        vk: &[u8],
        variant: ProofVariant,
        proof: &[u8],
        public_inputs: &[[u8; 32]],
    ) -> Option<()> {
        let bytes: Box<[u8]> = Box::from(proof);
        let proof = match variant {
            ProofVariant::Zk => ProofType::ZK(bytes),
            ProofVariant::Plain => ProofType::Plain(bytes),
        };

        // `H = ()` selects arkworks' pure-Rust BN254 arithmetic. zkVerify's `CurveHooks`
        // trait exists so a host can swap in accelerated pairing host-functions; this
        // runtime registers none, and picking `()` keeps verification self-contained in
        // WASM rather than depending on a host function the node may not provide.
        let result: Result<(), VerifyError> =
            ultrahonk_no_std::verify::<()>(vk, &proof, public_inputs);

        result.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A well-formed envelope around `proof_len` bytes of filler.
    fn envelope(magic: u8, version: u8, outer_count: u8, variant: u8, proof_len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(ENVELOPE_HEADER_LEN + proof_len);
        out.extend_from_slice(&[magic, version, outer_count, variant]);
        out.extend_from_slice(&(proof_len as u32).to_be_bytes());
        out.extend(core::iter::repeat(0xAB).take(proof_len));
        out
    }

    fn valid_envelope() -> Vec<u8> {
        envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 4, 0, 4544)
    }

    /// A valid `count_4` public-input array: 9 elements, canonical, non-zero nullifier.
    fn valid_public_inputs() -> Vec<[u8; 32]> {
        let mut inputs = vec![[0u8; 32]; 9];
        // current_date (index 2) must fit in a u64.
        inputs[2][24..].copy_from_slice(&1_800_000_000u64.to_be_bytes());
        // scoped_nullifier sits at 6 + D, D = 1.
        inputs[7][31] = 1;
        inputs
    }

    #[test]
    fn parses_a_well_formed_envelope() {
        let bytes = valid_envelope();
        let parsed = parse_envelope(&bytes).expect("well-formed envelope should parse");
        assert_eq!(parsed.outer_count, 4);
        assert_eq!(parsed.variant, ProofVariant::Zk);
        assert_eq!(parsed.proof.len(), 4544);
    }

    #[test]
    fn rejects_bad_magic_and_version() {
        assert!(parse_envelope(&envelope(0x00, ENVELOPE_VERSION, 4, 0, 64)).is_none());
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, 0x02, 4, 0, 64)).is_none());
    }

    #[test]
    fn rejects_unknown_proof_variant() {
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 4, 2, 64)).is_none());
    }

    #[test]
    fn rejects_truncated_and_padded_envelopes() {
        let bytes = valid_envelope();
        assert!(parse_envelope(&bytes[..bytes.len() - 1]).is_none());

        let mut padded = bytes;
        padded.push(0x00);
        assert!(parse_envelope(&padded).is_none());
    }

    #[test]
    fn rejects_proof_length_that_is_not_a_multiple_of_a_field_element() {
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 4, 0, 100)).is_none());
        assert!(parse_envelope(&envelope(ENVELOPE_MAGIC, ENVELOPE_VERSION, 4, 0, 0)).is_none());
    }

    #[test]
    fn accepts_the_count_4_public_input_layout() {
        assert_eq!(check_public_inputs(4, &valid_public_inputs()), Some(1));
    }

    #[test]
    fn rejects_the_old_rarimo_five_signal_layout() {
        // The exact shape `pallet-identity` was built for. It must not be mistaken for a
        // ZKPassport input array.
        assert!(check_public_inputs(4, &[[0u8; 32]; 5]).is_none());
    }

    #[test]
    fn public_input_count_tracks_the_disclosure_subproof_count() {
        // count_N exposes N + 5 public inputs.
        for outer_count in 4u8..=13 {
            let len = outer_count as usize + 5;
            let mut inputs = vec![[0u8; 32]; len];
            inputs[6 + (outer_count - BASE_SUBPROOF_COUNT) as usize][31] = 1;
            assert_eq!(
                check_public_inputs(outer_count, &inputs),
                Some((outer_count - BASE_SUBPROOF_COUNT) as usize),
                "count_{outer_count} should accept {len} public inputs",
            );
            assert!(check_public_inputs(outer_count, &inputs[..len - 1]).is_none());
        }
    }

    #[test]
    fn rejects_an_outer_count_below_the_base_subproof_count() {
        assert!(check_public_inputs(3, &vec![[0u8; 32]; 8]).is_none());
        assert!(check_public_inputs(0, &vec![[0u8; 32]; 5]).is_none());
    }

    #[test]
    fn rejects_non_canonical_field_elements() {
        let mut inputs = valid_public_inputs();
        inputs[0] = BN254_FR_MODULUS_BE;
        assert!(check_public_inputs(4, &inputs).is_none());

        let mut inputs = valid_public_inputs();
        inputs[0] = [0xFF; 32];
        assert!(check_public_inputs(4, &inputs).is_none());
    }

    #[test]
    fn accepts_the_largest_canonical_field_element() {
        let mut max = BN254_FR_MODULUS_BE;
        max[31] -= 1;
        let mut inputs = valid_public_inputs();
        inputs[0] = max;
        assert!(check_public_inputs(4, &inputs).is_some());
    }

    #[test]
    fn rejects_a_current_date_that_does_not_fit_in_a_u64() {
        let mut inputs = valid_public_inputs();
        inputs[2][23] = 1;
        assert!(check_public_inputs(4, &inputs).is_none());
    }

    #[test]
    fn rejects_a_zero_scoped_nullifier() {
        let mut inputs = valid_public_inputs();
        inputs[7] = [0u8; 32];
        assert!(check_public_inputs(4, &inputs).is_none());
    }

    #[test]
    fn rejects_an_unsupported_outer_circuit_variant() {
        assert!(lookup_vk(5).is_none());
        assert!(lookup_vk(13).is_none());
    }

    #[test]
    fn rejects_an_uninstalled_verification_key() {
        // count_4's asset is now the real bb 5.0.0 `-t evm` VK (see
        // docs/project/changelog for provenance), so the lookup must succeed for it.
        // Any variant not on the allowlist — or one whose asset is still an empty
        // placeholder, as every other `count_N` here is — must still fail rather than
        // hand an empty blob to the backend.
        assert!(lookup_vk(4).is_some(), "count_4's real VK asset must be usable");
        assert!(lookup_vk(5).is_none(), "count_5 is not on the allowlist at all");
    }

    #[test]
    fn count_4_vk_asset_is_the_real_bb_evm_vk() {
        // `bb write_vk -t evm` on BN254 UltraHonk is always exactly 1888 bytes
        // (28*64 + 3*32 — see this module's doc comment on `OUTER_CIRCUIT_VKS`). A
        // wrong size here means the asset was regenerated with the wrong
        // `--verifier_target`, or truncated in transit.
        let vk = lookup_vk(4).expect("count_4 VK must be installed");
        assert_eq!(vk.len(), 1888, "count_4 VK must be the 1888-byte bb -t evm UltraHonk VK");
    }

    // ---------------------------------------------------------------------------
    // Real bb 5.0.0 backend tests.
    //
    // The fixtures under `assets/test/` are copied byte-for-byte from the backend
    // fork's own `tests/data/bb5/recursive_{zk,plain}` — real `bb prove` output for a
    // Noir circuit that recursively verifies another proof, which also verifies under
    // `bb verify` itself. The recursive shape is deliberate: it is the closest
    // structural analog to ZKPassport's outer circuit, and it is the only fixture whose
    // pairing-point object is a genuine non-infinity point, so it exercises the
    // 136/118-bit limb reconstruction rather than the trivial all-zero path.
    //
    // These are *not* passport proofs, and they cannot be routed through
    // `verify_inner`: that circuit exposes 2 public inputs, and no `count_N` shape has
    // `N + 5 == 2`, so `check_public_inputs` rejects them by construction — correctly.
    // What they do prove is that everything downstream of the shape check is real:
    // envelope parsing hands the backend the right bytes and the right variant, the
    // public-input conversion is right, and the pairing check genuinely accepts valid
    // proofs and genuinely rejects tampered ones. `lookup_vk` and `check_public_inputs`
    // are covered independently above, and the count_4 layout invariant that ties the
    // two halves together is asserted against the real production VK below.
    // ---------------------------------------------------------------------------

    const REC_ZK_PROOF: &[u8] = include_bytes!("../assets/test/ultrahonk_bb5_recursive_zk/proof");
    const REC_ZK_VK: &[u8] = include_bytes!("../assets/test/ultrahonk_bb5_recursive_zk/vk");
    const REC_ZK_PUBS: &[u8] = include_bytes!("../assets/test/ultrahonk_bb5_recursive_zk/pubs");

    const REC_PLAIN_PROOF: &[u8] =
        include_bytes!("../assets/test/ultrahonk_bb5_recursive_plain/proof");
    const REC_PLAIN_VK: &[u8] = include_bytes!("../assets/test/ultrahonk_bb5_recursive_plain/vk");
    const REC_PLAIN_PUBS: &[u8] =
        include_bytes!("../assets/test/ultrahonk_bb5_recursive_plain/pubs");

    /// Chunk a raw bb `public_inputs` file into 32-byte words.
    fn split_pubs(raw: &[u8]) -> Vec<[u8; 32]> {
        assert_eq!(raw.len() % 32, 0, "a pubs file is a whole number of words");
        raw.chunks_exact(32)
            .map(|chunk| <[u8; 32]>::try_from(chunk).expect("chunks_exact(32) yields 32 bytes"))
            .collect()
    }

    /// Wrap raw proof bytes in a well-formed envelope, then run the whole path from
    /// `parse_envelope` through to the backend against an explicitly supplied VK.
    ///
    /// This is `verify_inner` with `lookup_vk` and `check_public_inputs` lifted out — the
    /// two steps that are specific to ZKPassport's circuit shape and so cannot accept a
    /// foreign fixture. Everything else is the production path, unmodified.
    fn verify_fixture(
        vk: &[u8],
        variant: ProofVariant,
        proof: &[u8],
        public_inputs: &[[u8; 32]],
    ) -> Option<()> {
        let mut bytes = Vec::with_capacity(ENVELOPE_HEADER_LEN + proof.len());
        bytes.extend_from_slice(&[
            ENVELOPE_MAGIC,
            ENVELOPE_VERSION,
            4,
            match variant {
                ProofVariant::Zk => 0,
                ProofVariant::Plain => 1,
            },
        ]);
        bytes.extend_from_slice(&(proof.len() as u32).to_be_bytes());
        bytes.extend_from_slice(proof);

        let envelope = parse_envelope(&bytes)?;
        assert_eq!(envelope.variant, variant);
        ultrahonk::verify(vk, envelope.variant, envelope.proof, public_inputs)
    }

    #[test]
    fn verifies_a_real_bb5_zk_proof() {
        assert_eq!(
            verify_fixture(
                REC_ZK_VK,
                ProofVariant::Zk,
                REC_ZK_PROOF,
                &split_pubs(REC_ZK_PUBS)
            ),
            Some(()),
            "a genuine bb 5.0.0 `-t evm` proof of a recursive circuit must verify",
        );
    }

    #[test]
    fn verifies_a_real_bb5_plain_proof() {
        assert_eq!(
            verify_fixture(
                REC_PLAIN_VK,
                ProofVariant::Plain,
                REC_PLAIN_PROOF,
                &split_pubs(REC_PLAIN_PUBS)
            ),
            Some(()),
            "a genuine bb 5.0.0 `-t evm-no-zk` proof of a recursive circuit must verify",
        );
    }

    #[test]
    fn rejects_a_proof_whose_declared_variant_is_wrong() {
        // The ZK and Plain flavors are different lengths and different transcripts. A
        // caller that mislabels one must be rejected, not silently reinterpreted.
        assert!(verify_fixture(
            REC_ZK_VK,
            ProofVariant::Plain,
            REC_ZK_PROOF,
            &split_pubs(REC_ZK_PUBS)
        )
        .is_none());
        assert!(verify_fixture(
            REC_PLAIN_VK,
            ProofVariant::Zk,
            REC_PLAIN_PROOF,
            &split_pubs(REC_PLAIN_PUBS)
        )
        .is_none());
    }

    #[test]
    fn rejects_single_bit_mutations_anywhere_in_a_real_proof() {
        // Sample offsets right across the proof so that a flip landing in any region —
        // pairing points, commitments, sumcheck univariates, evaluations, the opening
        // proof — is covered. The first 256 bytes are specifically the pairing-point
        // object, so this also confirms those words are genuinely being checked and not
        // just parsed and discarded.
        let pubs = split_pubs(REC_ZK_PUBS);
        let step = REC_ZK_PROOF.len() / 40;
        let mut checked = 0;
        for offset in (0..REC_ZK_PROOF.len()).step_by(step) {
            let mut mutated = REC_ZK_PROOF.to_vec();
            mutated[offset] ^= 0x01;
            assert!(
                verify_fixture(REC_ZK_VK, ProofVariant::Zk, &mutated, &pubs).is_none(),
                "flipping bit 0 of proof byte {offset} must be rejected",
            );
            checked += 1;
        }
        assert!(checked >= 20, "expected a meaningful spread of offsets");
    }

    #[test]
    fn rejects_mutated_public_inputs_for_a_real_proof() {
        for index in 0..REC_ZK_PUBS.len() {
            let mut raw = REC_ZK_PUBS.to_vec();
            raw[index] ^= 0x01;
            assert!(
                verify_fixture(REC_ZK_VK, ProofVariant::Zk, REC_ZK_PROOF, &split_pubs(&raw))
                    .is_none(),
                "flipping bit 0 of public-input byte {index} must be rejected",
            );
        }
    }

    #[test]
    fn rejects_a_real_proof_against_the_wrong_verification_key() {
        // The two fixtures are the same circuit proved in different modes, so their VKs
        // differ. Swapping in the count_4 VK is the sharper case: a real, well-formed,
        // correctly-sized VK for a different circuit entirely.
        let pubs = split_pubs(REC_ZK_PUBS);
        let count_4_vk = lookup_vk(4).expect("count_4 VK must be installed");
        assert!(verify_fixture(count_4_vk, ProofVariant::Zk, REC_ZK_PROOF, &pubs).is_none());
    }

    #[test]
    fn rejects_a_truncated_verification_key() {
        let pubs = split_pubs(REC_ZK_PUBS);
        assert!(verify_fixture(
            &REC_ZK_VK[..REC_ZK_VK.len() - 32],
            ProofVariant::Zk,
            REC_ZK_PROOF,
            &pubs
        )
        .is_none());
        assert!(verify_fixture(&[], ProofVariant::Zk, REC_ZK_PROOF, &pubs).is_none());
    }

    /// The load-bearing claim of this module's public-input docs, asserted against the
    /// real production VK asset with the backend's own parser.
    ///
    /// `combined_input_size` is bb's count of *all* words in the circuit's public-input
    /// region, semantic inputs plus the 8-word pairing-point object. If it equals
    /// `(4 + 5) + 8`, then `main/outer/count_4` really does expose exactly the nine
    /// semantic inputs this module documents, and the pairing object really is accounted
    /// for separately (bb carries it at the front of the proof, not in the public-input
    /// file). If ZKPassport ever changes that circuit's public interface, this fails here
    /// rather than in `check_public_inputs` silently reading the wrong field index.
    #[test]
    fn count_4_vk_matches_the_documented_public_input_layout() {
        use ultrahonk_no_std::key::VerificationKey;

        /// bb 5.0.0's pairing-point object: 2 G1 points, 2 limbs per coordinate.
        const PAIRING_POINTS_SIZE: u64 = 8;

        let vk = VerificationKey::<()>::try_from(lookup_vk(4).expect("count_4 VK installed"))
            .expect("the count_4 asset must parse as a bb 5.0.0 UltraHonk VK");

        let semantic_inputs = 4 + 5;
        assert_eq!(
            vk.combined_input_size,
            semantic_inputs + PAIRING_POINTS_SIZE,
            "count_4 must expose {semantic_inputs} semantic public inputs plus \
             {PAIRING_POINTS_SIZE} pairing-point words",
        );

        // Sanity: the outer circuit is large, and log_n must be within what the backend
        // (and bb 5.0.0's CONST_PROOF_SIZE_LOG_N of 25) supports.
        assert_eq!(vk.log_circuit_size, 22);
    }

    /// End-to-end through the *whole* production path, including `lookup_vk(4)` and
    /// `check_public_inputs`, with the real ZKPassport VK.
    ///
    /// There is no genuine `count_4` proof to test the accepting side with — producing one
    /// needs real passport data and satisfying subproofs — so this pins the rejecting side:
    /// a perfectly well-formed envelope and a perfectly well-shaped 9-element public-input
    /// array still fail, because the proof bytes are not a valid proof. The failure now
    /// comes from the pairing check rather than from a stubbed-out backend.
    #[test]
    fn rejects_a_well_formed_count_4_envelope_carrying_a_bogus_proof() {
        use pallet_identity_zk::ZkProofVerifier;
        assert!(!ZkPassportUltraHonkVerifier::verify(
            &valid_envelope(),
            &valid_public_inputs()
        ));
    }

    /// A real proof, correctly labelled, but presented as a `count_4` passport proof.
    /// `check_public_inputs` must reject it on shape before the backend ever sees it.
    #[test]
    fn rejects_a_foreign_circuits_proof_presented_as_a_passport_proof() {
        use pallet_identity_zk::ZkProofVerifier;

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[ENVELOPE_MAGIC, ENVELOPE_VERSION, 4, 0]);
        bytes.extend_from_slice(&(REC_ZK_PROOF.len() as u32).to_be_bytes());
        bytes.extend_from_slice(REC_ZK_PROOF);

        // Its own (2-element) public inputs: wrong shape for count_4.
        assert!(!ZkPassportUltraHonkVerifier::verify(
            &bytes,
            &split_pubs(REC_ZK_PUBS)
        ));

        // And padded out to count_4's 9 elements, so it passes the shape check and is
        // rejected by the pairing check instead.
        assert!(!ZkPassportUltraHonkVerifier::verify(
            &bytes,
            &valid_public_inputs()
        ));
    }
}
