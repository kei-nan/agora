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

Calls:
- `submit_asset_disclosure(ipfs_hash)` — any signed; mandatory annual renewal
- `register_conflict(entity_id, conflict_type)` — any signed
- `clear_conflict(entity_id)` — any signed (self-removal)
- `submit_whistleblower_report(content_hash, zk_proof, public_inputs)` — gated by ZK citizenship proof;
  stores `public_inputs[0]` as nullifier; `(nullifier, content_hash)` unique per citizen per report
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

