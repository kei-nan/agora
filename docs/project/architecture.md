# Architecture: runtime wiring & pipeline

## Runtime features

- `default = ["std", "dev-mode"]`
- `dev-mode` enables `PassthroughZkVerifier` (accepts all ZK proofs). Strip this feature
  for any testnet/mainnet build. Without it, `runtime/src/verifier.rs` uses the real
  `ZkPassportUltraHonkVerifier` (targets ZKPassport's Noir/UltraHonk `main/outer/count_4`
  circuit; replaces the former `RarimoGroth16Verifier` — see changelog entry 65). It enforces
  the proof envelope and public-input layout for real, and the UltraHonk pairing check itself
  is genuinely performed via `ultrahonk-no-std` (a bb-5.0.0 port) — but no real ZKPassport
  `count_4` proof has been run through it end-to-end yet. See `docs/project/zk-verifier.md`
  for the full status.


## Cross-pallet trait wiring (runtime/src/configs/mod.rs)

| Trait | Implemented by | Calls |
|---|---|---|
| `ZkProofVerifier` (`pallet_identity_zk`) | `PassthroughZkVerifier` (dev) / `ZkPassportUltraHonkVerifier` (prod) | UltraHonk pairing check via `ultrahonk-no-std` (bb 5.0.0 port) |
| `CitizenChecker<AccountId>` | `Runtime` | `pallet_identity_zk::is_active_citizen` + `TotalCitizens` |
| `CitizenSelector<AccountId>` | `Runtime` | `pallet_identity_zk::CitizenIndex` + `TotalCitizens` |
| `LawEnforcer` | `Runtime` | `pallet_constitution::invalidate_law_internal` |
| `TreasuryEnforcer` | `Runtime` | `pallet_treasury_ledger::freeze_department_internal` |
| `PetitionApprover` | `Runtime` | `pallet_voting::create_referendum_internal` |
| `LawEnactor` | `Runtime` | `pallet_constitution::enact_law_internal(tier, hash)` |
| `CitizenSuspender` | `Runtime` | `pallet_identity_zk::suspend_citizen_internal` |
| `AuditHook` | `pallet_audit::Pallet<Runtime>` | `AuditLog::insert(index, Pending entry)` |
| `pallet_elections::CitizenChecker<AccountId>` | `Runtime` | `pallet_identity_zk::is_active_citizen` |
| `MinisterChecker<AccountId>` | `Cabinet` (`pallet_executive::Pallet<Runtime>`) | `MinisterPortfolio::contains_key` + `PrimeMinister` check |
| `FreshLegislatureChecker<BlockNumber>` | `Runtime` | reads `pallet_elections::LastElectionBlock` |
| `AutoChallengeHook` | `Runtime` | `pallet_courts::Pallet::<Runtime>::auto_file_case(LawChallenge)` |
| `ZkProofVerifier` (`pallet_anticorruption`, own trait — distinct from `pallet_identity_zk`'s) | `PassthroughAntiCorruptionZkVerifier` (dev) / `ZkPassportAntiCorruptionZkVerifier` (prod) | dev: always `true`; prod: delegates straight to `pallet_identity_zk`'s `ZkPassportUltraHonkVerifier` (reuses the same ZKPassport outer circuit — the pallet's own whistleblower circuit is still unbuilt, see HANDOFF item 8) |
| `LegislatureOrigin` (`pallet_executive::Config`) | `pallet_legislature::EnsureLegislatureMotion<Runtime>` | gates `appoint_minister` / `dismiss_minister` / `declare_emergency` / `end_emergency` on a passed legislature motion |

**`pallet-emergency-council` is not wired into the runtime at all yet** — it isn't a dependency
in `runtime/Cargo.toml`, has no `impl pallet_emergency_council::Config for Runtime` in
`runtime/src/configs/mod.rs`, and has no `#[runtime::pallet_index(15)]` entry in
`runtime/src/lib.rs`'s `construct_runtime!` (index 15 is skipped: `pallet-elections` is 14,
`pallet-audit` is 16). The crate exists standalone under `pallets/pallet-emergency-council/`
with its own `Config` trait, but there is no cross-pallet trait wiring to document until it's
actually plugged in. `pallet_identity_zk::Config::EmergencyRotationOrigin` is `EnsureRoot` as a
placeholder for exactly this reason (see its doc comment in `configs/mod.rs`).


## Full citizen → law pipeline

**Ordinary law via citizen petition:**
```
submit_petition(topic_hash)
  → sign_petition(petition_id)  [× 1 000 citizens]
    → PetitionApprover::create_referendum  [auto, same tx]
      → Ordinary referendum, 14-day window (or epoch end if epoch active)
        → vote_referendum(referendum_id, in_favor)  [any active citizen, during active epoch]
        → finalize_referendum(referendum_id)  [after end_block, anyone]
          → if yes*100 >= 51*total: LawEnactor::enact_law(Ordinary, topic_hash)
            → Laws storage: Ordinary law, Active
```

**Structural law via legislature:**
```
propose_motion(create_constitutional_referendum call)  [legislature member]
  → vote_motion / close_motion  [passes at >50%]
    → create_constitutional_referendum(topic_hash) → Constitutional referendum (67% threshold)
      → finalize_referendum → enact_law(Structural, hash)
        → Law enters Provisional stage + auto court review (AI judge Level 2)
```

**Foundational law via legislature:**
```
propose_motion(create_foundational_referendum call)  [legislature member]
  → vote_motion / close_motion  [passes at >50%]
    → create_foundational_referendum(topic_hash) → Foundational referendum (75% threshold)
      → finalize_referendum → enact_law(Foundational, hash)
        → Law enters Provisional stage + auto court review (AI judge Level 2)
```

**Ordinary law enacted directly by legislature:**
```
propose_motion(encoded enact_law call)  [legislature member]
  → vote_motion / close_motion  [passes at >50%]
    → enact_law(Ordinary, content_hash) executes
```

