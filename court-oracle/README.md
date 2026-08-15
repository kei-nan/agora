# `court-oracle`

The off-chain AI-ruling oracle for `pallet-courts`. Polls on-chain `Cases` for
`CaseStatus::Filed` entries, builds a case-appropriate context from other on-chain storage, asks
Claude for a Level-0 AI ruling (see `/CLAUDE.md`'s "Court System (AI-First)" section), publishes
the full reasoning document to IPFS, and submits `submit_ai_ruling` (case_id, ruling_hash,
model_version, verdict) signed by a configured oracle account — the verdict is committed
on-chain at this point. Also polls `Cases` for `CaseStatus::AIRulingIssued` entries whose appeal
window has closed unappealed, and submits the second, separate `finalize_ruling(case_id)` call
that actually applies enforcement (pausing a law, freezing a department, suspending a citizen)
— see "`finalize_ruling` scheduling, and the verdict-binding fix" below.

**Read this whole file before running this anywhere real.** Large parts of the live-integration
path (chain RPC, the Claude API call, IPFS publishing) have never been exercised against a real
endpoint in the environment this was built in — see "What's real vs. assumed" below.

## A note on which tree this was built against

This crate was originally written inside a git worktree whose checked-out history stopped at
changelog entry #081, predating entries #082/#083 (the `committee-node`/`committee/` off-chain
components) and a set of `pallet-courts` additions: `CurrentAIModelVersion`/
`AIGovernanceCouncil`/`vote_approve_ai_model` governance gating `submit_ai_ruling`, and a
`CaseFilingBond` spam-prevention deposit on `file_case`. That gap was found and reconciled during
merge into the main tree: `submit_ai_ruling` really does take four arguments —
`(case_id: u32, ruling_hash: [u8; 32], model_version: u32, verdict: Verdict)` — and
`extrinsic.rs`/`main.rs` now match it (the `verdict` argument was added later still, by the fix
described in "`finalize_ruling` scheduling, and the verdict-binding fix" below). `main.rs`'s
`poll_once` reads `Courts::CurrentAIModelVersion` fresh from chain at the
start of every poll cycle and skips the cycle entirely (rather than ruling on cases it can't
submit for) if it's still `0` (no AI model ever governance-approved) — this was never exercised
against a live chain in this sandboxed environment, only unit-tested at the call-encoding level
(see `extrinsic.rs`'s test).

## What's real here

- **The JSON-RPC chain client** (`src/rpc.rs`): a real, working port of
  `desktop/src-tauri/src/rpc.rs`'s approach — raw JSON-RPC via `reqwest`,
  `twox128`/`blake2_128` storage-key hashing, `state_getKeysPaged`/`state_queryStorageAt`/
  `state_getStorage`, plus the write-side calls the read-only desktop app never needed
  (`state_getRuntimeVersion`, `chain_getBlockHash`, `system_accountNextIndex`,
  `author_submitExtrinsic`).
- **A real, independently-found bug, since fixed.** While porting `twox128_hex`, a known-answer
  check against `twox128("System") == "26aa394eea5630e07c48ae0c9558cef7"` (a standard Substrate
  reference vector) failed against `desktop/src-tauri/src/rpc.rs`'s formula as it stood in the
  worktree this crate was originally built in: `format!("{:016x}{:016x}", r0.to_le(),
  r1.to_le())`. `u64::to_le()` is a no-op on any little-endian host for the *numeric value*, and
  `{:016x}` prints a value's ordinary big-endian hex digits regardless — neither step produces
  the little-endian *byte sequence* TwoX-128 needs, silently breaking every desktop RPC command
  that reads chain storage. This crate's own `twox128_hex` uses `to_le_bytes()` + byte-by-byte
  hex encoding instead, which matches the known vector (see the test in `rpc.rs`). The desktop
  bug itself has since been fixed directly (`desktop/src-tauri/src/rpc.rs`, with its own
  regression test against the same reference vector) — found and flagged by this crate, fixed
  separately.
- **Extrinsic construction and signing** (`src/extrinsic.rs`): a real, hand-encoded
  `UncheckedExtrinsic`, sr25519-signed with `sp-core` pinned to the exact version (`36.1.0`) the
  runtime itself uses. The envelope (nonce, era, spec/tx version, genesis hash, the
  `TxExtension` tuple shape) is transcribed directly from `runtime/src/lib.rs`, confirmed
  identical to the version `desktop`/`oprf-committee-dev` already assume elsewhere in this
  codebase. The *call* itself (`pallet_courts` index 11, `submit_ai_ruling` call index 1, its
  4-argument `(case_id, ruling_hash, model_version, verdict)` shape) was read directly from
  `pallets/pallet-courts/src/lib.rs`, not guessed. Same for `finalize_ruling` (call index 4,
  just `(case_id,)` — no verdict argument, see below) — `build_signed` is generic over a small
  `CourtsCall` trait so both calls share one envelope/signing implementation.
- **Case-context building from a decoded `CaseSubject`** (`src/context.rs`): real, pure, unit
  tested for all four subject variants (`General`, `LawChallenge`, `TreasuryDispute`,
  `CitizenConduct`), including the honest "no further on-chain context exists" cases and the
  "record not found" cases (missing law, missing IPFS content).
- **Claude request/response formatting and ruling parsing** (`src/claude.rs`): the
  `VERDICT:`/`REASONING:` parser, user-message formatting, and response-JSON deserialization are
  real and unit tested — including against a literal fixture shaped like a real Anthropic
  Messages API response (thinking block + text block + `stop_reason`), and a `stop_reason:
  "refusal"` fixture (this service refuses to invent a ruling when Claude's safety classifiers
  decline — see the code).
- **IPFS CID math** (`src/ipfs.rs`): `cidv0_to_digest`/`digest_to_cidv0` are real, pure, and unit
  tested — they're the exact inverse of `desktop/src-tauri/src/commands/chain.rs`'s
  `hash_to_cid`, confirmed by reading that function, not re-derived independently.
- **SCALE mirror types** (`src/cases.rs`): `CaseStatus`/`CaseSubject`/`LawTier`/`LawStatus`/
  `AuditStatus`/`AuditEntry` are hand-written mirrors of the real pallet enums/structs, each
  variant/field order transcribed directly from the pallet source in this tree (cited per-type
  in the doc comments). `CaseStatus`/`CaseSubject` have a round-trip Encode→Decode test proving
  the mirror's variant ordering actually decodes what encoding would produce.
- **Key file decryption** (`src/keystore.rs`): the same real `age` (age-encryption.org/v1)
  passphrase-decryption pattern this codebase uses elsewhere for a service-held signing key —
  see that module's own honest tamper-resistance caveat.

## What's assumed / never executed

- **No live chain, live Claude API, or live IPFS daemon was reachable in the environment this
  was built in.** `main.rs`'s orchestration loop (`poll_once`/`rule_on_case`/the storage-scanning
  helpers), `ClaudeClient::rule`'s actual HTTP call, and `IpfsClient::add`'s actual HTTP call have
  never been run against anything real — only compiled. Everything above marked "real" is real in
  the sense of "compiles, and its pure logic is unit tested against literal fixtures," not
  "observed working end-to-end."
