# Probabilistic Detection + Legal/Social Backstop Approaches

*Research note, 2026-08-11. Scope: can Agora replace or shrink the live OPRF committee by
accepting probabilistic duplicate detection plus `pallet-courts` adjudication, self-declaration
plus bonding/audit, or a general tolerance for residual fraud? Filename is a fixed slot in a
shared numbering scheme; "issuance-time" specifically is covered by a different note.*

**Bottom line up front.** The first mechanism is refuted by arithmetic, not by judgment — a
public collision-tolerant structure over a low-entropy national ID cannot simultaneously flag
duplicates and hide identities, and the gap is ~15 bits wide, not a tuning parameter. The second
mechanism relocates the committee rather than removing it, but relocates it usefully: from a
liveness-critical per-registration service into a batch service. One genuinely free win fell out
of the analysis and is independent of everything else here: **annual reverification does not need
the committee at all** (§ *Reducing reliance*, "The one free win"), which cuts committee duty
cycle roughly 9×.

---

## Mechanism: Bloom-filter-style probable-duplicate flagging + courts adjudication

### What the OPRF is actually buying (this is the part that gets mis-stated)

It is easy to describe the OPRF as "a hash that hides the national ID." That framing is what
makes the Bloom-filter idea look plausible, and it is wrong. `IdentityAnchorRegistry`
(`pallets/pallet-identity/src/lib.rs:353`) publishes a 32-byte anchor per citizen in plain view
of every full node. It is safe to publish *only* because the anchor is
`OPRF_k(national_id, issuing_country)` under a key nobody holds in full — so an adversary cannot
compute candidate anchors offline and compare.

The OPRF's security property is therefore not output secrecy. It is **non-evaluability of the
comparison function without the committee**: a key-gated, rate-limited, on-chain-logged oracle
(`submit_oprf_query`/`submit_oprf_response`, call indices 15/16). That distinction decides this
entire section, because it yields a structural impossibility:

> On a public chain, any duplicate check the runtime can perform with no secret is a check the
> adversary can perform with no secret. There is no such thing as a publicly-verifiable dedup
> structure that resists offline enumeration of a low-entropy input.

Bloom filters, cuckoo filters, truncated hashes, LSH buckets, and "PSI with slack" are all in
that class. False-positive slack does not add entropy; it only adds noise proportional to ε, and
ε is bounded above by the honest system's tolerance for accusing innocent people. The noise is
*symmetric* — it degrades the attack exactly as much as it degrades the legitimate check.

### The entropy arithmetic

