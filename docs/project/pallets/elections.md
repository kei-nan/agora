# pallet-elections

### pallet-elections (crate: pallet-elections) — runtime index 14

Manages two separate concerns: (A) the Elections Commission's office elections, and (B) the
liquid-democracy delegate registry that periodically seats winners into pallet-legislature.

## A) Elections Commission

Elections Commission pallet: manages candidate registration, commissioner certification, and result certification.

Storage:
- `Commissioners`: `BoundedVec<AccountId, 20>`
- `Elections`: `election_id` → `ElectionInfo { office, start_block, end_block, status, winner, results_ipfs_hash }`
- `Candidates`: `(election_id, AccountId)` → `CandidateInfo { profile_ipfs_hash, status, deposit }`
- `NextElectionId`

Calls:
- `add_commissioner(account)` / `remove_commissioner(account)` — root
- `create_election(office, start_block, end_block)` — root or commissioner
- `register_candidate(election_id, profile_ipfs_hash)` — active citizen; reserves 1 AGR deposit
- `certify_candidate(election_id, candidate)` — commissioner only
- `submit_results(election_id, winner, results_ipfs_hash)` — commissioner only
- `certify_results(election_id)` — commissioner only; unreserves all candidate deposits

## B) Liquid Democracy Delegates + Legislature Elections

Manages the public delegate registry citizens back to express vote delegation, and runs
periodic elections that seat the top-N backed delegates (by backing count) into
pallet-legislature.

### Delegate identity

