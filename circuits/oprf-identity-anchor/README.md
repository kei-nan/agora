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

Four binaries and one shared library. The OPRF protocol is inherently two-round (blind → the
committee evaluates → unblind), so a single circuit cannot cover it; `query` is round one and
`anchor`/`disclosure` are round two.

| Package | ACIR opcodes | bb circuit size | Role |
|---|---:|---:|---|
| `query` | 6,733 | 12,113 | Round 1. Fork of `oprf-auth`. Blinds the identity input for the committee. |
| `anchor` | 44,462 | 60,168 | Round 2, standalone. Verifies the committee's response, emits the anchor. |
| `disclosure` | 46,423 | 63,473 | Round 2, **pipeline-integrated**. Same, shaped as an outer-circuit disclosure subproof. Adds an expiry check. |
| `migrate` | 87,715 | 115,113 | Dual evaluation under two committee generations, for OPRF scheme rotation. |

For reference, unmodified upstream `oprf-auth` measures **6,644** ACIR opcodes with the same
toolchain. The fork costs **+89 opcodes** — the personal-number slice, the populated check,
the country binding and the clear-value assertions.

`lib/identity-anchor` holds the derivation shared by all four, so the value `query` blinds and
the value `anchor` verifies cannot drift apart.

### Derivation

```
personal_number   = DG1[77..91]                     // flat MRZ 72..86 = TD3 line 2 pos 29-42
identity_input    = Poseidon2(DS_IDENTITY_INPUT,
                              pack_be(personal_number[14]),
                              pack_be(issuing_country[3]))
oprf_output       = verified_oprf(..., identity_input, DS_DLOG, DS_ANCHOR_OUT)
anchor            = Poseidon2(DS_ANCHOR, oprf_output, scheme_version)
```

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

### `oprf_identity_anchor` — 4 fields

| # | Name | Notes |
|---|---|---|
| 0 | `comm_in` | |
| 1 | `scheme_version` | must be non-zero; matches `pallet-identity`'s `OprfSchemeVersion` |
| 2 | `anchor` | the value stored in `IdentityAnchorRegistry` |
| 3 | `oprf_pk_hash` | `Poseidon2(pk.x, pk.y)` — which committee key produced the evaluation |

Maps to `AnchorProofVerifier::verify_registration_anchor(proof_bytes, anchor, scheme_version)`
and, with an identical layout, to `verify_reverification(proof_bytes, anchor)`.

### `oprf_identity_anchor_migrate` — 7 fields

| # | Name |
|---|---|
| 0 | `comm_in` |
| 1 | `old_scheme_version` |
| 2 | `new_scheme_version` |
| 3 | `old_anchor` |
| 4 | `new_anchor` |
| 5 | `old_oprf_pk_hash` |
| 6 | `new_oprf_pk_hash` |

Maps to `AnchorProofVerifier::verify_migration(proof_bytes, old_anchor, new_anchor)`.

There is exactly one `identity_input` binding in that circuit and both `verified_oprf` calls
consume it, so the two anchors are provably same-input by construction — no explicit equality
constraint is needed or present.

### `oprf_identity_anchor_disclosure` — 8 fields

| # | Name | Value |
|---|---|---|
| 0 | `comm_in` | |
| 1 | `current_date` | unix timestamp; drives the expiry check |
| 2 | `service_scope` | unused by this circuit; present to hold its slot |
| 3 | `service_subscope` | unused by this circuit; present to hold its slot |
| 4 | `param_commitment` | `Poseidon2(200, anchor, scheme_version, oprf_pk_hash)` |
| 5 | `nullifier_type` | always 0 |
| 6 | `scoped_nullifier` | always 0 |
| 7 | `oprf_pk_hash` | always 0 |

This is exactly the 8-field vector `prepare_disclosure_inputs` (upstream
`src/noir/lib/outer/src/lib.nr`) feeds to `verify_proof_with_type`, confirmed by counting a
real proof's public inputs (256 bytes = 8 × 32) and checking each slot's value. Slots 5–7 are
constants and are **not** optimised away by the compiler — they occupy their positions, which
is the "non-participating" mode the outer circuit already supports for facematch subproofs.

`200` is an Agora-specific proof-type tag. ZKPassport's own `PROOF_TYPE_*` constants currently
occupy 0–10; 200 is far enough outside that range to stay unambiguous if upstream adds more.

The anchor committee's `oprf_pk_hash` rides inside `param_commitment` rather than in slot 7,
because slot 7 belongs to ZKPassport's salted-*nullifier* OPRF — a different key for a
different purpose.

### Why `disclosure` exists, and why a Rust verifier must use it

`anchor` alone is **not safe to verify on-chain**. It proves the anchor came from the DG1
committed at `comm_in`, but `comm_in` is a *private* witness inside ZKPassport's outer
circuit — it is not among that proof's public inputs. So a standalone `anchor` proof and an
outer passport proof cannot be linked to each other on-chain, and a prover could pair a
genuine outer proof with an `anchor` proof over a `comm_in` of their own invention. That
defeats the entire Sybil check.

`disclosure` closes this by riding inside the outer proof, so there is one proof and one
`comm_in`, authenticated all the way back to `certificate_registry_root`.

