# pallet-executive

### pallet-executive (crate: pallet-executive) — runtime index 18, alias `Cabinet`

Parliamentary executive. A Prime Minister is chosen by ranked-choice vote among sitting
legislature members (PM investiture, below); the PM then nominates ministers to named
portfolios, which the legislature confirms via passed motion. Active ministers (PM + portfolio
holders) are **blocked from casting legislature votes** (incompatibility rule — separation of
executive and legislative power). One account holds at most one portfolio at a time.

Storage:
- `PrimeMinister`: `Option<AccountId>`
- `Portfolios`: `portfolio_id` → `Portfolio { name_hash: [u8;32] }` (name_hash = IPFS CID of terms of reference)
- `PortfolioMinister`: `portfolio_id` → `AccountId`
- `MinisterPortfolio`: `AccountId` → `portfolio_id` (enables O(1) is_active_minister)
- `NextPortfolioId`: `u32`
- `PendingMinisterNomination`: `portfolio_id` → `AccountId` — PM's staged pick, awaiting legislature confirmation
- `PmTenureHistory`: `BoundedVec<PmTenureRecord<AccountId, BlockNumber>, MaxPmTenureHistory>` —
  closed-out past PM tenures (start/end block per holder), used together with the in-progress
  tenure to compute how much of the trailing `PmOccupancyWindowBlocks` window any account has
  occupied
- `CurrentPmTenureStart`: `Option<BlockNumber>` — start block of the sitting PM's in-progress tenure
- `InvestitureRound`: `Option<InvestitureRoundInfo { nomination_end, voting_end }>` — the currently open PM investiture round, if any
- `PmNominees`: `BoundedVec<AccountId, MaxPmCandidates>` — nomination order also serves as the deterministic tie-break order
- `PmBallots`: `AccountId` (voter) → `BoundedVec<AccountId, MaxPmCandidates>` — ranked ballot, most-preferred first
- `NextVacancySweepBlock`: `BlockNumber` — when the conviction-vacancy sweep next runs
- `EndVotes`: `AccountId` → `bool` — cabinet members' votes to end an active emergency early (`vote_end_emergency`)

Config: `LegislatureOrigin = EnsureLegislatureMotion<Runtime>`, `MaxPortfolios = 20`,
`MaxEmergencyBlocks = 30 * DAYS` (= 216,000 blocks at this chain's real 12s/block time —
previously a hardcoded `432_000`, which was 30 days at a stale 6s/block assumption and
actually enforced a 60-day cap; fixed 2026-08-09), `RatificationWindowBlocks = 3 * DAYS`,
`SupermajorityNumerator/Denominator = 2/3`, `PmNominationWindowBlocks = 7 * DAYS`,
`PmVotingWindowBlocks = 7 * DAYS`, `MaxPmCandidates = 20`,
`VacancySweepIntervalBlocks = 1 * DAYS`, `PmOccupancyWindowBlocks = 365 * DAYS`,
`MaxPmOccupancyBlocks = 270 * DAYS`, `MaxPmTenureHistory = 128` (see "Term limits" below —
these replaced an earlier `PmConsecutiveTerms`/`MaxConsecutivePmTerms = 2` pair that no longer
exist in the code).

### PM investiture (ranked-choice among legislature members)

Replaces a formerly-existing direct `appoint_prime_minister`/`dismiss_prime_minister` pair
(single up/down motion) — removed because it let the legislature bypass a fair investiture
process whenever convenient. A vacancy is now always filled by an instant-runoff vote:

- `open_pm_investiture()` — anyone may call, but only while the PM seat is actually vacant and
  no round is already open. Starting the clock on an objective vacancy isn't a judgment call
  that needs a legislature vote. Opens a `PmNominationWindowBlocks`-long nomination window,
  immediately followed by a `PmVotingWindowBlocks`-long voting window.
- `nominate_pm(candidate)` — caller and candidate must both currently hold a legislature seat;
  rejects a candidate who would exceed `MaxPmOccupancyBlocks` of PM tenure within the trailing
  `PmOccupancyWindowBlocks` window (see "Term limits" below); bounded by `MaxPmCandidates`.
- `cast_pm_ballot(ranked_candidates)` — legislature member only, during the voting window;
  every ranked candidate must be a nominee this round, no duplicates; a later call replaces an
  earlier ballot from the same voter.
