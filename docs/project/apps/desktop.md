# Desktop app (Tauri 2)

Location: `desktop/`

- **Chain connectivity (changelog #087): a real embedded smoldot light client, in the JS
  frontend.** `desktop/src/chain/client.ts` drives `smoldot` via `@polkadot/api`'s `ScProvider`
  (a hand-written adapter bridges smoldot's async-iterator response API to the callback shape
  `ScProvider` expects; `@substrate/connect` itself isn't a dependency). `desktop/src/lib/
  invoke.ts` routes nine command names — `chain_status`, `fetch_proposals`, `fetch_laws`,
  `fetch_treasury`, `fetch_department_budgets`, `fetch_rulings`, `fetch_legislature_data`,
  `fetch_elections_data`, `fetch_anticorruption_data` — to `desktop/src/chain/queries.ts`
  (light-client-backed) instead of Tauri IPC when running inside Tauri; browser-dev mode
  (`npm run dev`) is untouched and still serves `mocks.ts`. Requires the node to be started
  with an explicit `--listen-addr /ip4/0.0.0.0/tcp/30333/ws` — a plain `--dev --tmp` node
  doesn't open a WebSocket-capable libp2p listener, and a webview's smoldot can only dial `ws`,
  not raw TCP. Bootnode discovery is dynamic (one plain `system_localListenAddresses` RPC call
  patches `desktop/public/chainspecs/dev-chainspec-raw.json`'s blank `bootNodes` at connect
  time) so the checked-in chain spec doesn't need to hardcode a peer ID. Proven to sync and
  answer real queries against a live local dev chain in a real production browser build, driven
  headlessly — see changelog #087 for the full verification writeup.
- **Tauri backend** (`src-tauri/src/`): still a JSON-RPC client talking directly to the running
  node at `127.0.0.1:9944` — kept, not deleted, because it remains the real implementation
  behind `auth_verify_nullifier`, `chain_submit_extrinsic`, and the QR-auth callback server's
  internal account lookup, none of which moved to the light client. The nine commands above are
  no longer called by the frontend through this path, but the Rust functions and their tests
  still exist as a reference/fallback (see `commands/chain.rs`'s top-of-module comment).
- **Chain commands** (`commands/chain.rs`, registered in `src-tauri/src/lib.rs`'s `tauri::generate_handler![...]`): `chain_status`, `fetch_proposals`, `fetch_laws`, `fetch_treasury`, `fetch_department_budgets`, `fetch_rulings`, `fetch_ipfs_content`, `auth_verify_nullifier`, `fetch_legislature_data`, `fetch_elections_data`, `fetch_anticorruption_data`, `chain_submit_extrinsic` — the read commands (all but `auth_verify_nullifier` and `chain_submit_extrinsic`) read from real chain storage via `state_getKeysPaged` + `state_queryStorageAt`. As of changelog #087 these Rust read commands are no longer the frontend's actual call path (see above) but remain correct and tested.
- **AI agent** (`commands/agent.rs`): `agent_ask(question, item_context, history)` — calls Claude API (`claude-sonnet-4-6`); reads `CLAUDE_API_KEY` env var; degrades gracefully offline
- **Auth** (`commands/auth.rs`): `auth_generate_challenge` generates UUID + embeds callback port in deep-link; `auth_poll_session` returns signed session; `auth_start_callback_server` spawns a local HTTP listener. `auth_verify_nullifier` (scans `Identity.NullifierRegistry` keys to confirm the mobile-reported nullifier exists on-chain before accepting the session) is called from the same auth flow but is actually implemented in `commands/chain.rs`, not `auth.rs`
- **Chain reads** (`commands/chain.rs`): `fetch_proposals` decodes `Voting.Referenda` (42-byte SCALE: petition_id(4) + topic_hash(32) + end_block(4) + state(1) + tier(1)) + `Voting.ReferendumTally` (8 bytes: yes(4) + no(4)); `fetch_rulings` cross-references `Courts.Cases` for IPFS ruling hashes; `fetch_department_budgets` decodes `DepartmentBudgets`/`DepartmentSpent` as u128 LE (16 bytes, 12 decimal places = 1 AGR); `format_agr(planck)` helper; `fetch_treasury` decodes `ExpenditureLog` as 52-byte SCALE (dept_id(4) + amount(16) + hash(32)); `fetch_legislature_data` decodes `Legislature.Members` (BoundedVec<AccountId>) + `Legislature.Motions` (77-byte SCALE: call_hash(32) + proposer(32) + ayes(4) + nays(4) + end_block(4) + executed(1)); `fetch_elections_data` decodes `PalletElections.Delegates` + `PalletElections.BackingCount`; `fetch_anticorruption_data` decodes `PalletAntiCorruption.AssetDisclosures` (ipfs_hash(32) + disclosed_at(4) + update_due_at(4), account recovered from the storage key)
- **Extrinsic submission**: `chain_submit_extrinsic` is registered and auth-gated (checks a valid, non-expired bearer token via `auth::require_valid_session` before relaying bytes to `author_submitExtrinsic`), but it is a thin passthrough that expects an already phone-signed extrinsic as input — no frontend file calls it, and the command's own doc comment in `commands/chain.rs` notes there is no phone-side flow yet that produces that signed input (QR-auth today only carries a signed challenge string, not a signed call). Treat it as plumbed but not yet connected to any UI flow, not as a working end-to-end feature like the read commands above.

Frontend pages (`src/pages/`): Proposals (with tier chip for constitutional referenda), Laws, Legislature (members + motions), Elections (delegates/backing), Courts, Treasury (department budget table + IPFS audit fetching), Anti-Corruption (asset disclosures), auth QR page (`AuthPage.tsx`), Claude AI sidebar panel.

Browser dev mode uses `desktop/src/lib/mocks.ts` stub data; when running as a native app, the nine chain-read commands above fire through the light client (`desktop/src/chain/`) and everything else fires through Tauri IPC as before.

TODOs:
- Mobile side of QR auth: phone NFC + ZK proof; `mobile/src/screens/AuthScreen.tsx` scaffolded (parses deep-link, signs challenge, POSTs to callback)

IPFS content fetching: **implemented** on all detail pages.
- `fetch_ipfs_content(hash_hex: String) -> String` Tauri command in `commands/chain.rs`
- Converts on-chain 32-byte SHA-256 digest → CIDv0 via bs58 multihash (0x1220 prefix)
- Fetches from `https://ipfs.io/ipfs/{cid}` with 30-second timeout
- `LawsPage`, `ProposalsPage`, `CourtsPage`, `TreasuryPage`, `AntiCorruptionPage` all fetch content on selection and pass full text to AI agent (`LegislaturePage` and `ElectionsPage` do not — no IPFS content fetching on those two)

