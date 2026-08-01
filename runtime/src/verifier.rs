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
//! # !! CURRENT STATUS: FAIL-CLOSED, NO PAIRING CHECK !!
//!
//! Every structural check below is real and enforced, but the UltraHonk pairing check
//! itself is **not implemented**, because no Rust verifier exists that can verify the
//! proofs ZKPassport's circuits actually produce. This was established experimentally,
//! not assumed — see [`ultrahonk`] for the exact evidence. [`ZkPassportUltraHonkVerifier`]
//! therefore rejects *every* proof. A non-dev-mode build compiles and is safe (it cannot
//! be tricked into accepting a forged proof), but it also cannot register any citizen.
//! This is the same posture the Rarimo-era code shipped in (its VK assets were never
//! populated, so `verify_inner` returned `false` for every proof too) — deliberately
//! preserved rather than papered over.
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
//! `pallet_identity_zk::register_citizen` reads this layout correctly: index `0`
//! (`certificate_registry_root`) for the allowlist check, index `len - 2` (`6 + D`,
//! `scoped_nullifier`) for the nullifier, and its `public_inputs` bound is
//! `ConstU32<18>` (the `count_13` ceiling, `13 + 5`). This was previously mismatched
//! against Rarimo's old indices; fixed once this module pinned the real layout down.

#![cfg(not(feature = "dev-mode"))]

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
/// See the module docs: this currently rejects every proof because no usable UltraHonk
/// verifier backend exists. It is wired in anyway so that the non-dev-mode runtime binds
/// a real, fail-closed verifier rather than a passthrough, and so the envelope /
/// public-input contract with the mobile app is pinned down and testable now.
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

/// Full verification. `Some(())` only if the proof is genuinely valid; `None` for any
/// malformed input *and* for "no verifier backend available", so the caller cannot
/// accidentally treat the second case as success.
fn verify_inner(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> Option<()> {
    let envelope = parse_envelope(proof_bytes)?;
    let vk = lookup_vk(envelope.outer_count)?;
    check_public_inputs(envelope.outer_count, public_inputs)?;

    ultrahonk::verify(vk, envelope.variant, envelope.proof, public_inputs)
}

/// The pluggable UltraHonk backend.
///
/// # Why this is empty, and what was actually tested
///
/// The obvious candidate is `ultrahonk-no-std`
/// (<https://github.com/zkVerify/ultrahonk_verifier>) — genuinely `no_std`
/// (`#![cfg_attr(not(feature = "std"), no_std)]`), genuinely in production inside
/// zkVerify's own Substrate WASM runtime, and with a `verify(vk, proof, pubs)` signature
/// that lines up exactly with `ZkProofVerifier::verify`. It was cloned at its newest tag
/// (`v0.3.2`) and tested directly rather than judged on its README:
///
/// * It verifies its own bundled test vector fine, so the crate itself works.
/// * ZKPassport pins **bb 5.0.0** (`.github/workflows/test.yml`, `@aztec/bb.js` in
///   `package.json`). `ultrahonk-no-std` v0.3.2 targets **bb 3.0.3**
///   (`scripts/generate_benchmark_projects.sh`).
/// * bb 5.0.0 was installed via `bbup` and used to prove a trivial Noir circuit with
///   `bb prove -t evm --write_vk`. The verification key matched byte-for-byte in size
///   (1888 bytes, exactly the crate's `VK_SIZE`), but the **proof did not**: 4544 bytes
///   against the 4800 the crate computes for the same `log_n = 5`.
/// * The 256-byte (8-word) gap is the pairing-point object: bb 3.0.3 carries 16 words of
///   it, bb 5.0.0 carries 8. Both the VK header (`combined_input_size` = 17 = 1 + 16 for
///   the old vector, 10 = 2 + 8 for the new proof) and the proof length agree on that.
/// * Patching `PAIRING_POINTS_SIZE` from 16 to 8 makes every length check pass — and
///   then verification fails inside sumcheck ("Total Sum differs from Round Target Sum"),
///   i.e. the transcript changed too. It is a real cryptographic divergence, not a
///   constant that can be bumped.
///
/// No newer tag, branch, or open PR upstream targets bb 4.x/5.x, no equivalent crate
/// exists on crates.io (`ultrahonk-no-std` is git-only and unpublished), and the nearest
/// Polkadot-adjacent alternative — `zkemail/polkavm-noir-verifier` — generates PolkaVM
/// *contracts* from `bb write_solidity_verifier` output and leans on EVM pairing
/// precompiles, so it cannot be linked into a runtime as a library.
///
/// # What unblocks this
///
/// Any one of: an upstream `ultrahonk-no-std` release targeting bb 5.x; a fork of it that
/// ports the bb 5.0.0 pairing-point/transcript changes (real cryptography work, and it
/// needs a ZKPassport-generated outer proof to test against); or ZKPassport publishing a
/// Rust verifier of their own (as of `d3a75ac` their `src/rust/` holds only
/// `masterlist-interpreter` and `redc-param-gen`, and their only shipped on-chain
/// verifiers are Solidity).
///
/// When one lands, implement [`verify`] and delete this comment. Nothing else in this
/// file needs to change — the envelope, the VK lookup and the public-input checks are all
/// already the right shape for it.
mod ultrahonk {
    use super::ProofVariant;

    /// Verifies an UltraHonk proof. Returns `Some(())` only on a genuinely valid proof.
    ///
    /// Currently always `None` — see the module docs. Deliberately fail-closed: there is
    /// no build configuration, feature flag or asset that turns this into `Some(())`
    /// without someone writing a real implementation here first.
    pub fn verify(
        _vk: &[u8],
        _variant: ProofVariant,
        _proof: &[u8],
        _public_inputs: &[[u8; 32]],
    ) -> Option<()> {
        None
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

    /// The whole point of the current state: nothing verifies. If this ever passes,
    /// either a real backend landed (update this test) or something is badly wrong.
    #[test]
    fn rejects_every_proof_while_no_backend_exists() {
        use pallet_identity_zk::ZkProofVerifier;
        assert!(!ZkPassportUltraHonkVerifier::verify(
            &valid_envelope(),
            &valid_public_inputs()
        ));
    }
}
