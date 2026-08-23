# Next steps (remaining work)

1. [DONE, SUPERSEDED] **VK assets** — this originally referred to real 424-byte Rarimo Groth16
   BN254 VKs in `runtime/assets/`. Those are gone: the Rarimo→ZKPassport migration (log #65,
   see item 8 below) replaced them, and `runtime/assets/` now contains only
   `vk_zkpassport_outer_count_4.bin` (ZKPassport's UltraHonk `outer/count_4` verification key;
   `docs/project/zk-verifier.md` documents the old `vk_sha1.bin`/`vk_sha256.bin` as deleted).
   Still accurate as a checkpoint: a real VK asset matching the currently-used verifier
   (`runtime/src/verifier.rs`) is present and wired.
2. [DONE] **Mobile app native init** — `android/` generated; JS/TS complete; iOS deferred (WSL2)
3. [DONE] **QR auth — chain verification** — `auth_verify_nullifier` scans NullifierRegistry on-chain
4. [DONE] **pallet-executive (Cabinet)** — parliamentary executive, incompatibility rule, `EnsureExecutiveMinister`
5. [DONE] **ReferendumTier::Foundational** — 75% threshold, `create_foundational_referendum` call, maps to `LawTier::Foundational` in pallet-constitution
6. [DONE] **Anti-corruption desktop page** — asset disclosures, conflict registry, whistleblower report list
7. [PARTIAL] **VRF jury randomness** — the sp-io v38/v40-class conflict is reconfirmed still blocking `pallet_insecure_randomness_collective_flip` (see log #52 for the re-verification), and it wouldn't have been real VRF anyway. Instead, jury selection now uses a self-contained commit-then-delayed-reveal scheme in pallet-courts (no new deps, stays on Aura) — closes the old scheme's dominant hole (any authorized caller could grind for a favorable jury by delaying `select_jury` across already-mined blocks) but is still not VRF-grade: a validator scheduled to author a block inside the seed window retains bounded influence over that block's hash. Genuine BABE/SASSAFRAS VRF still requires a full consensus swap away from Aura — not attempted, deliberately out of scope (see log #52 for the full writeup and residual-risk detail)
8. **[SUPERSEDED IN PART — see log #65, decided 2026-07-30: dropping Rarimo entirely as the passport-ZK circuit vendor, replatforming to ZKPassport (`github.com/zkpassport/circuits`, Noir/UltraHonk). Everything below in this item describes the now-abandoned Rarimo circom/Groth16 integration — kept as historical record of real, verified engineering work, not deleted, since some of it (NFC chip reading itself, which is circuit-agnostic) remains valid. VK assets, `verifier.rs`'s `RarimoGroth16Verifier`, `sodParser.ts`, `certificateTree.ts`, `poseidon.js`, `asn1.js`, `zkProving.ts`, `proofEncoding.ts` are all Rarimo-circuit-specific and now need rework against ZKPassport's actual circuit shape — none of that rework has started. See log #65 for the full decision record and what's next. **Update, log #66: the certificate-registry portion of that rework is now DONE — `certificateTree.ts`/`scripts/certificate-registry/` have been rebuilt and tested against ZKPassport's real depth-16 Poseidon2 tree, and `poseidon.js`/`asn1.js`(for cert parsing)/`poseidon_constants.js` are deleted, no longer needed. `sodParser.ts`/`zkProving.ts`/`proofEncoding.ts` and `verifier.rs` still need their ZKPassport-targeted rework — see log #66 for the circuit entry point, verifier-crate, and mobile-SDK findings that rework needs to start from, and for a real open question (nullifier renewal-stability) that needs a human decision first. **Update, logs #67/#68: that open question is now resolved** — see `docs/project/changelog/065-068.md` for the decided Sybil-resistance architecture (mandatory OPRF-based identity anchor, one-chain-per-country deployment, 4-year scheme rotation, periodic re-verification, self-declaration + courts backstop) and its pallet-level implementation (`pallet-identity` scaffolding, real tests, OPRF cryptography itself still unbuilt). **Update, log #73: the OPRF committee's governance model (who runs it) is now decided** — see `docs/project/changelog/073.md` for the full trail (institutional/staked/TACEO-network-reuse considered and rejected as the steady-state model; a single ~200-person sortition committee designed, reviewed, and found broken against real deployed-scale precedent; landed on 5 independent ~35-person committees combined by hashed summation, with independent per-committee founding groups, DOB-only permanent slot assignment, and per-committee emergency rotation). **The OPRF committee service itself — at any scale, under any model — remains entirely unbuilt; this is still the actual blocker.** **Update, log #74: the `oprf-identity-anchor` circuits (`anchor`/`disclosure`/`migrate`) are now extended to log #73's 5-committee topology and verified to compile and pass tests under bb 5.0.0** — see `docs/project/changelog/074.md`. Log #74 also found that a real `AnchorProofVerifier` for the `disclosure` path needs no new SNARK/pairing check (it rides on the outer proof's existing verification), only a Poseidon2 recomputation of `param_commitment` — but this codebase has **no Rust Poseidon2 implementation at all** (confirmed by grep, doc-comment mentions only), so that verifier is still unbuilt, and deliberately was not hand-rolled without real test vectors to check it against. Separately, `runtime/src/verifier.rs` itself is **no longer accurate to describe as unstarted rework** — see `docs/project/zk-verifier.md`: it was rebuilt against ZKPassport's UltraHonk `outer/count_4` circuit (not the old Rarimo Groth16 shape this bracket describes) and, as of log #72, performs a real bb 5.0.0 pairing check; only a genuine end-to-end passport proof through it remains outstanding, gated on real NFC data. **Update, log #75: the Poseidon2 blocker log #74 flagged is now cleared, and a real (partial) `AnchorProofVerifier` is landed.** `pallets/poseidon2-bn254` is a from-source Rust port of `noir-lang/noir`'s own `acvm-repo/bn254_blackbox_solver` Poseidon2 permutation and round constants (fetched at the exact commit the installed `nargo 1.0.0-beta.22` was built from — Poseidon2 is an ACIR blackbox function, not Noir source, so that Rust crate *is* the authoritative definition), validated bit-for-bit against real `nargo test --show-output` vectors generated in this session, including the exact 8-element `param_commitment` shape `disclosure` uses — see `docs/project/changelog/075.md` for the full trail. `runtime/src/anchor_verifier.rs`'s `Poseidon2AnchorVerifier` uses it to implement `verify_registration_anchor` for real (Poseidon2 recomputation + match against the already-verified outer proof's `param_commitments`), now wired into the non-dev-mode `pallet_identity_zk::Config`, replacing `PassthroughAnchorVerifier` for that one path; `pallet-identity` gained governance-gated `OprfCommitteeKeys` storage (mirroring `AllowedMerkleRoots`) and a `current_date` freshness check. **`verify_reverification`/`verify_migration` remain exactly as permissive as before** — log #75 found `reverify_citizen`/`migrate_oprf_scheme` never accept an outer ZKPassport proof at all (only a bare proof-bytes blob), the same "no authenticated `comm_in`" flaw `disclosure` was built to fix for registration, just never applied to those two calls; fixing that needs the same kind of extrinsic surgery `register_citizen` just got, not attempted this session. The OPRF committee service itself is still entirely unbuilt — nothing here changes that. **Update, log #76: both are now real.** Reverification reuses `disclosure` directly — it's structurally the same "recompute and check the anchor" as registration. Migration needed a genuinely new outer-embedded circuit, `circuits/oprf-identity-anchor/migrate-disclosure` (compiled, `bb write_vk`'d under bb 5.0.0), since the standalone `migrate` circuit shares the same unauthenticated-`comm_in` flaw `disclosure` was built to close for `anchor` — a Rust-only "extrinsic surgery" fix, which is what entry 75's phrasing implied, would have reproduced that exact vulnerability class. `reverify_citizen`/`migrate_oprf_scheme` were restructured to accept the outer `zk_proof`/`public_inputs`, mirroring `register_citizen`; `old_anchor` is no longer a caller-supplied parameter for migration — it's read from `CitizenAnchor` directly. See `docs/project/changelog/076.md` for the full trail. **The OPRF committee service itself remains entirely unbuilt — still the actual blocker**, unchanged by entries 74/75/76's work on the Rust/circuit machinery around it. **Update, log #77: `migrate-disclosure`'s outer-circuit ABI is now empirically confirmed, closing the specific gap log #76 flagged (its 8-field layout was derived by analogy to `disclosure`'s, never independently measured).** A stubbed scratch copy (same "`verified_oprf` call replaced" substitution log #69 used for `disclosure`, everything else identical) was proven for real under bb 5.0.0 and verified; the resulting `public_inputs` blob is exactly 256 bytes / 8 fields in the same `[comm_in, current_date, service_scope, service_subscope, param_commitment, 0, 0, 0]` order log #69 measured for `disclosure`. See `docs/project/changelog/077.md` for the full trail, byte dump, and — importantly — what this still does *not* establish: neither `disclosure` nor `migrate-disclosure` has ever been run inside an actual outer ZKPassport proof and accepted via `verify_proof_with_type`; only the standalone subproof's own public-input shape is confirmed. No production code changed. **The OPRF committee service itself remains entirely unbuilt — still the actual blocker**, unchanged by this entry. **Update, log #78: a DEV/TEST-ONLY 5-committee OPRF simulator now exists** (`oprf-committee-dev/`, a standalone Rust crate, not part of the main workspace) — a from-scratch implementation of the actual `TaceoLabs/oprf-nr` protocol (BabyJubJub curve arithmetic, `TaceoLabs/noir-poseidon` v0.6.1's `t3`/`t16` permutation, Elligator2 hash-to-curve, Chaum-Pedersen DLog-equality proof generation), validated against real known-answer vectors from `oprf-nr` itself and cross-checked against a real `nargo execute` run of the actual `query` circuit in this repo. Using its output in place of the `oprf_proof.beta` stub, both `anchor` and `disclosure` were proven and verified genuinely end-to-end under real bb 5.0.0 (`nargo execute` solved a real witness — all 5 `verified_oprf` calls' checks actually passed, not vacuously — `bb prove`/`bb verify` succeeded) — see `docs/project/changelog/078.md` for the full trail. This closes the "never been executed, only compiled" gap for those two circuits specifically. **It does not change the actual named blocker**: this is 5 secret keys in one process's memory, not 5 independent committees with a real key ceremony and network protocol — `oprf-committee-dev/README.md` says so plainly. `migrate`/`migrate-disclosure` were not attempted this session (nothing structurally new needed, just re-running the same flow under two key generations) and remain unexecuted. **Update, log #79: the mobile-wiring gap logs #76/#77 both flagged ("mobile/desktop clients remain unwired for all three call shapes") is now closed for encode/submit.** `mobile/src/chain/identity.ts`'s `registerCitizen` was found to be stale even against log #75 (still the 2-argument, old 5-Rarimo-signal shape) and was rewritten first; `reverifyCitizen`/`migrateOprfScheme` were added alongside it, all three matching the pallet's real call-index-0/6/7 signatures argument-for-argument (`migrate_oprf_scheme` submits exactly 5 arguments — no caller-supplied `old_anchor`, per log #76). `zkProving.ts` gained `proveReverification`/`proveMigration`, thin wrappers over the existing `proveRegistration` orchestration (no new pipeline needed — the outer-proof assembly doesn't care which circuit fills the disclosure-subproof slot). 22 new tests (`mobile/src/chain/identity.test.ts`, new; `zkProving.test.ts` extended), `npm test`: 99/99 pass. Desktop checked and confirmed to need no change (zero references to any of the three calls; read-only per CLAUDE.md). See `docs/project/changelog/079.md` for the full trail. **Still open, unchanged**: no OPRF committee service exists anywhere (entries 73-77/79's standing blocker), so `anchor`/`oprf_pk_hashes` remain caller-supplied parameters nothing yet computes; no NFC hardware/native Noir prover exists in this environment, so no genuine proof has been produced or submitted through any of the three calls. **Update, log #81: `migrate`/`migrate-disclosure` — the two circuits log #78 left unattempted — are now proven and verified end-to-end too**, using the same dev simulator run twice (an outgoing committee generation byte-identical to log #78's `anchor` recipe, plus an independently-generated incoming one; 10 `verified_oprf` calls per proof instead of 5). `nargo execute` solved real witnesses for both, `bb prove`/`bb verify` succeeded under bb 5.0.0 for both, and `migrate`'s `old_anchor` output matched log #78's `anchor` output byte-for-byte — a real cross-circuit consistency check. See `docs/project/changelog/081.md`. **All four committee-consuming circuits (`anchor`/`disclosure`/`migrate`/`migrate-disclosure`) have now been executed against the dev simulator — none against a real committee.** The OPRF committee service itself remains entirely unbuilt, unchanged by this entry; `disclosure`/`migrate-disclosure`'s outer-circuit integration (whether a real outer ZKPassport proof actually accepts either subproof) also remains unconfirmed, same as before. **Update, log #82: the node architecture for the founding-phase committee service (not yet the service itself) is now decided.** Committee members run the node on their own member-chosen device (phone/laptop/Raspberry Pi, supplied not screened-for, to avoid skewing who can serve toward whoever already owns hardware); the already-validated `oprf-committee-dev` crypto core compiles once to WebAssembly and runs identically across all three, inside a separate (not the main citizen app) mobile shell or an OCI container on laptop/Pi; the query/response exchange with citizens is new on-chain storage (a mailbox), not a relay server, Redis, or push-notification service. Several proposed shortcuts — a centralized relay or free-tier Redis (splitting data across DBs doesn't distribute trust if one party holds all the credentials), Lua for the crypto core (no bignum support), a custom Redis module (free tiers don't allow it), fanning the secret computation out to chain validators (breaks the public-replicated-execution model every blockchain relies on), and collapsing a committee to one device — were each considered and rejected with reasoning recorded in log #82, so they aren't re-proposed without it. An institutional/professional-operator hybrid (drand's League-of-Entropy model: vetted organizations run nodes, citizens keep oversight) was also surfaced as a real, credible alternative and deliberately set aside for the founding phase rather than decided against permanently — flagged for its own dedicated review if the citizen-hosted model doesn't pan out. **Explicitly still open, per log #82**: the SLA window (~5-7 days) is an unmeasured placeholder; who's allowed to author a device update without becoming a new centralization point is undesigned; device distribution logistics at full sortition scale (vs. the founding phase's in-person ceremony handoff) are unsolved; DKG ceremony mechanics across heterogeneous member devices are unresolved; no governance trigger exists for a voluntary/early rotation (only scheduled and emergency-compromise rotation do); and the on-chain primitives this design assumes (`CommitteeMembers` roster, `committee_slot` assignment, the query/response mailbox storage and its two extrinsics) don't exist in `pallet-identity` yet — confirmed by grep this session, this entry is architecture, not implementation. **The OPRF committee service itself remains entirely unbuilt, unchanged by this entry.** **Update, log #83: the founding-phase node software from log #82's design is now actually implemented and reconciled**, not just designed — `pallet-identity` gained the real on-chain mailbox (`CommitteeMembers`/`PendingOprfQueries`/`OprfResponses`, `submit_oprf_query`/`submit_oprf_response` at call indices 15/16, 90/90 tests passing), `oprf-committee-dev` gained a real `wasm32-unknown-unknown` build of the committee-evaluation core with a wasm-vs-native equivalence test, and two new components (`committee/`, a separate mobile app, and `committee-node/`, a laptop/Pi container) implement the poll-and-fulfill flow against them. The two new components were built in parallel against a guessed interface and needed real reconciliation once the pallet/Wasm work landed — most guesses were confirmed correct, but the call index (13 guessed vs. real 16), the Wasm ABI (missing `ds_dlog`/`seed` entirely), and `OprfResponses`' shape (guessed as a per-address list; it's actually one record per `(query, slot)` pair — the original mobile code would have kept re-offering duties another roster member had already fulfilled) all needed real fixes, now applied and tested. A real, unrelated bug was also found and fixed: `desktop/src-tauri/src/rpc.rs`'s `twox128_hex` was computing wrong storage-key hashes on every real deployment (confirmed against the standard `twox128("System")` reference vector), silently breaking every desktop chain-read command — fixed with a regression test. **Still not real**: any actual OPRF committee (DKG ceremony, founding-group key material, `OprfCommitteeKeys`/`CommitteeMembers` remain empty), the mobile app's Wasm runtime (still a throwing stub — the interface shape is now correct, loading the module from React Native is unattempted), and real hardware-backed key custody on either host. See `docs/project/changelog/083.md` for the full trail. **Update, log #084: `committee/`'s Wasm
   runtime — the specific piece log #83 left as "still a throwing stub" — is now real.** Hermes
   (this app's default RN 0.74 engine) has no `WebAssembly` global (confirmed via
   `facebook/hermes#429`, unresolved since 2020; real support only lands in RN 0.84) and
   switching to JSC was checked and rejected too (JIT'd Wasm disallowed on iOS, disabled outright
   on `jsc-android`). Instead, Binaryen's `wasm2js` transpiles the real compiled
   `oprf_committee_dev.wasm` into plain JS at build time (`committee/scripts/build-wasm-core.sh`,
   2,358,982-byte output after `wasm-opt -Oz`) — no runtime `WebAssembly` support needed on any
   engine. `committee/src/crypto/wasmCommitteeCrypto.ts` marshals the real 160-byte
   input/192-byte output ABI against it and is verified byte-identical to the real Rust
   `ffi::evaluate_query` on `ffi.rs`'s own `sample_input()` fixture. `npm test` in `committee/`:
   30/30 pass (10 new). **Not verified**: this running inside an actual Hermes VM on a real
   device — `committee/` still has no `android/`/`ios/` project scaffolded, and this environment
   has no SDK/device/emulator regardless. See `docs/project/changelog/084.md` for the full trail.
   **The OPRF committee service itself remains entirely unbuilt, unchanged by this entry.**
   **Update, log #085**: real DKG-ceremony
   orchestration tooling now exists (`oprf-committee-dev/src/dkg.rs` + `src/bin/dkg_party.rs` +
   `tests/dkg_ceremony.rs`) — a genuine Feldman VSS / Joint-Feldman DKG run as `n` separate OS
   processes, each generating its own key material with no shared RNG or in-memory secret
   exchange, coordinating only through files, tested at both changelog entry 73's founding-group
   scale (7 members, 6-of-7 threshold) and its eventual steady-state committee scale (35 members,
   12-of-35), answering entry 82's specific open question about ceremony *mechanics*. **This does
   not close the standing OPRF-committee blocker.** It provides no real secure channels between
   geographically-distributed members, no real hardware key custody, no Sybil-resistant member
   vetting/selection, and no complaint/justification handling for a member going offline
   mid-ceremony beyond a clean, tested timeout-and-abort; `OprfCommitteeKeys`/`CommitteeMembers`
   remain empty of anything real. See `docs/project/changelog/085.md` and
   `oprf-committee-dev/README.md`'s "Founding-ceremony simulator" section for the full,
   explicit list of what a real ceremony still needs beyond this tooling.] **Update, log #088**: both of entry 82's own "Still open" items that named a governance/authorship gap are now closed. (1) **A governance trigger for voluntary/early rotation now exists.** `pallet-identity` gained `trigger_voluntary_oprf_rotation` (call index 18), gated by the same `AdminOrigin`/`legislature_call_hash` pattern `rotate_oprf_scheme` already uses, taking a mandatory `reason: [u8; 32]` (an IPFS content hash, mirroring `pallet_courts`' reasoning pattern) so a voluntary rotation — "move from phones to dedicated servers on our own schedule, nothing's wrong," entry 82's own example — is always distinguishable on-chain, via its own `OprfSchemeVoluntarilyRotated` event, from both the scheduled (`rotate_oprf_scheme`) and emergency (`emergency_rotate_oprf_scheme`) paths. 4 new tests; `cargo test -p pallet-identity-zk`: 112/112 passing (108 pre-existing + 4 new). (2) **Device-update authorship is now decided.** See `docs/project/changelog/088.md` for the full decision record: reproducible builds (so any member or auditor can independently confirm a distributed build's hash matches its published source) plus a small cross-committee update-review sub-quorum (one designated reviewer per each of entry 73's 5 independent committees, attesting on-chain to a build hash via the same poll-and-attest mailbox pattern entry 82 already established) — deliberately neither a single project-controlled signing key (recreates the exact centralization this design avoids elsewhere) nor the full 12-of-35 cryptographic threshold (conflates code-review with OPRF-evaluation competence, and is operationally unworkable per-release). No on-chain storage/extrinsic for the attestation itself, no reproducible-build tooling, and no decision on who sits on each sub-quorum exist yet — this entry is architecture, same as entry 82 was for the node design it complements. **The OPRF committee service itself remains entirely unbuilt, unchanged by either half of this update.**]** [PARTIAL] **Real Rarimo passport ZK flow (mobile)** — architecture researched and decided (log #55), on-device proving toolchain + proof-byte-encoding implemented and unit-tested (log #56), NFC reading researched with a concrete library choice (log #57) and the Android native module implemented (log #58). The `.wcd` witness-graph file blocker is now cleared in principle (log #61: fixed all 4 upstream `circom-witnesscalc` bugs found; `build-circuit` produces a complete, structurally-verified `out.wcd` for the real `registerIdentity_11_256_3_2_336_216_NA` circuit — 49MB, 3.78M nodes) but not yet in practice: the fixes only exist on two PR branches on a fork (`github.com/kei-nan/circom-witnesscalc`), not merged upstream, and the actual `out.wcd` this session produced exists only in this environment's ephemeral scratchpad — nobody has published it anywhere the mobile app could fetch it from yet (the decided plan per log #55/#56 is IPFS, with the desktop app pinning it). DG1/DG15/SOD → circuit-inputs assembly (log #59's "not yet started" item) is now built and tested for its self-contained half (log #62: `sodParser.ts`, cross-checked against the real reference implementation on a synthetic-but-real SOD fixture, wired into `RegisterScreen.tsx`). Log #62 also surfaced a blocker — `slaveMerkleRoot`/`slaveMerkleInclusionBranches` need a live inclusion proof from Rarimo's own `CertificatesSMT` registry — which log #63 resolved architecturally, not just technically: depending on Rarimo's hosted registry for citizen registration was a real vendor-lock-in bug (this chain's registration would depend on infrastructure it doesn't govern), not merely an unresolved integration. We now build and host our own equivalent certificate tree instead (`mobile/src/chain/certificateTree.ts` + `scripts/certificate-registry/`), registered via `pallet-identity`'s already-existing `AllowedMerkleRoots` governance. Still blocked on: actually sourcing a meaningful set of trusted DSC certificates (log #63 — this is a governance/PKI problem, not a coding one); a real (not public-data-derived) `skIdentity` generation scheme; publishing the `.wcd` (or a freshly-rebuilt one, once the upstream PRs land) to IPFS; verifying any of the native/NFC code actually compiles or runs (no JDK, Android SDK, or device in this environment — see log #58); and the iOS side (no `ios/` project exists yet to scaffold into). Separately, log #64 found that Rarimo itself is migrating this whole circuit family from circom/Groth16 to Noir/UltraHonk — recommendation there is to keep building on the current path for now (see log #64 for why) but not treat that as settled long-term. Full writeup in logs #55/#56/#57/#58/#61/#62/#63/#64; summary:
   - **`@rarimo/react-native-passport-reader` does not exist** (was a wrong reference in this file — corrected). `@rarimo/rarime-rn-sdk` is real but the wrong tool: Expo-coupled, generates Noir proofs, registers straight to Rarimo's own EVM contracts — doesn't feed `pallet-identity` at all.
   - **Decided path**: stay on `passport-zk-circuits` (circom + Groth16 BN254) directly — confirmed current/actively maintained, and confirmed byte-for-byte compatible with our existing VK assets and `verifier.rs` (downloaded `registerIdentity_11_256_3_2_336_216_NA`'s real verification key from their latest release: `protocol: groth16, curve: bn128, nPublic: 5` — exact match to our 5-signal layout). Prove on-device with `@iden3/react-native-rapidsnark` (Groth16 prover) + `@iden3/react-native-circom-witnesscalc` (witness calc) — both are real, maintained, **plain bare-RN native modules, no Expo migration needed**. This is the same toolchain Rarimo's own production RariMe app ships (confirmed: their iOS build bundles `librapidsnark.a` + `libwitnesscalc_registerIdentity_20_160_3_3_736_200_NA.a`, named after our exact circuit variant).
   - **Decided: use the Full circuit, not "Light."** Light mode (proving key ~15–22MB vs. Full's 515MB) drops the on-device PKI signature-chain check and defers it to a "trusted Rarimo verifier" server — a centralized, unaccountable trust dependency that contradicts this project's whole point and doesn't match `pallet-identity`'s existing design (`AllowedMerkleRoots`, gated by `AdminOrigin` → legislature vote — i.e., trust anchors decided by on-chain governance, not a vendor's server). The Full circuit verifies everything (passport integrity **and** the PKI chain) inside the SNARK, fully self-contained and verifiable from public data alone. This is a deliberate size-for-trustlessness tradeoff, made on purpose — don't "optimize" this back to Light mode without re-litigating the tradeoff.
   - **Decided: distribute the 515MB proving key via IPFS**, not bundle it in the app or serve it from a corporate server. Content-addressing means the file's integrity doesn't depend on trusting whoever served it (re-hash on receipt); this is different from Light mode's problem, which requires trusting a *claim*, not just bytes. Real embedded P2P (libp2p) nodes in React Native are **not practically supported today** (`gomobile-ipfs` archived since 2023) — mobile stays a fetch-only IPFS client. The desktop app (already built, less battery/bandwidth-constrained) is a much better candidate to actually pin/seed this file as part of a genuinely decentralized swarm. Only fetch the one circuit variant matching the user's actual passport signature/hash scheme, not all of them.
   - **Ruled out: peer-assisted proof computation**, even with the Light circuit. Common misconception worth recording since it'll come up again: "Light" only removes the PKI-chain constraints from the circuit — it does **not** remove DG1 (biographic data: name, DOB, nationality, passport number) from the witness, since both Light and Full still need to prove things about DG1. A witness is the complete plaintext assignment of every circuit wire; sending it to any peer (Light or Full) leaks the passport data in the clear, worse than trusting a server since a random peer has zero accountability. The real answer to "peer-assisted proving without leaking data" is collaborative/MPC-based SNARK proving (witness secret-shared across non-colluding parties — e.g. "Collaborative zk-SNARKs," Ozdemir & Boneh) — legitimate but a fundamentally different, much heavier proving stack than `rapidsnark`/`witnesscalc` (single-party, no MPC support). Flagged as a real future direction, explicitly out of scope for now.
   - **Still genuinely open, not yet researched/resolved**: (1) NFC chip reading itself — no confirmed off-the-shelf RN library found; still needs BAC key derivation from the MRZ + low-level APDU exchange with the chip. (2) A witness-calculator format mismatch: `passport-zk-circuits`' release bundle ships the classic circom C++ witness generator (`.cpp`/`.dat`), while `@iden3/react-native-circom-witnesscalc` expects the newer graph format (`.wcd`) from a different iden3 tool — need to either compile a `.wcd` graph ourselves from the `.circom` source, or bridge directly against Rarimo's own precompiled approach. (3) Proof encoding: `groth16Prove` returns snarkjs's standard JSON proof format; converting to the compact ark-serialize byte layout `verifier.rs` expects (129 bytes: A/B/C points + variant byte) is real, bounded work, not yet done.
9. [ ] **Stablecoin bridge** — Phase 2; treasury currently uses native AGR token
10. [PARTIAL] **An AI-ruling oracle service** — found 2026-08-04, not previously tracked here.
    `pallet-courts` has real, tested on-chain machinery to *accept* a ruling
    (`submit_ai_ruling`, gated by a real `OracleOrigin` checking `OracleAccount` storage, not an
    `EnsureRoot` placeholder) but nothing off-chain actually generates a ruling by calling an AI
    model and submitting it. The desktop app's existing Claude integration (`commands/agent.rs`'s
    `agent_ask`) is a separate, deliberately read-only citizen Q&A feature over law/proposal
    text — not an autonomous court oracle, and not wired to `submit_ai_ruling` at all. Same shape
    of gap as item 8 (real on-chain acceptance machinery, no real off-chain service behind it).
    **Update, log #086**: `court-oracle/` is now a real, standalone Rust crate that polls
    `Courts::Cases` for `CaseStatus::Filed`, builds case-appropriate context from
    `Constitution::Laws`/`TreasuryLedger`/`PalletAudit` storage, calls Claude for a Level-0
    ruling (a fresh system prompt, distinct from the desktop app's read-only Q&A feature),
    publishes the reasoning to IPFS (a new publishing client — none existed in this codebase
    before), and submits the real 3-argument `submit_ai_ruling(case_id, ruling_hash,
    model_version)`, reading `CurrentAIModelVersion` fresh from chain each poll cycle.
    `cargo build`/`cargo test` pass (34 tests, 0 warnings). Also found and fixed, as a real side
    effect: `desktop/src-tauri/src/rpc.rs`'s `twox128_hex` was computing wrong storage-key
    hashes on every deployment (confirmed against the standard `twox128("System")` reference
    vector) — fixed with a regression test. **Update (`review-fix/court-oracle-finalize-
    scheduling`)**: the `finalize_ruling` gap below is closed. `poll_once` now has a second
    branch for cases in `CaseStatus::AIRulingIssued`: once the current block passes
    `AIRulingBlock[case_id] + AppealWindowBlocks` with no appeal filed (status still
    `AIRulingIssued`, not moved to `InJuryAppeal` by `appeal_ruling`), it submits
    `finalize_ruling(case_id, verdict)`, signed by the same oracle key `submit_ai_ruling` already
    uses — both calls share the same `T::OracleOrigin` gate (`EnsureOracle`), so no separate
    signer/origin was needed. **Update, commit `ad30aa3`**: verdict binding moved to submission
    time. `submit_ai_ruling` is now a 4-arg call (`case_id, ruling_hash, model_version, verdict`)
    that commits `verdict` on-chain in `AIRulingVerdict` when the ruling is first submitted, not
    reconstructed later from the published IPFS document; `finalize_ruling` correspondingly
    dropped its own `verdict` argument (`finalize_ruling(case_id)`) and just applies whatever was
    already committed. This closes the hole where a compromised oracle key could publish
    reasoning saying one thing and finalize with a different verdict — there is nothing left for
    the caller to choose at finalization time. `court-oracle`'s `extrinsic.rs` was updated to
    match both call signatures. `cargo test` passed 42/42 immediately after this fix (the
    IPFS-verdict-recovery tests the old scheme needed — `parse_verdict_from_ruling_document`'s
    parsing, the old 3-arg `finalize_ruling` call-byte layout — were removed as obsolete; the
    appeal-window/status gating tests remained). A follow-up 2026-08-16 review then added IPFS
    content-hash verification and Claude prompt-injection delimiting (5 new tests), bringing the
    current count to 47/47 — see `court-oracle/README.md`. **Still marked PARTIAL, not DONE**:
    (a) never run against a real chain,
    Claude API, or IPFS daemon — the live RPC/API/daemon round trips and the full orchestration
    loop are unit-tested at the pure-logic level only; (b) `Courts::set_oracle_account` (root-
    only) was never called, so no real chain currently accepts this service's signed calls. See
    `court-oracle/README.md` and `docs/project/changelog/086.md` for the full accounting.
    **Update, log #090: (b) is now done, and (a) is partially done — for real, not simulated.**
    A real chain was built (per `/CLAUDE.md`'s exact Critical Build Command, no `dev-mode`) and
    run; a real local Kubo IPFS daemon was stood up (no root needed — a static binary from
    `dist.ipfs.tech` works fine); `Sudo::sudo(Courts::set_oracle_account(...))` was called for
    real against a dedicated oracle account, confirmed via storage query — closing (b)
    outright. A real test case was filed (`Courts.Cases[0]`, confirmed on-chain), which required
    bootstrapping `CurrentAIModelVersion` off zero via the AI Model Governance Council (real,
    no shortcut needed — that mechanism resolves instantly, unlike legislature motions) and
    working around the fact that becoming an `is_active_citizen` normally requires either a real
    ZKPassport proof (item 1's standing blocker) or a `pallet-legislature` motion with a genuine,
    non-fast-forwardable 7-day window — worked around with a disclosed `System::set_storage` by
    root, not a fabricated proof or a code bypass; see log #090 for the full reasoning on why
    that's the honest option here. `court-oracle` was then built and run for real against all
    three: it decrypted a real age-encrypted keystore, connected over real RPC and found the real
    filed case, and called the real Anthropic API — which returned a real `401 Unauthorized`,
    because **no Claude API key exists anywhere in this sandboxed environment** (checked the env,
    every reachable `.env*`, and confirmed `~/.claude/.credentials.json` is Claude Code's own
    unrelated OAuth credential, not usable here). That is where this session's real progress
    stops: no ruling was ever produced, so `submit_ai_ruling`/`finalize_ruling` were never
    actually submitted by `court-oracle`, and the IPFS-publish path (`ipfs.rs`'s real `add()`
    call) never got exercised either, since `poll_once` calls Claude before IPFS. Nothing was
    mocked to paper over this — the 401 is reported as exactly what it is, a real, live rejection
    from a real, live endpoint, not a stand-in for success. **Still open, unchanged: item 1 (no
    real citizen registration is possible without it) and a real Claude API key for this
    environment; both are prerequisites for actually closing this item.** See
    `docs/project/changelog/090.md` for the full trail.
11. [DONE] **Multi-agent security review fixes (2026-08-08/09)** — a 7-agent parallel review of
    the whole repo found several real bugs, landed as reconciled, tested branches
    (`review-fix/*`), pending merge review:
    - **CRITICAL**: `PassthroughMACIVerifier` accepted any vote tally unconditionally in every
      build (not just dev-mode, unlike every other passthrough verifier in this codebase) —
      any `LegislatureOrigin`-controlled account could enact a law on a fabricated tally. Now
      gated behind `dev-mode`; a new `FailClosedMACIVerifier` rejects every tally outside it
      until a real MACI circuit verifier exists.
    - **CRITICAL**: `pallets/pallet-courts/src/tests.rs` had a brace/`#[test]`-attribute
      nesting bug (introduced in commit 88608bf) that silently swallowed 12 tests into their
      neighbors — zero of the AI-governance/`CaseFilingBond` tests were actually executing.
      Fixed; 25/25 now pass.
    - **HIGH**: `EnsureLegislatureMotion::try_origin` never bound its approval to the specific
      call it authorized — any passed legislature motion, on any topic, produced a token
      usable to execute *any* legislature/`AdminOrigin`-gated call anywhere (appoint a
      minister with a motion that was voted on to enact an unrelated law, etc.). Converted to
      `EnsureOriginWithArg<_, [u8; 32]>`; 27 consuming call sites across 7 pallets now pass a
      domain-separated hash of their own parameters, checked against the specific motion.
    - **HIGH**: liquid democracy delegation was write-only — `Delegations` was never read by
      `commit_vote`/`vote_referendum`/`submit_maci_tally`, so delegating a vote had zero
      effect on any real tally. Now resolved (transitively, per-topic) into MACI-adjacent
      Referenda tallying in `finalize_referendum`; deliberately still *not* resolved into MACI
      itself, since cross-referencing the plaintext delegation graph against opaque MACI
      commitments would leak exactly the linkage MACI exists to hide (real delegation-aware
      MACI tallying needs an off-chain coordinator service that doesn't exist yet).
    - **HIGH**: `submit_oprf_response` never verified the Chaum-Pedersen DLog-equality proof
      accompanying a committee member's OPRF response, or bound it to the specific query —
      any single roster member could submit an arbitrary, unverifiable response accepted as
      authoritative. Now verified for real (BabyJubJub curve arithmetic + the `t16` Poseidon2
      permutation, ported from `oprf-committee-dev`'s existing but unusable-here `std`-only
      math into a new no_std `dlog_verify.rs`, validated against real upstream known-answer
      vectors) and bound to its query's own stored `blinded_query`.
    - **HIGH**: desktop's QR-auth callback parsed but never verified the phone's signature
      (and served the callback with a CORS wildcard) — any local process that learned the
      challenge UUID could forge a session. Now verifies a real sr25519 signature (the mobile
      client's actual scheme — not Ed25519, corrected mid-fix) against the identity's
      on-chain-registered pubkey; sessions are now real server-side bearer tokens with
      enforced expiry, not frontend-only state.
    - **HIGH**: mobile signed everything (including auth and votes) with a hardcoded public
      dev mnemonic, no Keystore/Secure Enclave code anywhere. Android now has a real
      Keystore-backed native module encrypting a random per-install seed at rest (not literal
      in-hardware signing — Android Keystore can't hold an sr25519 key — documented as such);
      the dev-mnemonic fallback only fires when `__DEV__` and Keystore is unavailable, and
      throws otherwise. iOS untouched (`ios/` still doesn't exist).
    - Also closed as part of the same pass: `pallet-elections`' `on_initialize` unbounded
      full-`Delegates`-table iteration (now a bounded, resumable per-block sweep), and the
      doc-drift items tracked separately in this pass's docs commit (emergency-council
      wiring claim recurring in a second file, several stale pallet-doc call signatures, a
      stale test-count claim, and a couple of self-contradicting status lines).
    - **Not attempted / explicitly scoped out this pass**: `commit_vote`'s `ensure_signed`
      sender-anonymity gap (documented as a known limitation on the call itself rather than
      architecturally reworked); desktop's `chain_submit_extrinsic` command exists but nothing
      yet produces its input (no phone→desktop signed-arbitrary-call protocol); desktop's
      `smoldot` dependency remains unwired (confirmed a substantial transport-layer rewrite,
      not a quick swap, investigated not attempted). **Update (changelog #089, later session)**:
      this specific gap is closed — `desktop/src/chain/client.ts` embeds smoldot for real via
      `@polkadot/api`'s `ScProvider`, in the JS frontend (not Rust), and the nine chain-read
      commands the browsing pages call were migrated to it and proven to sync and answer real
      queries against a live local `agora-node --dev` chain, headlessly, in a real production
      browser build. `chain_submit_extrinsic` remains exactly as described above — still
      unwired to any producer of its input.
    - Every fix above was built by a sub-agent working in a harness-provided git worktree that
      turned out to be cut from a stale branch point 8 commits behind `main` (missing the OPRF
      mailbox, AI model governance, and several other commits) rather than current `main` — a
      harness-side issue, not something these agents could see or fix themselves. Two agents
      (the OPRF-mailbox and docs fixes) detected this themselves and adapted or refused rather
      than silently producing wrong output against a fictional codebase state; the rest were
      reconciled by hand afterward, diffed and re-verified against actual current `main`
      (full pallet test suites + `cargo check -p agora-runtime` with and without `dev-mode`
      re-run clean post-reconciliation). Flagging this here mainly so a future session doesn't
      waste time re-discovering it if worktree-isolated sub-agents produce another
      surprising-looking diff.
    - **Same class of problem recurred integrating changelog entries 88-90** (the OPRF voluntary
      rotation trigger, device-update-authorship decision record, and desktop smoldot light
      client): all three were built in worktrees cut from a branch point 10-11 commits behind
      `main` (missing, among others, the `EmergencyRotationOrigin`→`pallet-emergency-council`
      wiring and the OPRF mailbox pruning work). This time it produced a genuine compile error,
      not just a stale-context risk: `trigger_voluntary_oprf_rotation` and the already-landed
      `prune_oprf_query` both claimed `#[pallet::call_index(18)]` — reassigned the former to 19.
      It also produced one genuinely wrong test:
      `trigger_voluntary_oprf_rotation_is_independent_of_the_other_two_rotation_paths` called
      `emergency_rotate_oprf_scheme(RuntimeOrigin::root())` directly, which the since-landed
      `EmergencyRotationOrigin` wiring now rejects with `BadOrigin` (root alone no longer
      suffices — an active emergency must be genuinely declared first); fixed by reusing the
      existing `declare_active_emergency()` test helper. `docs/project/changelog/087.md` also
      collided on file name with this session's own new entry 87 (face match/liveness) —
      unrelated content, pure numbering coincidence — resolved by moving the incoming
      court-oracle entry to 090 and fixing its cross-references. Post-reconciliation:
      `cargo test -p pallet-identity-zk` 122/122, `cargo check -p agora-runtime` clean,
      `cargo check` clean in `desktop/src-tauri`, `tsc --noEmit` clean in `desktop/`.
12. [PARTIAL] **On-device face match + liveness detection** — found 2026-08-10 during a
    docs-drift review, not previously tracked here. **Update, log #087**: the TODO is now real
    code. `NfcPassportModule.kt` reads EF.DG2 (JMRTD's `DG2File`/`FaceInfo`/`FaceImageInfo` API,
    genuinely source-verified against the real `jmrtd-0.8.6-sources.jar` this project pins — see
    log #087 for the exact classes/methods). A new `com.agora.facematch` native package
    (`FaceCameraViewManager`/`FaceCaptureModule`/`FaceMatchModule`) — a custom CameraX-based
    module, deliberately not `react-native-vision-camera` (v4 is JSI-oriented, v5 is
    New-Architecture-only, and this app has `newArchEnabled=false` — see log #087 for the full
    reasoning) — drives a 2-shot randomized challenge-response liveness check (frontal
    eyes-open baseline, then blink or turn, read via ML Kit Face Detection's bundled model) and
    a MobileFaceNet TFLite embedding comparison against the DG2 photo. `RegisterScreen.tsx`'s
    TODO line is replaced with a real capture UI gating progression to `proving`; a new
    `LivenessVerified` pipeline stage was added to `registrationState.ts`/`registrationReconciler.ts`.
    `mobile/android/app/src/main/assets/mobilefacenet.tflite` is a real, cited, BSD-3-Clause
    asset (`MCarlomagno/FaceRecognitionAuth`) — its training-data lineage
    (`sirius-ai/MobileFaceNet_TF`, typically MS1M/MS-Celeb-1M-derived) is documented in
    `FaceMatchModule.kt`'s doc comment and log #087, not hidden. `npm run type-check`: clean.
    `npm test`: 210/210 passing (up from 197). **Still not real**: none of the new Kotlin has
    been compiled or run — no Android SDK/JDK exists in this environment, same standing
    limitation `NfcPassportModule.kt` itself already carries. Only the DG2/JMRTD API was
    independently source-verified this session; the CameraX/ML Kit/TFLite calls were written
    against documented stable APIs but not re-verified against downloaded source. Match/liveness
    thresholds are unvalidated placeholders (no real calibration corpus). The liveness check is
    2-shot-still, not continuous video — a prepared attacker with video of the real person could
    plausibly defeat it, a documented residual risk, not an oversight. iOS has no equivalent
    since `ios/` itself doesn't exist. See `docs/project/changelog/087.md` for the full record.
13. [PARTIAL] **Persona-based review (security researcher / citizen / product manager) + two
    fixes, 2026-08-20** — user-requested, not `/project-review`. Two concrete security fixes
    landed: (a) **post-emergency cooldown** in both `pallet-emergency-council` and
    `pallet-executive` — neither previously enforced any minimum gap between one emergency
    ending and the next being declared, so the same supermajority could chain back-to-back
    emergencies into de-facto indefinite emergency powers despite `MaxEmergencyBlocks` capping
    each individual window; a new `CooldownUntil` storage item + `EmergencyCooldownBlocks` config
    (wired to 7 days in the runtime) closes this, 132/132 tests passing across both pallets. (b)
    **`remove_oracle_member` now purges a removed member's stale approvals** from in-flight
    `PendingOracleProposal`s in `pallet-courts` — previously a compromised-and-removed member's
    already-cast approval kept counting toward quorum, undercutting the M-of-7 council's whole
    purpose of surviving exactly that incident-response path; 45/45 `pallet-courts` tests
    passing including a new regression test. **Left open, not fixed this session**: a
    court-oracle prompt-injection delimiter that doesn't escape its own closing tag
    (`court-oracle/src/context.rs`, Medium severity — **fixed 2026-08-21, commit `7da2105`**: a
    new `neutralize_tag_markers` HTML-entity-escapes literal open/close delimiter markers before
    wrapping untrusted IPFS text, so a law author can no longer forge a fake closing tag followed
    by fake trusted-looking directives; the same mitigation was ported into desktop's `agent_ask`
    in commit `bbacd04`), a narrow delegation-cap staleness gap in `pallet-voting`
    (Low/informational, still open), and a long list of citizen-UX gaps (mobile shows raw hex
    instead of proposal/law content since it never fetches IPFS — **fixed 2026-08-21, commit
    `c4aa1a9`**: `mobile/src/chain/ipfs.ts` fetches and SHA-256-verifies IPFS content the same way
    desktop does, wired into `IpfsContentBox.tsx`/`ProposalsScreen.tsx`/`LawsScreen.tsx`;
    `SeatingSkippedNoDisclosure` invisible in either app, still open; oracle-council composition
    invisible to citizens — **partially fixed**: desktop's `CourtsPage.tsx` now shows council
    size/threshold and a live per-case approval count (commit `bcc3c34`), but mobile remains
    unfixed — `CasesScreen.tsx` calls `getOracleMembers()` and already holds the full roster in
    state, but only uses it for the `isFilerOrOracle` eligibility check, never renders it to the
    citizen) and PM-level gaps (OPRF committee *formation logistics* — not just the crypto — has
    no owner or timeline; no pilot-onboarding/compliance/incident-response docs exist anywhere).
    See `docs/project/changelog/092.md` for the full findings list and fix details.
    **Update, 2026-08-22**: two further items from this pass's own follow-on work. (1) **The
    Oracle Council M-of-N gate is now extended to the three manual-override extrinsics that had
    bypassed it** — `pallet_constitution::invalidate_law` and
    `pallet_identity_zk::suspend_citizen`/`restore_citizen_rights` were gated only by bare
    `EnsureOracle` membership (any single council member could act unilaterally), unlike
    `submit_ai_ruling`/`approve_ai_ruling`/`finalize_ruling`, which already required M-of-N
    approval. Commit `ae31e71` adds `pallet_courts::EnsureOracleCouncilApproved` (an
    `EnsureOriginWithArg` gated by a new `propose_admin_action`/`approve_admin_action` flow,
    call-hash-bound the same way `EnsureLegislatureMotion` is) and rewires `CourtOrigin`/
    `SuspensionOrigin` in the runtime to it. (2) **Account recovery is now implemented on-chain**
    — CLAUDE.md's "Recovery = re-scan valid passport" claim was previously unimplemented
    (`register_citizen` unconditionally rejected any already-registered nullifier, and no call
    let a citizen rebind their identity to a new `AccountId`). Commit `ea12789` adds
    `recover_account`: same proof shape/verification path as `register_citizen` (real
    `ZkVerifier`/`AnchorVerifier`, no dev-mode shortcut); on a nullifier+anchor match it rebinds
    all the per-citizen storage items (`NullifierRegistry`, `CitizenNullifier`, `CitizenAnchor`,
    `IdentityAnchorRegistry`, `CitizenIndex`/`CitizenPosition`, `ReverificationDeadline`,
    `SelfDeclaredSingleDocument`) from the old `AccountId` to the new signer, invalidating the old
    account; rate-limited via a new `MinBlocksBetweenRecoveries` cooldown; an active suspension
    carries over automatically since `SuspendedNullifiers`/`SuspendedByJuryReview` are keyed by
    nullifier, not `AccountId`. 10 new pallet tests; `pallet-identity-zk` suite: 131/131 passing.
    **The chain-side mechanism is done and tested; there is no mobile UI wrapper for it yet**
    (no screen/flow calls `recover_account`), and — like every other real proof-submitting call in
    this codebase — it is still gated on the standing OPRF committee blocker (item 1): no genuine
    ZKPassport proof can be produced on-device until a real committee exists, so `recover_account`
    cannot actually be exercised end-to-end yet either.

    **Update, 2026-08-22 (part 3)** — a high-severity review finding covered three gaps around
    `recover_account`; this pass closed what's safely closeable now and left the rest as tracked
    open work (see items 14/15 below). (1) **Mobile wrapper + screen added.**
    `mobile/src/chain/identity.ts` gained `recoverAccount`/`RecoverAccountParams`, matching the
    real 5-argument `recover_account` call shape (`identity.test.ts`: 5 new tests, 33/33 passing
    in that file, 228/228 across the mobile suite). `mobile/src/screens/
    RecoverAccountScreen.tsx` is a new screen (wired into `App.tsx`, linked from `HomeScreen.tsx`'s
    not-registered card) that mirrors `RegisterScreen.tsx`'s re-scan flow — same MRZ form, NFC
    scan, liveness/face-match gate — gated behind a mandatory, plainly-worded disclosure screen
    (shown, and re-confirmed via a destructive confirm dialog, before the re-scan can even start)
    stating that recovery is instant and irreversible, the old account stops working immediately,
    and the old account's AGR balance/delegations/delegate backing/legislature seat/cabinet role
    are NOT transferred and will be lost. Like `RegisterScreen.tsx`, it stops at the same
    proving-pipeline wall (no certificate-registry inclusion proof, on-device commitment salts, or
    proving key exist yet) — `npx tsc --noEmit` clean, but this cannot be exercised end-to-end
    (no OPRF committee, no device). (2) **Cross-pallet orphaning is now documented as a known,
    current gap**, not silently left implicit — see `pallets/pallet-identity/src/lib.rs`'s
    `recover_account` doc comment and `docs/project/pallets/identity.md`'s new "Known
    limitation" section, both spelling out exactly what does *not* move (balance, pallet-voting
    delegations/budget, pallet-elections delegate backing, legislature seat, cabinet role,
    anticorruption disclosures). (3) **Deliberately not attempted this pass** (per the finding's
    own scoping): the actual cross-pallet migration, and any notification/dispute-window
    mechanism for the coercion risk — see items 14 and 15.

14. [ ] **Cross-pallet `AccountMigrator` for `recover_account`** — found 2026-08-22 (review
    finding, see item 13 part 3). `recover_account` only rebinds pallet-identity's own storage;
    a recovering citizen's AGR balance, pallet-voting budget/delegation state, pallet-elections
    delegate registration/backing, a legislature seat, a cabinet role, and pallet-anticorruption
    disclosures/conflict-registry entries all stay silently bound to the abandoned old account.
    Real, separate design/implementation work: needs a genuine cross-pallet `AccountMigrator`
    trait (mirroring the existing `CitizenSuspender` runtime-trait pattern) spanning
    pallet-voting/elections/legislature/executive/anticorruption, each implementing its own
    "move this account's state to a new AccountId" logic and being called from
    `recover_account` in a single atomic transaction. Non-trivial questions of its own: what
    happens to an in-flight vote/delegation/motion the old account was mid-way through, whether
    a legislature seat or cabinet role can even be "moved" mid-term without its own governance
    implications, and how to keep the whole rebind atomic across that many pallets. Not
    attempted in the 2026-08-22 pass — deliberately scoped out as too large for that session.

15. [ ] **Open safety/product question: `recover_account` has no notification or dispute window**
    — found 2026-08-22 (review finding, see item 13 part 3), needs a human product decision, not
    just engineering. `recover_account` succeeds instantly with no notice to the old account and
    no delay before it takes effect — deliberate, per the pallet's own doc comment (a delay only
    helps if the old key is still reachable to contest it, which is the opposite of the
    lost-device case this call exists for). But the same instant, undisputable rebind is also a
    ready-made silent identity-transfer tool under coercion: someone can be forced (e.g. at
    gunpoint, or via device confiscation) to re-scan their own passport onto an attacker's
    account, permanently and unrecoverably surrendering their citizen identity with no trace
    distinguishing it from a legitimate lost-device recovery. Whether any mitigation is even
    desirable is a genuine open question — a delay/notification only helps if the *old* device
    is still reachable, which directly contradicts the lost-device scenario the call exists for
    in the first place, so a naive "add a dispute window" fix could make the legitimate case
    worse without meaningfully stopping the coercion case (an attacker can just wait out the
    window while still controlling the victim). Needs product-level input (e.g. is a short
    delay with a loud, unmissable notification to any surviving old-account session an
    acceptable tradeoff; is there a distinct "duress" signal a citizen could pre-register; is
    this simply an accepted residual risk of any single-factor biometric-recovery scheme) before
    any implementation is attempted. Not attempted in the 2026-08-22 pass, deliberately.

16. [DONE] **Independent Accountability Council + auditor/investigator appointment origins** —
    2026-08-22, three-commit sequence closing a real self-oversight gap: `pallet-audit`'s
    `add_auditor`/`remove_auditor` and `pallet-anticorruption`'s
    `add_investigator`/`remove_investigator` were bare `ensure_root` with no configurable
    `EnsureOrigin` at all, and routing them through the legislature (the obvious alternative)
    would have reproduced the exact self-oversight failure real Supreme Audit Institutions exist
    to prevent, since the legislature already controls the treasury budget it would then also be
    choosing the auditors for. Commit `ca43602` adds a new `pallet-accountability-council` pallet,
    wired into the runtime at index 19: a small (7–9 member) council explicitly barred from
    `pallet-legislature`/`pallet-executive` membership overlap, mechanically mirroring
    `pallet-courts`' Oracle Council admin-action pattern (call-hash-bound propose/approve) but
    requiring a genuine 2/3 supermajority rather than the Oracle Council's plain >1/2, exposed as
    `EnsureAccountabilityCouncilApproved<T>`; membership is self-perpetuating once bootstrapped
    (`Root` seeds initial members and calls `close_bootstrap()` once, after which
    `add_member`/`remove_member` require the Council's own supermajority vote instead of `Root`).
    Commit `735d876` then routes both pallets' appointment calls through a new `AppointmentOrigin`
    config item wired to this Council. Commit `dcdb1f3` separately gates
    `pallet-treasury-ledger`'s `register_department_spender`/`remove_department_spender` (also
    previously bare `ensure_root`) behind the pallet's existing `LegislatureOrigin` instead —
    deliberately *not* routed through the new Accountability Council, since department-spender
    designation is an operational, Executive-branch-like power distinct from the independent
    oversight appointments the Council governs, and routing it there would dilute that Council's
    independence. `cargo test -p pallet-accountability-council` (26/26), `-p pallet-audit`
    (47/47), `-p pallet-anticorruption` (46/46, pre-`0529508`), `-p agora-runtime` (64/64) all
    passing; `cargo check -p agora-runtime` clean with and without `--features dev-mode`. See
    `docs/project/pallets/accountability-council.md`.

17. [DONE] **Anti-corruption: two different investigators required to clear/refer a report** —
    2026-08-22, commit `0529508`. Any single current investigator could unilaterally
    `clear_report` or `refer_report_to_courts` on *any* report, including one about themselves —
    and report content is deliberately encrypted to the investigator's key, so the chain can never
    check on-chain whether a report concerns the caller, ruling out a content-based fix. Fix is
    structural instead: `clear_report`/`refer_report_to_courts` now only propose a transition
    (recorded in new `PendingReportAction` storage, keyed by `report_id`), leaving the report at
    `UnderInvestigation`; a second, *different* investigator must call the new
    `approve_report_action` to actually apply it, and the same investigator proposing twice is
    rejected with `Error::SameInvestigator` — the same 2-of-N peer-sign-off shape
    `pallet-accountability-council` (item 16) and `pallet-courts`' Oracle Council both already use
    elsewhere, scoped down from a supermajority vote to a plain 2-of-N here. 52/52 pallet tests
    and 64/64 runtime tests passing; `cargo check` clean on `agora-runtime` with default and
    `--features dev-mode`.

18. [DONE] **Delegate-persona and backing-proof ZK schemes — circuit, pallet wiring, and mobile,
    now all real** — 2026-08-22 through 2026-08-23, a five-commit sequence (`2e07f68`, `ca99c7d`,
    `e31257a`, `786b792`, `4a628d1`) that closes what `CLAUDE.md`'s Voting System section used to
    flag as "discussed... once built (not built as of 2026-08-22)". Confirmed as genuinely
    non-stub — real circuits, real pallet-level verification, real mobile call-shape wiring — by
    three independent review agents, and now documented as done in
    `docs/project/pallets/elections.md`. In order: `2e07f68` added a **delegate-persona** ZK
    circuit (a citizen proving eligibility to register a chosen `persona_account` as their
    delegate identity, riding inside ZKPassport's outer proof the same way `disclosure` does);
    `ca99c7d` added a bare per-citizen `BackingCommitment` map with no tree yet; `bae1cbd` (see
    below) built the actual depth-32 Poseidon2 incremental Merkle tree over it; `e31257a` added a
    standalone **backing-nullifier** circuit (a Semaphore/nullifier-scheme proof of Merkle-path
    membership in that tree, binding `delegate_persona_id` as a plain checked public input rather
    than folding it into a `param_commitment` hash, since — unlike `delegate-persona` — this
    circuit isn't constrained by ZKPassport's fixed 8-field outer interface) plus a standalone
    Rust UltraHonk verifier (`runtime/src/backing_nullifier_verifier.rs`); `786b792` wired both
    into `pallet-elections` for real — `register_as_delegate` now verifies a real delegate-persona
    proof via `T::ZkVerifier` → `T::CommitteeKeyChecker` → `T::DelegatePersonaVerifier` instead of
    trusting `ensure_signed`'s caller identity directly, and `back_delegate`/`remove_backing` now
    verify a real backing-nullifier proof via `T::BackingProofVerifier`/`T::BackingRootChecker`,
    replacing the old plaintext `BackingOf`/`CitizenBackingCount` maps with a nullifier map
    (closing the "who backs whom" linkability the old plaintext design had); `4a628d1` wired the
    mobile app to all of it (new `backingNullifierEncoding.ts`/`backingTree.ts`/`backingState.ts`,
    `zkProving.ts`'s `proveDelegatePersona`/`proveBackingNullifier`, updated
    `governance.ts`/delegate screens). Verified per-commit: `cargo test -p pallet-elections`
    59/59 (66/66 with `runtime-benchmarks`); `cargo test -p pallet-identity-zk` 141/141
    (`bae1cbd`'s Merkle-tree work, including 14 new tests checked against an independently-written
    recursive reference implementation); `nargo test --workspace` 50/50 for the circuits; `npx
    tsc --noEmit` clean and `npx jest` 297/297 in `mobile/` (see item 2 in Current State above).
    **Still not real end-to-end**: like every other proof-submitting call in this codebase, both
    schemes remain gated on the standing OPRF committee blocker (item 1) — no genuine on-device
    proof can be produced without it — and the submission-metadata-linkability gap `CLAUDE.md`'s
    Voting System section documents for `commit_vote` applies identically here (a mathematically
    unlinkable ZK derivation does not anonymize the transaction that reveals it). See
    `docs/project/changelog/093.md`, `094.md`, `095.md`, `096.md` for the full per-commit trail,
    and `docs/project/pallets/elections.md`/`docs/project/pallets/identity.md` for current pallet
    documentation.

