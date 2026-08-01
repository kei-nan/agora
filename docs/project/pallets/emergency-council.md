# pallet-emergency-council

### pallet-emergency-council (crate: pallet-emergency-council) — runtime index 15

Time-limited emergency powers with a hard-coded constitutional sunset clause.

Storage:
- `Council`: `BoundedVec<AccountId, 15>`
- `ActiveEmergency`: `Option<EmergencyInfo { declared_at, expires_at, reason_hash, votes_to_declare, votes_to_end }>`
- `DeclareVotes`: `AccountId` → `bool` (reset each new emergency)
- `EndVotes`: `AccountId` → `bool`

Config:
- `MaxEmergencyBlocks = 432_000` (30 days at 6s/block — constitutional ceiling)
- `SupermajorityNumerator / Denominator = 2/3`

Calls:
- `add_council_member(account)` / `remove_council_member(account)` — root
- `vote_declare_emergency(reason_hash, duration_blocks)` — council member; duration clamped to max; activates on 2/3 supermajority
- `vote_end_emergency()` — council member; lifts on 2/3 supermajority

`on_initialize` hook: auto-expires `ActiveEmergency` when `expires_at <= current_block`, emits `EmergencyExpired`.