Keep `anchor` for testing and for any future flow that publishes `comm_in` some other way, but
**a production `AnchorProofVerifier` should be built on `disclosure`.**

## What the Rust verifier still has to enforce

The circuits cannot check these; they are on-chain obligations.

1. **`scheme_version` → `oprf_pk_hash` binding.** A proof asserts *which* committee key was
   used; nothing in-circuit says that key is the legitimate one for that scheme version. The
   chain must hold the governance-approved committee public key per scheme version and reject
   any mismatch. Without this, anyone who stands up their own OPRF key can mint unlimited
   anchors.
2. **`certificate_registry_root` / `circuit_registry_root` allowlisting.** Both are plain
   public inputs of the outer proof. `pallet-identity`'s existing `AllowedMerkleRoots` pattern
   is the natural home. The Agora-governed circuit registry must include these forked
   circuits' vkey hashes.
3. **`param_commitment` recomputation.** Recompute `Poseidon2(200, anchor, scheme_version,
   oprf_pk_hash)` from the values submitted with the extrinsic and check it equals the outer
   proof's `param_commitments[i]`.
4. **`current_date` freshness**, so an old proof cannot be replayed past the passport's expiry.

---

## Reproducing the build

Toolchain — already installed on the dev machine; do not reinstall:

- `nargo` **1.0.0-beta.22** at `/home/realize/.nargo/bin/nargo`. This matches the upstream
  repo's own pin (`.github/workflows/test.yml`, and `@noir-lang/noir_js: ^1.0.0-beta.22` in
  `package.json`). Changelog entry 65 found beta.25 crashed the *Rarimo* circuit with an ICE;
  that does not apply here — beta.22 compiles both the unmodified upstream circuit and this
  fork cleanly, verified in this session.
- `bb` **0.82.2** at `/home/realize/.bb/bb`.

```bash
cd circuits/oprf-identity-anchor

nargo compile --workspace     # 4 ACIR artifacts into target/
nargo test --workspace        # 15 tests
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

Verification keys can be produced for all four circuits (`bb write_vk`). Their `vk_hash`
values under bb 0.82.2 are recorded in changelog entry 69, but **expect them to change** under
bb 5.0.0 — do not treat them as stable identifiers yet.

---

## What is NOT done

Honest list. Several of these are blocking.

### Blocking — nothing here produces a real anchor without them

- **No OPRF committee service exists.** This is the big one. The circuits assume a threshold
  committee that evaluates the blinded query under a secret key and returns a Chaum-Pedersen
  DLog-equality proof. That is [`TaceoLabs/oprf-service`](https://github.com/TaceoLabs/oprf-service)
  — a separate, self-hostable, Postgres-backed, third-party-audited Rust service — and
  standing one up (key generation, threshold split across Agora's governance parties, node
  operation, `DS_DLOG` configuration) is **explicitly out of scope for this work and was not
  attempted**. Until it exists, `anchor`, `disclosure` and `migrate` cannot be executed at
  all, only compiled.
- **`anchor`, `disclosure` and `migrate` have never been executed.** They compile and produce
  verification keys, and their constraint systems are counted, but no witness has ever been
  solved for them because that needs a live committee response. Only `query` has a real
  witness, proof and verification.
- **No Rust verifier.** `runtime/src/verifier.rs` has no anchor verifier, and
  `PassthroughAnchorVerifier` in `runtime/src/configs/mod.rs` still accepts every proof in
  both build paths. The four on-chain obligations listed above are all unimplemented. (This
  file deliberately does not touch `runtime/src/verifier.rs`.)
- **Verifier-crate compatibility is still unresolved**, exactly as changelog entry 66 left it.
  bb 0.82.2 produced a valid prove/verify round-trip here, which is a useful data point, but
  it does **not** establish that `ultrahonk-no-std` v0.3.2 (targeting bb ≤ 3.0.3) can consume
  these proofs, nor that bb 5.0.0 — which upstream pins — produces the same format. Nobody has
  byte-diffed anything yet.

### Not verified

- **`disclosure`'s outer-circuit integration has never been run inside an actual outer proof.**
  The 8-field layout is empirically confirmed, but "the layout matches" is not the same as
  "the outer circuit accepts it". Confirming that needs the full subproof pipeline plus a
  committee.
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

- **`migrate` is only sound while the outgoing committee is honest.** A leaked outgoing key
  lets an attacker fabricate an `old_anchor` for an input they do not hold. This is why entry
  67 routes a *suspected* break through `pallet-emergency-council`'s emergency rotation and its
  disclosed degraded mode rather than through this circuit.
- **`disclosure` compiles with one warning** — "Return variable contains a constant value" for
  the three zero returns. They are required by the fixed outer-circuit interface and cannot be
  dropped. Expected, benign.
- **No DG11 support.** Entry 67 evaluated the explicitly-labelled `5F10` "Personal number" tag
  in DG11 and recommended against it: it would need a whole new data-group parser, integrity
  check and SOD hash-list entry, for a field that is *more* optional than DG1's slot. Nothing
  here revisits that.
- **`current_date` is trusted from the outer proof**, as upstream does. The chain must decide
  what freshness window it accepts.
