# ZK verifier status

`runtime/src/verifier.rs` implements `ZkPassportUltraHonkVerifier`, targeting ZKPassport's
Noir/UltraHonk `main/outer/count_N` circuits (replaces the former `RarimoGroth16Verifier` —
see `docs/project/changelog/065-068.md` entry 65 for why Rarimo was dropped). The module's
own doc comment is the authoritative reference for the proof envelope and public-input
layout — read it before touching this file, don't duplicate it here.

**Current status: fail-closed, no pairing check.** Every structural check (envelope parsing,
VK lookup, public-input canonicity) is real and enforced, but the actual UltraHonk pairing
verification is unimplemented — no `no_std`-compatible Rust crate exists yet that can verify
proofs shaped like ZKPassport's (bb 5.0.0 pairing-point/transcript format; the only candidate,
`ultrahonk-no-std`, targets bb 3.0.3 and is a real cryptographic mismatch, not a version-string
one — see the `mod ultrahonk` doc comment in `verifier.rs` for the full experimental writeup,
and `docs/project/changelog/069-070.md` entry 70). The runtime compiles and is safe (rejects
every proof) but cannot register any citizen for real until this lands.

`runtime/assets/vk_zkpassport_outer_count_4.bin` is now populated with the real VK: the
pinned `zkpassport/circuits` commit (`d3a75ac`, tag `bb-v5.0.0`) has `main/outer/count_4`
compiled with `nargo 1.0.0-beta.22`, and `bb write_vk -t evm` (bb 5.0.0) run against the
resulting ACIR — 1888 bytes, matching the expected UltraHonk-on-BN254 VK size exactly.
This does **not** unblock verification by itself: `lookup_vk(4)` now succeeds instead of
failing on a missing asset, but `verifier.rs`'s `ultrahonk::verify` is still a stub, so
every proof is still rejected — just for the real reason (no pairing backend) rather than
a missing-asset placeholder. See `docs/project/changelog/` for the exact commands run.

Mobile: `mobile/src/chain/proofEncoding.ts`'s `encodeUltraHonkProof` builds the envelope
`verifier.rs` expects; the two files are one contract, see `verifier.rs`'s module docs for
the exact byte layout.

**Removed, no longer relevant**: the Rarimo-era `ark-groth16`/`ark-serialize`/`ark-bn254`/
`ark-ff` dependencies, `scripts/convert_vk.py`, and `runtime/assets/vk_sha1.bin`/
`vk_sha256.bin` were deleted — none of them apply to an UltraHonk verifier.
