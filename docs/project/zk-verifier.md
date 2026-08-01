# ZK verifier status

`runtime/src/verifier.rs` implements `ZkPassportUltraHonkVerifier`, targeting ZKPassport's
Noir/UltraHonk `main/outer/count_N` circuits (replaces the former `RarimoGroth16Verifier` —
see `docs/project/changelog/065-068.md` entry 65 for why Rarimo was dropped). The module's
own doc comment is the authoritative reference for the proof envelope and public-input
layout — read it before touching this file, don't duplicate it here.

**Current status: complete and verifying.** Every structural check (envelope parsing, VK
lookup, public-input canonicity/shape) is real and enforced, and the UltraHonk pairing check
is now performed for real by `ultrahonk-no-std`
(<https://github.com/kei-nan/ultrahonk_verifier>, branch `bb-5.0.0-port`) — a fork of
zkVerify's verifier ported from bb 3.0.3 to bb 5.0.0, the version ZKPassport pins. The gap
was cryptographic, not a version string: the pairing-point object shrank from 16 words to 8
*and* the sumcheck transcript changed. See `docs/project/changelog/072.md` and the
`mod ultrahonk` doc comment in `verifier.rs`.

`runtime/assets/vk_zkpassport_outer_count_4.bin` holds the real VK: the pinned
`zkpassport/circuits` commit (`d3a75ac`, tag `bb-v5.0.0`) has `main/outer/count_4` compiled
with `nargo 1.0.0-beta.22`, and `bb write_vk -t evm` (bb 5.0.0) run against the resulting
ACIR — 1888 bytes. `count_4` is the only variant on the allowlist; other `count_N` shapes are
deliberately unlisted and rejected.

**Where the pairing-point object lives** (the one integration question that had to be settled
before this could be correct): bb 5.0.0 carries the 8-word pairing-point/aggregation object as
the first 8 words of the **proof** file, never in the `public_inputs` file — for every circuit,
recursive or not. So ZKPassport's documented `N + 5` public-input layout is exactly what is on
the wire, unshifted, and both `check_public_inputs` and `pallet-identity`'s index reads are
correct as written. Established from real bytes: the real `count_4` VK's
`combined_input_size` is `17 = 9 + 8`, and all six bb 5.0.0 fixtures satisfy
`combined_input_size == pubs_words + 8`. `verifier.rs`'s
`count_4_vk_matches_the_documented_public_input_layout` test pins this in CI.

**Still unproven end-to-end**: no genuine ZKPassport `count_4` proof has been verified by this
code, because generating one needs real passport NFC data and four satisfying subproofs. Every
layer is individually tested (including real bb 5.0.0 proofs verifying and mutations being
rejected), but the first real passport proof is the outstanding integration test. The fork's
bb 5.0.0 port is also our own work and unaudited.

Mobile: `mobile/src/chain/proofEncoding.ts`'s `encodeUltraHonkProof` builds the envelope
`verifier.rs` expects; the two files are one contract, see `verifier.rs`'s module docs for
the exact byte layout.

**Removed, no longer relevant**: the Rarimo-era `ark-groth16`/`ark-serialize`/`ark-bn254`/
`ark-ff` dependencies, `scripts/convert_vk.py`, and `runtime/assets/vk_sha1.bin`/
`vk_sha256.bin` were deleted — none of them apply to an UltraHonk verifier.
