# Code Review — Agora Democracy Chain

_Reviewed: 2026-06-20 · Branch: main (uncommitted working tree changes)_

---

## Critical Correctness Bugs

These must be fixed before any public deployment. Each one breaks a core democratic invariant or allows chain-level exploitation.

---

### 1. commit_vote accepts arbitrary nullifiers — breaks 1-person-1-vote

**File:** `pallets/pallet-voting/src/lib.rs:231`

`commit_vote` takes a caller-supplied `nullifier: [u8; 32]` but never checks that it matches the caller's registered nullifier in `CitizenNullifier` storage. The `AlreadyVoted` guard is keyed on `(proposal_id, nullifier)`, so one citizen can cast multiple vote commitments by supplying different nullifier values on each call.

**Scenario:** Alice calls `commit_vote(proposal_id=1, nullifier=X, commitment=C1)` then `commit_vote(proposal_id=1, nullifier=Y, commitment=C2)`. Both pass. The MACI off-chain tally counts both, giving Alice two votes.

**Fix:** Look up the caller's nullifier via `CitizenNullifier::<T>::get(&who)` and require the supplied nullifier to match it, or derive the nullifier from identity storage entirely rather than accepting it as a parameter.

---

### 2. Petitions can be triggered by non-citizens — Sybil referendum attack

**File:** `pallets/pallet-constitution/src/lib.rs:217` (also `submit_petition` at line 200)

Neither `submit_petition` nor `sign_petition` calls `CitizenChecker::is_active_citizen`. Any account can sign petitions. With cheap account creation and no citizenship requirement, an attacker can manufacture enough accounts to cross `PetitionThreshold` and auto-create a referendum with zero legitimate citizen participation.

**Scenario:** Attacker creates 1 000 fresh accounts and calls `sign_petition` once per account. The count reaches `PetitionThreshold`, `PetitionApprover::create_referendum` is called, and a referendum enters the voting queue based entirely on non-citizen signatures.

**Fix:** Add `ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive)` at the top of both `submit_petition` and `sign_petition`. Requires threading a `CitizenChecker` associated type through `pallet-constitution`'s `Config`.

---

### 3. finalize_ruling can deny the right to appeal

**File:** `pallets/pallet-courts/src/lib.rs:237`

`finalize_ruling` only checks `case.1 == CaseStatus::AIRulingIssued`. It does not verify that `AppealWindowBlocks` have elapsed since `AIRulingBlock`. The oracle can call `finalize_ruling` in the same block as `submit_ai_ruling`, triggering auto-enforcement (law paused, department frozen) before any citizen has a chance to appeal.

**Scenario:** Oracle calls `submit_ai_ruling` at block N → immediately calls `finalize_ruling` at block N. Status check passes. Case is finalized and enforced. A citizen who wanted to appeal at block N+1 finds the case already at `FinalRuling`.

**Fix:** In `finalize_ruling`, fetch `AIRulingBlock` and assert that `block_number() > ruling_block + AppealWindowBlocks` before allowing finalization.

---

### 4. ZK proof verified before public_inputs length check — potential panic

**File:** `pallets/pallet-identity/src/lib.rs:125`

`T::ZkVerifier::verify(...)` is called at line 125, but `ensure!(public_inputs.len() >= 6, ...)` is only checked at line 129. If the verifier returns `true` with fewer than 6 inputs, execution reaches `public_inputs[2][24..32]` at line 132 and panics (out-of-bounds on a `BoundedVec`).

**Scenario:** A buggy or future alternative `ZkVerifier` impl returns `true` on short input. Chain halts for any node that processes the extrinsic.

**Fix:** Move `ensure!(public_inputs.len() >= 6, Error::<T>::InvalidZKProof)` to before the `T::ZkVerifier::verify` call.

---

## Missing Features / Architectural Gaps

These are planned features that exist in CLAUDE.md but are absent or incomplete in the current code.

---

### 5. Separation of powers not enforced — all origins are EnsureRoot

**File:** `runtime/src/configs/mod.rs:198`

The `#[cfg(not(feature = "dev-mode"))]` block only swaps the ZK verifier. Every privileged origin — `SuspensionOrigin`, `OracleOrigin`, `LegislatureOrigin`, `HumanRightsOrigin` — is `EnsureRoot<AccountId>` in both dev and production builds. A single compromised sudo key can issue AI rulings, suspend citizens, enact laws, veto laws, and freeze departments unilaterally.

**Status:** All the planned role types (court multisig, legislature collective, HRC council) are `// TODO` comments in the pallets.

**Fix path:** Wire each origin to its appropriate collective or multisig. `LegislatureOrigin` → a referendum-weighted collective. `OracleOrigin` → a dedicated oracle account or committee. `HumanRightsOrigin` → an HRC collective. `SuspensionOrigin` → a court-controlled multisig. These don't need to be done at once but the production cfg gate should reject `EnsureRoot` for each one.

