---
name: project-review
description: Use when the user asks for a full project review/audit of Agora (democracy-chain) — "review the project", "audit the codebase", "how healthy is the project", "check for doc drift", "find what's actually broken vs what the docs claim". Fans out parallel subagents across the monorepo's components (chain/pallets, courts+oracle, identity/ZK/OPRF, voting+treasury+other pallets, mobile, desktop) plus a dedicated docs-vs-reality agent, then synthesizes one prioritized report. Not for reviewing a single diff/PR (use code-review) or a single security pass (use security-review).
---

# Agora Project Review

This project has a documented history of docs claiming things that aren't true in code — the
"Emergency Council" branch was fully described in CLAUDE.md as if wired in when it wasn't
(discovered 2026-08-04); HANDOFF.md drifted enough it had to be demoted to a pointer. So this
review treats **doc-vs-reality drift as a first-class finding category**, not an afterthought,
alongside correctness, security, and code quality.

## Step 0 — Live inventory (do this yourself, don't delegate it)

Before dispatching anything, spend under a minute establishing ground truth so subagent briefs
are accurate as of *right now*, not as of whatever CLAUDE.md/next-steps.md last said:

```bash
ls pallets/                                    # actual pallet list
grep -n "construct_runtime!" -A 40 runtime/src/lib.rs   # actual wired-in pallets + indices
ls -d */                                       # actual top-level components (things get added —
                                                # e.g. court-oracle/ exists now; check it against
                                                # CLAUDE.md's claim that no oracle service exists)
git log --oneline -20                          # recent work, so agents don't re-review what
                                                # just landed in the last commit for a different reason
```

Use this to adjust the component list and briefs below — don't hand a subagent a stale scope.

## Step 1 — Fan out domain agents in parallel

Dispatch these as `general-purpose` agents (not `Explore` — this needs judgment about security
and correctness, not just location-finding), **all in one message** so they run in parallel and
in the background. Each is explicitly **read-only**: no Edit/Write, this is a review, not a fix.

Give every agent the same report contract (see Step 3) and the same standing instruction:
*"Where CLAUDE.md, docs/project/README.md, docs/project/next-steps.md, or a component doc makes
a factual claim about your area, verify it against the actual code before repeating it. If it's
false or stale, that's itself a finding."*

Default component split (adjust based on Step 0's live inventory):

1. **Chain core & separation of powers** — `runtime/`, `node/`, and the constitutional/legislature/
   elections/emergency-council/executive pallets. Check: does `construct_runtime!` actually match
   what docs claim is wired in; do origin checks genuinely prevent executive from doing legislative
   things and vice versa; are there unbenchmarked/unweighted extrinsics; any unsafe code; do the
   pallet's own tests pass (`cargo test -p <pallet>`).

2. **Courts & AI oracle** — `pallets/pallet-courts/`, `court-oracle/`. This pair is where a stale
   claim was already caught mid-repo (CLAUDE.md's next-steps says no oracle exists off-chain, but
   `court-oracle/` was added in a recent commit) — verify current reality directly. Check: who is
   authorized to call `submit_ai_ruling` (`OracleOrigin` trust model), whether the commit-then-
   delayed-reveal jury randomness is grinding-resistant as claimed, auto-enforcement correctness
   (does an invalidated law actually get paused, does a frozen treasury tx actually stop funds).

3. **Identity, ZK & OPRF** — `pallets/pallet-identity/`, `circuits/`, `oprf-committee-dev/`,
   `committee/`, `committee-node/`. Check: does `runtime/src/verifier.rs` actually verify real
   UltraHonk proofs as claimed or only dev-simulator ones; is the nullifier scheme
   (`Poseidon2(national_id || country_code)`) implemented as specified; is there any key material
   or committee-share handling that's insecure by construction (not "no real committee exists yet"
   — that's known and tracked — but implementation bugs in what *does* exist).

4. **Voting, treasury & remaining pallets** — `pallet-voting`, `pallet-treasury-ledger`,
   `pallet-anticorruption`, `pallet-audit`. Check: are delegation caps actually enforced on-chain
   (not just documented as a design goal); is every treasury transaction actually tagged/audited
   as claimed, or are there paths that bypass the audit hook; does the liquid-democracy delegation
   graph handle cycles/revocation correctly.

5. **Mobile app** — `mobile/`. Check: does `NfcPassportModule.kt` and the wider native layer match
   what docs claim; is the "77 tests passing" figure current (`ls` the test files, don't trust the
   number); Secure Enclave/Keystore usage for the wallet key; confirm `ios/` really doesn't exist
   rather than repeating the claim.

6. **Desktop app** — `desktop/`. Check: QR-auth flow's actual trust boundary (does the bearer
   token grant more than "read + submit" as claimed); is the Claude AI agent integration actually
   read-only on-chain (can it construct a call that gets auto-signed, or does it genuinely require
   phone confirmation); smoldot light-client and IPFS-gateway error handling.

