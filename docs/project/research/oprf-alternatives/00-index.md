# OPRF-Alternatives Research — Index

*Research round, 2026-08-11. Five independent research passes, run in parallel, each pressure-testing
a different family of alternatives to Agora's live 5-committee OPRF identity-anchor service. Nothing
here is decided or implemented — this is a research corpus to reason from, not a changelog entry.
Each document was written by an agent with no visibility into the others' conclusions, then read and
synthesized here afterward.*

## The question this round was asked to answer

Agora's Sybil-resistance design needs a value derived from a citizen's national ID that is (a) the
same every time the same person registers, even across a passport renewal, and (b) invertible by
nobody — not the chain, not a service, not an attacker with unlimited offline compute. The current
answer is an OPRF: a committee holds a threshold-shared secret; a citizen's blinded query is
evaluated without the committee ever seeing the real ID. That requires a live, perpetually-available
committee — and building one from scratch has become the project's largest standing blocker (see
`docs/project/changelog/073.md`, `082.md`, `083.md`, `085.md`). The project owner asked: is there a
genuinely good alternative that avoids this dependency, using pure cryptography, other blockchains,
or novel mechanisms — and if not, what's the honest reason why not?

## Bottom line

**No alternative found removes the live-secret-holding service while preserving the guarantee Agora
actually needs.** That guarantee, precisely: the anchor must stay unevaluable even by a party that
*already knows* the national ID — because Agora's own threat model names the passport-issuing
government itself as a plausible adversary (a state deanonymizing a dissident or journalist it
already has on file). Every non-OPRF family tried here fails against exactly that adversary, for
structurally different reasons each time:

- **Cost-hardening** (no secret at all) prices *guessing*. The issuing government doesn't guess — it
  already knows every citizen's ID and can evaluate the public function directly for the whole
  population in an afternoon for about $50.
- **Probabilistic/Bloom-filter flagging + courts** is refuted by arithmetic before it gets to
  politics: a national ID has ~30 bits of entropy, distinguishing 10 million citizens pairwise needs
  ~46, and every filter design that's useful for dedup is also useful for the attacker — there's no
  ε that serves both. (This is real-world tested, not just theory: the US Interstate Crosscheck
  program ran almost exactly this experiment on ~146M voters and produced ~3M flags for ~600 real
  duplicates.)
- **Reusing an already-live external threshold network** (Internet Computer's vetKeys, Human Network,
  Threshold Network, etc.) turns out to require either a network that isn't actually *blind* (vetKD
  hides the derived key, not the input — a citizen's national ID would travel to 34 node operators in
  the clear) or a full rewrite of Agora's Noir circuits onto an incompatible curve.
- **Folding the committee into Agora's own validator set** is not structurally impossible the way the
  project's notes currently claim (that specific reasoning is wrong, see doc 03) — but Agora's actual
  validator set is ≤32 sudo-appointed accounts with no staking pallet, which is a *much* smaller and
  less independent trust base than 5×35 sortitioned citizens, and adopting it would collapse two
  currently-independent failure domains (chain integrity and identity secrecy) into one.
- **Issuance-time credentials** — having the passport-issuing government itself attest uniqueness,
  instead of reconstructing it cryptographically after the fact — is the one idea that structurally
  works and elegantly kills the reason the OPRF exists in the first place (renewal-stability). But it
  requires the government to change its passport/eID issuance pipeline, which is a multi-year policy
  and standards effort, not something this project can build alone. It also quietly assumes the
  issuing government is honest about not double-issuing — which is exactly the assumption the current
  OPRF design was built to *not* need.

This isn't a failure of the research — it's real signal. **World ID, the only other large deployed
system solving this exact problem, independently converged on the same architecture**: a distributed
threshold OPRF network for nullifier derivation (see doc 04). Two well-resourced teams landing on the
same design from different directions is evidence the design is closer to necessary than accidental.

## What to actually do with this

The "no clean escape" finding doesn't mean nothing changed. Five concrete, independently actionable
items fell out of this research, each smaller and more valuable than the original ask:

1. **Self-host the OPRF service instead of building one from scratch.** The project already rejected
   depending on TACEO's *live network* (vendor lock-in). Doc 02 makes the case that this rejection
   was correctly scoped to the network, not the code — `TaceoLabs/oprf-service` is Apache-2.0/MIT,
   includes a working DKG, and Agora's circuits are already wire-compatible with it. Running five
   instances of it under Agora's own governance replaces the largest unbuilt, unaudited piece of the
   plan with production-proven code, at zero vendor lock-in. → [doc 02](02-existing-threshold-networks.md)

