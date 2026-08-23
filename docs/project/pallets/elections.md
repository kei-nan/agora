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

### Delegate identity — now cryptographically separated via ZK personas

`register_as_delegate` no longer trusts `ensure_signed`'s caller identity directly. It requires
a real ZK proof of the `delegate-persona` circuit (`circuits/oprf-identity-anchor/
delegate-persona`, commit 2e07f68) riding inside a fresh outer ZKPassport proof — a genuinely
separate, on-demand proof event with its own 5-committee OPRF round-trip, not folded into
registration. The proof derives a stable per-citizen `delegate_persona_id` and binds a chosen
`persona_account` into `param_commitment` (anti-front-running). The runtime performs the real bb
5.0.0 pairing check (`T::ZkVerifier`), checks the 5 committee keys against governance-approved
keys for the given scheme version (`T::CommitteeKeyChecker` — without this, a prover could
fabricate their own "committee" and self-mint unlimited personas), then recomputes and checks the
`param_commitment` (`T::DelegatePersonaVerifier`, backed by
`runtime/src/anchor_verifier.rs::check_delegate_persona`). `DelegatePersonaUsed` is an
insert-once nullifier map on `delegate_persona_id`, so the same citizen cannot mint a second
persona. `persona_account` (required to equal the caller) is still an ordinary `T::AccountId` —
the same type `Delegates`/`SeatLegislature`/`DisclosureChecker` already key on — so
pallet-legislature's seating and pallet-anticorruption's disclosure gate needed no changes.

Backing is unlinkable too: `back_delegate`/`remove_backing` require a real `backing-nullifier`
circuit proof (`circuits/oprf-identity-anchor/backing-nullifier`) proving Merkle-path membership
of the citizen's `backing_commitment` in pallet-identity's published tree, at a slot index
range-checked *in-circuit* against the live `MaxBackingsPerCitizen` value (a checked public
input the runtime cross-checks against live governance state — not a plaintext per-citizen
counter this pallet maintains). `UsedBackingNullifier` (nullifier → `(submitter, delegate_persona_id)`)
replaces the old plaintext `BackingOf`/`CitizenBackingCount` maps entirely — no on-chain record
of *which citizen* backs *which delegate* survives, only that some nullifier currently backs a
given `delegate_persona_id`. `remove_backing` requires the *same* submitting account to reverse
its own action (see `UsedBackingNullifier`'s doc comment for the replay-griefing hole this
closes, since the `backing-nullifier` circuit deliberately binds no `AccountId`); this is not a
privacy regression since that account was already public in `back_delegate`'s own call data. One
consequence: `back_delegate` no longer has a `CannotBackSelf` check — the tx signer is not
cryptographically tied to the nullifier's underlying secret, so that check could not actually
prevent a determined delegate from spending one of their own `MaxBackingsPerCitizen` slots on
themselves via a cooperating relayer; the exposure is bounded to at most one of the
`BackingThreshold` backers a delegate needs, the same as any single legitimate citizen's backing
power.

**A residual gap that survives this (2026-08-23):** an unlinkable proof only anonymizes the
*derivation*, not the *transaction* that reveals it. That transaction is still a signed
extrinsic with a signer `AccountId`, a fee-payment source, and a block timestamp. If that account
is funded by a direct on-chain transfer from the citizen's real, identity-linked account, or
submits in close temporal proximity to other citizen-linked activity, ordinary chain analysis —
not cryptanalysis — can still deanonymize it. This is the same class of gap already documented
for MACI's `commit_vote` in `pallets/pallet-voting/src/lib.rs`'s doc comment and in `CLAUDE.md`'s
Voting System section; neither this repo nor `court-oracle/`/`committee-node/` (both authenticate
as a known council/committee member, not a pattern for pseudonymous relaying) nor
`pallets/pallet-treasury-ledger` (no faucet-like account-funding mechanism exists) currently has
any relayer, mixnet, or unsigned/ZK-gated submission path that would close it.

### Backing threshold

