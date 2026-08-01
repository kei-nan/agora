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

Config: `LegislatureOrigin = EnsureLegislatureMotion<Runtime>`, `MaxPortfolios = 20`

Calls (all `LegislatureOrigin` except `resign`):
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