2. **Move from a 5×35 sortitioned-citizen committee to a 5×8–15 named-institution committee**,
   confirmed independently by two separate research passes (docs 02 and 03) that never saw each
   other's work. Every real production threshold network surveyed — drand, Internet Computer, Sui
   Seal, TACEO itself — converged on 7–35 named, professionally-operated nodes; *none* converged on
   sortition. The institutional model also directly fixes the specific liveness problem (5–7 day SLA
   on consumer phones/Pis) that's been the project's own standing complaint, and it was already flagged
   in changelog #082 as "deserving its own adversarial review round" — it has now had two, from
   different angles, with the same answer. → [doc 02](02-existing-threshold-networks.md), [doc 03](03-validator-native-threshold.md)

3. **Fix `reverify_citizen` to skip the committee entirely for an unchanged passport.** It currently
   re-runs the full 5-committee evaluation every year, but it's a *continuity* check (same nullifier,
   same document), not a *uniqueness* check — the committee is only load-bearing on renewal. This is
   free, doesn't touch the security model, and cuts committee-required events roughly 9× (~10⁷/yr
   → ~1.1×10⁶/yr). → [doc 05](05-issuance-time-and-social-backstop.md)

4. **Check whether the anchor can be reformulated as a deterministic threshold BLS signature on a
   blinded input.** If so, it inherits 2024's "silent setup" threshold-cryptography results — no
   interactive DKG ceremony at all, which is the single most concretely blocking piece of the current
   plan (no production system anywhere runs a live DKG at 35 sortitioned parties, per the project's
   own prior research). This is checkable in days, not years, and doesn't require abandoning the
   current security model. → [doc 04](04-alternative-crypto-primitives.md)

