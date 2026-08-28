# Architecture: runtime wiring & pipeline

## Runtime features

- `default = ["std"]` (fixed 2026-08-09 — `dev-mode` used to be a default feature too, which
  meant the documented `cargo build --release` command silently shipped passthrough verifiers)
- `dev-mode` enables `PassthroughZkVerifier` (accepts all ZK proofs). It is opt-in only
  (`--features dev-mode`); strip it (i.e. just don't pass it) for any testnet/mainnet build.
  Without it, `runtime/src/verifier.rs` uses the real
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
| `ZkProofVerifier` (`pallet_anticorruption`, own trait — distinct from `pallet_identity_zk`'s) | `PassthroughAntiCorruptionZkVerifier` (dev) / `ZkPassportAntiCorruptionZkVerifier` (prod) | dev: always `true`; prod: delegates straight to `pallet_identity_zk`'s `ZkPassportUltraHonkVerifier` (reuses the same ZKPassport outer circuit — the pallet's own whistleblower circuit is still unbuilt, see `docs/project/next-steps.md` item 8) |
| `LegislatureOrigin` (`pallet_executive::Config`) | `pallet_legislature::EnsureLegislatureMotion<Runtime>` | gates `define_portfolio` / `appoint_prime_minister` / `dismiss_prime_minister` / `appoint_minister` / `dismiss_minister` on a passed legislature motion. **Not** the Cabinet's emergency-declaration calls: `vote_declare_emergency` / `vote_end_emergency` / `retract_emergency_vote` are gated by `is_cabinet_member` instead — the legislature only ratifies an already-active emergency after the fact, via the separately-`LegislatureOrigin`-gated `ratify_emergency` (see `docs/project/pallets/executive.md`) |
| `SuspensionOrigin` (`pallet_identity_zk::Config`) | `pallet_courts::EnsureOracleCouncilApproved<Runtime>` | manual `suspend_citizen` override only succeeds once the Oracle Council's M-of-N threshold has approved that exact call (the auto-enforcement path uses `CitizenSuspender` above instead) |
| `CourtOrigin` (`pallet_constitution::Config`, `pallet_treasury_ledger::Config`) | `pallet_courts::EnsureOracleCouncilApproved<Runtime>` | manual `invalidate_law` override and `unfreeze_department` each require the same Oracle Council M-of-N approval, not bare `Root` — a court-ordered law invalidation/treasury freeze can only be manually reversed by the same council that stands behind an actual ruling |
| `AppointmentOrigin` (`pallet_audit::Config`, `pallet_anticorruption::Config`) | `pallet_accountability_council::EnsureAccountabilityCouncilApproved<Runtime>` | `add_auditor`/`remove_auditor`/`add_investigator`/`remove_investigator` require the independent Accountability Council's 2/3 supermajority approval, closing the self-oversight risk of routing appointment through the legislature that also controls the audited/investigated treasury spend |
| `LegislatureChecker` / `ExecutiveChecker` (`pallet_accountability_council::Config`) | `Runtime` | reads `pallet_legislature::Members` / `pallet_executive::MinisterPortfolio`+`PrimeMinister` directly, backing `add_member`/`remove_member`'s `LegislatureOrExecutiveOverlap` rejection — an Accountability Council member can never simultaneously sit in the legislature or hold an executive/cabinet role |
| `pallet_elections::CommitteeKeyChecker` | `Runtime` | delegates to `pallet_identity_zk::Pallet::are_committee_keys_approved` — same `OprfCommitteeKeys` check `register_citizen` performs on itself, reused so a delegate-persona/backing proof can only reference an approved OPRF committee key generation |
| `pallet_elections::BackingRootChecker` | `Runtime` | delegates to `pallet_identity_zk::Pallet::is_valid_backing_commitment_root` — pallet-identity's own backing-commitment root history, so `back_delegate`/`remove_backing` can only reference a root pallet-identity actually published |
| `pallet_elections::DelegatePersonaVerifier` | `PassthroughAnchorVerifier` (dev) / `crate::anchor_verifier::Poseidon2AnchorVerifier` (prod) | dev: always `true`; prod: genuinely recomputes and checks the Poseidon2 `param_commitment` for a `register_as_delegate` outer proof, the same recomputation `pallet_identity_zk::Config::AnchorVerifier` performs for citizen registration |
| `pallet_elections::BackingProofVerifier` | `PassthroughZkVerifier` (dev) / `crate::backing_nullifier_verifier::BackingNullifierVerifier` (prod) | dev: always `true`; prod: real standalone UltraHonk pairing check against the `backing-nullifier` circuit, verified by `back_delegate`/`remove_backing` |
| `pallet_elections::DisclosureChecker` | `PalletAntiCorruption` (`pallet_anticorruption::Pallet<Runtime>`) | checked per candidate at legislature-seating time — a delegate whose asset disclosure has lapsed or was never filed is skipped in favor of the next-highest-backed eligible delegate (`SeatingSkippedNoDisclosure` emitted) |
| `pallet_identity_zk::RecoveryStateChecker` | `Runtime` | backs `recover_account`'s divest-first guards — reads pallet-elections (delegate persona), pallet-legislature (seat), pallet-executive (minister/PM role), and pallet-voting (`has_open_referendum_vote` against the bounded `OpenReferenda` list, `has_unclaimed_current_epoch_budget` against `CitizenClaimedEpoch`) directly, blocking recovery while any of that state would otherwise be silently orphaned or, for the two pallet-voting checks, actively double-spendable |

**`pallet-emergency-council` is wired into the runtime** — it's a real dependency in
`runtime/Cargo.toml`, has a real `impl pallet_emergency_council::Config for Runtime` in
`runtime/src/configs/mod.rs`, and sits at `#[runtime::pallet_index(15)]` in
`runtime/src/lib.rs`'s `#[frame_support::runtime]` macro (confirmed by reading the file
directly). `pallet_identity_zk::Config::EmergencyRotationOrigin` is still `EnsureRoot` as a
placeholder pending a dedicated collective for mainnet — see its doc comment in
`configs/mod.rs` — but that's a separate, tracked gap from whether the pallet is wired in at
all. (This section previously claimed the pallet wasn't wired in; that was stale — this is the
same false claim CLAUDE.md carried and corrected on 2026-08-08, recurring here in a second
file. Verify against `runtime/src/lib.rs` directly rather than trusting either doc if it
matters for what you're doing.)


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

