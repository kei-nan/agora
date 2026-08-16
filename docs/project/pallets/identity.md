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
- `SuspendedByJuryReview`: `[u8;32]` → `bool` (`ValueQuery`, absent/false is the safe default) —
  whether the *current* entry in `SuspendedNullifiers` (if any) came from a jury-reviewed
  conviction (a `pallet-courts` case that reached `CaseStatus::JurySeated` and was decided by
  `cast_jury_vote`'s majority) as opposed to an unappealed AI ruling or the manual
  `suspend_citizen` extrinsic (both oracle-only, no jury involved). Written alongside
  `SuspendedNullifiers` everywhere the latter is written, and cleared alongside it everywhere
  it's cleared (`revoke_citizen`, `restore_citizen_rights`, and the lazy-expiry branches of
  `is_active_citizen`/`is_suspended_by_jury_reviewed_conviction` — all four go through a shared
  private `clear_suspension` helper so the two maps can't desync). Consumers that need a higher
  evidentiary bar before acting on a suspension — e.g. `pallet-executive`'s conviction-triggered
  office-vacancy sweep, which deliberately does not auto-remove a sitting PM/minister on a bare,
  unappealed AI ruling — check this via `is_suspended_by_jury_reviewed_conviction`, not
  `SuspendedNullifiers` alone.
- `AllowedMerkleRoots`: set of valid passport Merkle roots (see the certificate-registry rework in changelog 065-068 — no longer Rarimo-sourced)
- `OprfSchemeVersion`: `u32` — current active identity-anchor scheme generation (see changelog 065-068, logs #67/#68)
- `IdentityAnchorRegistry`: `(scheme_version, anchor: [u8;32])` → `AccountId` — Sybil-resistance exclusion-check registry, deliberately separate from `NullifierRegistry`/`CitizenNullifier`; no event ever emits an anchor value, so it can never be correlated with voting activity
- `CitizenAnchor`: `AccountId` → `(scheme_version, anchor)` — reverse lookup, used by `migrate_oprf_scheme`
- `ReverificationDeadline`: `AccountId` → `BlockNumber` — periodic re-verification tracking
- `SelfDeclaredSingleDocument`: `AccountId` → `bool` — self-declaration attestation (courts backstop, see below)
- `CommitteeMembers`: `slot (0..NUM_COMMITTEES)` → `BoundedVec<AccountId, MaxCommitteeSize>` — OPRF committee roster per slot (changelog #82/#83 founding-phase node architecture)
- `OprfCommitteeKeys`: `(scheme_version, slot)` → `[u8; 32]` — governance-approved committee public-key hash per slot
- `NextQueryId` / `PendingOprfQueries`: `query_id` → the on-chain OPRF query mailbox a citizen posts a blinded query to (`OprfQueryRecord { submitter, blinded_query: [u8;64], posted_at }`)
- `OprfRound1Commitments`: `(query_id, committee_slot)` → `BoundedVec<OprfRound1Commitment, OprfThreshold>` (`ValueQuery`) — round-1 partial evaluation + FROST-style nonce commitments (`member, r_i, d_g, d_q, e_g, e_q`, all `[u8;64]`) submitted so far for this pair, in submission order. Reaching exactly `OprfThreshold` entries locks the qualifying set.
- `OprfRound2Responses`: `(query_id, committee_slot)` → `BoundedVec<OprfRound2Response, OprfThreshold>` (`ValueQuery`) — round-2 response scalars (`member, z_i: [u8;32]`), same shape/locking pattern as `OprfRound1Commitments`

Calls (params reflect the pallet's current structure post-#75/#76 restructuring):
- `register_citizen(zk_proof [≤4096 bytes], public_inputs [≤18 × [u8;32]], anchor, oprf_pk_hashes: [[u8;32]; NUM_COMMITTEES])`
  - Verifies the ZKPassport outer proof via `ZkVerifier`; the nullifier is *not* a separate
    argument — it's extracted directly from the proof's own public inputs
    (`scoped_nullifier`, see the module doc comment)
  - Checks passport expiry and country allowlist from public inputs, same as before
  - Freshness (`check_outer_proof_freshness`, shared with `reverify_citizen`/
    `migrate_oprf_scheme`): rejects a proof whose `current_date` public input is more than
    `MaxAnchorProofAge` older than chain time (`AnchorProofStale`) *and*, as of commit `0034e33`,
    more than `MaxAnchorProofClockSkew` in the future (`AnchorProofFuture`). `current_date` is a
    fully prover-controlled public input — the in-circuit check only constrains it against the
    passport's own expiry, never against real time — so before this upper-bound check, a single
    future-dated proof would make the staleness check's `now.saturating_sub(current_date)` clamp
    to 0 and pass the freshness check forever, keeping a citizen "verified" indefinitely off one
    proof.
  - Requires a mandatory OPRF identity-anchor check (`anchor` + `oprf_pk_hashes`, verified via
    `AnchorVerifier`) as the Sybil-resistance gate, rejecting if `anchor` already exists under
    the current `OprfSchemeVersion` in `IdentityAnchorRegistry`
- `revoke_citizen()` — swap-and-pop, clears suspension (both `SuspendedNullifiers` and `SuspendedByJuryReview`)
- `suspend_citizen(nullifier, until)` — `SuspensionOrigin` (EnsureRoot placeholder); always writes `SuspendedByJuryReview = false` — this extrinsic-driven path is oracle-only, never jury-reviewed
- `restore_citizen_rights(nullifier)` — `SuspensionOrigin`; clears both `SuspendedNullifiers` and `SuspendedByJuryReview`
- `add_allowed_merkle_root(root)` / `remove_allowed_merkle_root(root)` — `AdminOrigin`
- `reverify_citizen(proof)` — any registered citizen; extends `ReverificationDeadline` via `AnchorVerifier::verify_reverification`; a citizen past their deadline is treated as inactive by `is_active_citizen` (lazy check, no background sweep)
- `migrate_oprf_scheme(zk_proof, public_inputs, new_anchor, old_oprf_pk_hashes,
  new_oprf_pk_hashes)` — dual-evaluation OPRF-scheme rotation migration; targets the caller's own
  on-file scheme version + 1. Verifies the outer proof and its freshness, then checks committee
  keys and the migration proof itself, and only *after* all of that checks whether `new_anchor`
  is already taken (`NewAnchorAlreadyUsed`) — this verify-then-check ordering was a fix (commit
  `0034e33`): checking `NewAnchorAlreadyUsed` first would have let an attacker submit a bogus
  `zk_proof` with a guessed `new_anchor` and learn, from the returned error alone and at zero
  real proof-computation cost, whether that `(new_version, new_anchor)` pair already belongs to
  another citizen — leaking cross-citizen anchor-registry membership ahead of proof
  authentication.
- `rotate_oprf_scheme()` — `AdminOrigin`; scheduled-path advance of `OprfSchemeVersion` (the ~4-year cycle)
- `emergency_rotate_oprf_scheme()` — `EmergencyRotationOrigin`; out-of-cycle rotation if the current OPRF scheme is suspected broken. As of this session, wired in the runtime to `pallet_emergency_council::EnsureActiveEmergency<Runtime>` — **not** the bare `EnsureRoot` placeholder earlier versions of this doc described. Succeeds only when the caller is `Root` *and* `pallet_emergency_council::ActiveEmergency` is currently `Some(..)` (a real, council-declared, not-yet-lifted-or-expired emergency); root alone can no longer force this call. See `pallets/pallet-emergency-council/src/lib.rs`'s `EnsureActiveEmergency` doc comment and `runtime/src/configs/mod.rs` for the full wiring, and this pallet's `EmergencyRotationOrigin` Config-field doc comment above for the rationale.
- `declare_no_other_passport()` — any registered citizen; records a self-declaration attestation used only as an ex-post basis for a `pallet-courts` `CitizenConduct` case if later found false
- `set_oprf_committee_key(scheme_version, slot, oprf_pk_hash)` / `remove_oprf_committee_key(scheme_version, slot)` — `AdminOrigin`; governance-approved committee key management
- `add_committee_member(slot, who)` / `remove_committee_member(slot, who)` — `AdminOrigin`; committee roster management (changelog #82/#83)
- `submit_oprf_query(blinded_query: [u8; 64])` — any registered citizen (`is_citizen` gate); posts a query to the on-chain mailbox and assigns/emits a fresh `query_id`; committee members for the target slot (derived off-chain via `committee_slot_for`) poll `PendingOprfQueries` and answer via `submit_oprf_round1`/`submit_oprf_round2`
- `submit_oprf_round1(query_id, committee_slot, r_i: [u8;64], d_g: [u8;64], d_q: [u8;64], e_g: [u8;64], e_q: [u8;64])` — round 1 of a genuine `t`-of-`n` threshold OPRF evaluation (Option B, `docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`); caller must be on `CommitteeMembers[committee_slot]`; checks slot validity, query existence/expiry (`OprfQuerySlaBlocks`), and no duplicate submission from this caller for this pair; once the `OprfThreshold`-th commitment lands the qualifying set locks (`OprfRound1SetLocked`) and further round-1 submissions for that pair are rejected. Performs no cryptographic verification of the submitted points itself — a deliberate, documented scope boundary (see `OprfRound1Commitment`'s doc comment), not an oversight
- `submit_oprf_round2(query_id, committee_slot, z_i: [u8; 32])` — round 2; requires round 1 already locked (exactly `OprfThreshold` commitments) and caller to be one of that locked set's members; same no-crypto-verification scope boundary as round 1. Once every member of the locked set has submitted, the citizen's own client combines the round-1/round-2 data into the final proof off-chain (`oprf-committee-dev::threshold::combine_evaluations`/`combine_responses`) — the pallet never computes or stores that combination itself

`AdminOrigin`, like `pallet-legislature::EnsureLegislatureMotion` elsewhere, is `EnsureOriginWithArg<_, [u8; 32]>`: each call above passes a domain-separated hash of its own parameters, checked against the specific motion that authorized it, so one passed motion can't be replayed to authorize a different call.

Public helpers:
- `is_active_citizen(who)` — registered AND no active suspension AND not past `ReverificationDeadline`
- `is_citizen(who)` — registered regardless of suspension
- `citizen_at(index)` / `total_citizens()` — for jury selection
- `is_suspended_by_jury_reviewed_conviction(who)` — true only if currently suspended (same lazy-expiry semantics as `is_active_citizen`) AND that suspension's `SuspendedByJuryReview` flag is set; used where a higher evidentiary bar than a bare AI ruling is required (see `SuspendedByJuryReview`'s storage entry above)
- `suspend_citizen_internal(nullifier, until, jury_reviewed)` — called by pallet-courts (via the `CitizenSuspender` runtime trait) when a conduct ruling is finalized; `jury_reviewed` reflects which path `auto_finalize` took to get here (jury majority vs. an unappealed AI ruling) and is written straight into `SuspendedByJuryReview`

**ZK proof format and verifier**: this section is stale as of the Rarimo→ZKPassport migration (see changelog 065-068 log #65) and intentionally not corrected here — the 129-byte Groth16 format below and the VK-asset TODO are Rarimo-specific and no longer the live target. **Update**: `runtime/src/verifier.rs` is no longer unstarted rework — it was rebuilt against ZKPassport's UltraHonk `outer/count_4` circuit and, as of changelog entry 72, performs a real bb 5.0.0 pairing check (see `docs/project/zk-verifier.md`); only a genuine end-to-end passport proof through it remains outstanding, gated on real NFC data. `mobile/src/chain/{sodParser,zkProving,proofEncoding}.ts` are also already reworked for ZKPassport (confirmed by reading them directly as part of this update, not carried over from an older log): `sodParser.ts`'s SOD-parsing logic was never Rarimo-specific and needed no rewrite; `zkProving.ts` defines a real `NoirProver` seam and fails loudly rather than silently, but has no native Noir prover module bound in yet (`zkpassport/noir_rs` is the identified building block, unwired); `proofEncoding.ts` builds the envelope `verifier.rs` expects (per `docs/project/zk-verifier.md`). `AnchorProofVerifier` (the new trait backing `register_citizen`'s anchor check, `reverify_citizen`, and `migrate_oprf_scheme`) is wired to `PassthroughAnchorVerifier` — a dev-mode stub, same shape as the pre-existing `PassthroughZkVerifier`, accepting every proof unconditionally — only in the `dev-mode` build path. The non-dev-mode build is wired to the real `crate::anchor_verifier::Poseidon2AnchorVerifier` (`runtime/src/anchor_verifier.rs`, ~674 lines), which genuinely recomputes the Poseidon2 `param_commitment` against the already-verified outer proof for all three methods (registration/reverification via `disclosure`, migration via `migrate-disclosure`; HANDOFF log #75/#76). Changelog entry 73 decided the committee governance model (5 independent committees) and entry 74 extended the anchor/disclosure/migrate circuits to match and assessed what a real verifier needs (no new pairing check for the `disclosure` path, but a from-scratch Rust Poseidon2 implementation this codebase did not yet have at the time — since built, per the above) — see changelog entries 73/74 for the historical "still needed" list.

Old Rarimo-era proof format (kept for historical reference only, not current):
```
[0..32]   A  G1 compressed (ark-serialize LE, flags in byte 31)
[32..96]  B  G2 compressed (ark-serialize LE, flags in byte 63)
[96..128] C  G1 compressed
[128]     variant: 0=SHA-256 circuit, 1=SHA-1 circuit
```