7. **Docs vs. reality (dedicated agent)** — no code ownership of its own; cross-reads CLAUDE.md,
   `docs/project/README.md`, `docs/project/next-steps.md`, `docs/project/architecture.md`,
   `docs/project/zk-verifier.md`, and everything under `docs/project/apps/` and
   `docs/project/pallets/` against the live code (grep for the specific functions/files/indices
   each doc claims exist, check `Cargo.toml` workspace membership, check test counts). This agent's
   entire job is producing a list of claims that are stale, contradicted, or unverifiable — feed it
   the Step 0 inventory as a starting diff, not the full rediscovery burden.

Each agent's prompt should state the "why" (this project has a track record of doc drift and
partial wiring — verify, don't transcribe) so it makes good judgment calls on borderline items,
per this repo's general instruction that terse prompts produce shallow work.

## Step 2 — Do not poll

Agents run in the background; you'll get a notification per agent as it finishes. Don't sleep-loop
or re-check status — continue only once all seven have reported back, or surface partial synthesis
if the user asks before they're all in.

## Step 3 — Report contract (give this to every agent verbatim)

Each finding:
- `component` — which of the 7 areas above
- `category` — one of: `security`, `correctness`, `doc-drift`, `test-gap`, `code-quality`
- `severity` — `critical` / `high` / `medium` / `low` / `info`
- `location` — file:line
- `claim` (doc-drift only) — the exact claim and where it's made
- `evidence` — what was actually found (grep output, code excerpt, missing wiring, etc.)
- `summary` — one sentence

Ask each agent to report in under 400 words plus a compact finding list — full file dumps belong
in their own read tool calls, not the summary that comes back to you.

## Step 4 — Synthesize (you do this, not a subagent)

1. Merge all seven reports. Dedupe — the docs-vs-reality agent and a domain agent will sometimes
   catch the same drift from different angles; keep the more specific one.
2. Cross-reference: a `security`/`correctness` finding in a domain area that also has a
   `doc-drift` finding nearby is worth flagging as a pair (usually means the docs were written
   optimistically before the code was actually finished).
3. Rank: `security`/`correctness` at `critical`/`high` first, then `doc-drift` (this project cares
   about this specifically — CLAUDE.md and next-steps.md are meant to be load-bearing), then
   `test-gap`, then `code-quality`.
4. Present as a single prioritized list to the user. Don't auto-edit CLAUDE.md/next-steps.md to
   fix drift you found — flag it and let the user decide, since those files are also a record of
   *why* decisions were made, not just current state.
5. If the user wants the report kept, write it to a file rather than leaving it only in chat —
   `docs/project/changelog/` follows this repo's existing pattern of dated entries.

## Scope control

If the user names a specific area ("review the mobile app", "just check the court oracle"), skip
Step 1's fan-out and either handle it directly or dispatch a single matching agent — don't spin up
all seven for a narrowly scoped ask.
