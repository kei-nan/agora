# pallet-elections

### pallet-elections (crate: pallet-elections) — runtime index 14

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

