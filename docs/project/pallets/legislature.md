# pallet-legislature

### pallet-legislature (crate: pallet-legislature) — runtime index 13

Storage:
- `Members`: `BoundedVec<AccountId, 500>`
- `Motions`: `motion_id` → `Motion { call_hash, proposer, ayes, nays, end_block, executed }`
- `MotionVotes`: `(motion_id, AccountId)` → `bool`
- `NextMotionId`

Calls:
- `add_member(account)` / `remove_member(account)` — root
- `propose_motion(call_hash)` — member only; proposer's aye recorded immediately
- `vote_motion(motion_id, approve: bool)` — member only; **active ministers blocked** (incompatibility rule via `MinisterChecker`)
- `close_motion(motion_id)` — anyone, after `end_block`; passes if ayes * 100 >= 50 * total_members

`EnsureLegislatureMotion<Runtime>` origin — gates law enactment, budget epochs, minister appointments.
`MinisterChecker` trait — implemented by `Cabinet` (pallet-executive); blocks PM + portfolio ministers from voting.

