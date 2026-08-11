# `oprf-identity-anchor` — Agora's forked ZKPassport OPRF circuits

Noir circuits that derive Agora's **renewal-stable identity anchor** from a passport's MRZ
personal-number field, via an oblivious pseudorandom function (OPRF) committee.

This is the first entry into `circuits/`. It implements the cryptographic half of the
Sybil-resistance architecture decided in changelog entry 67 and already wired at the pallet
level in entry 68 (`pallets/pallet-identity/src/lib.rs`, `AnchorProofVerifier`). Build and
findings are recorded in changelog entry 69.

**Read the "What is NOT done" section before relying on any of this.** In particular, no OPRF
committee service exists, so nothing here can produce a real anchor yet.

---

## Why this fork exists

ZKPassport's default nullifier is `calculate_private_nullifier(dg1, e_content, sod_signature)`.
It is safe from brute force because an attacker cannot forge a government signature — and for
exactly that reason it changes on every passport renewal. Agora needs the opposite property: a
value that is *stable* across renewal so the same human cannot register twice with a new
document.

The stable substitute is the ICAO Doc 9303 Part 4 TD3 "personal number" field (MRZ line 2,
character positions 29–42). That field has far too little entropy to publish a bare hash of —
a 9-digit national ID is ~10⁹ candidates, trivially invertible — so it is run through a
threshold OPRF committee first. Entry 67 has the full reasoning, including the rejected
alternatives.

## What was forked, from where

