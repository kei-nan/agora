# pallet-executive

### pallet-executive (crate: pallet-executive) — runtime index 18, alias `Cabinet`

Parliamentary executive. The legislature appoints ministers to named portfolios via passed motions.
Active ministers are **blocked from casting legislature votes** (incompatibility rule — separation of
executive and legislative power). One account holds at most one portfolio at a time.

Storage:
- `PrimeMinister`: `Option<AccountId>`
- `Portfolios`: `portfolio_id` → `Portfolio { name_hash: [u8;32] }` (name_hash = IPFS CID of terms of reference)
- `PortfolioMinister`: `portfolio_id` → `AccountId`
- `MinisterPortfolio`: `AccountId` → `portfolio_id` (enables O(1) is_active_minister)
- `NextPortfolioId`: `u32`

Config: `LegislatureOrigin = EnsureLegislatureMotion<Runtime>`, `MaxPortfolios = 20`,
`MaxEmergencyBlocks = 30 * DAYS` (= 216,000 blocks at this chain's real 12s/block time —
previously a hardcoded `432_000`, which was 30 days at a stale 6s/block assumption and
actually enforced a 60-day cap; fixed 2026-08-09), `RatificationWindowBlocks = 3 * DAYS`,
`SupermajorityNumerator/Denominator = 2/3`

Portfolio/PM calls (all `LegislatureOrigin` except `resign`):
- `define_portfolio(name_hash)` — creates a new named cabinet portfolio
- `appoint_prime_minister(who)` — installs PM; auto-dismisses old PM if any
- `dismiss_prime_minister()` — removes current PM
- `appoint_minister(portfolio_id, who)` — installs minister; auto-vacates old holder + old portfolio of incoming
- `dismiss_minister(portfolio_id)` — removes minister from a portfolio
- `resign()` — any active minister may self-vacate

`EnsureExecutiveMinister<T>` origin — passes if signer is PM or holds a portfolio; returns `AccountId`.

Implements `MinisterChecker<AccountId>` from pallet-legislature: `is_active_minister(who)` returns true
if the account holds a portfolio OR is the PM. This is the cross-pallet trait that enforces the
incompatibility rule without circular dependencies.

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

