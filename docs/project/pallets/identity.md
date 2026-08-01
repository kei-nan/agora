# pallet-identity

### pallet-identity (crate: pallet-identity-zk) — runtime index 8

Storage:
- `NullifierRegistry`: `[u8;32]` → `AccountId`
- `CitizenNullifier`: `AccountId` → `[u8;32]`
- `CitizenIndex`: `u32` → `AccountId`  (dense, swap-and-pop on revoke)
- `CitizenPosition`: `AccountId` → `u32`  (reverse index for O(1) removal)
- `TotalCitizens`: `u32`
- `SuspendedNullifiers`: `[u8;32]` → `Option<BlockNumber>`
  - Key absent = not suspended; `None` = indefinite; `Some(block)` = suspended until that block
- `AllowedMerkleRoots`: set of valid passport Merkle roots (see the certificate-registry rework in changelog 065-068 — no longer Rarimo-sourced)
- `OprfSchemeVersion`: `u32` — current active identity-anchor scheme generation (see changelog 065-068, logs #67/#68)
- `IdentityAnchorRegistry`: `(scheme_version, anchor: [u8;32])` → `AccountId` — Sybil-resistance exclusion-check registry, deliberately separate from `NullifierRegistry`/`CitizenNullifier`; no event ever emits an anchor value, so it can never be correlated with voting activity
- `CitizenAnchor`: `AccountId` → `(scheme_version, anchor)` — reverse lookup, used by `migrate_oprf_scheme`
- `ReverificationDeadline`: `AccountId` → `BlockNumber` — periodic re-verification tracking
- `SelfDeclaredSingleDocument`: `AccountId` → `bool` — self-declaration attestation (courts backstop, see below)

Calls:
- `register_citizen(nullifier, zk_proof [≤4096 bytes], public_inputs [≤16 × [u8;32]], anchor, anchor_proof)`
  - Verifies ZK proof via `ZkVerifier` trait
  - Checks passport expiry via `public_inputs[2]` (expirationDate vs current timestamp)
  - Checks country allowlist via `public_inputs[5/6]` (country_code_hash) — see changelog 065-068 log #67: one chain per country is the decided deployment model, using this same allowlist
  - Verifies `anchor_proof` via the new `AnchorProofVerifier` trait and rejects if `anchor` already exists under the current `OprfSchemeVersion` in `IdentityAnchorRegistry` — the Sybil-resistance gate (mandatory, not opt-in; see changelog 065-068 log #67 for why, and what was rejected instead)
- `revoke_citizen()` — swap-and-pop, clears suspension
- `suspend_citizen(nullifier, until)` — `SuspensionOrigin` (EnsureRoot placeholder)
- `restore_citizen_rights(nullifier)` — `SuspensionOrigin`
- `add_allowed_merkle_root(root)` / `remove_allowed_merkle_root(root)` — `AdminOrigin` (EnsureRoot placeholder)
- `reverify_citizen(proof)` — any registered citizen; extends `ReverificationDeadline` via `AnchorProofVerifier::verify_reverification`; a citizen past their deadline is treated as inactive by `is_active_citizen` (lazy check, no background sweep)
- `migrate_oprf_scheme(old_anchor_proof, new_anchor_proof, consistency_proof)` — dual-evaluation OPRF-scheme rotation migration; targets the caller's own on-file scheme version + 1 (not the global `OprfSchemeVersion` directly — see changelog 065-068 log #68 judgment call 1 for why the literal reading has a real bug)
- `rotate_oprf_scheme()` — `AdminOrigin`; scheduled-path advance of `OprfSchemeVersion` (the ~4-year cycle from changelog 065-068 log #67)
- `emergency_rotate_oprf_scheme()` — `EmergencyRotationOrigin` (currently `EnsureRoot` placeholder — `pallet-emergency-council` doesn't yet export a reusable `EnsureOrigin`, see changelog 065-068 log #68 judgment call 3); out-of-cycle rotation if the current OPRF scheme is suspected broken
- `declare_no_other_passport()` — any registered citizen; records a self-declaration attestation used only as an ex-post basis for a `pallet-courts` `CitizenConduct` case if later found false

Public helpers:
- `is_active_citizen(who)` — registered AND no active suspension AND not past `ReverificationDeadline`
- `is_citizen(who)` — registered regardless of suspension
- `citizen_at(index)` / `total_citizens()` — for jury selection
- `suspend_citizen_internal(nullifier, until)` — called by pallet-courts on guilty verdict

**ZK proof format and verifier**: this section is stale as of the Rarimo→ZKPassport migration (see changelog 065-068 log #65) and intentionally not corrected here — the 129-byte Groth16 format below and the VK-asset TODO are Rarimo-specific and no longer the live target. `runtime/src/verifier.rs` and `mobile/src/chain/{sodParser,zkProving,proofEncoding}.ts` still need their ZKPassport-targeted rework (unstarted as of changelog 065-068 log #68). `AnchorProofVerifier` (the new trait backing `register_citizen`'s anchor check, `reverify_citizen`, and `migrate_oprf_scheme`) is currently wired to `PassthroughAnchorVerifier` in both build paths — a dev-mode stub, same shape as the pre-existing `PassthroughZkVerifier`, accepting every proof unconditionally until the real OPRF-committee cryptography exists (see changelog 065-068 log #68's "still needed" list).

Old Rarimo-era proof format (kept for historical reference only, not current):
```
[0..32]   A  G1 compressed (ark-serialize LE, flags in byte 31)
[32..96]  B  G2 compressed (ark-serialize LE, flags in byte 63)
[96..128] C  G1 compressed
[128]     variant: 0=SHA-256 circuit, 1=SHA-1 circuit
```

