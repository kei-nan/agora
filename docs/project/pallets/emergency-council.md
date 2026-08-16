# pallet-emergency-council

### pallet-emergency-council (crate: pallet-emergency-council) — runtime index 15

Time-limited emergency powers with a hard-coded constitutional sunset clause.

Storage:
- `Council`: `BoundedVec<AccountId, 15>`
- `ActiveEmergency`: `Option<EmergencyInfo { declared_at, expires_at, reason_hash, votes_to_declare, votes_to_end }>`
- `DeclareVotes`: `AccountId` → `bool` (reset each new emergency)
- `EndVotes`: `AccountId` → `bool`

Config:
- `MaxEmergencyBlocks = 216_000` (30 days at this chain's actual 12s/block time — constitutional ceiling)
- `SupermajorityNumerator / Denominator = 2/3`

Calls:
- `add_council_member(account)` / `remove_council_member(account)` — root
- `vote_declare_emergency(reason_hash, duration_blocks)` — council member; duration clamped to max; activates on 2/3 supermajority
- `vote_end_emergency()` — council member; lifts on 2/3 supermajority

`on_initialize` hook: auto-expires `ActiveEmergency` when `expires_at <= current_block`, emits `EmergencyExpired`.

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

