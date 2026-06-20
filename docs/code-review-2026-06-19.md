# Code Review — Democracy Chain Pallets
**Date:** 2026-06-19
**Scope:** Uncommitted working-tree changes — 5 new pallets + runtime wiring
**Files reviewed:** `pallets/pallet-identity`, `pallets/pallet-voting`, `pallets/pallet-treasury-ledger`, `pallets/pallet-courts`, `pallets/pallet-constitution`, `runtime/src/configs/mod.rs`, `runtime/src/lib.rs`

---

## Findings

Ranked most-severe first. All 10 are confirmed bugs (the cycle-depth finding is marked PLAUSIBLE due to a minor framing caveat, but the underlying vulnerability is real).

---

### 1. `appeal_ruling` — appeal window is entirely unenforced
**File:** [pallets/pallet-courts/src/lib.rs:168](../pallets/pallet-courts/src/lib.rs#L168)
**Severity:** Critical

`AppealWindowBlocks` is declared in `Config` (line 73) and configured in the runtime as `7 * DAYS`, and the `AppealWindowClosed` error variant exists (line 122) — but `appeal_ruling` only checks `case.1 == CaseStatus::AIRulingIssued`. The constant is never read anywhere in the pallet body.

Compounding problem: the block at which the AI ruling was issued is never stored. The `Cases` tuple is `(AccountId, CaseStatus, Option<[u8;32]>, CaseSubject)` with no timestamp field, so even adding the deadline check would require a storage schema change.

**Failure scenario:** An AI ruling issued at block 100 can be appealed at block 10,000,000. The 7-day window is a dead constant. `AppealWindowClosed` is unreachable.

**Fix:** Add `ruling_issued_at: BlockNumberFor<T>` to the `Cases` storage tuple when `submit_ai_ruling` fires. In `appeal_ruling`, check:
```rust
ensure!(
    frame_system::Pallet::<T>::block_number() <=
        ruling_issued_at + BlockNumberFor::<T>::from(T::AppealWindowBlocks::get()),
    Error::<T>::AppealWindowClosed
);
```

---

### 2. `PassthroughZkVerifier` — accepts all proofs in all build profiles
**File:** [runtime/src/configs/mod.rs:166](../runtime/src/configs/mod.rs#L166)
**Severity:** Critical

`PassthroughZkVerifier::verify` unconditionally returns `true`. It is wired as `type ZkVerifier = PassthroughZkVerifier` in the runtime with no `#[cfg(debug_assertions)]`, no cargo feature gate, and no compile-time guard. A release build accepts any ZK proof bytes (including empty).

**Failure scenario:** An attacker calls `register_citizen` with an arbitrary nullifier and zero-length `zk_proof` bytes on a production binary. The call succeeds and grants full voting, budget, and jury-eligibility rights without a real passport. All downstream identity checks treat the fake citizen as legitimate.

**Fix:** Gate the passthrough behind a feature flag so a release build cannot compile without a real verifier:
```rust
#[cfg(feature = "dev-mode")]
pub struct PassthroughZkVerifier;
#[cfg(feature = "dev-mode")]
impl pallet_identity_zk::ZkProofVerifier for PassthroughZkVerifier { ... }
```
And fail to compile `impl pallet_identity_zk::Config for Runtime` without a concrete verifier when `dev-mode` is absent.

---

### 3. `DelegationCap` — percentage cap is a dead config constant
**File:** [pallets/pallet-voting/src/lib.rs:26](../pallets/pallet-voting/src/lib.rs#L26)
**Severity:** High

`DelegationCap: Get<u8>` is declared in `Config` (line 26) and configured as `ConstU8<33>` with the comment "No single delegate may hold more than 33% of voting power." However, `T::DelegationCap::get()` is never called anywhere in the pallet. `delegate_vote` only enforces `MaxDelegationsPerDelegate` — an absolute count ceiling of 1,000.

**Failure scenario:** With 200 registered citizens, a single coordinated actor accumulates all 200 delegations on every topic (well under the 1,000 count ceiling), controlling 100% of legislative voting power. The constitutional 33% cap enforces nothing. The constant appears in the runtime as a `#[pallet::constant]`, so external callers and governance tooling see it as an active invariant when it is not.

**Fix:** Either implement the percentage check (requires a hook to get total citizen count into the voting pallet, e.g., via a `CitizenCounter` associated type), or remove `DelegationCap` from `Config` and rename `DelegationCapExceeded` to `MaxDelegatorsExceeded` to reflect what is actually enforced.

---

### 4. `propose_amendment` — no access control, silently resets deliberation clock
**File:** [pallets/pallet-constitution/src/lib.rs:108](../pallets/pallet-constitution/src/lib.rs#L108)
**Severity:** High

Three problems at the same call site:
1. `ensure_signed` accepts any account. `UnauthorizedAmendment` error exists (line 69) but is never returned.
2. `PendingAmendments::insert` silently overwrites any existing pending amendment, resetting `proposed_at` to the current block. There is no guard checking `PendingAmendments::contains_key`.
3. Only `Laws::contains_key` is checked — a `Paused` (court-invalidated) or `Repealed` law can receive amendments.

**Failure scenario:** Alice proposes a constitutional amendment at block 1000 (30-day deliberation required). At block 1001, Bob (any account) calls `propose_amendment` on the same `law_id` with any hash; `PendingAmendments::insert` overwrites Alice's proposal and resets the clock to block 1001. Repeating once per day makes ratification permanently unreachable.

**Fix:**
- Restrict origin (legislature-controlled origin or at minimum `ensure_root`).
- Guard with `ensure!(!PendingAmendments::contains_key(law_id), ...)` or explicitly require the caller to withdraw the existing amendment first.
- Add `LawStatus::Active` check before accepting the proposal.

---

### 5. `finalize_ruling` — no status guard, verdicts can be overwritten
**File:** [pallets/pallet-courts/src/lib.rs:201](../pallets/pallet-courts/src/lib.rs#L201)
**Severity:** High

`finalize_ruling` reads the case with `Cases::get`, then unconditionally transitions status to `FinalRuling` and inserts a verdict via `Rulings::insert`. There is no check on `case.1` before proceeding. `AlreadyRuled` error exists but is never returned here.

**Failure scenario (a):** Root calls `finalize_ruling(case_id, Overturned)` on a `Filed` case (no AI ruling issued). Auto-enforcement fires immediately — a law is paused or a department frozen with no underlying ruling.

**Failure scenario (b):** Root calls `finalize_ruling` a second time on an already-`FinalRuling` case. `Rulings::insert` overwrites the original verdict and enforcement fires again. If the subject law was already `Paused`, `invalidate_law_internal` silently sets a `Repealed` law back to `Paused` (see finding #10).

**Fix:** Add a status guard at the top of `finalize_ruling`:
```rust
ensure!(
    case.1 == CaseStatus::AIRulingIssued || case.1 == CaseStatus::InJuryAppeal,
    Error::<T>::InvalidStatus
);
```

---

### 6. `has_delegation_cycle` — cycles longer than `MaxDelegationDepth` hops are undetectable
**File:** [pallets/pallet-voting/src/lib.rs:309](../pallets/pallet-voting/src/lib.rs#L309)
**Severity:** High (PLAUSIBLE)

The cycle-detection loop runs for exactly `MaxDelegationDepth` iterations (configured as 10). If a cycle exists but spans more than 10 hops, the loop exhausts its iterations and returns `false`, silently accepting the delegation that closes the cycle.

**Failure scenario:** A ring of 11 delegates (A→B→C→…→K→A) on topic 0: each individual link passes the cycle check (10 hops never reaches the origin within the limit). The 11th link is accepted. Any future on-chain or off-chain process walking the full delegation chain for transitive vote weight computation enters an infinite loop (or exhausts block weight).

**Fix:** Return an error — not `false` — when the depth limit is reached without finding a clean chain end:
```rust
// At end of loop (depth exhausted without finding a clean None):
true  // conservatively treat as potential cycle and reject
```
Or return a dedicated `DelegationChainTooDeep` error from `delegate_vote` when `has_delegation_cycle` hits the depth limit without terminating cleanly.

---

### 7. `is_active_citizen` — suspension expiry is off by one block
**File:** [pallets/pallet-identity/src/lib.rs:206](../pallets/pallet-identity/src/lib.rs#L206)
**Severity:** Medium

```rust
Some(Some(until)) => frame_system::Pallet::<T>::block_number() >= until,
```

The storage comment (line 61) reads "Some(block) means suspended until that block." Under this semantic, block `until` is still within the suspension period — the citizen should be active starting at block `until + 1`. The `>=` comparison makes the citizen active at block `until` itself, one block too early.

**Failure scenario:** A citizen suspended until block 1000 can call `commit_vote` or `claim_fiscal_year_tokens` at block 1000 — one block before the sentence has legally served.

**Fix:** Change `>= until` to `> until`.

---

### 8. `revoke_citizen` — `.expect()` panics WASM executor on storage inconsistency
**File:** [pallets/pallet-identity/src/lib.rs:131](../pallets/pallet-identity/src/lib.rs#L131)
**Severity:** Medium

```rust
let pos = CitizenPosition::<T>::take(&who).expect("position exists if nullifier exists");
```

`CitizenNullifier::take` on line 127 already succeeded and irreversibly removed the nullifier entry. If `CitizenPosition` is missing (e.g., after a runtime upgrade with a partial storage migration), the `expect` panics the WASM executor — leaving the citizen half-revoked with the nullifier gone but the position index corrupted, and the block including that extrinsic failing to finalize.

**Fix:**
```rust
let pos = CitizenPosition::<T>::take(&who).ok_or(Error::<T>::NotRegistered)?;
```

---

### 9. `select_jury` — case status not updated, jury pool can be overwritten
**File:** [pallets/pallet-courts/src/lib.rs:184](../pallets/pallet-courts/src/lib.rs#L184)
**Severity:** Medium

After selecting a jury, the case status remains `InJuryAppeal`. A subsequent call to `select_jury` passes the status check again and overwrites `JuryPool` with a freshly randomized jury via `JuryPool::insert` (no guard).

**Failure scenario:** An adversary submits `select_jury` in consecutive blocks until a statistically favorable jury composition appears (block hash changes each block, so the draw changes). Previously notified jurors are silently replaced with no on-chain record of the re-draw.

**Fix:** Transition the case to a new `JurySeated` or `AwaitingJuryVerdict` status inside `select_jury`, and add `ensure!(case.1 == CaseStatus::InJuryAppeal, Error::<T>::InvalidStatus)` that rejects re-selection once a jury is seated.

---

### 10. `invalidate_law_internal` — silently resurrects repealed laws
**File:** [pallets/pallet-constitution/src/lib.rs:145](../pallets/pallet-constitution/src/lib.rs#L145)
**Severity:** Medium

`invalidate_law_internal` sets `law.1 = LawStatus::Paused` with no check on the current status. The `LawNotActive` error (line 66) exists but is never returned here or in the public `invalidate_law` dispatchable.

**Failure scenario:** A court case targets law_id 5, which has already been repealed. `finalize_ruling` calls `invalidate_law_internal(5)`. The `try_mutate` succeeds (the key exists), sets status from `Repealed` to `Paused`, and emits `LawInvalidated`. A dead repealed law now appears live-but-paused in storage. Any UI or on-chain query expecting `LawStatus::Repealed` instead sees `Paused`, silently re-enabling the law pending legislature review.

**Fix:** Add a status guard in both `invalidate_law_internal` and the `invalidate_law` dispatchable:
```rust
ensure!(law.1 == LawStatus::Active, Error::<T>::LawNotActive);
```

---

## Summary Table

| # | File | Line | Summary | Severity |
|---|------|------|---------|----------|
| 1 | `pallets/pallet-courts/src/lib.rs` | 168 | `appeal_ruling` never checks `AppealWindowBlocks`; window entirely unenforced | Critical |
| 2 | `runtime/src/configs/mod.rs` | 166 | `PassthroughZkVerifier` in all build profiles; anyone registers as citizen | Critical |
| 3 | `pallets/pallet-voting/src/lib.rs` | 26 | `DelegationCap` (33%) never read; only absolute delegator count enforced | High |
| 4 | `pallets/pallet-constitution/src/lib.rs` | 108 | Any account resets deliberation clock; `propose_amendment` has no access control | High |
| 5 | `pallets/pallet-courts/src/lib.rs` | 201 | `finalize_ruling` no status guard; verdicts overwriteable; fires enforcement on Filed cases | High |
| 6 | `pallets/pallet-voting/src/lib.rs` | 309 | Cycles >10 hops undetectable; delegation graph silently corrupted | High |
| 7 | `pallets/pallet-identity/src/lib.rs` | 206 | Suspension expiry off-by-one (`>=` should be `>`); active one block early | Medium |
| 8 | `pallets/pallet-identity/src/lib.rs` | 131 | `.expect()` panics WASM executor on storage inconsistency in `revoke_citizen` | Medium |
| 9 | `pallets/pallet-courts/src/lib.rs` | 184 | `select_jury` leaves status as `InJuryAppeal`; jury pool overwriteable | Medium |
| 10 | `pallets/pallet-constitution/src/lib.rs` | 145 | `invalidate_law_internal` sets `Paused` with no status guard; resurrects repealed laws | Medium |

## Recommended Fix Order

1. **Finding #2** — Gate `PassthroughZkVerifier` behind a cargo feature before any testnet deployment.
2. **Finding #1** — Store `ruling_issued_at` in `Cases` and enforce the appeal window in `appeal_ruling`.
3. **Finding #5** — Add status guard to `finalize_ruling` to prevent verdict overwrites.
4. **Finding #4** — Restrict `propose_amendment` origin and guard against overwriting pending amendments.
5. **Finding #3** — Implement or remove `DelegationCap` percentage enforcement.
6. **Findings #6–10** — Address before any public testnet launch.