Separate from citizen identity: a delegate's nullifier is `Poseidon2(national_id ||
country_code || "delegate")`, cryptographically unlinked from the citizen's own nullifier. The
delegate voluntarily publishes their real name and profile; the citizens backing them remain
anonymous.

### Backing threshold

A delegate becomes `Active` only once they have `BackingCount` ≥ `BackingThreshold` citizen
backers — this makes backing a meaningful signal rather than noise. Each citizen may back at
most `MaxBackingsPerCitizen` delegates simultaneously (constitutional parameter, default 5).

### Legislature elections

Every `ElectionCycleBlocks` blocks, `on_initialize` ranks all `Active` delegates by backing
count (stable sort, ties broken by storage order) and seats the top `LegislatureSeats` into
pallet-legislature via the `SeatLegislature` trait (`replace_members`). Defaults: 100 seats,
2-year cycle (`DefaultElectionCycleBlocks`), max 5 backings per citizen.

### Term limits / anti-entrenchment

A real, deployed term-limit system prevents a backed delegate from holding a legislature seat
indefinitely:

- `TermLengthBlocks` — length of a single term.
- `MaxConsecutiveTerms` — how many back-to-back terms a delegate may serve before a mandatory
  break (default 2, `DefaultMaxConsecutiveTerms`).
- `MandatoryBreakBlocks` — how long the forced break lasts (default 1 year,
  `DefaultMandatoryBreakBlocks`).
- `WarningWindowPct` — what fraction (1–50%) of the final term triggers a
  `DelegateTermWarning` event before it ends (default 10%).

Every block, `on_initialize` walks all delegates and, per `DelegateInfo`:

- **Active, warning not yet emitted, elapsed ≥ warning offset** (`term_length * (100 -
  warning_pct) / 100`, computed divide-first to avoid overflow on very long terms): emits
  `DelegateTermWarning { delegate, blocks_remaining }` and sets `warning_emitted`.
- **Active, elapsed ≥ `term_length`**: the term counts as served (an `Active` delegate serves
  the term uninterrupted, so elapsed time always counts as full); `consecutive_terms` is
  incremented and `DelegateTermExpired` is emitted. If `consecutive_terms >=
  MaxConsecutiveTerms`, the delegate moves to `OnBreak` with `break_until_block = now +
  MandatoryBreakBlocks`. Otherwise a fresh term starts immediately (`term_start_block = now`,
  `warning_emitted` reset).
- **OnBreak, `now >= break_until_block`**: the delegate returns to `Pending`
  (`consecutive_terms` reset to 0, `term_start_block`/`break_until_block` cleared,
  `DelegateBreakEnded` emitted); if their existing `BackingCount` still meets
  `BackingThreshold`, they are immediately re-activated (new term starts right away).

A delegate `OnBreak` cannot receive new backing (`back_delegate` rejects with
`DelegateOnBreak`) but keeps any backing they already had, so a fresh `BackingThreshold` check
on break-end can re-activate them without citizens having to re-back.

### Storage

Delegate registry:
- `Delegates`: `AccountId` → `DelegateInfo { display_name, profile_ipfs_hash, status,
  consecutive_terms, term_start_block, break_until_block, warning_emitted }`
- `BackingCount`: `AccountId` (delegate) → `u32`
- `BackingOf`: `(AccountId backer, AccountId delegate)` → `()` — prevents double-backing
- `CitizenBackingCount`: `AccountId` (citizen) → `u32` — enforced against `MaxBackingsPerCitizen`

Governance-controlled parameters (stored, changeable by governance, seeded from `Default*`
config at genesis):
- `BackingThreshold` (ordinary supermajority via `GovernanceOrigin`) — bounded by
  `BackingThresholdFloor`/`BackingThresholdCeiling` (constitutional, via `ConstitutionalOrigin`)
- `TermLengthBlocks`, `MaxConsecutiveTerms`, `MandatoryBreakBlocks`, `WarningWindowPct`
  (constitutional, via `ConstitutionalOrigin`)
- `LegislatureSeats`, `ElectionCycleBlocks`, `MaxBackingsPerCitizen` (constitutional, via
  `ConstitutionalOrigin`)
- `LastElectionBlock` — block of the last legislature election run (0 = none yet; first
  election fires at block `ElectionCycleBlocks`)

`DelegateStatus`: `Pending` (below threshold) / `Active` (≥ threshold, within term limits,
receives backing) / `OnBreak` (served `MaxConsecutiveTerms`, waiting out `break_until_block`).

### Calls

- `register_as_delegate(display_name, profile_ipfs_hash)` — active citizen; fails if already
  registered
- `back_delegate(delegate)` — active citizen; enforces `MaxBackingsPerCitizen`, rejects
  self-backing and backing an `OnBreak` delegate; auto-activates the delegate on crossing
  `BackingThreshold`
- `remove_backing(delegate)` — frees one backing slot; auto-deactivates (back to `Pending`) if
  the delegate falls below `BackingThreshold`
- `set_backing_threshold(threshold)` — `GovernanceOrigin`; must stay within
  `BackingThresholdFloor`/`BackingThresholdCeiling`
- `set_backing_bounds(floor, ceiling)` — `ConstitutionalOrigin`; also clamps the current
  threshold into the new bounds if needed
- `set_term_params(term_length, max_consecutive, mandatory_break, warning_pct)` —
  `ConstitutionalOrigin`; `warning_pct` must be 1–50
- `set_election_params(seats, cycle_blocks, max_backings_per_citizen)` —
  `ConstitutionalOrigin`; each field optional, `None` leaves it unchanged

### Config constants

- `CandidateDeposit` — AGR reserved by `register_candidate` (Elections Commission side)
- `MaxCommissioners`, `MaxCandidatesPerElection` — Elections Commission bounds
- `MaxDelegates` — hard cap on registered delegates, bounding `on_initialize` iteration
- `DefaultLegislatureSeats` (100), `DefaultElectionCycleBlocks` (2 years),
  `DefaultMaxBackingsPerCitizen` (5) — constitutional genesis defaults
- `DefaultBackingThreshold` (10), `DefaultBackingThresholdFloor` (5),
  `DefaultBackingThresholdCeiling` (500) — genesis defaults for the governance-tunable backing
  threshold and its bounds
- `DefaultTermLengthBlocks` (1 year), `DefaultMaxConsecutiveTerms` (2),
  `DefaultMandatoryBreakBlocks` (1 year), `DefaultWarningWindowPct` (10%) — genesis defaults for
  the term-limit system above