- **A real deployment needs `Courts::set_oracle_account` called by root**, naming this service's
  signing account, before any `submit_ai_ruling` it signs will be accepted. This service does not
  do that itself — it's a governance/root action outside this service's job (per the task this
  was built against), and this README says so rather than silently assuming it's been done.
- **A real deployment needs a reachable IPFS daemon with its HTTP API exposed** at
  `IPFS_API_URL` (default `http://127.0.0.1:5001`, the standard Kubo default). No IPFS publishing
  client existed anywhere in this codebase before this crate — confirmed by grep; every existing
  consumer (`desktop`'s `fetch_ipfs_content`) only *reads* content given an already-known hash.
  Alternatives (a hosted pinning service, e.g.) were not evaluated — see the honest note in
  `ipfs.rs`'s header comment and below.

## A real mismatch this crate cannot resolve on its own: `ruling_hash`

`submit_ai_ruling`'s own doc comment calls `ruling_hash` "the IPFS CID of the full reasoning
document," but the on-chain field is a fixed `[u8; 32]`, not a CID string. Every existing reader
in this codebase (`desktop`'s `hash_to_cid`) treats that 32-byte value as a **raw SHA-256 digest**
and re-derives a CIDv0 from it (`0x12 0x20` multihash header + base58) to fetch from a gateway.
That convention only round-trips if the content was added to IPFS as a single *raw* block — but a
standard `ipfs add` wraps content in a UnixFS/`dag-pb` structure even for one small file, so the
multihash digest inside the CID `ipfs add` actually returns is `sha256(dag-pb-wrapped bytes)`, not
`sha256(plaintext document bytes)`.

This crate picks the option that keeps `desktop`'s existing `fetch_ipfs_content` working: it
submits the digest bytes extracted from whatever CID `ipfs add` returns, not a hash of the
plaintext document. The cost — nobody can verify `ruling_hash` by hashing the plaintext
themselves; verifying means re-fetching from IPFS or re-running `ipfs add` and comparing CIDs. See
`ipfs.rs`'s header comment for the full reasoning. This was not evaluated against alternatives
(e.g. a hosted pinning service instead of a local daemon assumption) — flagged as unevaluated
rather than silently decided.

## `finalize_ruling` scheduling, and the verdict-binding fix

`submit_ai_ruling` starts the 7-day appeal clock (`AIRulingBlock`). Reading
`pallets/pallet-courts/src/lib.rs` closely: the actual verdict that drives enforcement (pause a
law, freeze a department, suspend a citizen) is only ever applied by `auto_finalize`, called
from either `cast_jury_vote` (the appeal path, deriving its verdict independently from real
jury votes) or `finalize_ruling` — a *separate* oracle-signed call, only callable once the
7-day appeal window has closed with no appeal filed.

In other words: `submit_ai_ruling` alone never enforces anything. A real deployment needs a
second call — `finalize_ruling`, signed by the oracle, sent again after the appeal window closes
— for an unappealed AI ruling to actually take legal effect. **This scheduling gap is
implemented**: `poll_once` (`src/main.rs`) has a second branch, alongside the existing
`CaseStatus::Filed` handling, for cases in `CaseStatus::AIRulingIssued`. `finalize_ruling` is
gated by the *same* `T::OracleOrigin` as `submit_ai_ruling` (see `EnsureOracle` in
`pallets/pallet-courts/src/lib.rs`), so this service's existing oracle signing key — already
loaded and used for `submit_ai_ruling` — is also the correct, and only, signer for this call; no
separate key or origin is needed.

- `should_finalize` (pure, unit tested) mirrors the pallet's own gate exactly: status must be
  `AIRulingIssued` (an appealed case moves to `InJuryAppeal` and is correctly never a candidate),
  and the current block must be strictly past `AIRulingBlock[case_id] + AppealWindowBlocks`
  (`config.appeal_window_blocks`, default `50_400` — `7 * DAYS` at this runtime's 12s block
  time, read from `runtime/src/configs/mod.rs`/`runtime/src/lib.rs`).
- A `finalize_processed: HashSet<u32>` (mirrors the existing `already_processed` for
  `submit_ai_ruling`) prevents a duplicate `finalize_ruling` submission while a prior one is still
  pending inclusion.

New config: `FINALIZE_RULING_CALL_INDEX` (default `4`, `#[pallet::call_index(4)]` read directly
from the pallet source) and `APPEAL_WINDOW_BLOCKS` (default `50_400`) — see `src/config.rs`.

**A second, separate gap this service used to work around client-side, since closed at the
pallet level instead.** Earlier, `submit_ai_ruling` recorded only `ruling_hash` (an evidence
pointer) and no verdict — so `finalize_ruling` took an explicit `verdict: Verdict` argument of
its own, with nothing on-chain tying it to the reasoning `submit_ai_ruling` had published. That
meant a compromised oracle key could publish reasoning saying one thing and finalize with the
opposite verdict, and this service had to work around the gap by re-fetching its own
just-published IPFS document at finalize time to recover the verdict it had already decided
once.

That's now fixed in `pallets/pallet-courts/src/lib.rs` itself, not just worked around here:
`submit_ai_ruling` takes a fourth argument, `verdict: Verdict`, and commits it on-chain
(`AIRulingVerdict`) in the same call that records `ruling_hash`. `finalize_ruling` no longer
takes a verdict argument at all — it applies exactly the value `submit_ai_ruling` committed,
so there is nothing left for the caller of `finalize_ruling` to choose. This service now mirrors
that: `rule_on_case` (`src/main.rs`) returns `(ruling_hash, verdict)` together, and `poll_once`
submits both in the one `submit_ai_ruling` call; the finalize branch just calls
`extrinsic::FinalizeRuling { case_id }` with nothing to reconstruct. The old
`fetch_ruling_verdict`/`parse_verdict_from_ruling_document` machinery (re-fetching the
published IPFS document at finalize time) has been removed entirely — it has no remaining
purpose once the chain enforces the same binding on its own.

## Configuration

All environment variables, see `src/config.rs` for defaults and full doc comments:

| Var | Default | Purpose |
|---|---|---|
| `NODE_RPC_URL` | `http://127.0.0.1:9944` | Chain JSON-RPC endpoint |
| `KEYS_FILE` | `/keys/court-oracle-secrets.age` | Age-encrypted file containing `oracle_account_seed` |
| `KEY_PASSPHRASE` / `KEY_PASSPHRASE_FILE` | (required, one of) | Passphrase to decrypt `KEYS_FILE` |
| `POLL_INTERVAL_SECS` | `60` | How often to poll `Courts::Cases` |
| `COURTS_PALLET_INDEX` | `11` | pallet-courts's runtime index |
| `SUBMIT_AI_RULING_CALL_INDEX` | `1` | `submit_ai_ruling`'s call index |
| `FINALIZE_RULING_CALL_INDEX` | `4` | `finalize_ruling`'s call index |
| `APPEAL_WINDOW_BLOCKS` | `50400` | `pallet-courts`'s `AppealWindowBlocks` (7 days at 12s blocks) — used client-side to decide when to attempt `finalize_ruling`; the chain enforces the real deadline independently |
| `IPFS_API_URL` | `http://127.0.0.1:5001` | Kubo-compatible IPFS daemon HTTP API |
| `CLAUDE_API_KEY` | (required) | Anthropic API key |
| `CLAUDE_MODEL` | `claude-opus-5` | Deliberately not the desktop app's `claude-sonnet-4-6` — this call produces a binding ruling, not a read-only chat answer |
| `DRY_RUN` | `false` | When true, builds and logs the ruling + IPFS publish but does not submit `submit_ai_ruling` |

Create a keys file:

```bash
echo '{"oracle_account_seed":"<64 hex chars>"}' | age -p > court-oracle-secrets.age
```

## Explicitly out of scope (per the task this was built against)

- Calling `Courts::set_oracle_account` to register this service's key on a real chain — a
  governance/root action.
- Jury-vote-driven finalization (`cast_jury_vote`'s own auto-finalize on reaching a majority) —
  that path is entirely on-chain and needs no off-chain scheduler. Only the no-appeal path
  (`finalize_ruling`) needed an off-chain caller, and that's what this service now does — see
  "`finalize_ruling` scheduling, and the verdict-binding fix" above.

## Test coverage — what's real, what isn't

`cargo test` (42 tests, all passing in this environment): case-context rendering for all four
`CaseSubject` variants; Claude request formatting and `VERDICT:`/`REASONING:` response parsing,
including a realistic-shaped JSON fixture and a refusal fixture; IPFS CID digest math
(round-trip, rejection of malformed input); SCALE encode/decode round-trips for the mirrored
`CaseStatus`/`CaseSubject`/`Verdict` enums; the `submit_ai_ruling` (now 4-argument, including
`verdict`) and `finalize_ruling` (now argument-free beyond `case_id`) extrinsic call-byte
layouts; storage-key prefix/hashing including the `twox128("System")` known-answer vector; and
`should_finalize`'s appeal-window/status gating (window still open, exactly at the deadline
block, one block past it, every non-`AIRulingIssued` status including appealed ones, and a
missing `AIRulingBlock` entry).

**Not covered, and cannot be covered in this environment**: the live chain RPC round trip, the
live Claude API call, the live IPFS daemon/gateway call, and therefore the full
`poll_once`/`rule_on_case` orchestration paths end-to-end. Say so plainly rather than mocking
these and claiming coverage that isn't real.
