# Alternative Cryptographic Primitives for Sybil-Resistant Deduplication

Research note, 2026-08-11. Scope: primitives *other than* OPRF, VDF/memory-hard cost-hardening, or
"reuse someone else's threshold network" (covered by sibling notes 02 and 03) that could answer
Agora's registration problem — **detect that the same human has registered twice, without revealing
the underlying national ID to anyone, and without a perpetually-live external secret-holding
service**.

## The shape of the problem (why the option space is small)

Everything below reduces to one observation, worth stating before the survey because it explains
why so many candidates collapse into each other:

> If the registrant can compute the anchor alone from public parameters, then so can an attacker —
> for every candidate national ID. Any *locally computable, deterministic* map from a low-entropy
> identifier to a published value is offline-brute-forceable by construction.

So a working design must break one of the four assumptions:

| Break | Approach | Status |
|---|---|---|
| (a) *locally computable* | a secret exists somewhere at query time | OPRF/MPC — current design |
| (b) *cheap* | raise per-evaluation cost | VDF / memory-hard KDF — sibling note 02 |
| (c) *low-entropy input* | anchor a high-entropy secret instead, and get uniqueness from whoever issued it | **issuance-time credentials — the main finding here** |
| (d) *published* | move the equality test to a party that already legitimately knows the identifier | the passport-issuing state |

Nothing in the literature escapes this taxonomy. Several candidates below look like escapes and
turn out, on inspection, to be (a) with a different party holding the secret, or (c) in disguise.
That is the useful result: the search space is not "many unexplored primitives", it is basically
these four columns, and Agora has only seriously explored the first.

## Candidates evaluated

### 1. Accumulators + ZK non-membership proofs over an on-chain set

**Mechanism.** Store all prior anchors in a sparse Merkle tree / RSA accumulator on chain. A new
registrant proves in Noir that their anchor is *absent* from the accumulator, publishes the anchor,
and it is inserted. Non-membership in a sparse Merkle tree is O(log N) and trivially cheap in a
SNARK; Agora's stack already does Merkle work of exactly this shape.

**Dependency-removal verdict: fully removes the live service — and fully fails the privacy
requirement.** The registrant still has to compute the anchor deterministically to know which leaf
to prove absence of, so the anchor is locally computable, so the published set is a dictionary an
attacker can grind against offline. The accumulator changes the *data structure*, not the
*information*. Making the leaves individually blinded (`Com(anchor, r_i)`) destroys checkability:
you cannot prove inequality against a hiding commitment whose randomness you do not know.

**Guarantee vs OPRF:** strictly weaker — it is the naive published-hash design with extra steps.

**Feasibility:** trivial, and irrelevant. Worth stating explicitly because "just use an accumulator"
is the first suggestion anyone makes, and it does not survive five minutes of analysis. Note the
accumulator *is* still the right structure to combine with any of the working candidates below.

### 2. FHE-based unbalanced PSI (Microsoft APSI, PEPSI, PSMT)