| Source | Pinned at | Used for |
|---|---|---|
| [`zkpassport/circuits`](https://github.com/zkpassport/circuits) | `d3a75acb8529e82c61be136a402553daec259257` (2026-07-21) — tags `bb-v5.0.0` and `noir-v1.0.0-beta.22` both point here | `src/noir/bin/oprf-auth/src/main.nr` (forked), `lib/utils`, `lib/commitment/common`, `lib/data-check/expiry` (git deps, unmodified) |
| [`TaceoLabs/oprf-nr`](https://github.com/TaceoLabs/oprf-nr) | tag `v1.0.0` | `oprf::blinded_query`, `oprf::oprf_output::verified_oprf` (unmodified) |
| [`noir-lang/poseidon`](https://github.com/noir-lang/poseidon) | tag `v0.3.0` | Poseidon2 (unmodified) |

At the time of writing, `d3a75ac` is still `main`'s HEAD upstream — the same commit changelog
entry 66 used. Nothing upstream has moved since.

Only **one upstream file is actually forked**: `oprf-auth/src/main.nr` → `query/src/main.nr`.
Everything else is either a new Agora circuit built to an upstream interface, or an unmodified
upstream library pulled in as a pinned git dependency.

### This fork is narrower than changelog entry 67 predicted

Entry 67 traced the blocker to `dsc-to-id/src/lib.nr:29`'s `calculate_private_nullifier` call
and concluded the fork would have to reach up into that circuit, regenerate its verification
key, and re-register it. **That turned out not to be necessary**, and the reason is worth
recording because it materially shrinks the trust-audit surface:

`hash_salt_dg1_dg2_hash_private_nullifier` — the commitment `oprf-auth` already checks —
commits over `salted_dg1.get_hash()` *as well as* the private nullifier. DG1 is therefore
already bound into the exact commitment the OPRF circuit consumes, and the personal-number
field is just a byte range of DG1. So the substitution can be made **entirely inside the
forked leaf circuit**, reading different bytes out of a value that was already authenticated.

Consequences:

- `csc-to-dsc`, `dsc-to-id` and `integrity` are untouched. Their verification keys and their
  entries in ZKPassport's `circuit_registry_root` stay valid as-is.
- Exactly one new verification key is introduced per new circuit here, not a re-keyed chain.
- The personal number inherits the SOD signature verification the pipeline already performs.
  No new trust anchor.

## The circuits

Five binaries and one shared library. The OPRF protocol is inherently two-round (blind → the
committee evaluates → unblind), so a single circuit cannot cover it; `query` is round one and
`anchor`/`disclosure`/`migrate`/`migrate-disclosure` are round two.

| Package | ACIR opcodes | bb circuit size | Role |
|---|---:|---:|---|
| `query` | 6,733 | 12,113 | Round 1. Fork of `oprf-auth`. Blinds the identity input, sent identically to all 5 committees. |
| `anchor` | 217,477 | 280,304 | Round 2, standalone. Verifies **5** committees' responses, emits the combined anchor. |
| `disclosure` | 219,440 | 283,679 | Round 2, **pipeline-integrated**. Same, shaped as an outer-circuit disclosure subproof. Adds an expiry check. Backs `verify_registration_anchor` and `verify_reverification`. |
| `migrate` | 433,740 | 555,380 | Dual evaluation under two committee generations (**10** total OPRF checks), for OPRF scheme rotation. Standalone — kept for testing, same unauthenticated-`comm_in` caveat as `anchor`. |
| `migrate-disclosure` | 435,707 | 558,896 | Round 2, **pipeline-integrated** form of `migrate` (changelog entry 76) — same dual evaluation, shaped as a disclosure subproof, with an expiry check. Backs `verify_migration`. |

For reference, unmodified upstream `oprf-auth` measures **6,644** ACIR opcodes with the same
toolchain. `query`'s fork costs **+89 opcodes** — the personal-number slice, the populated
check, the country binding and the clear-value assertions; it is unaffected by the 5-committee
change since it produces one blinded query regardless of how many committees receive it.

`anchor`/`disclosure`/`migrate`'s opcode counts above are ~4.9x their single-committee
predecessors (44,462 / 46,423 / 87,715 respectively, as measured under bb 0.82.2 before
changelog entry 74's 5-committee extension) — consistent with 5 (resp. 10) `verified_oprf`
calls dominating cost, not a regression. `migrate-disclosure` (changelog entry 76) costs
+1,967 opcodes over `migrate` — the expiry check and outer-layout plumbing, the same
increment `disclosure` pays over `anchor` (+1,963). Measured under bb 5.0.0, which by this
point is also what `runtime/src/verifier.rs` targets (changelog entry 72); `bb write_vk`
succeeds for all five circuits at this size, i.e. producing a verification key does not
itself require a live committee — only *executing* the circuit (`nargo execute` / `bb prove`)
does.

`lib/identity-anchor` holds the derivation shared by all five, so the value `query` blinds and
the value `anchor`/`migrate` verify cannot drift apart.

### Derivation

```
personal_number   = DG1[77..91]                     // flat MRZ 72..86 = TD3 line 2 pos 29-42
identity_input    = Poseidon2(DS_IDENTITY_INPUT,
                              pack_be(personal_number[14]),
                              pack_be(issuing_country[3]))

// Sent identically to all 5 committees (changelog entry 73) — not routed by hash, which
// would leak a low-entropy per-citizen signal on every registration.
for i in 0..5:
    oprf_output_i = verified_oprf(committee_i_proof, ..., identity_input, DS_DLOG, DS_ANCHOR_OUT)
    term_i        = Poseidon2(DS_ANCHOR, i, oprf_output_i, scheme_version)

anchor = term_0 + term_1 + term_2 + term_3 + term_4   // field addition, not another hash
```

The per-committee hash-then-sum combiner, and why it must hash each term before adding rather
than summing raw OPRF outputs, is changelog entry 73's design — see
`identity_anchor::derive_committee_anchor_term` / `combine_committee_anchors` for the full
security argument. `query` (below) is unaffected: the OPRF protocol only cares that the same
blinded query reaches every committee, which requires no circuit change to send to 5 recipients
instead of 1.

Domain separators follow ZKPassport's convention of an ASCII string read as a big-endian
integer:

| Constant | ASCII | Must agree with |
|---|---|---|
| `DS_IDENTITY_INPUT` | `AGORA_IDENTITY_ANCHOR_V1` | Agora circuits only |
| `DS_ANCHOR` | `AGORA_ANCHOR_V1` | Agora circuits + on-chain verifier |
| `DS_ANCHOR_OUT` | `AGORA_ANCHOR_OPRF_OUT` | Agora circuits only (client-side; the committee never sees it) |
| `DS_DLOG` | `DLOG Equality Proof` | **the deployed OPRF committee service** — this is the domain separator of the Chaum-Pedersen proof the service produces. Kept byte-identical to ZKPassport's `DS_DLOG`, which is the value TACEO's `oprf-service` is known to be driven with. |

### Deliberate constraints

Three asserts exist for reasons that are not obvious from the code alone:

1. **TD1 ID cards are rejected.** Their MRZ has a different layout; reading TD3 offsets out of
   one yields a well-formed but meaningless anchor. Agora is passport-only for v1.
2. **An all-filler personal number is rejected.** The field is optional in ICAO Doc 9303. A
   country that does not populate it emits 14 `<` characters for *every* citizen — so without
   this check the first person to register would consume the country's only possible anchor
   value and permanently lock out everyone else (the pallet rejects a reused anchor with
   `AnchorAlreadyUsed`). Entry 67 left "does the deployment country actually populate this
   field?" open and unresolvable from the spec; this assert turns that unknown from a silent,
   population-wide failure into an immediate one at proof time.
3. **DG1 must be supplied as a clear value, not as a hash.** `SaltedValue<T>` has a hash-only
   mode where `get_hash()` returns the supplied hash verbatim and `value` stays zeroed. In
   that mode `comm_in` would still verify while the extracted personal number was all zeroes,
   so every prover would derive the same identity input. Requiring clear-value mode is what
   makes the commitment chain actually constrain the bytes these circuits read.

The document-bound `salted_private_nullifier` is still an input (it is needed to reconstruct
`comm_in`) but its clear value is never used, and Agora's flow passes it in hash-only mode.
That is entry 67's required anchor/nullifier separation, enforced structurally.

---

## Public-input layout

Barretenberg orders UltraHonk public inputs as **public parameters in declaration order,
followed by return values**. Every layout below was read off a real `bb prove` run's
`public_inputs` file, not inferred from the source.

### `oprf_identity_anchor_query` — 3 fields

| # | Name | Notes |
|---|---|---|
| 0 | `comm_in` | `comm_out` of the integrity-check subproof; unchanged from upstream |
| 1 | `blinded_query_x` | BabyJubJub point sent to the committee |
| 2 | `blinded_query_y` | |

Verified by the OPRF committee nodes, **not on-chain**. Its job is to stop the committee being
used as a blind oracle for arbitrary inputs.

### `oprf_identity_anchor` — 8 fields

| # | Name | Notes |
|---|---|---|
| 0 | `comm_in` | |
| 1 | `scheme_version` | must be non-zero; matches `pallet-identity`'s `OprfSchemeVersion` |
| 2 | `anchor` | `term_0 + ... + term_4`, the value stored in `IdentityAnchorRegistry` |
| 3–7 | `oprf_pk_hashes[0..5]` | `Poseidon2(pk_i.x, pk_i.y)` per committee — which key produced committee `i`'s evaluation |

Changed by changelog entry 74 from the original single-committee 4-field layout (`comm_in`,
`scheme_version`, `anchor`, `oprf_pk_hash`) to check and combine all 5 committees per entry 73.

**Superseded as the production verification target by `disclosure`, below, same as `anchor`
below it — this earlier revision of this section claimed `verify_reverification` used this
standalone layout "with an identical layout" to registration; that was wrong for the same
reason a standalone `anchor` proof is unsafe for registration (see "Why `disclosure` exists").
Changelog entry 76 corrects this: both `verify_registration_anchor` and `verify_reverification`
are built on `disclosure`'s outer-embedded 8-field layout below.** Kept for testing only.

### `oprf_identity_anchor_migrate` — 15 fields

| # | Name |
|---|---|
| 0 | `comm_in` |
| 1 | `old_scheme_version` |
| 2 | `new_scheme_version` |
| 3 | `old_anchor` |
| 4 | `new_anchor` |
| 5–9 | `old_oprf_pk_hashes[0..5]` |
| 10–14 | `new_oprf_pk_hashes[0..5]` |

Confirmed against a real `bb prove` run as of changelog entry 081 (`oprf-committee-dev`'s
dual-committee-generation simulator; see that entry and `docs/project/next-steps.md` item 8):
`nargo execute` solved a genuine 10-`verified_oprf`-call witness and the resulting
`public_inputs` file is exactly this 15-field layout, `old_anchor` matching `oprf_identity_anchor`'s
own already-proven `anchor` output byte-for-byte (both driven from the same committee
generation and identity input).

**Superseded as the production verification target by `migrate-disclosure`, below, for the same
reason `disclosure` supersedes `anchor`** (changelog entry 76 — this standalone layout's
`comm_in` is an unauthenticated private witness of whatever outer proof it's paired with).
Kept for testing only; `AnchorProofVerifier::verify_migration` is built on `migrate-disclosure`.

There is exactly one `identity_input` binding in that circuit and all 10 `verified_oprf` calls
consume it, so the two combined anchors are provably same-input by construction — no explicit
equality constraint is needed or present. `migrate-disclosure` inherits this property
unchanged.

### `oprf_identity_anchor_disclosure` — 8 fields

| # | Name | Value |
|---|---|---|
| 0 | `comm_in` | |
| 1 | `current_date` | unix timestamp; drives the expiry check |
| 2 | `service_scope` | unused by this circuit; present to hold its slot |
| 3 | `service_subscope` | unused by this circuit; present to hold its slot |
| 4 | `param_commitment` | `Poseidon2(200, anchor, scheme_version, oprf_pk_hashes[0..5])` — 8-element hash |
| 5 | `nullifier_type` | always 0 |
| 6 | `scoped_nullifier` | always 0 |
| 7 | `oprf_pk_hash` | always 0 |

This is exactly the 8-field vector `prepare_disclosure_inputs` (upstream
`src/noir/lib/outer/src/lib.nr`) feeds to `verify_proof_with_type`, confirmed by counting a
real proof's public inputs (256 bytes = 8 × 32) and checking each slot's value. Slots 5–7 are
constants and are **not** optimised away by the compiler — they occupy their positions, which
is the "non-participating" mode the outer circuit already supports for facematch subproofs.
This outer-circuit-facing shape is fixed by upstream and does **not** grow with the number of
committees — that's exactly why all 5 committees' `oprf_pk_hashes` had to move inside
`param_commitment` (a single field) rather than occupy separate output slots, unlike the
standalone `anchor` circuit above where they're free to be separate public outputs.

`200` is an Agora-specific proof-type tag. ZKPassport's own `PROOF_TYPE_*` constants currently
occupy 0–10; 200 is far enough outside that range to stay unambiguous if upstream adds more.

The anchor committees' `oprf_pk_hashes` ride inside `param_commitment` rather than in slot 7,
because slot 7 belongs to ZKPassport's salted-*nullifier* OPRF — a different key for a
different purpose.

### `oprf_identity_anchor_migrate_disclosure` — 8 fields

| # | Name | Value |
|---|---|---|
| 0 | `comm_in` | |
| 1 | `current_date` | unix timestamp; drives the expiry check |
| 2 | `service_scope` | unused; present to hold its slot |
| 3 | `service_subscope` | unused; present to hold its slot |
| 4 | `param_commitment` | `Poseidon2(201, old_anchor, new_anchor, old_scheme_version, new_scheme_version, old_oprf_pk_hashes[0..5], new_oprf_pk_hashes[0..5])` — 15-element hash |
| 5 | `nullifier_type` | always 0 |
| 6 | `scoped_nullifier` | always 0 |
| 7 | `oprf_pk_hash` | always 0 |

Same fixed 8-field outer-circuit-facing shape as `disclosure` — the only room for the wider
migration payload is `param_commitment` itself, so it folds in both anchors, both scheme
versions, and all 10 committee-key hashes rather than occupying separate slots. `201` is a
second Agora-specific proof-type tag, deliberately distinct from `disclosure`'s `200` so a
migration commitment can never be substituted for a registration/reverification one (they have
different field counts and different Rust-side check obligations).

Confirmed against a real `bb prove` run as of changelog entry 081, same `oprf-committee-dev`
driver as `migrate` above: `nargo execute` solved a genuine witness, the resulting
`public_inputs` file is exactly `[comm_in, current_date, service_scope, service_subscope,
param_commitment, 0, 0, 0]`, and `bb verify` accepted the proof. This closes the gap entry 077
flagged — that entry only confirmed this layout via a stubbed scratch copy, never a real
committee-backed witness.

### Why `disclosure`/`migrate-disclosure` exist, and why a Rust verifier must use them

`anchor` (and, identically, `migrate`) alone is **not safe to verify on-chain**. Each proves its
output came from the DG1 committed at `comm_in`, but `comm_in` is a *private* witness inside
ZKPassport's outer circuit — it is not among that proof's public inputs. So a standalone
`anchor`/`migrate` proof and an outer passport proof cannot be linked to each other on-chain,
and a prover could pair a genuine outer proof with an `anchor`/`migrate` proof over a `comm_in`
of their own invention. For `anchor` that defeats the Sybil check; for `migrate` it defeats
both the same-human continuity the proof exists to establish and the freshness guarantee that
the citizen still holds a currently valid passport at migration time (changelog entry 76).

`disclosure`/`migrate-disclosure` close this by riding inside the outer proof, so there is one
proof and one `comm_in`, authenticated all the way back to `certificate_registry_root`.

Keep `anchor`/`migrate` for testing and for any future flow that publishes `comm_in` some other
way, but **a production `AnchorProofVerifier` is built on `disclosure`/`migrate-disclosure`,
as of changelog entry 76.**

## What the Rust verifier still has to enforce

The circuits cannot check these; they are on-chain obligations.

1. **`scheme_version` → `oprf_pk_hashes[i]` binding, per committee slot.** A proof asserts
   *which* 5 committee keys were used; nothing in-circuit says any of them is the legitimate
   key for that scheme version's committee `i`. The chain must hold the governance-approved
   committee public key for **each of the 5 slots**, per scheme version, and reject any
   mismatch on any slot. This is stricter than the single-committee case: a proof that reuses
   4 correct committee keys and substitutes an attacker-controlled 5th must still be rejected,
   or the "unpredictable if even one committee is honest" property (changelog entry 73)
   inverts into "forgeable if even one committee key check is skipped." Without this check at
   all, anyone who stands up their own OPRF key can mint unlimited anchors.
2. **`certificate_registry_root` / `circuit_registry_root` allowlisting.** Both are plain
   public inputs of the outer proof. `pallet-identity`'s existing `AllowedMerkleRoots` pattern
   is the natural home. The Agora-governed circuit registry must include these forked
   circuits' vkey hashes.
3. **`param_commitment` recomputation.** For registration/reverification, recompute
   `Poseidon2(200, anchor, scheme_version, oprf_pk_hashes[0..5])` (8-element hash) from the
   values submitted with the extrinsic and check it equals the outer proof's
   `param_commitments[i]`. For migration, recompute `Poseidon2(201, old_anchor, new_anchor,
   old_scheme_version, new_scheme_version, old_oprf_pk_hashes[0..5], new_oprf_pk_hashes[0..5])`
   (15-element hash) the same way — see changelog entry 76.
4. **`current_date` freshness**, so an old proof cannot be replayed past the passport's expiry.
   Applies to registration, reverification, and migration alike (all three now ride
   `disclosure`/`migrate-disclosure` subproofs, both of which carry `current_date`).
5. **The `anchor` combination itself is not re-verified on-chain** — the chain trusts the
   circuit's `term_0 + ... + term_4` constraint and only checks the *inputs* to it (the 5 pk
   hashes, on both sides of a migration) against governance-approved keys. This is
   intentional: recomputing the combination outside the SNARK would require the individual
   `oprf_output_i` values, which are exactly the private witnesses the proof exists to keep
   off-chain.

---

## Reproducing the build

Toolchain — already installed on the dev machine; do not reinstall:

- `nargo` **1.0.0-beta.22** at `/home/realize/.nargo/bin/nargo`. This matches the upstream
  repo's own pin (`.github/workflows/test.yml`, and `@noir-lang/noir_js: ^1.0.0-beta.22` in
  `package.json`). Changelog entry 65 found beta.25 crashed the *Rarimo* circuit with an ICE;
  that does not apply here — beta.22 compiles both the unmodified upstream circuit and this
  fork cleanly, verified in this session.
- `bb` at `/home/realize/.bb/bb` — **5.0.0** as of changelog entry 74 (the machine was upgraded
  from 0.82.2 sometime around entry 72's passport-verifier bb 5.0.0 port; this workspace's own
  circuits were last exercised end-to-end under 0.82.2 per entry 69, but `bb write_vk` for all
  four circuits was reconfirmed working under 5.0.0 in entry 74).

The `--workspace` flag on each command below is load-bearing, not decoration: this workspace's
`Nargo.toml` sets `default-member = "anchor"`, which has no `#[test]`s of its own, so a bare
`nargo test` run from this directory silently reports "Running 0 test functions" instead of
erroring — it's *not* telling you the tests are broken, just that it only ran the default
member. The real 15 tests live in `lib/identity-anchor` and only run with `--workspace` (or by
`cd`ing into `lib/identity-anchor` directly, or `--package identity_anchor`). We keep
`default-member = "anchor"` rather than removing it, because removing it also changes the
default scope of `nargo compile`/`nargo info`/`nargo execute` (they'd default to all 5 bin
packages instead of just `anchor`), which is a bigger behavior change than this note is worth
fixing via workspace restructuring. `default-member` also only accepts a single package path in
this nargo version (1.0.0-beta.22), not an array, so listing both `anchor` and
`lib/identity-anchor` as co-defaults isn't an option either.

```bash
cd circuits/oprf-identity-anchor

nargo compile --workspace     # 5 ACIR artifacts into target/
nargo test --workspace        # 18 tests (15 in identity_anchor, 3 in oprf_identity_anchor_query)
nargo info --workspace        # opcode counts

# End-to-end on the query circuit (the only one runnable without a committee):
nargo execute --package oprf_identity_anchor_query query_witness
mkdir -p target/bb
bb write_vk --scheme ultra_honk -b target/oprf_identity_anchor_query.json -o target/bb
bb prove    --scheme ultra_honk -b target/oprf_identity_anchor_query.json \
            -w target/query_witness.gz -k target/bb/vk -o target/bb
bb verify   --scheme ultra_honk -k target/bb/vk -p target/bb/proof -i target/bb/public_inputs
```

`query/Prover.toml` holds a fixture built on ZKPassport's own `SAMPLE_DG1` specimen. Its
`comm_in` is printed by
`nargo test --package oprf_identity_anchor_query --show-output`; regenerate it there if any
salted value changes.

Verification keys can be produced for all five circuits (`bb write_vk`) without needing a live
committee — VK generation only needs the compiled ACIR, not a satisfying witness. Their
`vk_hash` values under bb 0.82.2 are recorded in changelog entry 69; changelog entry 74
reconfirmed `bb write_vk` succeeds under bb 5.0.0 for the original four (now-larger,
post-5-committee) circuits, and changelog entry 76 confirmed it again for the new
`migrate-disclosure` circuit — but none of this byte-diffs the resulting `vk_hash`es against
entry 69's **or against each other**: do not treat any of these as stable identifiers yet, and
expect a fresh `vk_hash` once a genuine committee response lets `anchor`/`disclosure`/
`migrate`/`migrate-disclosure` actually execute.

---

## What is NOT done

Honest list. Several of these are blocking.

### Blocking — nothing here produces a real anchor without them

- **No OPRF committee service exists — now needed 5x over.** This is the big one, and
  changelog entry 74's 5-committee extension makes it strictly larger, not smaller: the
  circuits now assume **5 independent** threshold committees, each evaluating the blinded
  query under its own secret key and returning its own Chaum-Pedersen DLog-equality proof.
  That is [`TaceoLabs/oprf-service`](https://github.com/TaceoLabs/oprf-service) — a separate,
  self-hostable, Postgres-backed, third-party-audited Rust service — and standing up even
  *one* instance (key generation, threshold split across a committee's members, node
  operation, `DS_DLOG` configuration), let alone 5 independently-keyed ones plus the founding
  DKG ceremonies changelog entry 73 specifies, is **explicitly out of scope for this work and
  was not attempted**. Until at least 5 exist, `anchor`, `disclosure`, `migrate` and
  `migrate-disclosure` cannot be executed at all, only compiled.
- **`anchor`, `disclosure`, `migrate`, and `migrate-disclosure` have now all been executed —
  but only against `oprf-committee-dev`'s DEV-ONLY simulator, not a real committee.**
  `anchor`/`disclosure` in changelog entry 078, `migrate`/`migrate-disclosure` in entry 081.
  All four have a real solved witness, a real bb 5.0.0 proof, and a `bb verify` accepting it.
  This closes the "never been executed" gap specifically — it does **not** touch the actual
  blocker above: the simulator's 5 (or, for migrate, 10) key pairs are one process's RNG
  output, not real committees, so nothing here proves anything about a genuine citizen's
  identity.
- **Rust verifier — real for registration, reverification, and migration as of changelog
  entry 76 (all three of the "recompute and check `param_commitment`" family); the actual
  committee service is still the unbuilt part.** `runtime/src/anchor_verifier.rs`'s
  `Poseidon2AnchorVerifier` implements obligation 3 (`param_commitment` recomputation, via the
  `pallets/poseidon2-bn254` crate — a from-source port of `noir-lang/noir`'s own Poseidon2
  blackbox-solver implementation, validated against real `nargo`-produced test vectors) for
  all three extrinsics, each now accepting the outer `zk_proof`/`public_inputs` directly
  (`reverify_citizen`/`migrate_oprf_scheme` were restructured in entry 76 to match
  `register_citizen`'s shape — no more bare proof-bytes parameter). `pallet-identity` checks
  obligation 1 (the governance-approved per-committee-slot key registry, `OprfCommitteeKeys`)
  and obligation 4 (`current_date` freshness) directly in all three calls. Obligation 2
  (`certificate_registry_root`/`circuit_registry_root` allowlisting) is `AllowedMerkleRoots`,
  unchanged since before entry 75. See changelog entry 76 for the full trail.
- **Verifier-crate compatibility is no longer the open question it was.** Changelog entry 72
  landed the bb 5.0.0 port of `ultrahonk-no-std` for the passport (`outer`) verifier, and entry
  074 confirmed `bb write_vk --scheme ultra_honk` succeeds under bb **5.0.0** for all four
  circuits in this workspace, including the two largest (`migrate` at 433,740 ACIR opcodes /
  555,380 circuit size). Proof-level round-trips (not just VK generation) are now confirmed too
  — entries 078 and 081, via the dev simulator — for all four, the same way entry 72 confirmed
  it for the passport `outer` circuit. **Still open at the same level entry 72 closed for
  `outer`**: no circuit here has been proven against a *real* committee response, only the
  simulator's.

### Not verified

- **`disclosure`'s and `migrate-disclosure`'s outer-circuit integration have never been run
  inside an actual outer proof.** Both 8-field layouts are now empirically confirmed against a
  real, committee-simulator-backed `bb prove` run (entries 078/081), but "the standalone
  subproof's layout matches" is not the same as "the outer circuit's own recursive
  verification accepts it". Confirming that needs the full outer-proof assembly pipeline
  (a genuine ZKPassport passport proof) plus a committee, neither of which exists yet.
- **The 8-field layout was measured on a stubbed copy** of `disclosure` (in scratch, not in
  this repo) with the `verified_oprf` call replaced, since the real circuit cannot execute.
  Only the function body differed; the signature and returns were identical, and the ABI
  depends only on those. Still worth re-confirming against the real circuit once a committee
  exists.
- **Whether the deployment country populates the personal-number field at all.** Entry 67
  could not settle this for Israel from public sources; it needs a real passport sample. The
  all-filler assert makes the failure loud, but does not answer the question.
- **The circuits have never seen a real passport.** All testing is against ZKPassport's
  synthetic `SAMPLE_DG1`/`ID_CARD_SAMPLE_DG1` specimens.
- **No independent cryptographic review.** The OPRF library itself is audited (Least Authority,
  report in the `oprf-nr` repo); this fork's use of it is not.

### Known design limits, by choice

- **`migrate`/`migrate-disclosure` are only sound while the outgoing committee is honest.** A
  leaked outgoing key lets an attacker fabricate an `old_anchor` for an input they do not hold.
  This is why entry 67 routes a *suspected* break through `pallet-emergency-council`'s
  emergency rotation and its disclosed degraded mode rather than through this circuit.
- **`disclosure`/`migrate-disclosure` compile with one warning each** — "Return variable
  contains a constant value" for the three zero returns. They are required by the fixed
  outer-circuit interface and cannot be dropped. Expected, benign.
- **No DG11 support.** Entry 67 evaluated the explicitly-labelled `5F10` "Personal number" tag
  in DG11 and recommended against it: it would need a whole new data-group parser, integrity
  check and SOD hash-list entry, for a field that is *more* optional than DG1's slot. Nothing
  here revisits that.
- **`current_date` is trusted from the outer proof**, as upstream does. The chain must decide
  what freshness window it accepts.