A delegate becomes `Active` only once they have `BackingCount` ≥ `BackingThreshold` citizen
backers — this makes backing a meaningful signal rather than noise. Each citizen may back at
most `MaxBackingsPerCitizen` delegates simultaneously (constitutional parameter, default 5),
enforced by the `backing-nullifier` circuit's own in-circuit range check on `slot_index`, not by
any plaintext per-citizen counter on-chain.

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
- `BackingCount`: `AccountId` (delegate) → `u32` — incrementally maintained by
  `back_delegate`/`remove_backing`
- `DelegatePersonaUsed`: `[u8; 32]` (`delegate_persona_id`) → `()` — insert-once nullifier set,
  prevents a citizen from minting a second persona
- `DelegatePersonaIdOf`: `AccountId` (`persona_account`) → `[u8; 32]` (`delegate_persona_id`) —
  set once at registration, used by `back_delegate`/`remove_backing` to check a proof's claimed
  target actually matches the named delegate
- `UsedBackingNullifier`: `[u8; 32]` (`backing_nullifier`) → `(AccountId submitter,
  [u8; 32] delegate_persona_id)` — replaces the old plaintext `BackingOf`/`CitizenBackingCount`
  entirely; no on-chain record of which citizen backs which delegate, only that a nullifier
  currently backs a given persona. `submitter` is required again by `remove_backing` (see
  above)

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

- `register_as_delegate(persona_account, delegate_persona_id, zk_proof, public_inputs,
  scheme_version, oprf_pk_hashes, display_name, profile_ipfs_hash)` — active citizen;
  `persona_account` must equal the caller; verifies a real `delegate-persona` ZK proof (see
  above); fails if already registered or `delegate_persona_id` was already used
- `back_delegate(delegate, zk_proof, public_inputs)` — active citizen; verifies a real
  `backing-nullifier` proof against `delegate`'s persona id and the live
  `MaxBackingsPerCitizen`/backing-commitment-root state; rejects backing an `OnBreak` delegate
  or a reused nullifier; auto-activates the delegate on crossing `BackingThreshold`. No
  `CannotBackSelf` check (see above)
- `remove_backing(delegate, zk_proof, public_inputs)` — must be called by the same account that
  originally called `back_delegate` for this nullifier; frees the slot for reuse;
  auto-deactivates (back to `Pending`) if the delegate falls below `BackingThreshold`
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
- `AccountIdToBytes<AccountId>` — implemented by the runtime as a real byte-identity conversion
  (`AccountId32` is genuinely 32 raw bytes); binds `persona_account` into a delegate-persona
  proof's `param_commitment`.
- `ZkProofVerifier` — verifies the outer ZKPassport proof `register_as_delegate` submits; the
  same real bb 5.0.0 pairing check (`crate::verifier::ZkPassportUltraHonkVerifier`)
  `pallet_identity_zk::Config::ZkVerifier` uses.
- `DelegatePersonaVerifier` — recomputes and checks the `delegate-persona` circuit's
  `param_commitment`; backed by `runtime/src/anchor_verifier.rs::check_delegate_persona`.
- `CommitteeKeyChecker` — checks a set of committee-key hashes against pallet-identity-zk's
  governance-approved keys (`are_committee_keys_approved`), the same Sybil-resistance guarantee
  `register_citizen` enforces on itself.
- `BackingProofVerifier` — real standalone UltraHonk pairing check for a `backing-nullifier`
  proof; backed by `runtime/src/backing_nullifier_verifier.rs`.
- `BackingRootChecker` — checks a backing-commitment tree root against pallet-identity's own
  root history (`is_valid_backing_commitment_root`).
- `DisclosureChecker<AccountId>` — implemented directly on `pallet_anticorruption::Pallet<T>`
  (wrapping its `has_current_disclosure`); re-checked per candidate at election time in
  `run_election`, same as `CitizenChecker` above. Wired in the runtime as
  `type DisclosureChecker = PalletAntiCorruption`.
