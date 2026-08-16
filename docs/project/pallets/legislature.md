# pallet-legislature

### pallet-legislature (crate: pallet-legislature) — runtime index 13

Storage:
- `Members`: `BoundedVec<AccountId, 500>`
- `Motions`: `motion_id` → `Motion { call_hash, proposer, ayes, nays, end_block, executed }`
- `MotionVotes`: `(motion_id, AccountId)` → `bool`
- `NextMotionId`
- `PendingLegislatureApproval`: `(call_hash, proposer, ayes, total_members)` — planted by `close_motion`
  when a motion clears the floor; `ayes`/`total_members` are the tally frozen at close time (see below)

Calls:
- `add_member(account)` / `remove_member(account)` — root
- `propose_motion(call_hash)` — member only; proposer's aye recorded immediately
- `vote_motion(motion_id, approve: bool)` — member only; **active ministers blocked** (incompatibility rule via `MinisterChecker`)
- `close_motion(motion_id)` — anyone, after `end_block`; passes (plants the approval token) if
  `ayes * 100 >= PassageThreshold(51) * total_members`. This is only the *floor* — see below.

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