Inputs, for a one-chain-per-country deployment (changelog #67's model):

- `n` = registered citizens. Take `n = 10⁷` (a ~13M-population country's electorate).
- `N` = size of the national-ID candidate space. Real national IDs are 8–12 digits:
  `N ≈ 10⁸–10⁹` (27–30 bits). Nordic/Baltic/South African/Chinese schemes **embed the date of
  birth**, so conditioning on a known DOB collapses residual entropy to 10³–10⁵ (10–17 bits).
  National IDs are also routinely known to employers, banks, landlords, and breach corpora — the
  targeted-deanonymization threat model changelog #073 already names (a journalist, a dissident)
  assumes the attacker has the target's ID and wants to learn whether/where they registered.
- Poseidon2 BN254 evaluates at roughly 10⁵–10⁶/s/core. Enumerating `N = 10⁹` is single-digit
  CPU-hours, or minutes on a GPU. Cost is not a barrier at any plausible ID length.

**Bloom filter, sized properly.** With optimal `k = (m/n) ln 2`, `ε = 2⁻ᵏ` and
`m/n = 1.4427 · log₂(1/ε)`:

| ε | bits/element | filter size (n=10⁷) | innocent citizens flagged during rollout (≈ n·ε) | likelihood ratio for a targeted "is X registered?" query (1/ε) |
|---|---|---|---|---|
| 10⁻⁶ | 28.8 | 36 MB | 10 | 1,000,000 |
| 10⁻³ | 14.4 | 18 MB | 10,000 | 1,000 |
| 10⁻² | 9.6 | 12 MB | 100,000 | 100 |
| 10⁻¹ | 4.8 | 6 MB | 1,000,000 | 10 |
| 0.5 | 1.4 | 1.8 MB | 5,000,000 | 2 |

Two constraints pull in opposite directions:

- **Utility** needs `n·ε` to fit court capacity. Each flag is a case; each appeal seats 7 jurors.
  Even 10,000 cases is a national judicial crisis. Realistic ceiling: **ε ≤ 10⁻⁴**.
- **Privacy against targeted confirmation** needs the likelihood ratio near 1. A 50% prior with
  ε = 0.1 becomes a 91% posterior. For LR ≤ 2 you need **ε ≥ 0.5**.

The two requirements are **six orders of magnitude apart, in the wrong direction**. There is no
value of ε that satisfies both. And enumeration is worse than targeted confirmation: at ε = 10⁻⁶
with N = 10⁹, sweeping the whole candidate space returns 10⁷ true hits and ~10³ false ones —
**the attacker reconstructs the national ID of essentially every registered citizen** from public
chain data. Suppressing that to 10% precision requires ε ≥ 9n/N ≈ 0.09, back in the
5-million-false-accusations regime.

**Truncated-hash / bucketing variants fail by the same identity.** With `B` buckets, expected
honest collisions (false flags) is `C ≈ n²/2B`, and the anonymity set per bucket against
enumeration is `A = N/B`. Eliminating `B`:

```
A = 2·C·N / n²
```

With `n = 10⁷` and `N = 10⁹`: **an anonymity set of exactly 1 — i.e. none at all — already costs
50,000 false accusations.** For `N = 10⁸` it costs 500,000. The underlying reason is a pigeonhole
count: distinguishing `n` people pairwise needs on the order of `2·log₂ n = 46.5` bits of
identifier; a 9-digit national ID has 30. The scheme is ~16 bits short before any cryptography is
chosen, and lossy hashing cannot manufacture the missing bits.

**Empirically, this has been run at national scale and it went exactly as the math predicts.**
The US Interstate Crosscheck program matched voter records on name + date of birth — a low-entropy
quasi-identifier, structurally the same object as a truncated commitment. It flagged roughly
**3 million apparent double-voters, of which about 600 were real** ([Ansolabehere
report](https://cdn.factcheck.org/UploadedFiles/AnsolabehereReport.pdf); see also [Goel et al.,
*One Person, One Vote*, APSR](https://www.sas.upenn.edu/~marcmere/workingpapers/OnePersonOneVote.pdf)).
A ~5,000:1 false-to-true ratio, and the flags were used to purge legitimate voters. This is not a
hypothetical failure mode.

### The crux: can an AI judge adjudicate "same underlying identity" without seeing the PII?

This is the load-bearing question, and it does not survive contact.

The flag says: account B collides with account A. The court must decide whether B's
`(national_id, DOB)` equals A's. There are exactly four ways to get that answer, and each one
fails:

**(a) Plaintext disclosure to the court.** The forum is an AI judge (a Claude API call from
`court-oracle/`, which publishes its reasoning to public IPFS — `court-oracle/src/ipfs.rs`) and,
on appeal, **7 randomly sortitioned citizens** with pseudonymous accounts, no vetting and no
clearance (`select_jury`, `pallet-courts/src/lib.rs:488`). Handing a citizen's national ID to a
third-party LLM API and to 7 random strangers is a strictly worse disclosure than the on-chain
leak the whole architecture exists to prevent — and it is *targeted*, which is the exact threat
model changelog #073 optimized against when it dropped national ID from the committee-slot hash.

  It is worse still than it first appears. To *compare*, the court needs **both** values. So a
  false positive forces an innocent bystander (A) to disclose their national ID too. Every unit of
  false-positive slack you deliberately added is a quota of **forced privacy violations of
  innocent people**. In the OPRF design ε = 0 and no such disclosure event exists. The
  probabilistic design does not trade certainty for convenience; it trades certainty for a
  budgeted number of privacy violations, which inverts the design's premise.

**(b) A zero-knowledge inequality proof between the two parties.** Conceptually available: a
2-party private equality test where each side's input is bound to their authenticated passport by
a ZKPassport disclosure subproof. No committee needed. But (i) it requires both parties online and
cooperating, which is the *same* liveness dependency the OPRF was criticized for, relocated onto
every accused citizen; (ii) "refusal to cooperate ⇒ guilty" makes non-participation a conviction,
turning an unanswered notification, a lost phone, or a hospital stay into disenfranchisement, and
handing griefers a one-click weapon; (iii) it is pairwise, so *detecting* an unknown duplicate
rather than adjudicating a nominated one requires running it against the whole roll — an `O(n)`
PSI per registration over a low-entropy set, which is offline-brute-forceable again by the § above.

**(c) A trusted institutional referee** — a civil-registry official or the Elections Commission
sees both plaintexts in camera and testifies a boolean to the court. This is what real courts do
with sealed evidence, and it works. But it is a permanent institution holding a **plaintext PII
database**, which is a heavier and more attackable trust object than the OPRF committee (which
holds key shares and never sees plaintext). Note the real-world precedent lands the same way:
**ERIC**, the cross-state matcher that actually works, works because it is a confidential central
data processor operating on member states' plaintext under legal agreement — not a public
structure. The successful version of this pattern *is* a trusted secret-holding intermediary.

**(d) Non-identity, behavioral evidence** — shared device fingerprints, IPs, funding sources,
correlated voting. This is how real fraud investigation works, and it is **architecturally
unavailable here by deliberate design**: MACI/Semaphore make votes unlinkable on purpose; there is
no device or network telemetry and the project would not want any.

So the crux resolves negatively and cleanly:

> An AI judge can adjudicate identity-equality only by (i) receiving the PII, which is a worse
> disclosure than the one being avoided, or (ii) receiving a boolean from some oracle that already
> did the comparison — in which case **that oracle, not the court, is the Sybil-resistance
> mechanism**, and we have re-derived the thing we were trying to delete.

### Prerequisites that do not exist in the codebase

Even setting the crux aside, `pallet-courts` cannot host this today:

- **There is no evidence channel.** `file_case(origin, subject: CaseSubject)` takes *only* the
  enum (`pallets/pallet-courts/src/lib.rs:400`). `court-oracle/src/context.rs:44` states plainly
  that `CitizenConduct` "carries little on-chain context beyond the subject." The AI judge would
  be ruling on a case with no record.
- **There is no respondent.** No call lets an accused citizen answer, submit anything, or be
  notified. The pipeline is file → AI rules → optional appeal. Notice, response, discovery,
  representation, and a standard of proof all have to be designed and built.
- **The accusation target is trivially addressable.** `CitizenNullifier` is a public
  `StorageMap<AccountId → nullifier>` (`pallets/pallet-identity/src/lib.rs:284`), so anyone can
  read any citizen's nullifier off-chain state and file `CitizenConduct` against them.

---

## Mechanism: self-declaration + bonding + spot audit

The existing pattern (`declare_no_other_passport`, call index 10, backed by `CitizenConduct` —
changelog #67/#68) is a deterrent-plus-remedy for one narrow residual gap. The proposal is to
extend it: attest uniqueness, post a bond, and cryptographically verify only a sampled subset.

**The one part that genuinely works.** A sampled audit still needs a comparison oracle, so the
committee does not disappear — but its *duty cycle* collapses. This converts the committee from a
liveness-critical service (5-of-5 n-of-n combination, a 6-day `OprfQuerySlaBlocks` SLA, and
changelog #73's disclosed and uncorrected correlated-unavailability risk) into a **batch service
that can take weeks and retry freely**. That is a real reduction in the operational burden that
motivated this whole research thread. It removes the SLA. It does not remove the DKG ceremony, the
key custody, the member vetting, the governance, or the institution.

**Three things break the deterrence story, and they are not fixable by parameter choice.**

1. **Unlinkable Sybils destroy the audit multiplier.** IRS-style audit works because catching one
   understatement exposes the taxpayer's *entire* return, plus penalties, plus criminal exposure
   for a *named* person. Here, by design, Sybil accounts are mutually unlinkable and untraceable to
   the human behind them. Catching one Sybil at sampling rate `p` tells you **nothing** about the
   others and reaches no real person. An attacker registering `S` Sybils expects to lose `p·S` of
   them and keep `(1−p)·S`. Sampling is a linear tax on fraud, not a deterrent.
2. **Slashing has nothing to bite.** The only asset at risk is the bond posted by the fake identity.
   The attacker's real identity and real vote are unreachable — that is the anonymity guarantee
   working as specified. So deterrence reduces to "the attacker prices in the bond."
3. **Any bond large enough to deter is a poll tax.** Deterrence needs `p · bond > per-Sybil gain`.
   A referendum with billions at stake decided by 10,000 votes prices a marginal vote in the
   ~$10⁵ range; at `p = 1%` that implies a ~$10⁷ bond per registration. Meanwhile any
   *non-trivial* bond is legally unavailable in the jurisdictions this project targets: US 24th
   Amendment (1964) and *Harper v. Virginia Bd. of Elections*, 383 U.S. 663 (1966) foreclose
   wealth-conditioned voting; ICCPR Art. 25 and ECHR Protocol 1 Art. 3 point the same way. A
   state-funded bond deters nobody. **This is a hard blocker, not a tradeoff.**

**Timing kills the remedy even when detection succeeds.** Votes are receipt-free and unlinkable, so
once an epoch is tallied a fraudulent ballot cannot be identified or subtracted. Exclusion is only
possible *before* tally. A contested case needs an AI ruling, then `AppealWindowBlocks = 7 days`
(`runtime/src/configs/mod.rs:586`), then a jury seating and vote — realistically 2–4 weeks. So any
registration inside roughly the last month before an epoch is effectively unauditable, and an
attacker will simply register there. The real-world fix is standard and worth adopting regardless
of which mechanism wins: **a registration cutoff N days before each voting epoch**, exactly as
every real jurisdiction uses a 15–30 day registration deadline.

**Precedent, honestly read.** Tax self-assessment is the canonical "trust but verify with teeth"
system and it is a *leaky* one: [IRS](https://www.irs.gov/statistics/irs-the-tax-gap) projects a
voluntary compliance rate of **85%** and a **$696B** gross tax gap for TY2022, with the individual
audit rate down to about **0.25%** ([GAO-22-104960](https://www.gao.gov/products/gao-22-104960)).
That is the *success case* for this model — and it leaks ~15% of the base. Elections are routinely
decided by margins under 1%. The tolerance that makes the model acceptable for revenue collection
is two orders of magnitude too loose for vote eligibility.

---

## Real-world precedent for accepting residual fraud risk

Real voter rolls are not cryptographically sound and never have been. The stack is: attestation
under penalty of perjury → post-hoc list maintenance (NVRA §8, DMV/SSA/death/NCOA feeds, ERIC
cross-state matching) → criminal prosecution. Measured residual fraud is very small: the [Brennan
Center](https://www.brennancenter.org/our-work/research-reports/debunking-voter-fraud-myth) reports
incident rates between **0.0003% and 0.0025%**; [News21/ASU](https://votingrights.news21.com/article/election-fraud/)
found **10 cases of in-person impersonation over 12 years** across ~146M registered voters. Even
the heaviest machinery ever built leaves residue: UIDAI has cancelled roughly **600,000 duplicate
Aadhaars** — about 145/day over nine years — despite centralized 1:N biometric dedup at 1.3B scale,
and the CAG audit criticized the dedup pipeline for it.

**So residual fraud is normal and a courts backstop is a legitimate supplement.** The question is
whether it can be load-bearing *here*, and the honest answer is that Agora has deliberately removed
the three properties that let real democracies tolerate leaky rolls:

| Property that bounds fraud in real elections | Present in Agora? |
|---|---|
| High marginal human cost per fraudulent unit (show up, sign, risk a felony) | **No** — once the crypto is passed, marginal Sybil cost ≈ 0 and fraud is remote and scriptable |
| Linkability of a fraudulent ballot back to a person for prosecution | **No** — unlinkability is a designed-in guarantee |
| A durable, independently re-countable record (paper, risk-limiting audits) | **No** — receipt-freeness removes the audit trail on purpose |
| Reversibility (courts can void and re-run an election) | Partially, via governance — but with no way to identify which ballots were fraudulent |

The uncomfortable conclusion: **Agora needs *more* prevention than a paper democracy, not less.**
Real systems can be relaxed at the roll because they are strict downstream. Agora is maximally
strict downstream about privacy, which is precisely what forces strictness at the roll.

The one precedent that maps cleanly is **hybrid tiering by event type** — heavy verification at the
rare event, cheap continuity checks in between. That is how passport and driving-licence regimes
already work. Applied here it does not remove the committee, but it does shrink the committee's job
by an order of magnitude (next section).

### Reducing reliance: the one free win

`reverify_citizen` (`pallets/pallet-identity/src/lib.rs`) currently demands a **fresh 5-committee
OPRF anchor evaluation every year** (`ReverificationPeriod = 365 * DAYS`,
`runtime/src/configs/mod.rs:277`). It compares the recomputed anchor to `CitizenAnchor` and does
**no exclusion check** — it is a continuity check, not a uniqueness check.

For a citizen whose passport has not changed, that is redundant. The ZKPassport **scoped nullifier
is stable for an unchanged document** and is already bound to the account in `NullifierRegistry`.
A fresh outer proof carrying the same nullifier proves everything reverification needs — same
person, same still-valid document — with no committee involvement whatsoever. The anchor is only
load-bearing when the *document changes* (renewal), which is exactly the case it was designed for.

The load this removes is not marginal. At `n = 10⁷` and annual reverification: ~27,400
reverifications/day, each requiring `t = 12` responses from each of 5 committees ⇒ **~1.6M
`submit_oprf_response` extrinsics/day, ≈ 114 per 6-second block, forever** — carried by 175 member
devices, several of which are phones and Raspberry Pis (changelog #82/#83). Restricting the
committee to first registrations (~1.2×10⁵/yr) plus renewals (~10⁶/yr) drops committee-required
events from ~10⁷/yr to ~1.1×10⁶/yr: **a ~9× reduction, and it removes the standing per-citizen
annual obligation entirely.** This is independent of every other idea in this document and appears
to be a straightforward win. It should be validated as its own work item.

*(A further reduction — chaining a renewal to the previous document by proving possession of both
old and new passports, so renewals also skip the committee — is adjacent to the issuance-time note
and is flagged there rather than developed here. Its obvious gaps: lost or physically cancelled old
documents, and it never covers first registration.)*

---

## Comparison table

| Dimension | OPRF committee (current) | Probabilistic flagging + courts backstop |
|---|---|---|
| What the adversary must break | Threshold key material in ≥1 of 5 committees (n-of-n combination: one honest committee suffices) | Nothing cryptographic — enumerate a 27–30-bit space offline |
| Published on-chain | 32-byte anchor, not offline-computable without the key | Filter/bucket structure that is, by construction, a public membership oracle |
| Recovers the whole electorate's national IDs from public data? | No | **Yes**, at any ε compatible with usable dedup |
| Innocent citizens accused | 0 | 10⁴–10⁶ per rollout; ≥50,000 to buy an anonymity set of *one* |
| Who ever sees plaintext PII | Nobody — the query is blinded | The AI judge (a third-party LLM API), 7 sortitioned citizens, and public IPFS reasoning |
| Prevention vs. detection | Prevention at registration | Detection after the fact — and unlinkable ballots mean post-tally detection has no remedy |
| Liveness requirement | Hard: 5-of-5 committees within a 6-day SLA, every registration *and* every annual reverification | None at registration; instead an accused citizen must be reachable and cooperative on demand |
| Institutional burden | DKG, key custody, 175 devices, member accountability, rotation governance | Court capacity for 10⁴–10⁶ identity trials, a discovery/evidence subsystem, sealed-PII handling |
| Legal viability | Fine | Bonding variant runs into poll-tax doctrine (*Harper*, 24th Am.) |
| Failure mode | Registration stalls (retryable; no false accusations) | Mass false accusation, forced PII disclosure, permanent public record against innocents |
| Build status in this repo | Circuits, mailbox, Wasm core, DKG tooling all real; **no live committee** | Filter design refuted above; `file_case` has no evidence field, no respondent, no PII channel |

---

## Griefing and abuse risk

Verified against the code, not assumed:

- **Filing a case is effectively free.** `auto_finalize` releases the bond **in full regardless of
  verdict** — the comment at `pallets/pallet-courts/src/lib.rs:739-747` says so explicitly and
  explains why no bad-faith slashing was invented. There is no loser-pays rule. `CaseFilingBond`
  (`ConstU128<1_000_000_000_000>`) is an interest-free deposit, not a cost.
- **Targets are trivially addressable**, since `CitizenNullifier` is publicly readable state.
- **The accusation is the payload.** Under mechanism 1, the point of filing is not to win — it is
  to force the target into a disclosure proceeding. A griefer accuses a journalist, the court
  process demands the identity comparison, and the target either discloses or refuses. That is a
  deanonymization weapon dressed as due process, and it is *cheaper* than any attack the current
  design admits.
- **Reputational harm survives acquittal.** Cases and rulings are permanent public chain state.
  There is no expungement.
- **Retry-until-favorable-jury.** Bonds are refunded, there is no double-jeopardy rule, and the
  jury seed retains a disclosed residual "last revealer" manipulation window
  (`JurySeedDelayBlocks = 10 * MINUTES`; see the extensive doc comment on `Config::JurySeedDelayBlocks`).
  A determined attacker can refile until a 4-of-7 majority lands their way.
- **The presumption dilemma has a hard deadline.** Suspend on accusation and griefing
  disenfranchises innocents on schedule; don't suspend and Sybils vote before resolution. Batched
  voting epochs make this a fixed date, not a soft tradeoff.
- **Automated flagging is a court-DoS vector.** If a filter hit auto-files a case
  (`auto_file_case` exists and bypasses bonding entirely), then anything that induces collisions
  induces unbonded case load.

Minimum mitigations if any version of this is pursued: loser-pays or partial bond forfeiture on
frivolous filings; a rate limit per filer against `CitizenConduct`; an explicit respondent role
with a response window; a sealed-evidence channel that never reaches the AI oracle or IPFS; and
double-jeopardy semantics.

---

## Open questions

1. Does any target jurisdiction have a national ID with genuinely ≥64 bits of non-public entropy?
   If one existed, the § arithmetic changes qualitatively and this whole family reopens. Nothing
   found suggests one does.
2. Is the reverification finding correct in full? Specifically: is the ZKPassport scoped nullifier
   provably stable across *any* re-scan of an unchanged document (including chip re-personalization
   or a reissued-but-same-number document), and does the outer proof expose it in a form
   `reverify_citizen` can check? This needs verification against the real circuit before being
   treated as banked.
3. What is the actual court throughput ceiling, in cases/month, given a 7-citizen jury drawn from
   an `n`-person electorate? Nobody has sized it. Mechanism 1 needs 10⁴–10⁶; the answer is
   probably ~10²–10³.
4. Can `pallet-courts` ever handle sealed evidence, given that the AI judge is a third-party API
   call and rulings publish to public IPFS? If not, the court is structurally incapable of
   adjudicating any PII question, which constrains more than this document.
5. Does MACI, as this project will deploy it, actually support excluding a suspended citizen's
   signup before an epoch tally? The entire "detect and remedy before the epoch closes" story
   depends on it and it has not been confirmed.
6. Is a registration cutoff before each epoch acceptable, and what should it be?

---

## Verdict

**Mechanism 1 is not a real alternative — it is refuted, not merely disfavoured.** The arithmetic
is decisive in two independent places: a national ID carries ~30 bits where pairwise
distinguishability among 10⁷ people needs ~46, so no public collision-tolerant structure can be
simultaneously useful and private (an anonymity set of *one* already costs 50,000 false
accusations); and the false-positive slack that was supposed to buy privacy is in fact a budgeted
quota of forced PII disclosures by innocent people, which inverts the design's premise rather than
relaxing it. Crosscheck ran this experiment at national scale — 3 million flags, ~600 real — and
the outcome is on the record. The PII-in-court crux then closes the door from the other side: the
AI judge can only decide identity-equality by receiving the PII (a worse disclosure than the one
being avoided, to a third-party API and 7 random citizens, with the reasoning published to IPFS) or
by consuming a boolean from an oracle that already made the comparison — and that oracle *is* the
mechanism we were trying to delete. So yes, this family relocates the problem into a different,
harder pipeline, and the destination pipeline is the least mature component in the project.
**Mechanism 2 is more honest but only half a win**: shifting the committee from
every-registration to sampled-audit genuinely converts a liveness-critical service into a batch
service, which is worth real money in operational burden — but it removes the SLA, not the
institution, and its deterrence story is broken from three directions at once (unlinkable Sybils
make audits a linear tax rather than a deterrent; slashing can only reach the fake identity's own
bond; and any bond large enough to matter is a poll tax). The genuinely valuable output of this
research is narrower and unrelated to the framing: **annual reverification is doing a committee
round-trip it does not need**, and fixing that cuts committee duty cycle ~9× and eliminates the
standing per-citizen annual obligation — without weakening Sybil resistance at all. Pursue that;
do not pursue publishing a probabilistic identity structure on a public chain.