**Mechanism.** The state of the art for "small client set vs huge server set": the client encrypts
its item under BFV (SEAL), the server homomorphically evaluates a polynomial whose roots are its set
items, returns encrypted results; communication is linear in the client set and logarithmic in the
server's. Microsoft's [APSI](https://github.com/microsoft/APSI) is a real, maintained C++ library;
[PEPSI](https://www.usenix.org/system/files/usenixsecurity24-mahdavi.pdf) (USENIX Sec '24) improves
the unbalanced case further, and
[PSMT](https://petsymposium.org/popets/2024/popets-2024-0114.pdf) (PoPETs 2024) covers the case
where the set is *segmented across multiple holders* — superficially the closest fit to a committee.

**The decisive finding:** APSI's own security model
([Labeled PSI from FHE with Malicious Security](https://www.microsoft.com/en-us/research/publication/labeled-psi-from-fully-homomorphic-encryption-with-malicious-security/),
CCS 2021) adds an **OPRF pre-processing step** specifically to stop the client from brute-forcing
the server's set items. The best-in-class FHE PSI construction, faced with exactly Agora's
low-entropy-items problem, solved it by bolting an OPRF on. That is strong external confirmation
that the project's instinct was right, and it also means adopting APSI does not remove the OPRF, it
inherits it.

**Dependency-removal verdict: not at all.** The FHE evaluation is done *by the party holding the
set*, per query, live. A blockchain cannot be that party — it has no secrets and every node sees
every input.

**Guarantee vs OPRF:** different-in-kind (hides the *query* from the set-holder rather than hiding
the *key* from the querier) but strictly more operationally demanding, and it still needs the OPRF.

**Feasibility:** poor. BFV evaluation is not something a Substrate runtime or a Noir circuit does.

### 3. Public-key encryption with equality test (PKEET) — the "equality-only FHE" question

This is the specific question the assignment asked me to chase: is a scheme restricted to *only*
equality testing meaningfully more practical than general FHE? **Yes, dramatically — and it fails
for the same reason a plain hash fails.**

**Mechanism.** [PKEET](https://www.sciencedirect.com/science/article/abs/pii/S0020025519308771)
(Yang–Tan–Huang–Wong lineage) lets any party test whether two ciphertexts, even under *different*
public keys, encrypt the same plaintext, without any secret key. It is pairing-based and fast —
milliseconds, not the seconds-to-minutes of general FHE. Deterministic public-key encryption
(Bellare–Boldyreva–O'Neill) gives the same public-testability even more cheaply. So the narrow
primitive genuinely exists and genuinely is orders of magnitude more practical than general FHE.

**Why it does not help.** The literature is explicit that PKEET is broken against low-entropy
plaintexts: "the tester can recover the message from a given ciphertext by exhaustively guessing the
message offline, and resisting this attack has been identified as a challenging task". The proposed
fix — Group PKEET (G-PKEET) — works by restricting who may test, i.e. by introducing an authorized
tester holding a trapdoor. That tester is a live secret-holding service. **Equality-only FHE
collapses to either (i) the brute-forceable naive design or (ii) the OPRF committee with a different
name.**

**Verdict on the project's blanket FHE rejection:** it should be *narrowed*, not overturned. The
correct statement is not "FHE is too slow for this" — a narrow equality test is fast. The correct
statement is "public equality-testability is exactly the property that makes low-entropy inputs
grindable, so no equality-test primitive of any efficiency can solve this". That is a stronger and
more durable reason to say no.

### 4. Laconic PSI / non-interactive PSI with a reusable digest

**Mechanism.** [Laconic PSI](https://eprint.iacr.org/2021/728) (TCC 2021, and the pairing-based
[follow-up](https://eprint.iacr.org/2022/529), CCS 2022): a receiver hashes a huge set into a tiny
digest; a sender with one element sends one short message; communication is independent of set size,
and **the digest is reusable across unboundedly many senders**. This is the closest thing in the
literature to "the set lives on chain and nobody needs to be online".

**Dependency-removal verdict: partial, and in the wrong direction.** The reusable object is the
*receiver's digest*, but only the receiver — who holds the corresponding trapdoor — learns the
intersection. Agora needs the *chain* to learn the answer, and the chain cannot hold a trapdoor. You
would end up with a single registry operator holding a decryption key, which is a *worse* trust
concentration than a 5×35 threshold committee, not a better one.

**Guarantee vs OPRF:** weaker (single trusted receiver vs threshold), and it gives the answer to the
wrong party.

**Feasibility:** research-grade pairing constructions, no production library. Not viable.

### 5. Blacklistable anonymous credentials (BLAC / EPID signature-based revocation)

This is the most interesting near-miss and deserves care, because it *looks* like a clean escape.

**Mechanism.** In [EPID](https://eprint.iacr.org/2007/194.pdf) and BLAC, each past registration
publishes a pair `(B_i, K_i = B_i^f_i)` with a *random* base `B_i`. A new registrant, holding secret
`f`, proves in zero knowledge that `K_i ≠ B_i^f` for every entry. Under DDH each published pair
leaks nothing, and — crucially — **no party holds any secret**: the prover can evaluate against any
base themselves. Intel ships this in billions of TPMs; the same machinery gives per-scope stable
pseudonyms via DAA "basenames".

**Why it does not directly solve Agora's problem.** The unlinkability argument requires `f` to be
high-entropy. If `f` is the national ID (or any deterministic function of it), an attacker tests
candidate IDs against every published `(B_i, K_i)` and wins — the *same* offline dictionary attack,
just with an exponentiation per guess. So BLAC/EPID does not break assumption (a); it presupposes a
break of (c), i.e. it presupposes that a high-entropy per-person secret was issued to exactly one
person by someone. **It is a mechanism for enforcing uniqueness of an already-unique credential, not
for establishing uniqueness.**

**Second problem: cost.** Verification (and here, proving) is linear in the list size. The
literature is blunt: this "is impractical as the size approaches thousands of entries", which is why
[PEREA](https://www.freehaven.net/anonbib/cache/perea-tissec11.pdf) and BLACR exist — and PEREA's
fix is a bounded *revocation window*, which is meaningless when the "blacklist" is the entire
citizenry. At national scale (10^6–10^8 prior registrations) an O(N) in-circuit inequality proof per
registration is not a close call.

**Dependency-removal verdict: fully removes the live service, but only atop an issuance-time
uniqueness guarantee, and only at unaffordable cost.** Keep it in mind as the *enforcement* half of
candidate 6, where the list is per-scope and small, not as the dedup mechanism.

### 6. Issuance-time anonymous credentials (DAA / BBS+ / mDL / EUDI-style)

**Mechanism.** Stop trying to establish uniqueness cryptographically after the fact. Get it from the
party that already has a legally enforced, audited, non-duplicative issuance process: the passport-
issuing state. The state signs a ZK-friendly credential over a *citizen-chosen high-entropy secret*;
Agora's circuit verifies that signature and derives a scope-stable pseudonym from the secret. See
the dedicated section below — this is the substantive finding of this note.

**Dependency-removal verdict: fully removes the live service at query time.** Requires an issuer at
enrolment (once per document lifetime), and a published revocation list — both are ordinary PKI
operations, not a perpetual threshold-crypto service.

**Guarantee vs OPRF: different in kind, not weaker.** OPRF gives "no single party, including the
issuer, can link or invert". Issuance-time credentials give "no *verifier* can link or invert; the
issuer knows who it enrolled but not what they do". Agora's own threat model — targeted
deanonymization of named dissidents and journalists — is what makes this a real, not cosmetic,
difference. See below.

**Feasibility with Noir/UltraHonk:** the heavy part is verifying an issuer signature in-circuit.
ZKPassport's circuits already verify RSA-4096 and multiple ECDSA curves non-natively in Noir for the
SOD/DSC chain, so a BBS+ or DAA verification sits in the same cost class as work the project already
ships. It is expensive but not novel.

### 7. Blind-signature "one token per person" issuance

**Mechanism.** Chaumian: a registration authority (Elections Commission, or the passport authority)
checks the national ID *in the clear* against its own duplicate list, then blind-signs a token over
a citizen-chosen secret. The token is unlinkable to the identity at redemption; the chain sees only
an unlinkable credential.

**Dependency-removal verdict: partially.** There is still a live service, but it holds an *ordinary
signing key in an HSM* — no DKG, no resharing, no 175-person sortition ceremony, no n-of-n liveness
requirement. Operationally this is perhaps two orders of magnitude cheaper than the current design,
and it is the thing an existing government institution can actually be asked to run. It can be
upgraded to a threshold signature later without changing the client.

**Guarantee vs OPRF: strictly weaker on one axis.** The issuer sees the national ID in plaintext and
knows the set of enrolled citizens (it already does — it issued the passports). It does not learn
the pseudonym, so it cannot link votes to people. The gap versus OPRF is that a compromised issuer
can silently enrol phantom citizens; the current committee design has the same failure mode if a
threshold is corrupted, so this is a difference of degree.

**Feasibility:** high. This is the most boring, most deployable candidate in the note.

### 8. Silent-setup threshold cryptography — removes the DKG, not the committee

**Mechanism.** [Threshold Encryption with Silent Setup](https://eprint.iacr.org/2024/263)
(Garg–Kolonelos–Policharla–Wang, CRYPTO 2024) derives the joint public key deterministically from
parties' *locally generated* public keys — no interactive DKG, asynchronous setup, dynamic
threshold, multiverse support. There is a maintained
[Rust implementation](https://github.com/guruvamsi-policharla/silent-threshold-encryption);
encryption <7 ms, partial decryption <1 ms. *hinTS: Threshold Signatures with Silent Setup* (IEEE
S&P 2024) does the same for threshold signatures.

**Why it matters here:** the single hardest operational problem in Agora's current plan is
coordinating a real DKG ceremony across 5×35 sortition-selected citizens, and the project's own
review found no production system runs DKG anywhere near that many parties. Silent setup makes
committee formation a *no-ceremony* operation: members publish a key, the group key is a
deterministic function of the published keys, and membership can change dynamically.

**Honest caveat: there is no silent-setup OPRF.** These are encryption and signature schemes. You
cannot drop this into `verified_oprf` and get an anchor. Two possible relevances: (i) if the design
ever migrates from a threshold *PRF* to a threshold *decryption* or *signature* formulation (e.g.
candidate 7 made threshold), silent setup deletes the entire DKG workstream; (ii) it is worth
someone checking whether the anchor could be reformulated as a deterministic threshold *signature*
(BLS is a deterministic, and hinTS gives silent setup for it) evaluated on a blinded input — a
blinded-BLS-signature anchor is functionally an OPRF and would inherit silent setup. That is a
concrete, checkable research question, not a claim.

**Dependency-removal verdict: does not remove the live service; removes the ceremony.** Partial, but
targeted at the exact thing that is currently blocking.

### 9. Witness encryption / witness PRFs / time-lock puzzles

**The theoretically correct answer, and a dead end in practice.** Worth two paragraphs because the
*shape* of the right answer is instructive.

A **witness PRF** ([Zhandry, TCC 2016](https://eprint.iacr.org/2014/301)) is a PRF for an NP
language `L`: anyone can compute `F(x)` *if and only if* they hold a witness for `x ∈ L`. Set `L` =
"there exists a genuine ICAO passport, signed under a trusted DSC, whose personal-number field is
`x`". Then `F(x)` is exactly Agora's anchor: deterministic, stable across renewal (the state re-signs
the same personal number), and **unforgeable by an attacker doing a dictionary attack, because a
guess is not a witness — the attacker cannot forge the issuing state's signature.** A witness PRF
is, precisely, an OPRF with the server deleted. That is the primitive the project actually wants.

It does not exist in usable form. Zhandry's construction needs multilinear maps with multilinearity
~2^d in circuit depth; the survey literature calls these "heavy tools... suffering from many
non-trivial attacks". Constructions avoiding multilinear maps route through iO, and iO remains "far
from practical" even after 2025's [Diamond iO](https://eprint.iacr.org/2025/236). Witness encryption
proper is in the same state — [eprint 2025/1364](https://eprint.iacr.org/2025/1364.pdf) and
[2026/175](https://eprint.iacr.org/2026/175.pdf) are real progress toward *implementable* WE for
restricted languages, but "passport signature chain verification" is squarely general-NP. Time-lock
puzzles are a cost mechanism, i.e. sibling note 02's territory, and give no secrecy against a
patient attacker. **Verdict: dead end for the next several years. Revisit if implementable WE ever
covers signature-verification languages.**

## Real-world proof-of-personhood systems compared

| System | Dedup mechanism | Live external dependency? | Privacy guarantee |
|---|---|---|---|
| [World ID (Orb)](https://world.org/blog/announcements/worldcoin-foundation-unveils-new-smpc-system-deletes-old-iris-codes) | Iris code compared against all prior codes under [SMPC](https://eprint.iacr.org/2024/705) (World Foundation + TACEO, Inversed Tech, Automata) | **Yes** — MPC parties, live at enrolment | Iris codes never decrypted; only comparison distance revealed. Trusts non-collusion of MPC parties |
| [World ID 4.0](https://world.org/blog/engineering/introducing-world-id-4.0) | Same, plus **a distributed OPRF network** (Shamir-shared per-RP key, threshold of nodes) for nullifier computation | **Yes** — OPRF nodes queried per registration | Same as Agora's design. Independently converged on OPRF |
| World ID document credentials | Document number as the uniqueness signal; World's own docs flag the flaw: someone can report a document lost and get a new number | Yes (issuer-adjacent) | Encrypted document copy held as credential |
| [Proof of Humanity](https://proofofhumanity.id/) | Video submission + vouching by existing members, disputes to Kleros | No secret-holder; needs an active juror/challenger economy | **None** — the registry is public identities with faces |
| [BrightID](https://brightid.org/) | Social-graph analysis from seed-trusted nodes | Analysis service / graph maintainers | Pseudonymous; sybil resistance is statistical, repeatedly gamed |
| [Idena](https://idena.io/) | Synchronous global "flip" ceremony every epoch — you must be a human, in real time, at the same moment as everyone else | **No secret holder**, but demands *user* liveness every ~2 weeks | Strong (no biometrics, no ID). Cost: brutal UX, small population |
| [Humanity Protocol](https://docs.humanity.org/) | Palm-vein biometric matched against stored encrypted templates by zkProofer nodes / staked validators | **Yes** — off-chain biometric matching service | Templates off-chain encrypted, hashes on-chain; trusts node operators |
| [Self / self.xyz](https://self.xyz/) | One nullifier per passport per app, derived on-device from passport data | **No** | Strong per-app unlinkability — but a re-issued passport yields a **new** nullifier; explicitly does not detect the same human re-registering with a new document |
| [Rarimo Freedom Tool](https://docs.rarimo.com/freedom-tool/) | Nullifier + commitment in a registration tree | No | Their own docs concede: *"the nullifier can be bypassed by reissuing the identity"* |
| [ZKPassport](https://docs.zkpassport.id/) | Scoped identifiers per app domain | No (but is a node operator on TACEO's live OPRF network for other uses) | Explicitly states it "cannot prevent a single person with multiple passports from onboarding multiple times"; renewal handled by out-of-band "linking protocols", not cryptography |
| [Aadhaar UID Token](https://uidai.gov.in/) (India, ~1.4B people) | UIDAI derives a **per-agency token, unique and stable per resident per agency**, issuer-side | Yes — UIDAI is queried, but it is the identity authority itself, not extra infrastructure | Agency cannot recover the Aadhaar number or correlate across agencies. Trusts UIDAI completely |
| [Austria bPK](https://www.bmdw.gv.at/Ministerium/DasBMDW/Stammzahlenregisterbehoerde/Bereichsspezifische_Personenkennzeichen.html?lang=en) (since ~2004) | SourcePIN Register Authority derives `bPK = H(sourcePIN ‖ sector)` | Issuer-side derivation | Non-invertible, non-correlatable across sectors. Trusts the register authority |
| [German eID Restricted Identification](https://www.bsi.bund.de/EN/Themen/Unternehmen-und-Organisationen/Standards-und-Zertifizierung/Technische-Richtlinien/TR-nach-Thema-sortiert/tr03110/TR-03110_node.html) (BSI TR-03110, live since 2010) | Card computes a sector-specific pseudonym from a chip-held private key — **no online authority in the loop** | **No** | Cross-sector unlinkable, stable within a sector. **Known flaw: card replacement produces a different pseudonym** — the exact renewal problem Agora is trying to solve |
| **Agora (current design)** | 5×35 citizen committees, vOPRF over the MRZ personal-number field, hashed summation | **Yes** — all 5 committees must respond per registration | Anchor unpredictable if ≥1 of 5 committees honest |

Three things fall out of this table:

1. **No production system solves Agora's exact requirement without a live service.** The ones with no
   live secret-holder (Self, Rarimo, ZKPassport, German eID RI) all break on document renewal and
   say so in their own documentation. The ones that survive renewal (Aadhaar, bPK, World ID) all put
   an authority or an MPC network in the loop.
2. **World ID 4.0 independently converged on a threshold OPRF network for nullifier derivation.** The
   largest deployed PoP system, with far more resources, arrived at Agora's architecture. That is a
   meaningful validation of the design and simultaneously bad news for "there must be a cheaper way".
3. **The renewal-stability requirement is the entire source of the OPRF dependency.** Every
   no-committee system in the table would work for Agora if renewal were allowed to reset identity.
   That requirement should be re-litigated explicitly as a product decision, not treated as fixed.

## The issuance-time-credential idea, in depth

### The core inversion

Agora is trying to derive uniqueness from a *field printed in a document*. But the uniqueness fact
it wants is not a property of the field — it is a property of the issuance process. A state issues
at most one valid passport per citizen at a time, backed by a national population register, biometric
deduplication, criminal penalties, and audit. That guarantee already exists, is stronger than
anything a 175-person citizen committee will produce, and is *legally enforceable*. The OPRF exists
to reconstruct, cryptographically and from the outside, a fact the issuer already knows.

The inversion: have the issuer attest the uniqueness fact directly, over a value the citizen chooses.

### The concrete construction

1. Citizen generates a high-entropy secret `s` on device (Secure Enclave / Android Keystore) and
   commits `C = Com(s)`.
2. At passport enrolment or a subsequent eID enrolment, the state verifies identity as it already
   does, checks its register that this person has no live credential, and issues a **multi-message
   signature** (BBS+, or DAA/EPID-style) over `(C, issuing_country, validity, …)`.
3. Agora's anchor becomes `anchor = H(s ‖ scope)` — deterministic, stable, and **high-entropy**, so
   publishing it leaks nothing and cannot be ground.
4. Registration proof, in Noir: "I know `s` opening a commitment `C` that carries a valid issuer
   signature under a trusted national key, and `anchor = H(s ‖ scope)`" — plus the existing
   ZKPassport disclosure checks. No committee, no live query, no blinding round-trip.
5. Renewal: the citizen presents the *same* `s`; the state re-signs. Anchor unchanged. **The renewal
   problem disappears — this is the requirement that forced the OPRF in the first place.**
6. Loss of device / loss of `s`: the state revokes the old credential (published revocation list,
   ordinary PKI), issues a new one over a fresh `s'`. The chain honours the revocation, retires the
   old anchor, admits the new one. This is the only path that touches the chain's uniqueness
   invariant, and it is auditable.

### What real-world standards are closest

- **DAA / EPID** ([Brickell–Chen–Li](https://eprint.iacr.org/2007/194.pdf)) — the exact primitive:
  issuer-guaranteed one-credential-per-device, unlinkable signatures, and per-**basename**
  pseudonyms that are stable within a scope and unlinkable across scopes. Deployed in billions of
  TPMs and in Intel SGX attestation. It is not a research proposal; it is the shipped answer to
  "one entity, many unlinkable appearances".
- **German eID Restricted Identification** (BSI TR-03110) — a *government* deployment of exactly
  this shape since 2010: sector-specific pseudonyms computed on-card, no online authority, provably
  unlinkable across sectors. Its documented weakness is precisely the one step 5 above fixes: a
  replacement card yields a new pseudonym, because the key lives on the chip rather than with the
  citizen. **Agora should put the secret on the citizen's phone, not in the document chip**, and
  that single design change turns a known-broken deployed scheme into a working one.
- **Austria's bPK / India's UID Token** — production proof that "issuer derives a stable,
  non-invertible, sector-scoped identifier" works at national scale (Austria ~20 years, India ~1.4B
  people). Both are hash-based and issuer-computed, so they are trust-the-authority rather than
  ZK — but they establish the *institutional* pattern is acceptable to real governments.
- **EUDI Wallet / eIDAS 2.0** — the closest thing to a mandate. Regulation (EU) 2024/1183 requires
  Member States to integrate "privacy-preserving technologies, such as zero knowledge proof" into
  wallets; the [ARF's ZKP topic](https://eudi.dev/latest/discussion-topics/g-zero-knowledge-proof/)
  evaluates BBS+/BBS# and zk-SNARK approaches, and explicitly discusses **pseudonymity** — "users
  can derive verifiable pseudonyms combining unique attributes with RP-specific context while hiding
  both from providers and RPs" — and *correlation proofs* for RPs that legitimately need to link
  presentations of the same user. That is Agora's requirement, written into an EU framework
  document. The ARF also notes Member States "reserve the right to limit each user to one unique
  PID", i.e. issuance-side uniqueness is already policy.
- **ICAO DTC** — [Type 2 (eMRTD-PC bound)](https://www.icao.int/sites/default/files/TRIP/Publications/Digital-Travel-Credentials-DTC.pdf)
  DTCs are issued *by the passport authority*, which is the hook a uniqueness credential would hang
  on. But current DTC work targets border control, boarding, visas and travel authorisation; there
  is no ZK or pseudonym content in the specification today. DTC is the right *vehicle* and the wrong
  *payload* — a lobbying target, not an integration target.
- **[Personhood credentials](https://arxiv.org/abs/2408.07892)** (Adler, Hitzig, Jain, Siddarth et
  al., 2024; 30+ authors across OpenAI, Microsoft, MIT, Harvard) — the academic framing of exactly
  this architecture: anonymous credentials issued once per human by "a range of trusted
  institutions — governments or otherwise", bounded by nullifiers, verified by ZKP. Useful as the
  citable articulation of the design and its governance tradeoffs.

### How big an ask is this, honestly

Very big, and asymmetric. Getting one government to issue a BBS+ credential over a citizen-chosen
commitment is a multi-year standards-and-legislation effort — new key material, new issuance
software at every passport office, HSMs that support pairing curves (the ARF specifically flags
"lack of pairing curve support in hardware security modules" as an adoption blocker), and a legal
basis. Compared with "write an OPRF committee service", it is not obviously slower in wall-clock
terms — Agora has been blocked on the committee for months and the committee also requires
recruiting and coordinating 175 citizens with legal accountability — but it is not something the
engineering team can unilaterally execute.

Two things make it less hopeless than it sounds. First, **Agora is a project for real government
adoption**; a government willing to adopt a blockchain constitution is, by construction, a
government that could be asked to add a credential to its eID issuance. The political ask and the
project's premise are the same ask. Second, **the EU is already building the pipe** — an eIDAS-2
wallet with mandated ZKP support and per-Member-State unique PIDs is 80% of the required
infrastructure, arriving on a public timeline, funded by someone else.

### The trust model actually changes

This must be stated plainly rather than buried:

- **OPRF committee:** Agora trusts *its own* 5×35 citizens, and specifically that at least one
  committee stays honest. It does *not* trust the passport-issuing government's honesty about
  duplication — a corrupt state issuing two passports to one loyalist is detected, because the
  anchor derives from the national ID and would collide.
- **Issuance-time credential:** Agora trusts *each issuing government* not to issue two uniqueness
  credentials to one person. A state that wants ballot-stuffing can simply mint credentials. There
  is no cryptographic backstop; the backstop is audit, transparency logs of issuance counts, and
  the same institutional accountability that protects paper elections.

For a domestic-adoption scenario — one country running Agora for its own citizens — this may be
acceptable, because that government *already* controls the electoral roll and could stuff it by
other means; the credential adds no new attack. For a multi-country or adversarial-state scenario
it is materially weaker than the OPRF design. **The right question is not which is more secure in
the abstract, but whether Agora's threat model includes a hostile issuing state — and if it does,
the current design should be honest that it also has no answer to that state simply refusing to
issue passports to dissidents.**

A hybrid is available and probably right: **issuance-time credential where a government supports it;
OPRF-anchored fallback where it does not**, with the two paths distinguished on chain so the
security level of each registration is visible rather than averaged away.

## Open questions

1. **Is renewal-stability actually a requirement, or an inherited assumption?** Everything expensive
   in the current design descends from it. What is the concrete attack if a renewed passport creates
   a new anchor and the citizen simply re-registers, with the old anchor retired via a ZK link proof
   using a citizen-held salt? The residual gap is only a citizen who *deliberately* discards their
   salt to double-register — which is a real attack, but a much narrower one than the current design
   implicitly defends against, and it may be addressable by rate-limiting or by a
   cost-hardened fallback (sibling note 02).
2. **Can the anchor be reformulated as a deterministic threshold signature (BLS) on a blinded input?**
   If yes, hinTS-style silent setup deletes the DKG ceremony —
   the single largest unbuilt piece of the current plan — without changing the security argument.
   This is the highest-value item in this list because it is checkable in days, not years.
3. **Does the German eID RI failure mode generalise to any chip-held-key design?** If Agora ever
   considers deriving the anchor from a passport chip key (Active Authentication), it inherits the
   documented renewal break. Confirm this is already ruled out.
4. **What would a transparency log of issuance counts buy?** If a government publishes a signed,
   append-only count of uniqueness credentials issued per period, does that convert "trust the
   issuer" into "detect a cheating issuer after the fact"? This is a Certificate-Transparency-shaped
   idea applied to personhood, and I found no one doing it.
5. **Would a blind-signature issuance service (candidate 7) run by the Elections Commission be
   politically easier than a citizen OPRF committee?** It is far simpler to operate, and Agora
   already has a `pallet-elections` Elections Commission origin to hang it on.

## Verdict

**Top pick: the issuance-time credential (candidate 6), with the citizen-held secret rather than a
chip-held key, and with candidate 7 (blind-signature issuance by the Elections Commission) as the
deployable interim that uses the same client-side shape.** It is the only candidate that fully
removes the perpetually-live secret-holding service, and it does so by solving the renewal problem
outright rather than working around it — which is notable because renewal-stability is the sole
reason the OPRF exists. Everything else surveyed either collapses back into "a party holds a secret
at query time" (FHE-PSI, PKEET's authorized-tester fix, laconic PSI), fails outright on low-entropy
inputs (accumulators, plain equality-testable encryption, BLAC with an ID-derived secret), or is a
decade from practicality (witness PRFs — which are, precisely and frustratingly, the exact primitive
this problem wants).

**Biggest risk:** the trust model genuinely changes, and not in Agora's favour under its own stated
threat model. The OPRF design assumes a possibly-hostile state and defends against it; the issuance
credential assumes the state is honest about not double-issuing, and has no cryptographic recourse
if it isn't. Combined with the fact that no government today issues anything of this shape — ICAO
DTC has the wrong payload, eIDAS 2 has the right direction but no timeline for this specific
credential, and the German deployment that comes closest is broken in exactly the way Agora cares
about — the honest summary is: **this is the architecturally correct answer and a multi-year
political dependency, so it should be pursued as a parallel track and a designed-in second
registration path, not as a replacement that unblocks the project this quarter.** The item that
could unblock this quarter is open question 2 — silent setup for the committee's key material,
which keeps the current cryptography and deletes the ceremony that is actually blocking.

---

### Sources

- [Large-Scale MPC: Scaling Private Iris Code Uniqueness Checks to Millions of Users](https://eprint.iacr.org/2024/705) (World Foundation / TACEO)
- [World Foundation unveils new SMPC system, deletes iris codes](https://world.org/blog/announcements/worldcoin-foundation-unveils-new-smpc-system-deletes-old-iris-codes)
- [Introducing World ID 4.0](https://world.org/blog/engineering/introducing-world-id-4.0) — distributed OPRF network for nullifier computation
- [Microsoft APSI](https://github.com/microsoft/APSI) and [Labeled PSI from FHE with Malicious Security](https://www.microsoft.com/en-us/research/publication/labeled-psi-from-fully-homomorphic-encryption-with-malicious-security/) (CCS 2021)
- [PEPSI: Practically Efficient PSI in the Unbalanced Setting](https://www.usenix.org/system/files/usenixsecurity24-mahdavi.pdf) (USENIX Security 2024)
- [Summation-based Private Segmented Membership Test from Threshold-FHE](https://petsymposium.org/popets/2024/popets-2024-0114.pdf) (PoPETs 2024)
- [Group public key encryption with equality test against offline message recovery attack](https://www.sciencedirect.com/science/article/abs/pii/S0020025519308771) (Information Sciences)
- [Laconic Private Set Intersection and Applications](https://eprint.iacr.org/2021/728) (TCC 2021); [from Pairings](https://eprint.iacr.org/2022/529) (CCS 2022)
- [Enhanced Privacy ID: A Direct Anonymous Attestation Scheme](https://eprint.iacr.org/2007/194.pdf)
- [PEREA: Practical TTP-Free Revocation of Repeatedly Misbehaving Anonymous Users](https://www.freehaven.net/anonbib/cache/perea-tissec11.pdf) and [BLACR](https://homes.luddy.indiana.edu/kapadia/papers/blacr-ndss-draft.pdf)
- [Threshold Encryption with Silent Setup](https://eprint.iacr.org/2024/263) (CRYPTO 2024) + [implementation](https://github.com/guruvamsi-policharla/silent-threshold-encryption)
- [How to Avoid Obfuscation Using Witness PRFs](https://eprint.iacr.org/2014/301) (Zhandry, TCC 2016); [Diamond iO](https://eprint.iacr.org/2025/236); [Implementable WE from Arithmetic ADPs](https://eprint.iacr.org/2026/175.pdf)
- [BSI TR-03110 (German eID, Restricted Identification)](https://www.bsi.bund.de/EN/Themen/Unternehmen-und-Organisationen/Standards-und-Zertifizierung/Technische-Richtlinien/TR-nach-Thema-sortiert/tr03110/TR-03110_node.html); [known renewal limitation](https://blog.xot.nl/2012/05/08/the-new-german-eid-card-has-security-privacy-and-usability-limitations/index.html)
- [bpK#: Delegatable Pseudonyms and Their Applications to National eID Systems](https://arxiv.org/pdf/2605.30212) (Krenn, Lesaignoux, Ramacher)
- [Austria bPK / SourcePIN Register Authority](https://www.bmdw.gv.at/Ministerium/DasBMDW/Stammzahlenregisterbehoerde/Bereichsspezifische_Personenkennzeichen.html?lang=en)
- [UIDAI: Virtual ID & UID Token](https://uidai.gov.in/en/media-resources/media/aadhaar-in-prints/5617-virtual-id-uid-token-both-to-be-accepted-as-aadhaar.html)
- [EUDI ARF: Zero Knowledge Proof discussion topic](https://eudi.dev/latest/discussion-topics/g-zero-knowledge-proof/)
- [ICAO Digital Travel Credentials](https://www.icao.int/sites/default/files/TRIP/Publications/Digital-Travel-Credentials-DTC.pdf)
- [Personhood credentials](https://arxiv.org/abs/2408.07892) (Adler, Hitzig, Jain, Siddarth et al., 2024)
- [Who Watches the Watchmen? A Review of Subjective Approaches for Sybil-Resistance in Proof of Personhood Protocols](https://www.frontiersin.org/journals/blockchain/articles/10.3389/fbloc.2020.590171/full)
- [Rarimo Freedom Tool docs](https://docs.rarimo.com/freedom-tool/); [ZKPassport: Where are we now?](https://safefoundation.org/blog/safe-research-zk-passport-where-are-we-now) (Safe Foundation)
- [Google Password Checkup design](https://security.googleblog.com/2019/02/protect-your-accounts-from-data-breaches.html) — the canonical production "is my low-entropy secret in a set" system, which also uses an OPRF
