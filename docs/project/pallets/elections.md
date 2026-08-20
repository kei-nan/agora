# pallet-elections

### pallet-elections (crate: pallet-elections) — runtime index 14

Liquid Democracy Delegates + Legislature Elections: manages the public delegate registry
citizens back to express vote delegation, and runs periodic elections that seat the top-N
backed delegates (by backing count) into pallet-legislature — entirely automatic, no
committee or human certification step anywhere in the flow.

This pallet used to also run a separate "Elections Commission" subsystem (commissioners,
named "office" elections, candidate registration/certification, result submission/certification
— formerly `call_index` 0–6). It was removed: it certified an election's outcome on nothing
but a commissioner's say-so, with no on-chain tally behind `submit_results` at all, and
nothing in this system's actual design turned out to need a citizen-facing "elect one person
to a named office" mechanism. Legislature seats now fill automatically via the backing
mechanism below, and the Prime Minister is chosen by the legislature itself via
pallet-executive's ranked-choice investiture (see `docs/project/pallets/executive.md`). Nothing
replaces the removed subsystem — it's deleted, not rebuilt; `call_index` 0–6 are deliberately
left unused rather than reassigned. See `docs/project/changelog/` for the full removal
rationale.

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
pallet-legislature via the `SeatLegislature` trait (`replace_members`). Citizenship is
re-checked at election time (not just trusted from whenever `Active` status was last granted)
so a delegate suspended since (e.g. an Overturned `CitizenConduct` court ruling) can never be
seated on stale status. Defaults: 100 seats, 2-year cycle (`DefaultElectionCycleBlocks`), max 5
backings per citizen.

Asset-disclosure currency is checked the same way, alongside citizenship: `T::DisclosureChecker`
(implemented by pallet-anticorruption — see `docs/project/pallets/anticorruption.md`) must
return `true` for a candidate to be seated. A delegate who is Active, an active citizen, and
ranked within the top `LegislatureSeats` by backing, but whose asset disclosure has lapsed or
was never filed, is skipped — excluded from the candidate pool entirely, so the next-highest-
backed eligible delegate fills the freed seat instead, and `Event::SeatingSkippedNoDisclosure {
account }` is emitted per skipped account. This is deliberately a skip, not a hard error on the
whole `run_election` call: `on_initialize` runs unconditionally every block past the cycle
boundary, so failing outright would freeze legislature seating for everyone until manual
intervention, over one official's lapsed paperwork.

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

Each block, a bounded sweep (`MaxDelegateSweepPerBlock` entries per block, resuming from
`DelegateSweepCursor` where the last block left off — never an unbounded full-map scan)
examines delegates and, per `DelegateInfo`:

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
- `DelegateSweepCursor`: `Option<AccountId>` — resume point for the bounded per-block term sweep
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

- `MaxDelegates` — hard cap on registered delegates, bounding `on_initialize` iteration
- `MaxDelegateSweepPerBlock` — cap on how many `Delegates` entries the per-block term sweep
  examines, regardless of how many delegates are registered
- `DefaultLegislatureSeats` (100), `DefaultElectionCycleBlocks` (2 years),
  `DefaultMaxBackingsPerCitizen` (5) — constitutional genesis defaults
- `DefaultBackingThreshold` (10), `DefaultBackingThresholdFloor` (5),
  `DefaultBackingThresholdCeiling` (500) — genesis defaults for the governance-tunable backing
  threshold and its bounds
- `DefaultTermLengthBlocks` (1 year), `DefaultMaxConsecutiveTerms` (2),
  `DefaultMandatoryBreakBlocks` (1 year), `DefaultWarningWindowPct` (10%) — genesis defaults for
  the term-limit system above

### Cross-pallet traits

- `CitizenChecker<AccountId>` — implemented by pallet-identity-zk in the runtime; gates
  `register_as_delegate`/`back_delegate`, and is re-checked at election time in `run_election`.
- `SeatLegislature<AccountId>` — implemented by pallet-legislature; `replace_members(winners)`
  is called at the end of each election cycle to install the winning delegates as the full
  legislature membership.
- `DisclosureChecker<AccountId>` — implemented directly on `pallet_anticorruption::Pallet<T>`
  (wrapping its `has_current_disclosure`); re-checked per candidate at election time in
  `run_election`, same as `CitizenChecker` above. Wired in the runtime as
  `type DisclosureChecker = PalletAntiCorruption`.
