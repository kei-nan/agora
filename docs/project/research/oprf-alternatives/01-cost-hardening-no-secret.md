# Cost-Hardening / No-Secret Approaches

*Research note, 2026-08-11. Evaluates replacing Agora's 5-committee OPRF identity anchor with a
purely local, secret-free hardening function (VDF and/or memory-hard KDF). Nothing in this note is
implemented; no code was changed.*

**Bottom line up front:** the arithmetic does not work, and it fails for a reason that is more
fundamental than parameter tuning — see [Verdict](#verdict). The one salvageable idea is
[k-anonymous bucketed anchors](#the-one-salvage-worth-considering), which is a different design, not
a tuned version of this one.

---

## Mechanism

Today the anchor is `OPRF_k(Poseidon2(DS_IDENTITY_INPUT, personal_number, issuing_country))`,
where `k` is threshold-shared across 5 committees
(`circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`). The secret `k` is the *entire*
reason a committee has to exist and stay online.

The no-secret family replaces `OPRF_k(·)` with a deliberately expensive but fully public function:

```
anchor = Hard(Poseidon2(DS_IDENTITY_INPUT, personal_number, issuing_country), public_salt, cost)
```

with three candidate `Hard`:

1. **Memory-hard KDF** — Argon2id / scrypt. Cost is imposed as *memory bandwidth × area-time*, which
   is what actually punishes GPUs and ASICs. Argon2id is the current standard (RFC 9106: 2 GiB /
   t=1 / p=4, or 64 MiB / t=3 / p=4 when memory is tight).
2. **VDF** — Wesolowski or Pietrzak: `y = x^(2^T)` in a group of unknown order, plus a proof π that
   the T squarings were done. Wesolowski's π is one group element, verified with ~2 exponentiations
   by a 128-bit prime; Pietrzak's π is `O(log T)` elements with `O(log T)` verification. Both need
   either an RSA modulus (trusted setup — i.e. a ceremony, which reintroduces exactly the kind of
   external dependency we are trying to delete) or a class group of an imaginary quadratic field (no
   setup, roughly an order of magnitude slower and far worse in-circuit).
3. **ZK-friendly sequential hash** — an iterated Poseidon2 chain (or MinRoot/Sloth++-style algebraic
   VDF) computed *inside* the Noir circuit, so no separate proof of correct evaluation is needed.

The dedup property is preserved trivially in all three: `Hard` is deterministic, so the same
`(personal_number, issuing_country)` always yields the same anchor, and
`IdentityAnchorRegistry<(scheme_version, anchor)>` in `pallets/pallet-identity/src/lib.rs` works
completely unchanged.

### Which of these can actually live in Agora's circuit

This is the first hard constraint, and it eliminates option 1 immediately.

The anchor is not computed on a server — it must be derived **inside a Noir/UltraHonk circuit**, from
DG1 bytes that the ZKPassport commitment chain has already authenticated, because that binding is the
only thing that stops a registrant from simply making up a national ID. So `Hard` must either be
computed in-circuit, or be accompanied by a proof that is *verified* in-circuit.

- **Argon2id / scrypt in-circuit: infeasible by ~4 orders of magnitude.** Argon2id at the low
  RFC 9106 setting (64 MiB, t=3) performs on the order of 2^20 Blake2b-based compressions over 1 KiB
  blocks. A Blake2b/SHA-2-class compression in a BN254 circuit is ~2–3×10^4 gates even with
  Barretenberg's optimized blackbox gadgets. That is ~2×10^10 gates. Mobile UltraHonk proving is
  practical in the ~10^6–10^7 gate range (ZKPassport quotes <10 s for an ECDSA-verification proof on
  a modern Android phone, and iOS already hits RAM limits at much smaller sizes —
  [ZKPassport FAQ](https://docs.zkpassport.id/faq)). Proving an Argon2 evaluation is not a tuning
  problem; it is off by 3–4 orders of magnitude, and the memory-hardness is exactly what makes it
  un-arithmetizable. Proving it in a zkVM with folding is the same 10^10-cycle problem wearing a hat.
  **There is no succinct proof of an Argon2 evaluation.** So the primitive that is economically
  *best* for this job is the one Agora structurally cannot use.
- **Wesolowski VDF verified in-circuit: plausible but heavy.** Verification is ~256 modular
  squarings/multiplications on a 2048-bit RSA modulus; with `noir_rsa`/bignum that lands somewhere
  around 1–5×10^6 gates — the upper edge of a phone's budget, on top of the existing ZKPassport outer
  proof. Class-group verification (the version without a trusted setup) is far worse and should be
  assumed impractical.
- **Iterated Poseidon2 in-circuit: cheap per unit, but the unit is tiny.** At a generous ~2^22-gate
  budget and ~30–60 constraints per Poseidon2-t3 permutation, you can afford roughly 10^5 sequential
  permutations.

### Why VDFs are the wrong tool here specifically

The assignment's framing — "an attacker can throw thousands of cores at a parallelizable hash but not
at a VDF" — is true for a *single* evaluation and false for the attack that matters. An exhaustive
dictionary search over candidate national IDs is **embarrassingly parallel across candidates**. The
attacker never needs to speed up one VDF evaluation; they run 10^6 independent evaluations
concurrently. Sequentiality bounds *latency*, and the metric that decides this question is
**total dollars per evaluation**, not latency. On that metric VDFs are actively bad: repeated modular
squaring is the single most ASIC-friendly workload in cryptography (Chia's timelord ASICs reach ~10^6
IPS as a 3-VDF cluster — [Chia timelord docs](https://docs.chia.net/timelord-architecture/)), whereas
memory-hardness is specifically designed to make dedicated hardware *not* help.

Two further strikes against the VDF branch:

- The SNARK-friendly algebraic VDFs — MinRoot, VeeDo, Sloth++ — had their sequentiality assumption
  broken at CRYPTO 2024: exponentiation latency can be reduced by parallel computation, contradicting
  the design's core premise ([Cryptanalysis of Algebraic Verifiable Delay
  Functions](https://eprint.iacr.org/2024/873)). Ethereum consequently declined to ship MinRoot and
  **dropped VDFs from its roadmap entirely** in the 2024 "Splurge" writeup
  ([Buterin](https://vitalik.eth.limo/general/2024/10/29/futures6.html)). Adopting a primitive that
  the best-funded team in the space just abandoned, for a national election system, needs a much
  better reason than "it removes a committee".
- The sound branch (groups of unknown order) needs either an RSA trusted setup ceremony — a live
  multi-party ritual, i.e. the thing we were trying to avoid — or class groups, which the circuit
  cannot afford.

**So the realistic no-secret design is: an iterated Poseidon2 chain in-circuit.** Everything below
prices that, with the VDF variant as an optimistic upper bound.

---

## Why this removes the external-service dependency

This part genuinely delivers, and the operational saving is large — it deserves to be stated fairly
before the numbers demolish it.

- No secret exists, so nothing needs custody, DKG, resharing, or rotation ceremonies.
- No liveness requirement. The current design is **n-of-n across 5 committees** (changelog #73) with a
  48h–7day SLA; registration is a post-and-wait-and-retry loop. Cost-hardening makes registration a
  single synchronous offline operation: scan passport → prove → submit. That is a materially better
  citizen experience and removes an entire class of "I couldn't register" failure.
- Deletes, or makes dead, a very large amount of built infrastructure: `committee/`,
  `committee-node/`, `oprf-committee-dev/`, the DKG ceremony work, TPM sourcing, device logistics for
  35 founding members, the `submit_oprf_query`/`submit_oprf_response` mailbox (call indices 15/16),
  `dlog_verify`, `OprfCommitteeKeys`, `committee_slot_for`, and the `query`/DLEQ circuits.
- Removes the founding-phase bootstrap paradox entirely (5×7-person founding groups holding
  disproportionate power for up to 4 years).
- Removes a standing political attack surface: 35 named humans who can be pressured, subpoenaed, or
  bought.

None of that is in dispute. The question is only whether the security it replaces is adequate.

---

## Threat model and parameter estimates

### Stated assumptions

| Assumption | Value used | Basis |
|---|---|---|
| Attacker cloud cost | $0.01 per vCPU-hour ≈ $2.8×10⁻⁶ per core-second | spot/preemptible pricing, order-of-magnitude |
| GPU/ASIC advantage over an honest phone | 10³–10⁵× on $/evaluation | ASIC squaring engines; GPU field-arithmetic farms |
| Honest-user budget | ≤60 s wall clock on a mid-range Android phone | registration UX; must also fit the ZKPassport proof |
| Poseidon2-t3 in UltraHonk | ~30–60 constraints | Barretenberg optimized gadget, order-of-magnitude |
| Native Poseidon2-BN254 | ~1–3 µs/permutation/CPU core; ~10⁷–10⁸/s/GPU | field-arithmetic throughput, order-of-magnitude |
| Value of deanonymizing a national electorate | ≥ $10⁸ to a hostile state | judgment call, stated explicitly |

Every one of these is an estimate. They are all within an order of magnitude, and the conclusion
survives being wrong by two.

### Entropy of the MRZ personal-number field

The circuit reads the ICAO Doc 9303 TD3 personal-number field — in practice the country's national ID
number. Real formats:

| Country | Format | Space, DOB unknown | Space, DOB known |
|---|---|---|---|
| Sweden (personnummer) | `YYMMDD-NNNC` | ~3.7×10⁷ (2²⁵) | ~10³ |
| Estonia (isikukood) | `GYYMMDDSSSC` | ~3.7×10⁷ (2²⁵) | ~10³ |
| South Africa | `YYMMDDSSSSCAZ` | ~3×10⁸ (2²⁸) | ~10⁴ |
| Spain (DNI) | 8 digits + letter | 10⁸ (2²⁷), sequential by issue date so effectively far less | 10⁸ |
| US (SSN, post-2011) | 9 digits | ~10⁹ (2³⁰) | ~10⁹ |
| India (Aadhaar) | 12 digits, first ≠ 0/1 | ~8×10¹¹ (2³⁹) | ~8×10¹¹ |

Note the pattern: **most European national ID formats embed the date of birth**, so adding DOB to the
hash input adds *zero* entropy — it is already in the number. And DOB is typically known for the
people this system most needs to protect (named journalists, dissidents, opposition figures). For
those formats the targeted search space is **~10³**. No cost function survives a 1000-candidate
search space: at 60 s/guess that is 17 core-hours, about $0.17.

So treat 2²⁵–2³⁰ as the realistic untargeted space, 2³⁹ as the optimistic ceiling, and 2¹⁰–2¹³ as the
targeted case.

### What one guess can be made to cost

- **Iterated Poseidon2 in-circuit (the only fully-viable option):** ~10⁵ permutations. Attacker cost
  ≈ 10⁵ × 3 µs = 0.3 core-seconds ≈ **$8×10⁻⁷/guess** on CPU, and roughly **$10⁻⁹/guess** on GPU.
- **Wesolowski VDF (optimistic upper bound):** 60 s of RSA-2048 squaring on a phone ≈ 6×10⁶
  squarings. Against a $10M ASIC farm amortized over 3 years, that is ~**$6×10⁻⁷/guess**.
- **Argon2id at 2 GiB (the number we cannot actually use):** the Specops 2025 benchmark measured
  ~490 H/s on an 8×RTX 5090 rig, with a single ~$2,100 AMD EPYC matching it — parameters
  unfortunately unstated in the coverage
  ([CyberInsider](https://cyberinsider.com/argon2-algorithm-dramatically-slows-password-cracking-by-high-end-gpus/),
  [Finopotamus](https://www.finopotamus.com/post/specops-research-a-2-100-cpu-cracks-argon2-faster-than-an-8-gpu-rig)).
  Amortized hardware plus power puts that near **$10⁻⁶/guess**. Note that this best case is only
  ~1 order of magnitude better than the in-circuit chain, and it is unreachable.

**A ~$10⁻⁶/guess ceiling is the whole story.** It doesn't matter much which primitive you pick.

### Cost to exhaust the space

At $10⁻⁶/guess (the *optimistic* end):

| Search space | Cost of a complete dictionary |
|---|---|
| 2²⁵ (Sweden/Estonia) | ~$34 |
| 2²⁸ | ~$270 |
| 2³⁰ (US SSN) | ~$1,100 |
| 2³⁹ (Aadhaar) | ~$550,000 |
| 2⁴⁷ (needed for $10⁸ security) | ~$10⁸ |

At the realistic in-circuit figure ($10⁻⁹/guess on GPU), divide all of these by 1,000.

And because the salt is public and fixed per country, this is a **one-time cost that deanonymizes the
entire electorate at once**, not a per-victim cost. For a Swedish-format ID, $34 of cloud compute
builds a table mapping every on-chain anchor to a named citizen, permanently.

To reach $10⁸ of attacker cost you need ~2⁴⁷ of input entropy. No national ID format has it. The gap
is **6–20 orders of magnitude** depending on country.

### The attack that ends the discussion

Everything above assumes the attacker has to *guess*. The adversary Agora most needs to defend
against — the incumbent government's interior ministry, which issued the passports — **holds the
national ID registry**. It does not guess. It computes the anchor once per known citizen:

> 10M citizens × 60 s of honest-cost hardening ÷ 1000 cores ≈ **7 core-days ≈ $50**, and it can
> parallelize this to finish in an afternoon.

A public function is evaluable by anyone who knows the input. Cost hardening raises the price of
*searching*; it does nothing at all when there is nothing to search. **This is not a weaker version of
the OPRF guarantee — it is a categorically different one that fails against the primary adversary.**
The OPRF's secret key is not primarily a brute-force defence; it is what makes the anchor
*unevaluable* by a party that knows the input exactly.

For completeness: mixing in a citizen-chosen secret (PIN/passphrase) would defeat the registry
attack, but it destroys Sybil resistance outright — a citizen who deliberately uses two passphrases
gets two anchors — and breaks recovery. Nothing else in a passport is both stable across renewal and
high-entropy; the low entropy is structural, not a choice of field.

---

## Comparison to the current 5-committee OPRF design

| Dimension | 5-committee OPRF | Cost-hardening (no secret) |
|---|---|---|
| Guarantee type | Cryptographic secrecy — anchor is unevaluable without ≥1 honest committee | Economic cost — anchor is evaluable by anyone, just slowly |
| vs. passport-issuing government | **Protects.** State knows every national ID but cannot evaluate the PRF | **Fails.** ~$50 to map the whole registry |
| vs. untargeted outside attacker | Protects (needs to corrupt all 5 committees) | $34–$550k for a whole-population dictionary, depending on ID format |
| vs. targeted attack, DOB-encoding ID | Protects | Fails — ~10³ candidates |
| External live service required | Yes — 5 committees, n-of-n, 35+ members | **None** |
| Registration UX | Post query → wait 48h–7d → retry on timeout | Single offline operation, seconds |
| Liveness/availability risk | High and permanent | Zero |
| Key custody / DKG / resharing | Required, unbuilt, the project's #1 blocker | Not applicable |
| Bootstrap paradox | Real (5×7 founding groups, up to 4 years of concentrated power) | None |
| Post-quantum | Grover halves the exponent but the key stays secret | Grover √-speedup on an already-broken search |
| Parameter ratcheting | Rotate key every ~4 years, `migrate_oprf_scheme` | Same hook works — but ceiling is set by the *oldest phone in the electorate*, which improves slower than attacker hardware |
| Circuit complexity | vOPRF + Chaum-Pedersen DLEQ ×5 | Simpler (delete `query`/DLEQ) but +10⁵ Poseidon2 permutations |
| Code deleted if adopted | — | `committee/`, `committee-node/`, `oprf-committee-dev/`, mailbox extrinsics, `dlog_verify`, `OprfCommitteeKeys` |
| Legal posture (GDPR) | Keyed hash with distributed key — strongest available pseudonymisation | A hashed low-entropy identifier on an immutable public ledger; EU regulators treat this as pseudonymous personal data, not anonymous |

The ratchet row deserves emphasis: because the anchor must stay *stable* for dedup, raising the cost
parameter requires the entire population to re-register. The existing 4-year `scheme_version` rotation
is a genuinely good fit for that hook. But the parameter is bounded by the weakest phone you are
willing to exclude from voting, and inclusivity pressure pushes that *down* over time while ASIC and
GPU economics push attacker capability *up*. The ratchet runs the wrong way.

---

## Integration feasibility with Agora's stack

If adopted, the changes are well-localized — this is the least of the problems:

- **`circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`** — `derive_identity_input()` stays
  exactly as-is (it already produces the right OPRF client input). `derive_committee_anchor_term()`,
  `combine_committee_anchors()`, `NUM_COMMITTEES`, `DS_DLOG`, `DS_ANCHOR_OUT` all disappear, replaced
  by one `harden(input, salt, rounds)`.
- **Circuits** — `query/` and the DLEQ verification path can be deleted outright; `anchor/`,
  `migrate/`, `disclosure/`, `migrate-disclosure/` keep their shape with a different derivation.
  Roughly half the workspace goes away.
- **`pallets/pallet-identity/src/lib.rs`** — `IdentityAnchorRegistry`, `CitizenAnchor`,
  `AnchorAlreadyUsed`, `NewAnchorAlreadyUsed`, `OprfSchemeVersion` and `migrate_oprf_scheme` are all
  **unchanged**; the dedup surface is untouched. Removed: `OprfCommitteeKeys`, `check_committee_keys`,
  `PendingOprfQueries`/`OprfResponses`, `submit_oprf_query`/`submit_oprf_response` (call indices
  15/16 — a call-index change is a runtime-upgrade concern), `dlog_verify`, `committee_slot_for`,
  `OprfQuerySlaBlocks`. `AnchorProofVerifier`'s `param_commitment` drops `oprf_pk_hashes`.
- **Mobile** (`RegisterScreen.tsx`) — the post-query/poll-for-5-responses/retry state machine
  collapses to a single local computation. This is the biggest UX win in the whole proposal.
- **The one new cost:** ~10⁵ Poseidon2 permutations added to a proof that already has to fit in phone
  RAM alongside the ZKPassport outer proof — and iOS RAM limits are already a known pain point for
  Barretenberg proving. This needs measuring before anything else.

**Side observation, unrelated to this proposal but found while reading the code:**
`submit_oprf_query` gates on `Self::is_citizen(&who)`, but a citizen needs OPRF responses *before*
they can call `register_citizen`. As written, a new registrant cannot obtain the anchor they need in
order to become a citizen. That looks like a real circular gap in the current OPRF path and is worth
checking independently of this research note. (It is also, incidentally, what currently rate-limits
the committee-as-oracle dictionary attack — the vOPRF's own known weakness, where an attacker with
query access can build a dictionary through the committee.)

---

## Open questions / what would need to be measured or validated before adopting this

1. **The launch country's actual MRZ personal-number format.** The entire decision reduces to this one
   fact. If it is Aadhaar-shaped (2³⁹, no DOB embedded) the picture is bad but arguable; if it is
   Swedish/Estonian-shaped (2²⁵, DOB embedded) it is unarguable. Also still unconfirmed from the
   existing work: whether the country populates the field at all.
2. **Measured mobile proving ceiling.** How many sequential Poseidon2 permutations can a $150 Android
   phone prove in ≤60 s *on top of* the ZKPassport outer proof, and what does iOS RAM allow? This
   single number caps the achievable hardening.
3. **Measured attacker throughput.** Native and GPU Poseidon2-BN254 permutations/second, to replace the
   1–3 µs estimate with a real defender:attacker ratio.
4. **Legal review.** EU regulators (EDPB, Spanish AEPD) have published guidance that hashing
   low-entropy identifiers yields pseudonymised — not anonymised — personal data. Writing that to an
   immutable public ledger with no deletion path is a distinct legal problem from the cryptographic
   one, and it applies to *any* anchor scheme, but cost-hardening makes it much harder to defend.
5. Whether the 4-year `scheme_version` rotation can realistically carry a population-wide
   re-registration event, given it would now be a *security requirement* rather than hygiene.

### The one salvage worth considering

**Truncated / bucketed anchors for k-anonymity.** Publish only ~20 bits of the hardened anchor, so
each anchor value collides with thousands of citizens. Inversion then returns a *set*, not a person —
which blunts even the registry-holder attack, because the ministry learns only which bucket a citizen
falls in. The cost is that dedup stops being exact: a collision becomes "flag for secondary review"
rather than "reject", which needs a human or court process and is itself an attack surface (a
deliberate collision becomes a denial-of-registration weapon). This is a genuinely different design
with a genuinely different security story, and it is the only variant in this family that survives
the registry-holder attack. It deserves its own note rather than a paragraph here.

---

## Real-world precedent

Searching specifically for cost-hardening applied to *identity deduplication* (not password storage)
turned up no production system doing it — and a consistent record of the opposite conclusion:

- **Signal / mobile contact discovery** is the closest and most instructive analogue: dedup on a
  low-entropy identifier (phone numbers) via hashing. Academic work showed hashed phone numbers can be
  reversed in **0.14 ms** by lookup and **<0.5 ms** by brute force, and concluded that hashing phone
  numbers "does not provide any protection"
  ([Hagen et al., ACM TOPS 2022](https://dl.acm.org/doi/10.1145/3546191);
  [project site](https://contact-discovery.github.io/)). Signal's answer was **not** a bigger cost
  factor — it was trusted hardware plus rate limiting
  ([Signal blog](https://signal.org/blog/private-contact-discovery/)). Nobody in this space believed
  cost hardening could close a low-entropy gap.
- **Privacy-preserving record linkage** (national statistics, health registries) reached the same
  place independently: plaintext hashes of national identifiers are dictionary-attackable, so the
  field standardized on **keyed** hashes with the key held by a trusted third party who never sees the
  data ([Hall & Fienberg PPRL survey](https://www.cs.cmu.edu/~rjhall/linkage_survey_final.pdf);
  [NIH PPRL strategy](https://www.nia.nih.gov/sites/default/files/2023-08/pprl-linkage-strategies-preliminary-report.pdf);
  [French public statistics](https://link.springer.com/article/10.1186/s12911-016-0366-4)). That
  trusted third party is structurally the same role as Agora's OPRF committee — and even keyed-hash
  PPRL has been attacked by graph-matching
  ([Vidanage et al.](https://users.cecs.anu.edu.au/~Peter.Christen/publications/vidanage2020cikm.pdf)).
- **World ID** — the only large deployed system solving Agora's exact problem — went the *other* way,
  twice: OPRF nodes for nullifiers where "computing candidate nullifiers is infeasible without
  collusion of a threshold of OPRF nodes", and an SMPC system for iris-code uniqueness
  ([World ID protocol specs](https://github.com/worldcoin/world-id-protocol/blob/main/docs/world-id-4-specs/README.md);
  [Large-Scale MPC, arXiv 2405.04463](https://arxiv.org/pdf/2405.04463)). They accepted the live-service
  burden deliberately.
- **VDFs for anything identity-shaped:** nothing found. The relevant VDF literature is randomness
  beacons and consensus. The two load-bearing datapoints are negative:
  [Cryptanalysis of Algebraic VDFs (CRYPTO 2024)](https://eprint.iacr.org/2024/873) breaking
  MinRoot/VeeDo/Sloth++ sequentiality, and
  [Ethereum dropping VDFs from its roadmap](https://vitalik.eth.limo/general/2024/10/29/futures6.html).
  Background: [Boneh–Bünz–Fisch VDF survey](https://crypto.stanford.edu/~dabo/pubs/papers/VDFsurvey.pdf).
- **Memory-hard functions** are well-founded as a primitive — the theory is solid
  ([Argon2 spec](https://www.password-hashing.net/argon2-specs.pdf), RFC 9106) — but every deployment
  is password storage, where the input is *chosen* and can be made high-entropy. That is precisely the
  property a national ID lacks.

---

## Verdict

**Not viable as a replacement, and the reason is structural rather than parametric.** The decisive
failure is not that 10⁵ Poseidon2 permutations are too cheap (though they are — a whole-population
dictionary costs $34–$1,100 for typical European ID formats, and the required 2⁴⁷ of input entropy
exceeds every real national ID format by 6–20 orders of magnitude). It is that the adversary this
system most needs to withstand — the passport-issuing state — already holds every citizen's national
ID and therefore never has to search: it computes the public function directly for ~$50 and maps the
entire on-chain registry. Cost hardening prices *guessing*; the OPRF's secret makes the function
*unevaluable*. Those are different guarantees, and only the second one is adequate for a
government-facing electoral roll. The secondary findings reinforce this: memory-hard functions, the
only economically respectable primitive in the family, cannot be proven in a ZK circuit at all (off by
~10⁴), and the SNARK-friendly VDFs that could be were cryptanalytically broken in 2024 and abandoned
by Ethereum. **The single biggest risk if this were adopted anyway is irreversibility**: anchors are
written to an immutable public ledger, an attacker can precompute the dictionary *before* registration
even opens, and the day someone publishes the table there is no key to rotate and no way to unpublish
— the electorate is deanonymized permanently. The 5-committee OPRF's operational burden is real and
this note does not minimize it, but it buys a guarantee this family cannot approximate. The
constructive next step is not tuning cost parameters; it is either the k-anonymous bucketed-anchor
variant above, or a different branch of the OPRF-alternatives search that keeps a secret in existence
while reducing who has to stay online to hold it.
