# Funding Identity Infrastructure Through Agora's Own Treasury

*Addendum, 2026-08-11, written directly against the actual pallet code (not via subagent) in response
to a follow-up question: can the OPRF committee (or whichever operator model gets adopted) be paid for
using Agora's own on-chain budget/treasury system, instead of needing external funding?*

## Short answer

**Yes — this is a good fit, it's already partially decided, and Agora already has the exact
primitives needed.** Changelog #073 recorded "compensation, if any, stays treasury-funded" as the
answer to a *different* rejected idea (registrants paying committee members directly — see below),
but never specified how. This note specifies it, grounded in the real pallets, and it also strengthens
the case for the institutional-operator model recommended in
[00-index.md](00-index.md): named institutions have real operating costs (hosting, staff, compliance)
that a treasury stipend is a natural fit for, in a way it isn't for sortitioned private citizens.

## The primitives that already exist

Checked directly against the code, not assumed:

- **`pallet-treasury-ledger`** (`pallets/pallet-treasury-ledger/src/lib.rs`) is a real, working
  department-budget ledger: `allocate_budget(department_id, amount)` is gated by
  `EnsureLegislatureMotion` (a passed legislature vote), writes to `DepartmentBudgets`;
  `record_expenditure` is callable only by a department's registered `DepartmentSpenders` account and
  is checked against the cap (`InsufficientBudget` if it would exceed the allocation); every
  expenditure fires `T::AuditHook::on_expenditure` automatically. Departments can be frozen
  independently by a court ruling (`CourtFrozenDepartments`) or by `pallet-audit`
  (`AuditFrozenDepartments`) — two separate freeze axes, either one blocks spending.
- **Important architectural fact, worth being explicit about**: this pallet holds **no `Currency`
  type and moves no actual tokens** — `Config` has no balance-transfer trait, only a generic
  `Balance` number. It is a transparency/cap-enforcement ledger over spending that happens
  elsewhwere, which matches CLAUDE.md's own description ("real-time public budget ledger... adapt
  Polkadot OpenGov... **Stablecoin-based to start (fiat bridge Phase 2)**"). In other words: this
  pallet is designed to make a *real government's real tax revenue* transparently trackable and
  capped on-chain — it is not, today, a novel on-chain tax-collection mechanism in its own right.
  That distinction matters for where the money actually comes from (see below).
- **`pallet-legislature`** motions are the only way to call `allocate_budget` — a proposal, `t` days
  of voting (`MotionDurationBlocks`), a passage threshold of total members, then anyone can close it.
  `EnsureLegislatureMotion` is a strict single-use origin (a passed motion for call A can never be
  replayed to authorize call B).
- **`pallet-executive`** portfolios/ministers (`define_portfolio`, `appoint_minister`) are the natural
  candidate for a department's registered `DepartmentSpender` — a minister with a portfolio (e.g. an
  "Identity & Elections Infrastructure" portfolio) executes spend within the legislature-approved cap.
- **`pallet-audit`** already wires into the `AuditHook` for exactly this kind of oversight, and can
  independently freeze a misbehaving department.

Nothing here needs to be invented. It needs a `department_id` allocated to this purpose and a
legislature motion.

## The concrete design

1. Legislature defines an **"Identity & Elections Infrastructure"** department (or reuses an existing
   Elections Commission budget line — `pallet-elections` already exists for candidate eligibility and
   result certification and is a natural political home for this).
2. A periodic `allocate_budget` motion sets that department's cap for the period (e.g. annually, same
   cadence as `reset_department_spent`).
3. Each committee operator — under the institutional-operator model from
   [02-existing-threshold-networks.md](02-existing-threshold-networks.md)/[03-validator-native-threshold.md](03-validator-native-threshold.md),
   this is 5×8–15 named institutions, not 175 sortitioned citizens — receives a **flat, scheduled
   stipend** for availability/operation, recorded via `record_expenditure` by the portfolio's
   minister, capped by the department budget, and automatically audit-hooked.
4. Every payment is public, on-chain, and freezable by courts or audit the moment something looks
   wrong — which is a strictly *better* accountability story than "the project pays a vendor
   off-chain," and it's free: this is exactly what the treasury ledger already does for every other
   department.

## Why this must be a flat stipend, not per-query or registrant-funded

This isn't a new constraint — it's already on record, and worth restating so the design doesn't
accidentally violate it:

- Changelog #073 explicitly considered and rejected **registrants paying committee members directly**
  — flagged as a poll tax on voting eligibility (the same *Harper v. Virginia*/24th Amendment concern
  raised again in [05-issuance-time-and-social-backstop.md](05-issuance-time-and-social-backstop.md)'s
  bonding analysis) and, separately, because paying per-approval creates a direct incentive to approve
  regardless of correctness.
- The same logic rules out **paying per query response** even from the treasury: an operator paid per
  `submit_oprf_response` is incentivized to respond (correctly or not) as often as possible, not to be
  honest. The stipend has to be for *being available and operating the node*, decoupled from any
  individual query's outcome — the same "no judgment call, purely mechanical work" framing changelog
  #082 already used to justify why founding-phase members don't need vetting for the crypto itself.

This is a real constraint the budget-allocation design has to honor, not just an implementation
detail: `record_expenditure` should be called on a fixed schedule (e.g. monthly per operator), never
triggered by `submit_oprf_response` volume.

## The bootstrap question, and why it isn't a new problem

The obvious follow-up: `allocate_budget` requires a passed legislature motion, which requires
`pallet-legislature::Members` to exist — but Members can only be voted onto the roll by other Members
(`add_member`/`remove_member`), and there's no legislature entry in the current genesis presets
(`runtime/src/genesis_config_presets.rs` sets only `balances`, `aura`, `grandpa`, and `sudo` — no
`legislature: LegislatureConfig`). So today, the first legislature members would have to be seated by
the `sudo` root key.

