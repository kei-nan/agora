# pallet-emergency-council

### pallet-emergency-council (crate: pallet-emergency-council) — runtime index 15

Time-limited emergency powers with a hard-coded constitutional sunset clause.

Storage:
- `Council`: `BoundedVec<AccountId, 15>`
- `ActiveEmergency`: `Option<EmergencyInfo { declared_at, expires_at, reason_hash, votes_to_declare, votes_to_end }>`
- `DeclareVotes`: `AccountId` → `bool` (reset each new emergency)
- `EndVotes`: `AccountId` → `bool`
- `CooldownUntil`: `BlockNumber` (added 2026-08-20) — block before which `vote_declare_emergency`
  is rejected; set to `now + EmergencyCooldownBlocks` whenever an emergency ends, by any path
- `Bootstrapped`: `bool` (added `748625f`) — see "Bootstrap lock" below

Config:
- `MaxEmergencyBlocks = 216_000` (30 days at this chain's actual 12s/block time — constitutional ceiling)
- `EmergencyCooldownBlocks = 50_400` (7 days at 12s/block — added 2026-08-20)
- `SupermajorityNumerator / Denominator = 2/3`

Calls:
- `add_council_member(account)` / `remove_council_member(account)` — root, but **only while
  bootstrap is open** (`Bootstrapped == false`; see below) — once `close_bootstrap` has been
  called, both fail unconditionally, even for root
- `vote_declare_emergency(reason_hash, duration_blocks)` — council member; duration clamped to
  max; rejected with `EmergencyCooldownActive` if called before `CooldownUntil`; activates on 2/3
  supermajority
- `vote_end_emergency()` — council member; lifts on 2/3 supermajority, sets `CooldownUntil`
- `close_bootstrap()` — root, one-time; requires at least one council member already seated
  (`Error::NoMembersToBootstrap` otherwise). Sets `Bootstrapped = true`; there is no call that
  ever flips it back.

### Bootstrap lock (fixed `748625f`, 2026-09-04) — permanent freeze, no alternate path

Before this fix, `add_council_member`/`remove_council_member` were permanently
`ensure_root`-gated with no bootstrap lock, unlike `pallet-accountability-council`'s equivalent
pattern — since a real `SudoConfig` key exists in genesis, a compromised sudo key could
unilaterally pack or purge the Emergency Council at any time. `close_bootstrap` closes that:
while `Bootstrapped == false`, root may freely add/remove members to seed the initial council;
once closed, `add_council_member`/`remove_council_member` refuse unconditionally
(`Error::BootstrapClosed`) for everyone, including root, and bootstrap can never reopen.

**This pallet has no alternate post-bootstrap membership path — unlike `pallet-legislature`'s
identical-looking lock** (see `docs/project/pallets/legislature.md`), which keeps taking new
membership from `pallet_elections`' automatic seating (`SeatLegislature::replace_members`)
regardless of whether its own bootstrap is closed. The Emergency Council has no self-governance
mechanism of its own — its 2/3 supermajority votes govern *declaring/ending an emergency*, not
council *composition* — so once `close_bootstrap` is called here, the Council's membership is
**frozen for good**: there is no call, origin, or governance path anywhere in this pallet that
can ever add, remove, or replace a member again. This is a deliberate tradeoff (closing the
sudo-capture hole is worth a permanently fixed roster), not an oversight, but it means the
initial bootstrap membership must be gotten right before `close_bootstrap` is ever called in
production.

`on_initialize` hook: auto-expires `ActiveEmergency` when `expires_at <= current_block`, emits
`EmergencyExpired`, sets `CooldownUntil`.

**Post-emergency cooldown (fixed 2026-08-20)**: previously, neither this pallet nor
`pallet-executive`'s independent emergency mechanism enforced any minimum gap between
emergencies — only `AlreadyActiveEmergency` blocked a *second concurrent* one. The same
supermajority that declares an emergency could therefore redeclare a fresh one the block after
the previous one ended (sunset expiry or early `vote_end_emergency`), chaining into de-facto
indefinite emergency powers despite each individual window being honestly capped by
`MaxEmergencyBlocks` — defeating the sunset clause's stated purpose. `CooldownUntil` now blocks
`vote_declare_emergency` for `EmergencyCooldownBlocks` after any emergency ends. See
`docs/project/changelog/092.md`.

### `EnsureActiveEmergency<T>` — cross-pallet origin (added this session)

Previously, nothing outside this pallet actually read `ActiveEmergency`/`Council` — declaring an
emergency flipped a storage flag and emitted events, but unlocked no concrete power elsewhere in
the runtime, and `pallet-identity`'s `EmergencyRotationOrigin` (the one place in the codebase that
referenced this pallet by name as its intended gate) was bound to a bare `EnsureRoot` placeholder.

`EnsureActiveEmergency<T>` is a new `EnsureOrigin<T::RuntimeOrigin>` impl in this pallet
(`src/lib.rs`, structured like `pallet_legislature::EnsureLegislatureMotion`) whose `try_origin`
succeeds only when the underlying origin is `Root` *and* `ActiveEmergency::<T>::get()` is
`Some(..)` at call time. It is deliberately layered on top of `Root` rather than replacing it —
an origin that accepted any signed/unsigned caller during an active emergency would be a larger
attack surface, not a smaller one. There is no writer of `ActiveEmergency` other than the real
`vote_declare_emergency` supermajority path (cleared by `vote_end_emergency` or the hard-coded
`on_initialize` sunset), so this origin cannot be made to succeed without a genuine
council-declared emergency.

The runtime now binds `pallet_identity_zk::Config::EmergencyRotationOrigin` (both the `dev-mode`
and non-`dev-mode` impls in `runtime/src/configs/mod.rs`) to
`pallet_emergency_council::EnsureActiveEmergency<Runtime>`, gating `emergency_rotate_oprf_scheme`
— see `docs/project/pallets/identity.md`. This is still the *only* concrete emergency power wired
up; no other pallet reads `ActiveEmergency`/`Council` yet. Tests: `ensure_active_emergency_*` in
this pallet's `src/tests.rs`, plus `emergency_rotate_oprf_scheme_*` in
`pallets/pallet-identity/src/tests.rs` (which wires the real `EnsureActiveEmergency` into its
mock runtime rather than stubbing it, to test the cross-pallet integration for real).

