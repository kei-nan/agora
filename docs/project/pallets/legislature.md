# pallet-legislature

### pallet-legislature (crate: pallet-legislature) — runtime index 13

Storage:
- `Members`: `BoundedVec<AccountId, 500>`
- `Motions`: `motion_id` → `Motion { call_hash, proposer, ayes, nays, end_block, executed }`
- `MotionVotes`: `(motion_id, AccountId)` → `bool`
- `NextMotionId`
- `PendingLegislatureApproval`: `(call_hash, proposer, ayes, total_members, planted_at)` — planted
  by `close_motion` when a motion clears the floor; `ayes`/`total_members` are the tally frozen at
  close time (see below); `planted_at` is the block the token was written, used by
  `clear_stale_approval` to tell a genuinely stale token from a fresh one
- `Bootstrapped`: `bool` (added `748625f`) — see "Bootstrap lock" below

Calls:
- `add_member(account)` / `remove_member(account)` — root, but **only while bootstrap is open**
  (`Bootstrapped == false`; see below) — once `close_bootstrap` has been called, both fail
  unconditionally, even for root. `add_member` also refuses a sitting Accountability Council
  member (`Error::AccountabilityCouncilMember`, checked via a new `Config::
  AccountabilityCouncilChecker`, fixed `86d003b`) — the reverse of the join-time check
  `pallet_accountability_council::add_member` already performs, and mirrors the same
  legislature/Council overlap bar `pallet_elections`'s post-bootstrap automatic seating already
  enforces (see `docs/project/pallets/elections.md`). Before this fix, root could seat a sitting
  Accountability Council member into the legislature during bootstrap with no equivalent check.
- `propose_motion(call_hash)` — member only; proposer's aye recorded immediately
- `vote_motion(motion_id, approve: bool)` — member only; **active ministers blocked** (incompatibility rule via `MinisterChecker`)
- `close_motion(motion_id)` — anyone, after `end_block`; passes (plants the approval token) if
  `ayes * 100 >= PassageThreshold(51) * total_members`. This is only the *floor* — see below.
- `clear_stale_approval()` — any current member; discards an unconsumed `PendingLegislatureApproval`
  token once `PendingApprovalExpiryBlocks` have passed since `planted_at`, recovering the
  legislature from a proposer (or every consuming member) who never executes the queued action —
  `close_motion` otherwise refuses to overwrite a pending token, which would block every future
  motion from passing.
- `close_bootstrap()` — root, one-time; requires at least one member already seated
  (`Error::NoMembersToBootstrap` otherwise). Sets `Bootstrapped = true`; there is no call that
  ever flips it back.
- `emergency_reseed_legislature(members: BoundedVec<AccountId, MaxMembers>)` — root, and **only**
  when `Members` is currently empty (`ensure!(Members::<T>::get().is_empty(),
  Error::<T>::LegislatureNotEmpty)`); also rejects an empty `members` list
  (`Error::NoMembersProvided`). See "Emergency reseed" below.

### Emergency reseed (added 2026-09-13, HIGH-severity fix)

`pallet_elections::run_election` used to call `SeatLegislature::replace_members` with an empty
winners list whenever an election cycle seated zero eligible delegates (all disqualified via
disclosure lapse, Accountability-Council overlap, or genuinely zero backing) — see
`docs/project/pallets/elections.md`'s "Empty-eligible-pool skip" section. Combined with the
Bootstrap lock above (`add_member`/`remove_member` refuse unconditionally, even for root, once
`Bootstrapped == true`), that would have **permanently bricked the legislature**: an empty
`Members` means no account can call `propose_motion` (member-only), and post-bootstrap there is
no other call in this pallet that can add one back. `pallet_elections` is now fixed to never call
`replace_members` with an empty list (skips the reseat and emits
`SeatingSkippedNoEligibleCandidates` instead, leaving the existing legislature untouched), but
this pallet also gained `emergency_reseed_legislature` as a backstop for the exact bricked state,
in case it's ever reached anyway (e.g. it already happened once on a live chain before the
elections-side fix landed, or some future bug reproduces it).

