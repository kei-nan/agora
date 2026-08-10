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
- `CommitteeMembers`: `slot (0..NUM_COMMITTEES)` → `BoundedVec<AccountId, MaxCommitteeSize>` — OPRF committee roster per slot (changelog #82/#83 founding-phase node architecture)
- `OprfCommitteeKeys`: `(scheme_version, slot)` → `[u8; 32]` — governance-approved committee public-key hash per slot
- `NextQueryId` / `PendingOprfQueries`: `query_id` → the on-chain OPRF query mailbox a citizen posts a blinded query to
- `OprfResponses`: `(query_id, slot)` → `OprfResponseRecord` (evaluation, DLog-equality proof, submitting committee's pubkey) — a committee member's answer to a pending query

Calls (params reflect the pallet's current structure post-#75/#76 restructuring):
- `register_citizen(zk_proof [≤4096 bytes], public_inputs [≤18 × [u8;32]], anchor, oprf_pk_hashes: [[u8;32]; NUM_COMMITTEES])`
  - Verifies the ZKPassport outer proof via `ZkVerifier`; the nullifier is *not* a separate
    argument — it's extracted directly from the proof's own public inputs
    (`scoped_nullifier`, see the module doc comment)
  - Checks passport expiry and country allowlist from public inputs, same as before
  - Requires a mandatory OPRF identity-anchor check (`anchor` + `oprf_pk_hashes`, verified via
    `AnchorVerifier`) as the Sybil-resistance gate, rejecting if `anchor` already exists under
    the current `OprfSchemeVersion` in `IdentityAnchorRegistry`
- `revoke_citizen()` — swap-and-pop, clears suspension
- `suspend_citizen(nullifier, until)` — `SuspensionOrigin` (EnsureRoot placeholder)
- `restore_citizen_rights(nullifier)` — `SuspensionOrigin`
- `add_allowed_merkle_root(root)` / `remove_allowed_merkle_root(root)` — `AdminOrigin`
- `reverify_citizen(proof)` — any registered citizen; extends `ReverificationDeadline` via `AnchorVerifier::verify_reverification`; a citizen past their deadline is treated as inactive by `is_active_citizen` (lazy check, no background sweep)
- `migrate_oprf_scheme(old_anchor_proof, new_anchor_proof, consistency_proof)` — dual-evaluation OPRF-scheme rotation migration; targets the caller's own on-file scheme version + 1
- `rotate_oprf_scheme()` — `AdminOrigin`; scheduled-path advance of `OprfSchemeVersion` (the ~4-year cycle)
- `emergency_rotate_oprf_scheme()` — `EmergencyRotationOrigin` (currently `EnsureRoot` placeholder); out-of-cycle rotation if the current OPRF scheme is suspected broken
- `declare_no_other_passport()` — any registered citizen; records a self-declaration attestation used only as an ex-post basis for a `pallet-courts` `CitizenConduct` case if later found false
- `set_oprf_committee_key(scheme_version, slot, oprf_pk_hash)` / `remove_oprf_committee_key(scheme_version, slot)` — `AdminOrigin`; governance-approved committee key management
- `add_committee_member(slot, who)` / `remove_committee_member(slot, who)` — `AdminOrigin`; committee roster management (changelog #82/#83)
- `submit_oprf_query(blinded_query: [u8; 64])` — any registered citizen; posts a query to the on-chain mailbox for committee members to answer
- `submit_oprf_response(query_id, committee_slot, evaluation: [u8; 64], committee_pubkey: [u8; 64], dlog_proof)` — must be a member of `committee_slot`'s roster; verifies a Chaum-Pedersen DLog-equality proof binding the response to this specific query and to a `committee_pubkey` matching the governance-approved `OprfCommitteeKeys` entry (see `dlog_verify.rs`) — an unverified, unbound response is rejected, not just accepted-and-trusted

`AdminOrigin`, like `pallet-legislature::EnsureLegislatureMotion` elsewhere, is `EnsureOriginWithArg<_, [u8; 32]>`: each call above passes a domain-separated hash of its own parameters, checked against the specific motion that authorized it, so one passed motion can't be replayed to authorize a different call.

Public helpers:
- `is_active_citizen(who)` — registered AND no active suspension AND not past `ReverificationDeadline`
- `is_citizen(who)` — registered regardless of suspension
- `citizen_at(index)` / `total_citizens()` — for jury selection
- `suspend_citizen_internal(nullifier, until)` — called by pallet-courts on guilty verdict

**ZK proof format and verifier**: this section is stale as of the Rarimo→ZKPassport migration (see changelog 065-068 log #65) and intentionally not corrected here — the 129-byte Groth16 format below and the VK-asset TODO are Rarimo-specific and no longer the live target. **Update**: `runtime/src/verifier.rs` is no longer unstarted rework — it was rebuilt against ZKPassport's UltraHonk `outer/count_4` circuit and, as of changelog entry 72, performs a real bb 5.0.0 pairing check (see `docs/project/zk-verifier.md`); only a genuine end-to-end passport proof through it remains outstanding, gated on real NFC data. `mobile/src/chain/{sodParser,zkProving,proofEncoding}.ts` are also already reworked for ZKPassport (confirmed by reading them directly as part of this update, not carried over from an older log): `sodParser.ts`'s SOD-parsing logic was never Rarimo-specific and needed no rewrite; `zkProving.ts` defines a real `NoirProver` seam and fails loudly rather than silently, but has no native Noir prover module bound in yet (`zkpassport/noir_rs` is the identified building block, unwired); `proofEncoding.ts` builds the envelope `verifier.rs` expects (per `docs/project/zk-verifier.md`). `AnchorProofVerifier` (the new trait backing `register_citizen`'s anchor check, `reverify_citizen`, and `migrate_oprf_scheme`) is wired to `PassthroughAnchorVerifier` — a dev-mode stub, same shape as the pre-existing `PassthroughZkVerifier`, accepting every proof unconditionally — only in the `dev-mode` build path. The non-dev-mode build is wired to the real `crate::anchor_verifier::Poseidon2AnchorVerifier` (`runtime/src/anchor_verifier.rs`, ~674 lines), which genuinely recomputes the Poseidon2 `param_commitment` against the already-verified outer proof for all three methods (registration/reverification via `disclosure`, migration via `migrate-disclosure`; HANDOFF log #75/#76). Changelog entry 73 decided the committee governance model (5 independent committees) and entry 74 extended the anchor/disclosure/migrate circuits to match and assessed what a real verifier needs (no new pairing check for the `disclosure` path, but a from-scratch Rust Poseidon2 implementation this codebase did not yet have at the time — since built, per the above) — see changelog entries 73/74 for the historical "still needed" list.

Old Rarimo-era proof format (kept for historical reference only, not current):
```
[0..32]   A  G1 compressed (ark-serialize LE, flags in byte 31)
[32..96]  B  G2 compressed (ark-serialize LE, flags in byte 63)
[96..128] C  G1 compressed
[128]     variant: 0=SHA-256 circuit, 1=SHA-1 circuit
```

