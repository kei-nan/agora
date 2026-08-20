# pallet-anticorruption

### pallet-anticorruption (crate: pallet-anticorruption) — runtime index 17

Three accountability pillars for elected officials and public servants.

Storage:
- `AssetDisclosures`: `AccountId` → `AssetDeclaration { ipfs_hash, disclosed_at, update_due_at }`
- `ConflictRegistry`: `(AccountId, entity_id: u32)` → `ConflictEntry { conflict_type, registered_at }`
- `WhistleblowerReports`: `report_id` → `WhistleblowerReport { content_hash, submitted_at, status, nullifier }`
- `ReportNullifiers`: `(nullifier [u8;32], content_hash [u8;32])` → `bool` (dedup guard)
- `NextReportId`: `u32`
- `Investigators`: `BoundedVec<AccountId, 20>`

`ConflictType` enum: `FinancialInterest` | `FamilyRelation` | `FormerEmployer` | `BusinessPartner`

`ReportStatus` enum: `Pending` → `Flagged` → `UnderInvestigation` → `Cleared` | `ReferredToCourts`

Enforcement: `Pallet::has_current_disclosure(who)` returns `true` only if `who` has an
`AssetDisclosures` entry whose `update_due_at` has not yet passed. **Now wired with teeth**
(previously it was pure record-keeping, exercised only by this pallet's own unit tests):
`pallet_elections::DisclosureChecker<AccountId>` is a trait defined in pallet-elections (the
consumer) — `fn has_current_disclosure(who: &AccountId) -> bool` — that this pallet implements
directly on its own `Pallet<T>` (wrapping the inherent function above unchanged), the same
"consumer defines, provider implements on `Pallet<T>`" idiom already used for
`pallet_elections::SeatLegislature`/`pallet_legislature::Pallet<T>` and
`pallet_treasury_ledger::AuditHook`/`pallet_audit::Pallet<T>`. `runtime/src/configs/mod.rs` wires
`pallet_elections::Config::DisclosureChecker = PalletAntiCorruption` (the real implementation, not
a no-op). pallet-elections' `run_election` (its periodic `on_initialize` legislature-seating hook
— there's still no `register_candidate` call to gate; the Elections Commission subsystem that had
one was removed in commit `7d9a753`, see `docs/project/pallets/elections.md`) now checks
`T::DisclosureChecker::has_current_disclosure` per candidate alongside the existing
`CitizenChecker` re-check, at the same seating-time point: a delegate who would otherwise be
seated (Active, active citizen, ranked in the top `LegislatureSeats` by backing) but lacks a
current disclosure is **skipped, not hard-errored** — excluded from the candidate pool so the
next-highest-backed eligible delegate fills the seat instead, and
`Event::SeatingSkippedNoDisclosure { account }` is emitted so the skip is visible on-chain. Skip-
and-fall-through rather than failing the whole `on_initialize` call was a deliberate choice: this
hook runs unconditionally every block past the cycle boundary, so an error would freeze
legislature seating entirely until manual intervention, over one official's lapsed paperwork —
see `run_election`'s doc comment in `pallets/pallet-elections/src/lib.rs` for the full rationale.

Calls:
- `submit_asset_disclosure(ipfs_hash)` — any signed; mandatory annual renewal
- `register_conflict(entity_id, conflict_type)` — any signed
- `clear_conflict(entity_id)` — any signed (self-removal)
- `submit_whistleblower_report(content_hash, zk_proof, public_inputs)` — gated by ZK citizenship
  proof. Requires `public_inputs.len() >= MIN_PUBLIC_INPUTS` (checked before indexing anything or
  calling the verifier), then checks `public_inputs[SERVICE_SCOPE_INDEX]` /
  `[SERVICE_SUBSCOPE_INDEX]` against this call's own `WHISTLEBLOWER_REPORT_SERVICE_SCOPE`/
  `SUBSCOPE` constants — domain separation so a proof generated for a different purpose (e.g.
  `pallet-identity::register_citizen`) can't be replayed here even though it's a valid,
  observable-on-chain ZK proof of citizenship. Only after both of those and the verifier call
  (`T::ZkVerifier::verify`) succeed does it extract the per-citizen nullifier — from
  `public_inputs[public_inputs.len() - 2]` (`scoped_nullifier`), the same extraction
  `pallet_identity::Pallet::register_citizen` uses, **not** `public_inputs[0]`
  (`certificate_registry_root`, shared by every citizen at a given registry state, which was the
  bug fixed in commit `1f3941e`: using it as the dedup key would have collapsed dedup to
  essentially just `content_hash`, since `public_inputs[0]` doesn't vary per citizen at all).
  `(nullifier, content_hash)` is then checked/stored unique per citizen per report via
  `ReportNullifiers`.
  **Not sender-anonymous** despite the module doc's original "anonymous ZK whistleblower" framing
  and `CLAUDE.md`'s "ZK whistleblower" label: it's a normal `ensure_signed` extrinsic, so the
  reporter's `AccountId` is public block data, and pallet-identity's `CitizenNullifier` map
  (`AccountId -> nullifier`) lets any chain observer join that `AccountId` to the nullifier stored
  in this call's `WhistleblowerReport` and learn which registered citizen filed a given report, and
  when. What *is* protected: the report content itself never touches chain state — only
  `content_hash`, the IPFS pointer to a document encrypted to the investigator's key off-chain,
  is stored. Same structural gap as `pallet-voting::commit_vote` (documented at length on that
  call itself); see `submit_whistleblower_report`'s doc comment in
  `pallets/pallet-anticorruption/src/lib.rs` for the full writeup. No fix attempted here — would
  need an unsigned/ZK-gated submission path or a relayer, which doesn't exist anywhere in this
  repo yet.
- `flag_report(report_id)` — investigator: Pending → Flagged
- `open_investigation(report_id)` — investigator: Flagged → UnderInvestigation
- `clear_report(report_id)` — investigator: UnderInvestigation → Cleared
- `refer_report_to_courts(report_id)` — investigator: UnderInvestigation → ReferredToCourts;
  emits `ReportReferredToCourts`; investigator then files a case in pallet-courts
- `add_investigator(account)` / `remove_investigator(account)` — root

ZK verifier: `PassthroughAntiCorruptionZkVerifier` (dev-mode) / `ZkPassportAntiCorruptionZkVerifier` (prod).
Production impl reuses the same ZKPassport UltraHonk outer circuit as pallet-identity
(`crate::verifier::ZkPassportUltraHonkVerifier`) — not Rarimo/Groth16, which this codebase
migrated away from. The pallet's own whistleblower-specific circuit is still a different,
not-yet-built circuit (this binding only keeps the two verification paths consistent in the
meantime); it also inherits `verifier.rs`'s fail-closed behavior.

Config: `MaxInvestigators = 20`, `AssetDisclosureRenewalBlocks = 2_628_000` (~1 year at this chain's
actual 12s/block time — previously documented as `5_256_000`, which was ~1 year at a stale 6s/block
assumption and worked out to ~2 years at the real block time).