This looks like a circularity — *funding the founding committee needs a legislature, but a real
legitimate legislature needs registered voters, who need... the committee.* It isn't a new one,
though. It's the exact shape of the bootstrap problem [[project_oprf_committee_governance]] already
named and already solved for the committees themselves: *"a founding phase is necessary, not
optional — sortition needs an existing pool of anchored citizens to draw from, but producing an
anchor needs a working committee... same shape as a blockchain's first validators not being
selectable by a token vote before tokens exist."* Agora's whole genesis design already runs on a
sudo-bootstrapped founding phase (sudo seeds initial legislature members / portfolios, exactly as it
seeds Aura/GRANDPA authorities); funding the founding-phase OPRF committees is one more thing that
bootstrap phase needs to do, not a new category of problem. Once the first cohort of citizens is
anchored and a real elected legislature exists, ongoing committee-budget renewal moves to normal
motion-and-vote governance — which is itself a nice legitimacy story: **citizens' own elected
legislature decides what to pay the institutions that guard their voting eligibility**, publicly and
auditably, rather than that being fixed by project maintainers or an off-chain grant.

## Where the money actually comes from

This is a different question from *allocation*, and the codebase currently assumes an answer rather
than building one:

1. **Real government tax revenue, bridged in as stablecoin** — this is what CLAUDE.md's Treasury
   section actually describes ("Stablecoin-based to start (fiat bridge Phase 2)"), and it's the
   natural fit for the project's stated goal of *real government adoption*: a government that adopts
   Agora already collects taxes through its existing systems; Agora's job is to make the *spending* of
   that revenue transparent, capped, and auditable on-chain, not to invent tax collection. Under this
   model "collect funds via the tax system" is close to literally true — it's the same tax system a
   government already runs, now with an on-chain, publicly-auditable spend ledger sitting on top of
   it, of which identity infrastructure is one department among others (courts, elections, the
   existing departments).
2. **A native on-chain mechanism** — transaction fees or AGR issuance routed to the treasury. The
   runtime already has `pallet-transaction-payment` and `pallet-balances` wired
   (`runtime/src/configs/mod.rs`), so the pieces exist, but **fee routing to the treasury is not
   currently built** — this would need its own design pass (where do fees go today; is inflation via
   `pallet-balances` acceptable given the project's existing objection to stake-weighted mechanisms
   captured in [02-existing-threshold-networks.md](02-existing-threshold-networks.md)'s "AGR-staked
   network" rejection). This path matters mainly for a deployment with **no cooperating government** —
   a testnet, a pilot, or a diaspora/non-state community — where there's no real tax revenue to bridge
   in yet.

For the project's actual target scenario (a real government adopting Agora), path 1 is the fit that
requires no new construction: the treasury ledger already assumes it, and identity-infrastructure
compensation just becomes one more department budget line, funded the same way every other department
already is meant to be.

## Interaction with the institutional-operator recommendation

This closes a gap the earlier research round left open. [02](02-existing-threshold-networks.md) and
[03](03-validator-native-threshold.md) independently recommended moving from 175 sortitioned citizens
to 5×8–15 named institutions, partly *because* professional operators can meet the availability bar
citizens on personal phones/Pis can't — but neither addressed how those institutions get compensated
for real infrastructure costs (hosting, HSMs, staff, compliance). A citizen-hosted model could get
away with "devices are supplied, no ongoing payment" (changelog #082's framing); an institution being
asked to run production infrastructure for years cannot. Treasury-funded flat stipends, allocated
through the exact department-budget machinery above, is the natural way to pay for that — and it's
the same drand precedent doc 02 already cites (League of Entropy operators mostly donate infra, but
several are compensated orgs, and the governance-funded pattern is well established there).

## Open questions

1. **Does `pallet-elections` (the Elections Commission) already have, or need, its own budget
   department separate from a general "Identity Infrastructure" one?** Politically these might want
   to be distinct line items even if operationally similar.
2. **Fee-routing to the treasury is unbuilt.** If the project ever needs the native-token funding path
   (no cooperating government yet), this is real, unstarted design work — not to be confused with
   the department-allocation machinery above, which already exists.
3. **Stipend sizing and schedule** — genuinely unresourced. Changelog #073's own SLA numbers (48h→5-7
   day windows) were already flagged as unmeasured placeholders; stipend amounts have the same
   problem and probably want real-world benchmarking against comparable roles (drand operator
   costs, election-monitoring-body budgets).
4. **Should the freeze mechanism (`CourtFrozenDepartments`/`AuditFrozenDepartments`) be a required
   design review item before this ships?** It already exists and already applies automatically to any
   department, including this one — worth confirming that's actually sufficient recourse if an
   institutional operator is later found to be dishonest, or whether removal-from-committee (a
   `pallet-identity`-side action) needs to be coupled to the funding freeze so a dishonest operator
   can't keep collecting stipends after being removed.

## Verdict

Genuinely a good idea, and cheaper to build than it looks — the budget-allocation and audit machinery
already exists and needs no new pallet work, just a new department id and a legislature motion. It
also resolves a real gap in the institutional-operator recommendation from the earlier research round
(who pays named institutions to operate infrastructure for years). The one piece that isn't free is
the funding *source*: for a real-government deployment, this rides on the fiat-bridge/stablecoin
treasury model the project already intends, and needs no new construction; for a deployment with no
cooperating government, it would need a genuinely new fee-routing or issuance mechanism that doesn't
exist yet and deserves its own design pass before being relied on.
