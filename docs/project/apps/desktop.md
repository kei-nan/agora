# Desktop app (Tauri 2)

Location: `desktop/`

- **Tauri backend** (`src-tauri/src/`): JSON-RPC client talks directly to the running node at `127.0.0.1:9944`
- **Chain commands** (`commands/chain.rs`): `chain_status`, `fetch_proposals`, `fetch_laws`, `fetch_treasury`, `fetch_rulings` — all read from real chain storage via `state_getKeysPaged` + `state_queryStorageAt`
- **AI agent** (`commands/agent.rs`): `agent_ask(question, item_context, history)` — calls Claude API (`claude-sonnet-4-6`); reads `CLAUDE_API_KEY` env var; degrades gracefully offline
- **Auth** (`commands/auth.rs`): `auth_generate_challenge` generates UUID + embeds callback port in deep-link; `auth_poll_session` returns signed session; `auth_start_callback_server` spawns a local HTTP listener; `auth_verify_nullifier` scans `Identity.NullifierRegistry` keys to confirm the mobile-reported nullifier exists on-chain before accepting the session
- **Chain reads** (`commands/chain.rs`): `fetch_proposals` decodes `Voting.Referenda` (42-byte SCALE: petition_id(4) + topic_hash(32) + end_block(4) + state(1) + tier(1)) + `Voting.ReferendumTally` (8 bytes: yes(4) + no(4)); `fetch_rulings` cross-references `Courts.Cases` for IPFS ruling hashes; `fetch_department_budgets` decodes `DepartmentBudgets`/`DepartmentSpent` as u128 LE (16 bytes, 12 decimal places = 1 AGR); `format_agr(planck)` helper; `fetch_treasury` decodes `ExpenditureLog` as 52-byte SCALE (dept_id(4) + amount(16) + hash(32))

Frontend pages: Proposals (with tier chip for constitutional referenda), Laws, Courts, Treasury (department budget table + IPFS audit fetching), auth QR page, Claude AI sidebar panel.

Browser dev mode uses `desktop/src/lib/mocks.ts` stub data; the real Tauri commands fire when running as a native app.

TODOs:
- Mobile side of QR auth: phone NFC + ZK proof; `mobile/src/screens/AuthScreen.tsx` scaffolded (parses deep-link, signs challenge, POSTs to callback)

IPFS content fetching: **implemented** on all detail pages.
- `fetch_ipfs_content(hash_hex: String) -> String` Tauri command in `commands/chain.rs`
- Converts on-chain 32-byte SHA-256 digest → CIDv0 via bs58 multihash (0x1220 prefix)
- Fetches from `https://ipfs.io/ipfs/{cid}` with 30-second timeout
- `LawsPage`, `ProposalsPage`, `CourtsPage`, `TreasuryPage` all fetch content on selection and pass full text to AI agent