- `finalize_pm_investiture()` — anyone, once the voting window has closed. Tallies by instant-
  runoff (`run_instant_runoff`): each round counts every ballot's most-preferred
  still-active candidate; a strict majority wins outright, otherwise the candidate with the
  fewest such votes is eliminated (ties broken toward eliminating whoever was nominated later)
  and the process repeats. Installs the winner as PM (`PmInvestitureFinalized`), or, if there
  were no nominees or no ballots at all, leaves the seat vacant (`PmInvestitureFailedNoWinner`)
  — a fresh round must then be opened.

### Removing/replacing and resigning the PM

- `remove_and_replace_prime_minister(successor)` — `LegislatureOrigin`. Constructive vote of no
  confidence: removes the sitting PM and installs `successor` atomically in the same motion.
  Deliberately not a plain no-confidence vote (remove with no successor) — that pattern lets a
  majority topple a government for purely obstructive reasons with no obligation to agree on a
  replacement. Requiring the successor to be named in the same vote means the office is never
  left vacant by this path, and removal is only possible when a replacement already has
  support. `successor` must be a current legislature member and must not already be at the
  `MaxPmOccupancyBlocks` occupancy cap (same check as `nominate_pm`).
- `resign_as_pm()` — sitting PM only. Vacates the office immediately (no successor named) and
  opens the seat for a fresh `open_pm_investiture()` round.

### Minister nomination and confirmation

- `nominate_minister(portfolio_id, candidate)` — Prime Minister only. Stages a nominee for a
  portfolio; legislature must separately confirm before it takes effect.
- `appoint_minister(portfolio_id, who)` — `LegislatureOrigin`, and only for the exact account
  the PM staged via `nominate_minister` for that portfolio — legislature confirms the PM's
  pick, it doesn't originate one. Auto-dismisses whoever currently holds the portfolio, and
  auto-vacates any other portfolio the incoming account already holds.
- `dismiss_minister(portfolio_id)` — `LegislatureOrigin`; removes the minister from a portfolio.
- `resign()` — any active minister may self-vacate their own portfolio.
- `define_portfolio(name_hash)` — `LegislatureOrigin`; creates a new named cabinet portfolio
  (`name_hash` = IPFS CID of its terms of reference).

`EnsureExecutiveMinister<T>` origin — passes if signer is PM or holds a portfolio; returns `AccountId`.

Implements `MinisterChecker<AccountId>` from pallet-legislature: `is_active_minister(who)` returns true
if the account holds a portfolio OR is the PM. This is the cross-pallet trait that enforces the
incompatibility rule without circular dependencies.

### Term limits

A rolling-window occupancy cap, not a consecutive-terms counter: no account may hold the PM
office for more than `MaxPmOccupancyBlocks` out of the trailing `PmOccupancyWindowBlocks` blocks
(default: no more than 270 of the trailing 365 days, ~74%). This replaced an earlier
`PmConsecutiveTerms`/`MaxConsecutivePmTerms` pair that no longer exist in the code — a plain
consecutive-term counter resets to 0 the instant anyone else holds the office even briefly, so a
one-block puppet reinstall (install a cooperating placeholder for a single block, then reinstall
the real incumbent) would have reset the counter and let occupancy continue indefinitely; a
rolling window can't be reset that way, since it sums actual time occupied rather than counting
discrete "terms."

`CurrentPmTenureStart` tracks the sitting PM's in-progress tenure; every path that ends a tenure
(resignation, removal, conviction-triggered vacancy) calls `close_current_pm_tenure`, which
closes it into `PmTenureHistory` (bounded by `MaxPmTenureHistory`, with the oldest record dropped
as a last-resort safety valve if that bound is hit) and clears `CurrentPmTenureStart`.
`pm_occupancy_in_window` sums, over `PmTenureHistory` plus the in-progress tenure if the account
being checked is the sitting PM, however much of each tenure falls inside
`[now - PmOccupancyWindowBlocks, now]`. Both `nominate_pm` and
`remove_and_replace_prime_minister` reject a candidate/successor whose occupancy would exceed
`MaxPmOccupancyBlocks`.

### Conviction-triggered vacancy sweep

`on_initialize` runs `run_vacancy_sweep` every `VacancySweepIntervalBlocks` (default daily),
re-checking the sitting PM and every sitting minister against
`CitizenChecker::is_suspended_by_jury_reviewed_conviction` and auto-vacating anyone suspended
since they took office (`OfficeVacatedForConviction`). The set checked is always tiny (one PM,
a handful of ministers), so polling costs nothing meaningful — this is a periodic self-contained
poll rather than a cross-pallet hook wired through pallet-courts/pallet-identity.

