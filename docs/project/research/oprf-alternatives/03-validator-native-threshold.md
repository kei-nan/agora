# Validator-Native Threshold Cryptography (Pressure-Testing the Rejected Idea)

**Status:** research only. Nothing here changes a decision. Written 2026-08-11.

Changelog [#082](../../changelog/082.md) rejected "fanning the secret-key computation out to
blockchain validators/chain participants generally" on this stated ground:

> Structurally incompatible with how blockchains work: every validator re-executes every
> transaction to verify it, so anything computed on-chain is visible to the whole network by
> construction — the opposite of what a secret that must stay with one committee member needs.

This document pressure-tests that. The short version up front: **the stated reasoning is factually
wrong as written and should be corrected in #082 — but the conclusion it supports survives, for
entirely different reasons than the ones recorded.** Validators absolutely can hold a live
distributed secret and answer blind queries with it; several production networks do exactly that
today. Agora still shouldn't, because of what Agora's validator set actually *is*, what the anchor's
lifetime requires, and what the citizen committee is *for* politically.

---

## How BEEFY and off-chain workers actually work

### BEEFY is not the precedent it looks like

I checked this precisely rather than assuming, and the assignment's suspicion was right.

BEEFY validators each hold **their own independent ECDSA (secp256k1) key** under a dedicated
`KeyTypeId`. There is no DKG, no secret sharing, and no threshold key anywhere in BEEFY.
`sp-consensus-beefy` exposes `KnownSignature` ("a commitment signature, accompanied by the id of the
validator it belongs to") and `SignedCommitment` ("a commitment with matching validators'
signatures") — a plain multi-signature, one discrete signature per validator, collected together
([docs.rs/sp-consensus-beefy](https://docs.rs/sp-consensus-beefy/latest/sp_consensus_beefy/)). The
choice of ECDSA is purely an Ethereum-bridging concession (EVM has the precompile), explicitly
described as a temporary scheme
([Polkadot wiki](https://wiki.polkadot.com/learn/learn-consensus/),
the old W3F BEEFY research page, which now 301s to <https://research.parity.io/>). Signing happens in a
**client-side gadget** — nodes gossip votes over libp2p on round-specific topics and assemble
justifications there, never inside the runtime
([polkadot-sdk `client/consensus/beefy`](https://github.com/paritytech/polkadot-sdk/tree/master/substrate/client/consensus/beefy)).

So BEEFY establishes exactly one useful thing: **validators routinely hold key material in the node
keystore, use it off the replicated state machine, and submit only the result on-chain.** That much
is real precedent and it is enough to kill the literal "anything computed on-chain is public" framing
in #082 — nobody was proposing computing the OPRF *in a transaction*.

What BEEFY does **not** establish is the load-bearing part: a secret whose *confidentiality* is the
security property. Every BEEFY key is individually public-verifiable and individually replaceable;
compromise of one is attributable and bounded. An OPRF share is the opposite — silent when leaked,
catastrophic when enough leak, and retroactively catastrophic (see below).

### Off-chain workers: what they can and can't do

OCW facts, checked against the host-function surface and Parity's own writeup
([`sp_io::offchain`](https://docs.rs/sp-io/latest/sp_io/offchain/index.html),
[Parity blog](https://www.parity.io/blog/substrate-off-chain-workers-secure-and-efficient-computing-intensive-tasks)):

- OCWs run **outside consensus**, in their own Wasm environment, after block import. They are never
  part of block execution, so their working is not replicated.
- They can read the node's **local keystore**, but only app-specific subkeys grouped by
  `KeyTypeId` — a real isolation boundary, and genuinely inaccessible to on-chain code.
- They have a **node-local key-value DB** that is *not* part of state and not consensus-critical.
  On-chain code can write into it (`offchain_index`) but cannot read it back. So yes: an OCW can
  hold a DKG-derived share that on-chain code provably cannot see.
- Results return only as signed/unsigned **transactions**.
- Available host functions are HTTP client, local storage, `random_seed`, `timestamp`,
  `sleep_until`, `network_state`, `is_validator`, `submit_transaction`. **There is no peer-to-peer
  messaging API.** No gossip, no direct libp2p send.

Two consequences matter a lot.

**First — an OCW cannot run a normal interactive DKG.** DKG requires authenticated point-to-point
delivery of private shares. OCWs have outbound HTTP and nothing else. You would either need a
non-interactive DKG posting encrypted shares through chain storage (Groth-style NIDKG, which is
essentially what Agora's own mailbox pattern already does), or you'd have to ship a **custom client
gadget** in `node/src/service.rs` with libp2p access — i.e. the BEEFY/`dkg-gadget` shape, not the OCW
shape. The assignment's "BEEFY-like pattern *or* OCWs" framing conflates two quite different
engineering projects; the BEEFY-like one is the viable one and it is substantially more work.

**Second — OCW participation is unenforceable.** OCWs default to running only when the node is an
authority and can be switched off entirely by the operator; Agora's own `node/src/service.rs:186`
gates them behind `config.offchain_worker.enabled`. Substrate has no native accountability hook for
"validator failed to do OCW work" the way it does for equivocation or missed blocks. Any liveness
guarantee has to be rebuilt on-chain from scratch (observe missing `submit_oprf_response`, penalise)
— which is exactly the machinery `pallet-identity`'s mailbox already has for citizen members, so
nothing is saved there.

---

## Does OCW-hosted validator OPRF genuinely escape the original objection?

**The literal objection: yes, it escapes it.** Off-chain execution with keystore-held shares and
only the blinded response submitted on-chain is a real, supported pattern. #082's sentence, read
literally, is over-broad and wrong. It should be rewritten.

**The substantive objection: no — the idea fails for four other reasons, and three of them are worse
than the one on record.**

### 1. Agora's validator set is a ≤32-slot genesis list with no staking and no session pallet

This is the decisive practical fact, and it is checkable in the repo right now:

- `runtime/src/configs/mod.rs:101,110` — `MaxAuthorities = ConstU32<32>` for both Aura and Grandpa.
- `runtime/src/genesis_config_presets.rs` — authorities come straight from genesis (`--dev`: one,
  Alice; `local`: two, Alice and Bob).
- There is **no `pallet-session` and no `pallet-staking`** anywhere in `runtime/Cargo.toml` or
  `runtime/src/lib.rs`. `SessionKeys` is declared but no session pallet rotates it.

So "validator-native" today means: hand the key to voter eligibility to **at most 32 accounts fixed
at genesis and changeable only by sudo or a runtime upgrade**. Against the design in
[#073](../../changelog/073.md) — 5 independent committees, ~35 members each, 12-of-35 threshold,
so that compromising the anchor requires 60+ independent people across 5 disjoint groups — this is
not a close call. Five disjoint sub-groups drawn from 32 authorities gives you groups of six with a
2-of-6 threshold. #082 already rejected "just 5 devices" for reducing 60 people to 5 machines; 5
groups of 6 datacenter nodes is nearer to that rejected shape than to the accepted one.

### 2. If Agora ever *does* add staking, this becomes the already-rejected AGR-staked network

#073 rejected an open AGR-staked OPRF network because it "lets capital buy influence over voter
eligibility itself, a categorically worse thing to let money capture than ordinary block
production," citing World ID/TACEO choosing vetted institutional operators over staking for this
exact role. A PoS validator set *is* a stake-weighted set. Validator-native OPRF on a future
NPoS Agora is that rejected model wearing a different name. The only version that isn't is the
current permissioned genesis set — which is the *institutional* model, also already on record.

### 3. It collapses two deliberately independent trust boundaries into one

Today: compromising the validator set buys you censorship and halting — loud, attributable,
recoverable. Compromising the OPRF committees buys you the ability to brute-force national ID
numbers out of published anchors — silent, unattributable, and **retroactive**: the anchor for a
given scheme version lives ~4 years, so a key recovered in year 3 deanonymises every registration
since year 0. These failure modes currently require attacking two disjoint populations. Merging them
means one adversary who captures the validator set gets both, and — worse — the same keystore that
holds session keys holds the OPRF share, so one host compromise is one compromise of both.

Note also that validators are bonded (elsewhere; not here) against *attributable* faults. Silently
copying a share out to a third party leaves no on-chain evidence and is not slashable by any known
mechanism. "Validators are already trusted" is true for the wrong property.

### 4. The anchor's stability requirement fights validator-set churn

The anchor must be identical for the same person across ~4 years and across passport renewal. A key
held by "the current validator set" must therefore be **reshared on every validator-set change** —
per session or per era on a normal PoS chain — using CHURP-style proactive resharing, forever,
without ever regenerating. This is precisely the part of the design space #073 already found to be
the fragile one. It is also the part the shiniest precedents *don't* cover: Sui and Aptos run a
**fresh DKG every epoch** specifically because their output (randomness) is allowed to be
discontinuous. Agora's isn't.

The only validator-ish set for which per-epoch resharing isn't a problem is one that churns slowly
and is permissioned — which lands you back on the drand/League-of-Entropy institutional-operator
hybrid that #082 explicitly set aside "not permanently." **That, not validator-native OPRF, is the
alternative actually worth a dedicated review round.**

---

## What changes operationally if adopted

Assuming it were adopted anyway, honestly accounted:

- **Removed:** citizen recruitment, device supply and in-person handoff, TPM sourcing, the
  balenaCloud fleet-update question, and — probably — the `committee/` mobile app. Real savings; the
  founding-phase logistics in #082 are the ugliest unsolved part of the current plan.
- **Not removed:** `committee-node/` doesn't disappear, it becomes the validator-side gadget — the
  same poll/evaluate/submit loop, the same Wasm crypto core, the same `submit_oprf_response`. The
  mailbox in `pallet-identity` stays as-is (arguably it stays *because* OCWs have no P2P API).
- **Still required:** a DKG ceremony, now among validators — and now **repeated on every set
  change** rather than once per 4-year scheme rotation. `oprf-committee-dev/src/dkg.rs` (#085) is
  reusable, but it is a file-coordinated ceremony; it would need real authenticated channels, which
  on OCWs means encrypted-shares-through-chain-storage (a new NIDKG implementation) or a custom
  gadget with libp2p.
- **New work not currently on any list:** a client gadget in `node/`, a liveness/accountability
  mechanism for non-responding validators (no native hook exists), and a resharing scheduler tied to
  authority-set changes.
- **Governance impact on #073:** larger than "just changes who's eligible." The 5×35 sizing, the
  hashed-summation-across-5 argument (which buys "safe if even one committee is honest"), the
  `committee_slot = H(DOB) mod 5` disjointness construction, the bonded-deposit +
  `CitizenConduct` accountability path, and the 50,000-anchor sortition handoff **all become
  inapplicable or meaningless** against a set of ≤32 authorities. #073 wouldn't be amended; it would
  be replaced.
- **The unpriced loss:** the citizen committee is not only a cryptographic device. It is the claim
  that citizens collectively hold the key to their own eligibility gate. Moving it to validators
  makes voter eligibility a function of infrastructure operators. For a system pitched at real
  government adoption, that is a political regression, not a neutral hosting change.

---

## Comparison table

| Dimension | Citizen-committee plan (current, #073/#082/#083/#085) | Validator-native OPRF (OCW / gadget) |
|---|---|---|
| Who holds shares | 5×35 sortitioned citizens (5×7 in founding phase) | ≤32 genesis Aura authorities (`MaxAuthorities = ConstU32<32>`) |
| Threshold | 12-of-35 per committee, all 5 committees must respond | ~2-of-6 if split 5 ways; or one flat set with no independence |
| "Safe if one group honest" property | Yes (hashed summation across 5 independent committees) | Lost — one operator population |
| Trust boundary vs. chain integrity | Independent of validators | **Collapsed into** validator trust; same keystore, same hosts |
| Selection legitimacy | Sortition among anchored citizens | Genesis/sudo appointment (today); stake-weighted if PoS later — the model #073 rejected |
| Liveness | Weak (~5-7 day SLA placeholder, consumer devices, n-of-n across 5) | Strong (professional always-on infra) — the single genuine win |
| Accountability for silent share leak | Bonded deposit + `pallet-courts` `CitizenConduct` | None; no native OCW-participation hook, nothing slashable |
| DKG frequency | Once per founding group; fresh DKG per ~4-year scheme rotation | Once **plus resharing on every authority-set change**, forever |
| Ceremony transport | On-chain mailbox + in-person founding ceremony | OCWs have **no P2P API** → needs NIDKG-over-chain or a custom libp2p gadget |
| Code that survives | All of it | Crypto core, mailbox, `committee-node` loop; `committee/` app and all sortition governance dropped |
| Maturity | Implemented and self-tested; no real committee exists | Not started; needs node-level gadget work Agora has never done |

---

## Real-world precedent

Searched properly; the results cut both ways and the honest reading is "the mechanism is real, the
governance shape isn't validators."

- **Internet Computer vetKD / vetKeys — the strongest genuine precedent, and it's close to an
  OPRF.** Subnet nodes hold a DKG-generated master key threshold-shared among them and jointly
  derive keys for requesters; the derived key is encrypted under the requester's transport public
  key so *neither the nodes nor the canister* learn it, and derivation is deterministic — same input,
  same key. Production key lives on a **34-node fiduciary subnet**; the test key on a 13-node subnet
  ([IC docs](https://internetcomputer.org/docs/references/vetkeys-overview),
  [DFINITY](https://medium.com/dfinity/the-internet-computer-s-privacy-era-vetkeys-unlocked-4ded7c206c38)).
  This is "the node set as a blind, deterministic key-derivation oracle," live. But IC node providers
  are **NNS-vetted, named, datacenter operators with rare membership churn** — the institutional
  model, not a permissionless validator set.
- **Sui and Aptos on-chain randomness — validator-set DKG in production, wrong shape for us.** Sui
  validators run a DKG at the start of **every epoch** to bootstrap the randomness beacon, and
  disable randomness for the epoch if participation is too low
  ([Sui docs](https://docs.sui.io/guides/developer/advanced/randomness-onchain)). Aptos does a
  per-epoch **weighted** DKG feeding a weighted VUF (AIP-41, "Roll";
  [eprint 2024/198](https://eprint.iacr.org/2024/198)). Both prove the mechanism at validator scale;
  both rely on the output being *public* and *discontinuous across epochs*, which is the opposite of
  the anchor's requirement.
- **Webb / Tangle `dkg-substrate` — the direct Substrate precedent.** A Substrate chain whose session
  authority set ran threshold-ECDSA (GG20) DKG via a client-side gadget with on-chain pallets
  governing the lifecycle, externally reviewed by SRLabs in 2023
  ([repo](https://github.com/tangle-network/dkg-substrate),
  [review](https://blog.webb.tools/external-srlabs-code-review/)). This is the single best
  refutation of #082's "structurally incompatible" wording. Note Tangle has since restructured
  around restaking/"Blueprints"; the DKG-bridge line is not a thriving, load-bearing production
  system to copy.
- **Shutter Network keypers — a chain that wanted threshold crypto and deliberately did *not* use
  its validators.** Keypers run DKG and threshold decryption for Shutterized Gnosis Chain; the design
  is explicitly consensus-agnostic, and the keyper committee is a **separate, governance-selected,
  permissioned set distinct from the validator set**
  ([Shutter](https://blog.shutter.network/shutterized-gnosis-chain-is-now-live/),
  [Gnosis](https://www.gnosis.io/blog/shutterized-gnosis-chain-is-live)). A live production
  counter-vote against exactly this idea.
- **TACEO:OPRF — the closest thing to Agora's actual workload.** Live public beta, **eight
  independent node operators**, threshold ~5-of-8, verifiable threshold OPRF producing unlinkable
  nullifiers ([TACEO](https://core.taceo.io/articles/oprf-beta/)). Operators are independent
  organisations, not any chain's validators. Same call World ID made, already on record in #073.
- **Academic:** threshold OPRF constructions are well-established (e.g.
  [eprint 2024/1032](https://eprint.iacr.org/2024/1032)); latency costs of threshold cryptosystems in
  blockchains are surveyed in [arXiv 2407.12172](https://arxiv.org/pdf/2407.12172). I found **no**
  system, production or academic, running a threshold OPRF *whose key is held by a chain's own
  permissionless validator set*.

---

## Open questions

1. Should #082's rejection text be corrected in place? It is wrong as written and a future reader
   will re-derive the correction, as this document did. Recommend rewording to "rejected because it
   collapses the identity-secrecy trust boundary into the chain-integrity one, and because Agora's
   authority set is far too small and too permissioned for this role" — keeping the conclusion,
   fixing the reason.
2. Does the **institutional-operator hybrid** deserve the dedicated review #082 promised it? vetKD
   (34 vetted nodes) and TACEO (8 independent operators) both landed there independently. That is
   now three data points, and it is the model that actually fixes the liveness problem the citizen
   plan has.
3. If Agora ever adds `pallet-session`/`pallet-staking`, does the ≤32 `MaxAuthorities` cap get
   revisited? Nothing in this document depends on it staying, but every argument above scales with
   it.
4. Is per-scheme-version key **stability** genuinely non-negotiable? Everything about the
   validator-native shape gets easier if the anchor could be re-derived — but re-derivation breaks
   duplicate detection across renewal, which is the whole point.
5. Could validators serve as a *sixth* committee (adding availability without removing any citizen
   committee)? Under hashed summation, one additional honest committee strictly increases security
   and costs nothing in trust — but it strictly *decreases* liveness in an n-of-n combination. Worth
   noting only to be explicit that it is not free.

---

## Verdict

**No — stick with the current plan.** The specific reason recorded in changelog #082 is wrong and
should be fixed: BEEFY shows validators routinely hold node-local keys and produce artifacts off the
replicated state machine, off-chain workers can genuinely hold key material that on-chain code
cannot read, and Internet Computer's vetKD, Sui's and Aptos's per-epoch randomness DKGs, and Webb's
`dkg-substrate` are all real, live counterexamples to "structurally incompatible with how blockchains
work." But correcting the reason does not rescue the idea. Agora's validator set is at most 32
accounts fixed at genesis, with no session or staking pallet to rotate them — so validator-native
OPRF today means handing the key that protects every citizen's national ID to a sudo-appointed
handful, and tomorrow, if staking arrives, it means the stake-weighted eligibility oracle #073
already rejected on anti-plutocracy grounds. On top of that it merges two deliberately independent
failure domains (a validator-set compromise would become an identity-database compromise, silently
and retroactively for the anchor's ~4-year life), it demands perpetual proactive resharing that
neither the Sui/Aptos precedent nor the current DKG tooling covers, and it discards the
`H(DOB) mod 5` disjointness, hashed-summation, and sortition-legitimacy properties that are the
point of the design. The one real win — professional always-on operators instead of a 5-7 day
consumer-device SLA — is achievable *without* touching the validator set, via the
institutional/professional-operator hybrid that #082 set aside rather than rejected. That is the
alternative worth a real review round; validator-native OPRF is not.