`emergency_reseed_legislature` is deliberately **not** a general override of a functioning
legislature — `ensure!(Members::<T>::get().is_empty(), ...)` makes it structurally unusable
against any non-empty `Members`, so it can never be used to unilaterally pack or purge a working
legislature the way the pre-`748625f` unlocked `add_member`/`remove_member` could.

**This is a placeholder, not the intended long-term mechanism** — gating it on bare `Root` means
it currently resolves to whoever holds the single genesis sudo key, not a collective, exactly
like the two other placeholder Root-gated origins this codebase is already honest about:
`pallet_constitution::Config::RevocationOrigin` and `pallet_elections::Config::
ConstitutionalOrigin` (both still bare `EnsureRoot<AccountId>` in `runtime/src/configs/mod.rs` —
see `CLAUDE.md`'s "Two placeholder origins" note). A real deployment should wire this to a proper
collective/governance origin (e.g. a supermajority of the Accountability Council, or another body
structurally independent of the legislature it would be reseeding) before mainnet.

### Bootstrap lock (fixed `748625f`, 2026-09-04)

Before this fix, `add_member`/`remove_member` were permanently `ensure_root`-gated with no
bootstrap lock, unlike `pallet-accountability-council`'s equivalent pattern — since a real
`SudoConfig` key exists in genesis, a compromised sudo key could unilaterally pack or purge the
legislature at any time. `close_bootstrap` closes that: while `Bootstrapped == false`, root may
freely add/remove members to seed the initial legislature; once closed, `add_member`/
`remove_member` refuse unconditionally (`Error::BootstrapClosed`) for everyone, including root,
and bootstrap can never reopen.

Unlike `pallet-emergency-council`'s identical lock (see `docs/project/pallets/
emergency-council.md`), closing bootstrap here does **not** freeze legislature membership for
good: `pallet_elections`' automatic top-N delegate seating (`SeatLegislature::replace_members`,
run every election cycle — see `docs/project/pallets/elections.md`) is the pallet's real ongoing
membership mechanism and is completely untouched by `Bootstrapped` — it keeps replacing the
membership on its normal schedule whether or not bootstrap has been closed.

### Tier-aware thresholds (fixed 2026-08-16: the legislature-motion path used to enforce a
single flat threshold — see `pallet-constitution`'s doc for the full supermajority-bypass
gap this closed)

`EnsureLegislatureMotion<Runtime>` implements two `EnsureOriginWithArg` overloads on the same
underlying `PendingLegislatureApproval` token:
- `Arg = [u8; 32]` (hash only) — usable as soon as the token exists, i.e. the motion cleared the
  51% floor at close time. Used by every legislature-gated pallet whose calls don't need more
  than that floor: treasury-ledger, executive, elections, identity, voting.
- `Arg = ([u8; 32], u8)` (hash + required percentage) — the token must *also* clear the given
  percentage, checked against the `ayes`/`total_members` tally frozen when the motion closed
  (not re-derived from live `Members` state, which could have changed since). Used exclusively
  by pallet-constitution to enforce Structural (67%) / Foundational (75%) supermajorities on
  law enactment, amendment, and repeal — see `docs/project/pallets/constitution.md` for exactly
  which calls use which percentage and why the percentage can't be spoofed by a proposer.

`close_motion`'s 51% is therefore a floor, not the last word: a motion that clears it and gets
`MotionPassed` emitted may still fail authorization at execution time if the call being
authorized demands more than 51% and the real tally doesn't meet that higher bar.

`EnsureLegislatureMotion<Runtime>` origin — gates law enactment, budget epochs, minister appointments.
`MinisterChecker` trait — implemented by `Cabinet` (pallet-executive); blocks PM + portfolio ministers from voting.

