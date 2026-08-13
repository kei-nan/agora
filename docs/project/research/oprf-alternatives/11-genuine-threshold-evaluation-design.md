# Genuine Threshold OPRF Evaluation — Design and Implementation

*2026-08-13. Direct follow-on from [00-index.md](00-index.md)'s institutional-operator
recommendation. Status: **implemented and tested at the protocol, on-chain, and wasm-core
layers; `committee-node`'s orchestration loop is specified but not yet wired up.** See
"Implementation status" below for the precise breakdown of what's built and verified versus
what remains.*

## The problem this closes

Tracing the actual code (`pallets/pallet-identity/src/lib.rs`, `oprf-committee-dev/src/`) found
that the design did not deliver the threshold property its own governance decision
(changelog #073, "12-of-35") describes:

- Every member of a committee held an **identical copy** of that committee's one secret, not a
  distinct share (`oprf-committee-dev/src/committee.rs`'s dev simulator was explicit about
  this: "a single freshly-generated scalar, held entirely by this one process").
- `pallet-identity`'s `OprfResponses` accepted and finalized the **first** valid response for a
  given `(query_id, committee_slot)` and rejected any further one as `DuplicateResponse`. No
  code path anywhere combined multiple members' responses.

Net effect: any single committee member's server was fully and permanently sufficient to answer
on behalf of its entire committee, cryptographically. Adding more members added availability and
jurisdictional/legal diversity — real properties — but not the "compromising this needs `t`
independent parties" property the sizing was actually built around.

## The two options considered, and why Option B won

**Option A** — do the `t`-of-`n` combination *inside the Noir circuit* — was attempted first and
abandoned mid-implementation: it requires verifying Lagrange coefficients against the field the
secret sharing actually lives in (BabyJubJub's own scalar field), which is a *different,
non-native* field relative to the circuit's own proving field. Checking that relation soundly
inside a SNARK needs real foreign-field arithmetic (the same class of problem as this project's
existing non-native RSA verification, but a new instance of it) — attempting a shortcut using the
circuit's native field arithmetic would have been unsound, not just imprecise: a malicious
prover could forge a passing proof without holding a real combination.

**Option B** — a genuine two-round threshold protocol (FROST, adapted from single-generator
Schnorr signatures to this system's two-generator Chaum-Pedersen DLEQ relation) — moves that same
combination math *off* the circuit entirely, into ordinary Rust running in the correct field
directly (no non-native arithmetic problem exists outside a SNARK). The Noir circuits
(`anchor`/`disclosure`/`migrate`/`migrate-disclosure`) need **zero changes** — confirmed, not
assumed: a combined proof produced by this session's implementation was checked against the
*actual, unmodified* `verify_dlog_equality` from the real `TaceoLabs/oprf-nr` v1.0.0 dependency
(cached locally, the exact library the circuits import), and it passes.

## Protocol, in brief

From a real DKG (Feldman VSS — `oprf-committee-dev/src/dkg.rs`'s existing, pre-built math):
member `i` holds share `s_i`; the group public key `Y` and each member's own public share `Y_i`
are derivable by anyone from the DKG's public commitments.

1. **Round 1**: each responding member computes their partial evaluation `R_i = s_i·Q` and
   publishes it alongside two FROST-style nonce-commitment pairs — two nonces per member, not
   one, specifically to defend against the Drijvers et al. rogue-nonce attack on naive two-round
   Schnorr-family aggregation.
2. **Set locking**: once exactly `t` distinct members have submitted round 1 for a query, that
   set is fixed.
3. **Round 2**: each locked member computes a response scalar under a challenge shared by the
   whole set (binding factors preventing cross-session mixing), using their own share and a
   Lagrange coefficient — both computed from public data, no non-native arithmetic needed since
   this happens in ordinary Rust, not a circuit.
4. **Combine**: summing the `t` response scalars yields `(e, z)` — an ordinary, single Chaum-
   Pedersen proof against the group key `Y`, verified exactly as a non-threshold proof would be.

## What was actually built and verified

- **`oprf-committee-dev/src/threshold.rs`** (new): the full protocol above. Tested against a
  real DKG run (not a shortcut) at 3-of-5, 6-of-7 (changelog #073's founding-group scale), and
  2-of-2; confirmed different qualifying subsets reconstruct the identical evaluation (the
  property that makes the anchor stable regardless of which members happened to respond); Lagrange
  coefficients cross-checked against `dkg.rs`'s independently-written reconstruction. **41/41
  crate tests pass.**
- **Cross-checked against the real Noir dependency, not just this crate's own Rust port**: a
  combined proof was fed into a throwaway Noir package importing the actual `oprf-nr` v1.0.0
  `verify_dlog_equality` and passed — the strongest available confirmation that Option B
  genuinely needs no circuit change.
- **`oprf-committee-dev/src/ffi.rs`**: extended with `oprf_round1`/`oprf_round2_response`,
  keeping the secret-touching computation wasm-portable across phone/laptop/Pi exactly as
  `oprf_evaluate_query` already was (changelog #082's "one crypto core" decision) — confirmed
  present in the actual compiled `wasm32-unknown-unknown` artifact, and confirmed to reproduce a
  verifiable combined proof driven **entirely through the FFI boundary** (not just the native
  Rust functions underneath). Public-data-only aggregation math (binding factors, Lagrange
  coefficients, the shared challenge) deliberately stays outside the wasm boundary — it touches
  no secret material, so a native caller can call it directly; see `ffi.rs`'s module docs.
- **`pallets/pallet-identity`**: `submit_oprf_response`/`OprfResponses`/`OprfResponseRecord`
  replaced with `submit_oprf_round1`/`submit_oprf_round2` and
  `OprfRound1Commitments`/`OprfRound2Responses` — a genuine two-round on-chain bulletin board.
  Deliberately does **no cryptographic verification** itself (purely structural: registered
  member, no double-submission, set-size gating) — the combination's correctness is checked
  where OPRF proofs have always been checked, client-side at `register_citizen` time; see
  `OprfRound1Commitment`'s doc comment for the reasoning and its accepted limitation
  ("identifiable abort" — pinning fault on a specific misbehaving member — is a real, separate,
  harder problem not attempted here). The now-fully-unused `dlog_verify.rs` (the old on-chain
  crypto check) was removed rather than left dead. **99/99 pallet tests pass**, including 15 new
  tests for the two-round flow. Runtime builds clean in both `dev-mode` and real-verifier
  configurations, with a new `OprfThreshold` config constant (placeholder `12`, same "changelog
  #073 said ~12-of-35, not yet really sized" caveat every other constant here already carries).

## `committee-node`'s orchestration loop — now implemented too

The gap this section originally described (main.rs/extrinsic.rs still submitting the retired
single-response call) is closed. Three parallel, independently-specified pieces — RPC reads for
the two new storage maps plus `MEMBER_INDEX` config, extrinsic encoding for
`submit_oprf_round1`/`submit_oprf_round2`, and wasm-host wrappers for `oprf_round1`/
`oprf_round2_response` — were built against the real interfaces (no guessing: each was handed
the exact struct/function signatures already landed in the pallet and `ffi.rs`) and then
integrated: `main.rs` now polls each pending query through both rounds, tracking per-query
progress (which seed round 1 used) in memory; round 2's aggregation math
(`threshold::binding_factor`/`lagrange_coefficient`/`combined_challenge`/
`aggregate_nonce_commitments`) runs as a genuine native dependency on `oprf-committee-dev`, not
through the wasm boundary, per `ffi.rs`'s own reasoning that public-data math doesn't need
wasm-portability. A new `GROUP_PUBKEY_HEX` config value (this committee's group public key,
needed for the challenge computation, not derivable from chain state since only its hash is
stored on-chain) and the AccountId-to-DKG-index mapping (via `CommitteeMembers[slot]`'s roster
*order* — position `i` is party index `i+1`) closed the remaining gaps found while wiring the
pieces together. 34/34 tests pass, zero-warning build, dead code from the retired single-response
path removed (not left silently around) — see `committee-node/README.md`'s "Option B" section
for the full detail.

**What genuinely remains unverified — stated plainly, not implied away**: none of this has run
against a real chain (no chain is running in this environment) or inside a real multi-party
exchange with other real nodes. No real committee, no real DKG ceremony, and no real institutional
operators exist yet — this is protocol and orchestration code that compiles and passes its own
unit tests, not something proven against a live deployment. That's a different, later kind of
verification this session's tooling cannot provide.

## The citizen-facing cost — reframed, not forgotten

The original ask this thread grew from was to track how hard this makes registration for a real
citizen, and to come back to it rather than let it block progress. Under Option A that concern
was mobile proving time (a heavier circuit). Under Option B the circuit is unchanged, so that
specific cost disappears — but a different one replaces it, worth flagging with the same
seriousness: **registration latency.** A citizen's registration now depends on *two* sequential
rounds of on-chain coordination among committee members per committee (wait for `t` members to
notice and submit round 1, then wait for the *same* `t` members to notice round 1 locked and
submit round 2) — roughly double the wait of the old single-response design, compounded across
all 5 committees (changelog #073's existing "n-of-n across 5 committees" structure), and multiplied
by however long real committee members actually take to notice and respond, which changelog
#073's own SLA figures already flag as unmeasured. **Not measured here, deliberately** — same
treatment as the mobile-proving question before it: noted clearly, not blocking, to be picked up
with real numbers once there's a committee to measure against.

## Verdict

Option B is implemented and verified at every layer that can be verified without a live chain or
a real committee: the cryptographic protocol, its wasm packaging, the on-chain mailbox, and now
`committee-node`'s own orchestration loop end to end. What's left — a real DKG ceremony, real
institutional operators, an actual multi-node exchange against a live chain, and the
citizen-facing latency question above — are concrete, scoped, and explicitly not silently
dropped. This document and the code it describes should now be read as an implementation record,
not a design proposal.