5. **Correct changelog #082's stated reasoning for rejecting "validators as the committee."** The
   conclusion (don't do it) is right, but the given reason ("computed on-chain is public by
   construction") is factually wrong — BEEFY and off-chain workers are real, live counterexamples.
   The actual reasons are that Agora's validator set is far too small/permissioned for this role and
   that it would collapse two independent trust boundaries into one. Worth fixing so a future reader
   doesn't waste time re-deriving the correction. → [doc 03](03-validator-native-threshold.md)

Two more, smaller but worth knowing about:

- **Possible real bug**, unrelated to this research question: `submit_oprf_query` gates on
  `Self::is_citizen(&who)`, but a new registrant needs the OPRF responses *before* `register_citizen`
  can succeed — as written, a new registrant may not be able to obtain the anchor they need to become
  a citizen. Flagged in doc 01; worth checking independently.
- **Add a registration cutoff before each voting epoch** (e.g. 15–30 days, matching how every real
  jurisdiction runs voter registration). Doc 05 notes that because ballots are receipt-free and
  unlinkable, any dispute resolution after tally has no remedy — so late registrations need to be
  closed off before an epoch, regardless of which identity architecture is used.

## The longer-term parallel track

**Issuance-time credentials** (doc 04) is worth pursuing as a genuine parallel track, not because
it's ready now but because it's architecturally the right answer if the political dependency ever
clears: the EU's eIDAS 2.0 / EUDI Wallet framework already mandates ZKP support and explicitly
discusses derived pseudonyms in its Architecture Reference Framework, so the infrastructure is being
built by someone else on a public timeline. The honest framing: pursue this as a second registration
path a government can opt into, with the security level of each path visible on-chain rather than
averaged away — not as a replacement for the OPRF path this year.

## Documents in this folder

| Doc | Question | Verdict |
|---|---|---|
| [01-cost-hardening-no-secret.md](01-cost-hardening-no-secret.md) | Can a VDF or memory-hard KDF replace the secret entirely? | No — defeated by any adversary who already knows the input, not just guessers. Salvage idea: k-anonymous bucketed anchors (different design, own note). |
| [02-existing-threshold-networks.md](02-existing-threshold-networks.md) | Can Agora consume an already-live external threshold network instead of building one? | No live network is both blind and circuit-compatible — but self-host TACEO's permissively-licensed code under Agora's own institutional operators. |
| [03-validator-native-threshold.md](03-validator-native-threshold.md) | Can Agora's own validators be the committee, via off-chain workers/a BEEFY-like gadget? | No — not for the reason on record (which is wrong and should be corrected), but because Agora's validator set is too small/permissioned and merges two trust domains that should stay separate. |
| [04-alternative-crypto-primitives.md](04-alternative-crypto-primitives.md) | Is there a genuinely different cryptographic primitive (PSI, witness encryption, issuance-time credentials, equality-only FHE)? | Issuance-time credentials structurally work and kill the renewal-stability requirement that forces the OPRF to exist — but it's a multi-year government-adoption dependency, not an engineering task. Everything else collapses back into "a party holds a secret" or fails on low-entropy input. |
| [05-issuance-time-and-social-backstop.md](05-issuance-time-and-social-backstop.md) | Can probabilistic flagging + the AI-court system, or self-declaration + bonding, replace the committee? | No — refuted by entropy arithmetic (Bloom-filter dedup and privacy pull in opposite directions by 6 orders of magnitude) and by law (bonding large enough to deter is a poll tax). Free, unrelated win found: annual reverification doesn't need the committee at all. |
| [06-world-id-considered.md](06-world-id-considered.md) | Can Agora just depend on World ID instead of building this? | No, on all three readings (replace the stack / borrow their OPRF network / accept as an optional path) — their non-Orb path has the same renewal-stability weakness, their OPRF network is scoped to their own ecosystem, and their operator has an active 2025-2026 pattern of government bans/shutdowns (Philippines, Thailand, Kenya, South Korea), which is disqualifying for a real-government-adoption platform regardless of the cryptography. |
| [07-treasury-funded-infrastructure.md](07-treasury-funded-infrastructure.md) | Can Agora's own budget/treasury system pay for the committee instead of needing external funding? | Yes — the department-budget/legislature-motion/audit-hook machinery already exists and needs no new pallet work, just a department id and a motion. Must be a flat stipend (not per-query or registrant-funded — already ruled out in changelog #073 for poll-tax and bad-incentive reasons). Resolves a gap in the institutional-operator recommendation (who pays named operators). The funding *source* still depends on a real government's tax revenue via the planned stablecoin bridge; a no-government deployment would need new fee-routing work that doesn't exist yet. |
| [08-cloud-hosting-providers.md](08-cloud-hosting-providers.md) | Which crypto-payment-friendly cloud provider should host committee nodes? | Wrong question — a shared Agora-controlled provider/account collapses the 12-of-35 threshold into one admin credential. Answer is a vetted **menu** for each independent operator to choose from (Cherry Servers, Vultr, 1984 Hosting, FlokiNET, Phala Cloud for confidential computing; Akash as a pilot slot only, weaker than expected in 2026), with an institution's own infrastructure ranked first, plus a checkable concentration cap so the menu doesn't quietly re-centralize by default convergence. |
| [09-cloud-security-hardening.md](09-cloud-security-hardening.md) | How should `committee-node` be hardened for cloud deployment? | Concrete, code-grounded spec: the passphrase must never share a disk with the encrypted key file (the existing examples do); confidential VMs help but their attestation is forgeable (TEE.Fail, Oct 2025) so enable them without trusting them; diversity across the 5 committees' provider/region/jurisdiction matters more than any single node control. Surfaced a real blocking bug in `submit_oprf_response`'s extrinsic encoding — fixed the same session. 35-item ranked checklist. |
| [10-cloud-deployment-summary.md](10-cloud-deployment-summary.md) | What did this actually produce? | A working deployment path added to `committee-node/deploy/` (hardened compose file, host-hardening script, secret-manager wrapper, operator runbook) plus the extrinsic bug fix — not just documents. Signed/verified release images and a few deeper code hardening items (graceful shutdown, zeroization) remain open, deliberately not done in this pass. |
| [11-genuine-threshold-evaluation-design.md](11-genuine-threshold-evaluation-design.md) | Fix the "one server is the whole committee" gap — build real per-member secret sharing. | **Implemented end to end.** Option A (in-circuit combination) was attempted and abandoned mid-build — needs unsound non-native field arithmetic in the circuit. Option B (a FROST-adapted 2-round protocol, combination done off-circuit in Rust) was built instead: genuine threshold protocol + wasm FFI core in `oprf-committee-dev` (verified against the real Noir dependency, 47/47 tests), a real 2-round on-chain mailbox in `pallet-identity` (99/99 tests), Noir circuits unchanged, and `committee-node`'s orchestration loop fully wired to all of it (34/34 tests, zero-warning build). Never run against a real chain or a live multi-node exchange — no chain runs in this environment and no real committee/DKG ceremony exists yet. |

Each document has its own `## Verdict` section and cites real, checked sources — start there if you
only have time for one section per doc.