This deliberately checks `is_suspended_by_jury_reviewed_conviction`, not just
`is_active_citizen`: a bare, unappealed Level-0 AI ruling from pallet-courts is **not** enough
on its own to remove a sitting PM/minister — only a suspension a jury actually reviewed (i.e.
the case reached `CaseStatus::JurySeated` and was decided by `cast_jury_vote`'s majority, per
pallet-courts) is. The consequence (losing the office) is large enough to warrant the same
evidentiary bar the court system already applies elsewhere, and requiring it here also closes
off filing a bogus, never-appealed case as a way to force someone out — the same reasoning
behind requiring a named successor for `remove_and_replace_prime_minister`. A suspended-but-
not-jury-reviewed office holder isn't exempt from consequences: legislature can still remove
them via a no-confidence vote at any time, and they're blocked from being re-nominated (see
`nominate_pm`) if the suspension outlives their term. `CitizenChecker` is implemented by
pallet-identity-zk in the runtime; `LegislatureMembership` by pallet-legislature.

### Emergency powers (a second, separate mechanism from `pallet-emergency-council`)

The Cabinet has its own time-limited emergency-declaration mechanism, distinct from and
independent of `pallet-emergency-council`. **The legislature does not gate the initial
declaration** — only `ratify_emergency` (after the fact) uses `LegislatureOrigin`; declaring
and ending an emergency are both cabinet-only actions:

- `vote_declare_emergency(reason_hash, duration_blocks)` — **`is_cabinet_member` (any minister
  or the PM), not `LegislatureOrigin`**. First voter's `reason_hash`/`duration_blocks` (clamped
  to `MaxEmergencyBlocks`) lock in the proposal; once a 2/3 cabinet supermajority has voted,
  `ActiveEmergency` is set and the legislature's `RatificationWindowBlocks` clock starts.
- `ratify_emergency()` — **`LegislatureOrigin`**. The legislature ratifies (or, by inaction,
  lets it lapse) an already-active emergency; it does not pre-approve the declaration.
- `vote_end_emergency()` — **`is_cabinet_member`**. Cabinet supermajority vote clears
  `ActiveEmergency` early (independent of whether it was ratified).
- `retract_emergency_vote()` — **`is_cabinet_member`**. Withdraws a cabinet member's own
  pending declare-vote before the emergency activates.

Design intent: the executive declares under time pressure without waiting on the legislature;
the legislature's role is to ratify after the fact or let the declaration lapse, and it can
also vote (via ordinary cabinet-supermajority mechanics) to end an emergency early. Do not
confuse this with `pallet-emergency-council`'s time-locked powers, which are a separate pallet
with its own sunset clause.

**Quorum floor (commit `98c5040`)**: both `vote_declare_emergency` and `vote_end_emergency` now
require `cabinet_size() >= 2` before evaluating the supermajority ratio at all (rejected with
`CabinetTooSmallForEmergencyVote` otherwise). Before this, a freshly-invested PM with no
confirmed ministers yet had `cabinet_size() == 1`, and a single vote trivially satisfies "2/3 of
1" — letting a lone PM unilaterally declare (or end) an emergency with no real supermajority
involved.

**`EndVotes` purged on every office-vacating path (same commit)**: `EndVotes` (a cabinet member's
pending vote to end an active emergency early) is now cleared alongside the existing
`DeclareVotes` purge everywhere an office empties out — `resign_as_pm`, `dismiss_minister`,
minister `resign()`, `appoint_minister`'s outgoing-minister branch, `install_pm`'s displacement
branch, and both branches of `run_vacancy_sweep` (PM and minister). Previously only
`DeclareVotes` was purged on these paths, so a departed member's stale "end emergency" vote kept
counting toward `vote_end_emergency`'s tally even as the live-computed cabinet-size denominator
shrank around it — inflating the effective vote share left behind by anyone who exited office.

**Post-emergency cooldown (fixed 2026-08-20)**: neither this pallet nor
`pallet-emergency-council` previously enforced any minimum gap between emergencies — only
`AlreadyActiveEmergency` blocked a *second concurrent* one. The same cabinet supermajority that
declares an emergency could therefore redeclare a fresh one the block after the previous one
ended (lapse, sunset expiry, or early `vote_end_emergency`), chaining into de-facto indefinite
emergency powers despite `MaxEmergencyBlocks` capping each individual window. A new
`CooldownUntil` storage item, set to `now + EmergencyCooldownBlocks` on every path that ends an
emergency, now blocks `vote_declare_emergency` (`EmergencyCooldownActive`) until it elapses.
`EmergencyCooldownBlocks = 7 * DAYS` (50,400 blocks) in the runtime, mirroring
`pallet_emergency_council::Config::EmergencyCooldownBlocks`. See
`docs/project/changelog/092.md`.