---

### 6. Jury selection entropy is manipulable by block authors

**File:** `pallets/pallet-courts/src/lib.rs:332`

`pick_random_jurors` uses `frame_system::Pallet::<T>::parent_hash()` as the sole entropy source (acknowledged in the code comment at line 325). A block author selecting a jury can withhold the block if the parent hash yields an unfavorable jury and retry on the next slot.

**Status:** The code comment already flags this. It is a known placeholder, not an oversight.

**Fix path:** Replace `parent_hash` with Babe epoch randomness (`pallet_babe::RandomnessFromOneEpochAgo`) or a commit-reveal scheme. This is a significant dependency addition but critical for any real deployment.

---

### 7. Amendment deliberation period ignores law tier

**File:** `pallets/pallet-constitution/src/lib.rs:184`

`ratify_amendment` unconditionally enforces `ConstitutionalDeliberationBlocks` (30 days) on all amendments, regardless of whether the law's tier is `Ordinary` or `Constitutional`. The law's `LawTier` field (`law.0`) is read in `propose_amendment` to check the law exists, but never consulted in `ratify_amendment` to select the appropriate deliberation window.

**Scenario:** A trivial Ordinary law amendment (e.g., correcting a typo in a regulation) is blocked for 30 days, same as a constitutional change.

**Fix:** Read `law.0` inside `ratify_amendment` and use a shorter `OrdinaryDeliberationBlocks` constant (or zero) for `LawTier::Ordinary`, reserving `ConstitutionalDeliberationBlocks` for `LawTier::Constitutional`.

---

### 8. No repeal path for laws

**File:** `pallets/pallet-constitution/src/lib.rs:30`

`LawStatus::Repealed` and `Event::LawRepealed` are defined but no extrinsic or internal function ever sets a law to `Repealed`. The only status transitions available are: `enact_law` → `Active`, `invalidate_law` / `veto_law` → `Paused`. A paused law can receive amendments; a repealed law should be terminal. Currently these two states are indistinguishable on-chain.

**Fix:** Add a `repeal_law(origin: OriginFor<T>, law_id: u32)` extrinsic gated on `LegislatureOrigin` that sets `LawStatus::Repealed` and emits `LawRepealed`. Guard it against re-repealing a law that is already `Repealed`.

---

### 9. submit_proposal has no citizenship check and no duration bounds

**File:** `pallets/pallet-voting/src/lib.rs:214`

Any signed account (not just registered citizens) can call `submit_proposal`. The caller-supplied `duration_blocks` is unchecked — a duration of `0` creates an instantly-expired proposal that can never receive votes; a duration of `u32::MAX` creates a proposal expiring ~272 years in the future with no prune path.

**Fix:** Add `ensure!(T::CitizenChecker::is_active_citizen(&who), Error::<T>::CitizenNotActive)`. Add `ensure!(duration_blocks >= MinProposalDuration::get() && duration_blocks <= MaxProposalDuration::get(), ...)` with appropriate constants, or simply enforce `type ReferendumDurationBlocks` rather than accepting it as a parameter.

---

## Desktop Bug

### 10. Auth QR polling interval leaks on rapid re-invocation

**File:** `desktop/src/context/AuthContext.tsx:52`

`requestQr` stores the `setInterval` handle as a local `const poll`. If the user clicks Login a second time before the 5-minute `setTimeout` fires, a new polling interval starts with no reference to the old one. The old interval continues running every 2 seconds until its own 5-minute timeout cancels it. Two overlapping poll loops issue concurrent `auth_poll_session` calls to the backend.

**Fix:** Promote the interval handle to a `useRef` and call `clearInterval(pollRef.current)` at the top of `requestQr` before starting a new one.

---

## Not Bugs (Investigated, Refuted)

- **revoke_citizen partial state removal** — initially flagged as a potential inconsistency when `CitizenNullifier::take` succeeds but `CitizenPosition::take` fails. REFUTED: Substrate FRAME wraps every extrinsic in a storage transaction; returning `Err` rolls back all mutations atomically.

- **QV saturating_mul cost bypass** — flagged as a potential free vote reallocation at extreme vote counts. REFUTED: reaching `u64::MAX` saturation requires `vote_count ≥ sqrt(u64::MAX) ≈ 4.3 billion`, which exceeds `u32::MAX` and would require more tokens than `u64` can represent.

---

## Also Noted (lower severity, not in top 10)

- `CaseStatus::Enforced` is defined but never set anywhere — dead code variant and event.
- `select_jury` and `delegate_vote` use static weights that don't account for the variable number of storage reads their inner loops perform. Should be benchmarked.
- `AgentContext.ask` sends stale `messages` history if the user submits two questions before the first response resolves — the second request's history omits the in-flight first question.
