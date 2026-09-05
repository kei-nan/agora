//! Court-oracle orchestration loop (see README.md): polls `Courts::Cases` for `CaseStatus::Filed`
//! entries, builds a case-appropriate context from other on-chain storage, asks Claude for a
//! Level-0 AI ruling, publishes the full reasoning document to IPFS, and submits
//! `submit_ai_ruling` signed by the configured oracle account.
//!
//! Also polls for cases in `CaseStatus::AIRulingIssued` whose appeal window
//! (`AIRulingBlock[case_id] + AppealWindowBlocks`) has closed with no `appeal_ruling` call in
//! between, and calls `finalize_ruling(case_id)` for them — the second, separate oracle-signed
//! call `pallet-courts` requires before an unappealed AI ruling actually takes effect
//! (auto-enforcement: pausing a law, freezing a department, suspending a citizen). See
//! `should_finalize` below for the pure deadline/status logic. `finalize_ruling` takes no
//! verdict argument of its own -- it applies whatever `submit_ai_ruling` already committed
//! on-chain (`AIRulingVerdict`) when this service first ruled on the case, so `rule_on_case`
//! determines and submits the verdict once, up front, rather than this service reconstructing
//! it a second time at finalize.
//!
//! **Never run against a live chain, live Claude API, or live IPFS daemon in this sandboxed
//! environment** — no network egress to any of the three is available here. See README.md for
//! exactly what is real (compiles, unit-tested pure logic) vs. assumed (the live-integration
//! path, never executed).
//!
//! Also see README.md's "Operational robustness" section for three later hardening fixes: a
//! per-case exponential-backoff-then-give-up circuit breaker (`retry.rs`) so a persistently
//! failing case doesn't burn Claude API budget forever; on-disk persistence of which case_ids
//! have already been submitted (`state.rs`) so a crash-restart doesn't forget and redundantly
//! re-bill Claude for one; and a specific diagnostic (in `poll_once` below) for the case where
//! this service's oracle account has been removed from `Courts::OracleMembers`.

mod cases;
mod claude;
mod config;
mod context;
mod extrinsic;
mod ipfs;
mod keystore;
mod retry;
mod rpc;
mod state;

use cases::{AuditEntry, CaseRecord, CaseStatus, CaseSubject, LawRecord, Verdict};
use config::Config;
use context::SubjectContext;
use retry::{FailureOutcome, RetryTracker};
use rpc::RpcClient;
use state::PersistedState;

use anyhow::Context as _;
use codec::{Decode, Encode};
use serde::Serialize;
use sp_core::crypto::{AccountId32, Ss58Codec};
use sp_core::Pair as _;
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse().unwrap()),
        )
        .init();

    let config = Config::from_env()?;
    tracing::info!(
        node_rpc_url = %config.node_rpc_url,
        claude_model = %config.claude_model,
        dry_run = config.dry_run,
        "starting court-oracle — see README.md for what's real vs. assumed"
    );

    let claude_api_key = std::env::var("CLAUDE_API_KEY").map_err(|_| {
        anyhow::anyhow!(
            "CLAUDE_API_KEY not set — required to generate rulings, refusing to start without it"
        )
    })?;

    let passphrase = config.resolve_passphrase()?;
    let secrets = keystore::load(&config.keys_file, &passphrase)?;
    let seed = secrets.oracle_account_seed_bytes()?;
    let account_id =
        AccountId32::from(<sp_core::sr25519::Pair as sp_core::Pair>::from_seed(&seed).public().to_raw());
    tracing::info!(oracle_account = %account_id.to_ss58check(), "loaded oracle signing key");
    tracing::warn!(
        "this account must be registered on-chain as an Oracle Council member via \
         `Courts::add_oracle_member` (root-only) before submit_ai_ruling/finalize_ruling calls \
         signed by it will even be accepted as proposals -- and, under the M-of-N design, a \
         ruling only actually takes effect once enough OTHER council members' instances \
         approve the same case too. This service does not register itself or coordinate with \
         other instances, see README.md's 'Oracle Council' section"
    );

    let rpc = RpcClient::new(config.node_rpc_url.clone());
    let claude_client = claude::ClaudeClient::new(claude_api_key, config.claude_model.clone());
    let ipfs_client = ipfs::IpfsClient::new(config.ipfs_api_url.clone());

    // Loaded from disk so a crash-restart doesn't forget which cases were already submitted and
    // redundantly re-run (and re-bill) a Claude call for one whose ruling already made it
    // on-chain — see state.rs's module doc comment. `persisted.processed`/`persisted.finalized`
    // are what `main.rs` used to call `already_processed`/`finalize_processed`.
    let mut persisted = PersistedState::load(&config.state_file).unwrap_or_else(|e| {
        tracing::error!(
            error = %e,
            state_file = %config.state_file.display(),
            "failed to load STATE_FILE -- starting from empty processed/finalized sets. If this \
             file exists and is simply corrupt, a case already ruled on before this restart may \
             now trigger one redundant (billed) Claude call before its idempotent on-chain guard \
             rejects the resubmission -- see state.rs"
        );
        PersistedState::default()
    });
    tracing::info!(
        state_file = %config.state_file.display(),
        already_processed = persisted.processed.len(),
        already_finalized = persisted.finalized.len(),
        "loaded persisted case-tracking state"
    );

    // Per-case exponential backoff + give-up circuit breaker (see retry.rs) so a case that keeps
    // failing doesn't re-trigger a fresh billed Claude call (or a fresh submission attempt)
    // every single poll_interval_secs forever. Two independent trackers: a case backing off on
    // the rule/submit path is unrelated to one backing off on the finalize path.
    let retry_base = Duration::from_secs(config.retry_base_delay_secs);
    let retry_max = Duration::from_secs(config.retry_max_delay_secs);
    let mut rule_retry = RetryTracker::new(retry_base, retry_max, config.retry_max_attempts);
    let mut finalize_retry = RetryTracker::new(retry_base, retry_max, config.retry_max_attempts);

    let mut interval = tokio::time::interval(Duration::from_secs(config.poll_interval_secs));

    loop {
        interval.tick().await;
        if let Err(e) = poll_once(
            &rpc,
            &config,
            &claude_client,
            &ipfs_client,
            &seed,
            &account_id,
            &mut persisted,
            &mut rule_retry,
            &mut finalize_retry,
        )
        .await
        {
            tracing::error!(error = %e, "poll cycle failed, will retry next interval");
        }
    }
}

/// The document actually published to IPFS — the full reasoning record. `ruling_hash`
/// submitted on-chain is derived from the CID this publish produces (see ipfs.rs's header
/// comment for why it's the CID's digest, not a plain hash of this JSON's bytes).
#[derive(Serialize)]
struct RulingDocument<'a> {
    case_id: u32,
    subject_kind: &'a str,
    model: &'a str,
    verdict: &'a str,
    reasoning: &'a str,
}

#[allow(clippy::too_many_arguments)]
async fn poll_once(
    rpc: &RpcClient,
    config: &Config,
    claude_client: &claude::ClaudeClient,
    ipfs_client: &ipfs::IpfsClient,
    seed: &[u8; 32],
    oracle_account: &AccountId32,
    persisted: &mut PersistedState,
    rule_retry: &mut RetryTracker,
    finalize_retry: &mut RetryTracker,
) -> anyhow::Result<()> {
    let model_version = fetch_current_ai_model_version(rpc).await?;
    if model_version == 0 {
        tracing::warn!("CurrentAIModelVersion is 0 (no AI model has ever been governance-approved) — submit_ai_ruling would reject every call with NoApprovedAIModel; skipping this poll cycle entirely rather than ruling on cases we can't submit for");
        return Ok(());
    }

    // Diagnostic for the key-rotation/removal gap: `submit_ai_ruling`/`finalize_ruling` are both
    // gated by `T::OracleOrigin` (`EnsureOracle`), which rejects with `BadOrigin` at *dispatch*
    // time if the signer isn't in `OracleMembers` -- `author_submitExtrinsic` itself still
    // returns a tx hash in that case (origin checks run during block execution, not transaction-
    // pool validation), so a removed oracle account would otherwise show up only as a
    // mysteriously-never-applied ruling with no error at all, or (on a follow-up cycle re-
    // attempting the same still-`Filed` case) a confusing repeat with no specific cause. Checking
    // membership directly against chain state up front turns that into an unambiguous, specific
    // log line instead -- and, as a bonus, avoids burning a Claude call this cycle on rulings
    // that could not be accepted anyway.
    let oracle_members = fetch_oracle_members(rpc).await?;
    if !oracle_members.iter().any(|m| m == oracle_account) {
        tracing::error!(
            oracle_account = %oracle_account.to_ss58check(),
            oracle_member_count = oracle_members.len(),
            "this account is NOT currently in Courts::OracleMembers -- submit_ai_ruling/\
             finalize_ruling calls signed by it will be accepted into a block but rejected at \
             dispatch with BadOrigin (author_submitExtrinsic would still report a tx hash, which \
             is NOT confirmation of success). Was this account removed via remove_oracle_member, \
             or does it still need add_oracle_member run for it? Skipping this poll cycle rather \
             than submitting rulings that cannot take effect."
        );
        return Ok(());
    }
    // Needed for the finalize-scheduling branch below; fetched once per cycle (not once per
    // case — the appeal deadline check only cares about "now", which doesn't change mid-cycle).
    let current_block = rpc.get_current_block_number().await?;

    let prefix = rpc::storage_prefix("Courts", "Cases");
    let keys = rpc.get_keys_paged(&prefix).await?;
    if keys.is_empty() {
        tracing::debug!("no cases on-chain");
        return Ok(());
    }
    let values = rpc.query_storage_at(&keys).await?;

    for (key_hex, value_hex) in keys.iter().zip(values.iter()) {
        let Some(value_hex) = value_hex else { continue };
        let Some(case_id) = cases::decode_u32_map_key(key_hex) else {
            tracing::warn!(key = %key_hex, "could not extract case_id from storage key — skipping");
            continue;
        };
        let raw = match hex::decode(value_hex.trim_start_matches("0x")) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(case_id, error = %e, "Cases value was not valid hex");
                continue;
            }
        };
        let case: CaseRecord = match Decode::decode(&mut &raw[..]) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(case_id, error = ?e, "could not decode Cases value — storage shape mismatch?");
                continue;
            }
        };
        let (filer, status, _ruling_hash, subject) = case;

        match status {
            CaseStatus::Filed => {
                if persisted.processed.contains(&case_id) {
                    continue;
                }
                let now = Instant::now();
                if !rule_retry.should_attempt(case_id, now) {
                    if rule_retry.has_given_up(case_id) {
                        tracing::debug!(case_id, "still in given-up state from an earlier poll cycle — needs manual attention, not retrying without a restart");
                    } else {
                        tracing::debug!(case_id, "still backing off from a recent failure — skipping this poll cycle");
                    }
                    continue;
                }
                tracing::info!(case_id, filer = %filer.to_ss58check(), subject = ?subject, "found filed case, ruling");

                match rule_on_case(rpc, config, claude_client, ipfs_client, &filer, case_id, &subject).await {
                    Ok((ruling_hash, verdict)) => {
                        if config.dry_run {
                            tracing::info!(case_id, ruling_hash = %hex::encode(ruling_hash), verdict = ?verdict, "DRY_RUN set — not submitting submit_ai_ruling");
                            rule_retry.record_success(case_id);
                            persisted.processed.insert(case_id);
                            save_persisted_state(persisted, &config.state_file);
                            continue;
                        }
                        let call = extrinsic::SubmitAiRuling { case_id, ruling_hash, model_version, verdict };
                        match extrinsic::build_signed(
                            rpc,
                            seed,
                            config.courts_pallet_index,
                            config.submit_ai_ruling_call_index,
                            call,
                        )
                        .await
                        {
                            Ok(extrinsic_hex) => match rpc.submit_extrinsic(&extrinsic_hex).await {
                                Ok(tx_hash) => {
                                    tracing::info!(case_id, tx_hash, oracle_account = %oracle_account.to_ss58check(), "submitted submit_ai_ruling");
                                    rule_retry.record_success(case_id);
                                    persisted.processed.insert(case_id);
                                    save_persisted_state(persisted, &config.state_file);
                                }
                                Err(e) => {
                                    log_retry_outcome(case_id, "author_submitExtrinsic failed for submit_ai_ruling", &e, rule_retry.record_failure(case_id, now));
                                }
                            },
                            Err(e) => {
                                log_retry_outcome(case_id, "failed to build/sign extrinsic", &e, rule_retry.record_failure(case_id, now));
                            }
                        }
                    }
                    Err(e) => {
                        log_retry_outcome(case_id, "failed to produce a ruling for this case", &e, rule_retry.record_failure(case_id, now));
                    }
                }
            }
            CaseStatus::AIRulingIssued => {
                if persisted.finalized.contains(&case_id) {
                    continue;
                }
                let now = Instant::now();
                if !finalize_retry.should_attempt(case_id, now) {
                    if finalize_retry.has_given_up(case_id) {
                        tracing::debug!(case_id, "still in given-up state from an earlier poll cycle for finalize_ruling — needs manual attention, not retrying without a restart");
                    } else {
                        tracing::debug!(case_id, "still backing off from a recent finalize_ruling failure — skipping this poll cycle");
                    }
                    continue;
                }
                let ruling_block = match fetch_ai_ruling_block(rpc, case_id).await {
                    Ok(b) => b,
                    Err(e) => {
                        tracing::error!(case_id, error = %e, "failed to read AIRulingBlock — skipping this poll cycle");
                        continue;
                    }
                };
                if !should_finalize(&CaseStatus::AIRulingIssued, ruling_block, current_block, config.appeal_window_blocks) {
                    // Still within the appeal window (or ruling_block missing) — nothing to do
                    // yet. Not an error: this is the normal state for most of a case's appeal
                    // window.
                    continue;
                }
                tracing::info!(case_id, "appeal window closed unappealed, finalizing");
                if config.dry_run {
                    tracing::info!(case_id, "DRY_RUN set — not submitting finalize_ruling");
                    finalize_retry.record_success(case_id);
                    persisted.finalized.insert(case_id);
                    save_persisted_state(persisted, &config.state_file);
                    continue;
                }
                // No verdict to determine or pass here: finalize_ruling(case_id) applies
                // whatever verdict submit_ai_ruling already committed on-chain when this
                // service first ruled on the case (see module doc comment).
                let call = extrinsic::FinalizeRuling { case_id };
                match extrinsic::build_signed(
                    rpc,
                    seed,
                    config.courts_pallet_index,
                    config.finalize_ruling_call_index,
                    call,
                )
                .await
                {
                    Ok(extrinsic_hex) => match rpc.submit_extrinsic(&extrinsic_hex).await {
                        Ok(tx_hash) => {
                            tracing::info!(case_id, tx_hash, oracle_account = %oracle_account.to_ss58check(), "submitted finalize_ruling");
                            finalize_retry.record_success(case_id);
                            persisted.finalized.insert(case_id);
                            save_persisted_state(persisted, &config.state_file);
                        }
                        Err(e) => {
                            log_retry_outcome(case_id, "author_submitExtrinsic failed for finalize_ruling", &e, finalize_retry.record_failure(case_id, now));
                        }
                    },
                    Err(e) => {
                        log_retry_outcome(case_id, "failed to build/sign finalize_ruling extrinsic", &e, finalize_retry.record_failure(case_id, now));
                    }
                }
            }
            _ => continue,
        }
    }

    Ok(())
}

/// Saves `persisted` to `state_file`, logging (not propagating) any failure — a save failure
/// should never abort an otherwise-successful poll cycle (the on-chain submission already
/// happened), but it does mean this restart-safety net has a hole until the next successful
/// save, which is worth an operator's attention.
fn save_persisted_state(persisted: &PersistedState, state_file: &std::path::Path) {
    if let Err(e) = persisted.save(state_file) {
        tracing::error!(
            error = %e,
            state_file = %state_file.display(),
            "failed to persist case-tracking state to disk -- a crash before the next \
             successful save could cause a redundant (billed) Claude call and resubmission \
             attempt on restart for the case just processed (chain-side idempotency still \
             prevents that resubmission from double-applying anything, see state.rs)"
        );
    }
}

/// Logs a failed processing attempt for `case_id` at the severity appropriate to what
/// `RetryTracker::record_failure` decided: a routine "will retry in Ns" at `warn`, or a
/// prominent, distinctly-worded "giving up" at `error` once the circuit breaker trips — see
/// retry.rs's module doc comment for why this is scoped to be simple (in-memory, per-process)
/// rather than a durable job-queue.
fn log_retry_outcome(case_id: u32, context: &str, error: &anyhow::Error, outcome: FailureOutcome) {
    match outcome {
        FailureOutcome::WillRetry { attempt, delay } => {
            tracing::warn!(
                case_id,
                error = %error,
                attempt,
                retry_in_secs = delay.as_secs(),
                "{context} — will retry after backoff"
            );
        }
        FailureOutcome::GaveUp { attempts } => {
            tracing::error!(
                case_id,
                error = %error,
                attempts,
                "{context} — GIVING UP on this case after {attempts} failed attempts, needs \
                 manual attention. It will not be retried again until this service restarts \
                 (which resets its in-memory retry count) — see retry.rs. If this case's \
                 content is genuinely unrulable (e.g. malformed context), it will immediately \
                 fail and give up again after restart, so a human should investigate why before \
                 just bouncing the process."
            );
        }
    }
}

/// Reads `pallet_courts::OracleMembers` — a plain `StorageValue<BoundedVec<AccountId,
/// MaxOracleMembers>, ValueQuery>`, no map key. A `BoundedVec`'s SCALE encoding is identical to
/// a plain `Vec`'s (a compact length prefix followed by the items) — the bound is enforced at
/// construction/mutation time on-chain, not in the wire format — so decoding as `Vec<AccountId32>`
/// here is exact, not an approximation. Used once per poll cycle to give an unambiguous,
/// specific diagnostic if this service's configured oracle account is no longer a registered
/// member (see the `BadOrigin` doc comment at its call site in `poll_once`) rather than letting
/// that show up only as silently-inert submissions.
async fn fetch_oracle_members(rpc: &RpcClient) -> anyhow::Result<Vec<AccountId32>> {
    let key = rpc::storage_prefix("Courts", "OracleMembers");
    let Some(bytes) = rpc.get_storage(&key).await? else { return Ok(vec![]) };
    Vec::<AccountId32>::decode(&mut &bytes[..]).context("could not decode Courts::OracleMembers")
}

/// Pure decision logic for whether an `AIRulingIssued` case should be finalized this poll
/// cycle — extracted from `poll_once`'s I/O so it can be unit tested directly, mirroring this
/// crate's existing split between pure logic (tested) and network orchestration (compiled only,
/// see README.md's "What's assumed / never executed"). Deliberately mirrors
/// `pallet_courts::finalize_ruling`'s own on-chain gate exactly (`status == AIRulingIssued` and
/// `now > ruling_block + AppealWindowBlocks`, strict `>` not `>=`) so this service doesn't waste
/// extrinsics attempting a finalize the chain would reject anyway — the chain's own check
/// remains the actual authority regardless of what this function decides.
fn should_finalize(
    status: &CaseStatus,
    ruling_block: Option<u32>,
    current_block: u32,
    appeal_window_blocks: u32,
) -> bool {
    if *status != CaseStatus::AIRulingIssued {
        // Covers both "still Filed" (no ruling yet) and "appealed" (InJuryAppeal/JurySeated/...)
        // — an appealed case must never be finalized via this no-appeal path, regardless of how
        // much time has passed. `appeal_ruling` moves status off `AIRulingIssued` the moment an
        // appeal is filed, so this one check is sufficient.
        return false;
    }
    let Some(ruling_block) = ruling_block else {
        // AIRulingBlock should always be set alongside an AIRulingIssued status (both written
        // by submit_ai_ruling in the same call) — missing means either a storage-shape mismatch
        // or a read race with a not-yet-finalized reorg. Treat as "not ready" rather than
        // guessing a deadline.
        return false;
    };
    let deadline = ruling_block.saturating_add(appeal_window_blocks);
    current_block > deadline
}

/// Reads `pallet_courts::AIRulingBlock[case_id]` — the block `submit_ai_ruling` was called at
/// for this case, used to compute its appeal deadline. `Blake2_128Concat`-hashed `u32` map key,
/// same pattern as `map_key_u32`'s other callers.
async fn fetch_ai_ruling_block(rpc: &RpcClient, case_id: u32) -> anyhow::Result<Option<u32>> {
    let key = map_key_u32("Courts", "AIRulingBlock", case_id);
    let Some(bytes) = rpc.get_storage(&key).await? else { return Ok(None) };
    Ok(u32::decode(&mut &bytes[..]).ok())
}

/// Builds case context, asks Claude, publishes the reasoning document to IPFS, and returns
/// both the `ruling_hash` and `verdict` to submit on-chain together (via
/// `submit_ai_ruling(case_id, ruling_hash, model_version, verdict)`, which commits the verdict
/// at submission time — see module doc comment). Does not itself submit the extrinsic — that's
/// `poll_once`'s job, so this function stays testable-by-construction (every side effect is
/// delegated to an injected client) even though it isn't unit-tested directly here (it does
/// real I/O throughout — see this crate's README on what actually has test coverage).
async fn rule_on_case(
    rpc: &RpcClient,
    config: &Config,
    claude_client: &claude::ClaudeClient,
    ipfs_client: &ipfs::IpfsClient,
    filer: &AccountId32,
    case_id: u32,
    subject: &CaseSubject,
) -> anyhow::Result<([u8; 32], Verdict)> {
    let subject_context = build_subject_context(rpc, config, subject).await?;
    let case_context = context::render_case_context(case_id, &filer.to_ss58check(), &subject_context);

    let ruling = claude_client.rule(case_id, &case_context).await?;

    let verdict = match ruling.verdict {
        claude::Verdict::Upheld => Verdict::Upheld,
        claude::Verdict::Overturned => Verdict::Overturned,
    };
    let verdict_str = match verdict {
        Verdict::Upheld => "Upheld",
        Verdict::Overturned => "Overturned",
    };
    let document = RulingDocument {
        case_id,
        subject_kind: context::subject_kind(subject),
        model: &config.claude_model,
        verdict: verdict_str,
        reasoning: &ruling.reasoning,
    };
    let document_bytes = serde_json::to_vec_pretty(&document)?;

    let cid = ipfs_client.add(&format!("case-{case_id}-ruling.json"), document_bytes).await?;
    let ruling_hash = ipfs::cidv0_to_digest(&cid)?;
    tracing::info!(case_id, cid = %cid, verdict = verdict_str, "published ruling reasoning to IPFS");

    Ok((ruling_hash, verdict))
}

/// Fetches whatever additional on-chain context is appropriate for `subject`, per
/// `context::SubjectContext`'s own documentation of what exists for each variant.
async fn build_subject_context(
    rpc: &RpcClient,
    config: &Config,
    subject: &CaseSubject,
) -> anyhow::Result<SubjectContext> {
    match subject {
        CaseSubject::General => Ok(SubjectContext::General),
        CaseSubject::CitizenConduct { nullifier, suspension_blocks } => {
            Ok(SubjectContext::CitizenConduct { nullifier: *nullifier, suspension_blocks: *suspension_blocks })
        }
        CaseSubject::LawChallenge { law_id } => {
            let law = fetch_law(rpc, *law_id).await?;
            // Best-effort IPFS content fetch by the law's content hash, mirroring the desktop
            // app's `fetch_ipfs_content` convention (CIDv0 derived from the raw hash, fetched
            // from the public ipfs.io gateway). Not implemented here beyond a direct HTTP GET —
            // see README.md: no IPFS daemon/gateway is reachable in this sandboxed environment,
            // so this path has never actually been exercised.
            let content = match &law {
                Some((_, _, _, content_hash)) => fetch_ipfs_gateway_content(content_hash).await,
                None => None,
            };
            Ok(SubjectContext::LawChallenge { law_id: *law_id, law, content })
        }
        CaseSubject::TreasuryDispute { department_id } => {
            let budget = fetch_u128_value(rpc, "TreasuryLedger", "DepartmentBudgets", *department_id)
                .await?
                .unwrap_or(0);
            let spent = fetch_u128_value(rpc, "TreasuryLedger", "DepartmentSpent", *department_id)
                .await?
                .unwrap_or(0);
            let frozen = fetch_bool_value(rpc, "TreasuryLedger", "FrozenDepartments", *department_id)
                .await?
                .unwrap_or(false);
            let expenditures = fetch_expenditures_for_department(rpc, *department_id).await?;
            let audit_entries = fetch_audit_entries_for_department(rpc, config, *department_id).await?;
            Ok(SubjectContext::TreasuryDispute {
                department_id: *department_id,
                budget,
                spent,
                frozen,
                expenditures,
                audit_entries,
            })
        }
    }
}

/// Reads `pallet_courts::CurrentAIModelVersion` (a plain `StorageValue<u32, ValueQuery>`, no
/// map key) fresh from chain. `submit_ai_ruling` requires the submitted `model_version` to
/// match this exactly, and rejects with `NoApprovedAIModel` if it's still the ValueQuery
/// default of 0 (no model ever governance-approved) — a missing storage entry decodes the
/// same way, so `None` is treated identically to `Some(0)` here.
async fn fetch_current_ai_model_version(rpc: &RpcClient) -> anyhow::Result<u32> {
    let key = rpc::storage_prefix("Courts", "CurrentAIModelVersion");
    let Some(bytes) = rpc.get_storage(&key).await? else { return Ok(0) };
    Ok(u32::decode(&mut &bytes[..]).unwrap_or(0))
}

async fn fetch_law(rpc: &RpcClient, law_id: u32) -> anyhow::Result<Option<LawRecord>> {
    let key = map_key_u32("Constitution", "Laws", law_id);
    let Some(bytes) = rpc.get_storage(&key).await? else { return Ok(None) };
    match LawRecord::decode(&mut &bytes[..]) {
        Ok(record) => Ok(Some(record)),
        Err(e) => {
            tracing::warn!(law_id, error = ?e, "could not decode Laws value");
            Ok(None)
        }
    }
}

/// Best-effort fetch of a law's full text from the public IPFS gateway, mirroring
/// `desktop/src-tauri/src/commands/chain.rs`'s `fetch_ipfs_content`/`hash_to_cid` convention
/// (the on-chain hash is treated as a raw SHA-256 digest, wrapped in a CIDv0 header). Returns
/// `None` on any failure — unreachable gateway, non-200, non-UTF8 body, **or a content-hash
/// mismatch** (see `fetch_and_verify_ipfs_content` below) — rather than propagating an error: a
/// missing law text should not abort ruling on the case, just leave that context honestly
/// marked unavailable (see `context::render_case_context`). Critically, a hash mismatch is
/// treated the same as "unavailable," never as "available" — the caller never sees content that
/// failed verification, only `None` or genuinely-verified text.
async fn fetch_ipfs_gateway_content(content_hash: &[u8; 32]) -> Option<String> {
    match fetch_and_verify_ipfs_content(content_hash).await {
        Ok(text) => Some(text),
        Err(e) => {
            // A hash mismatch specifically is worth a louder log than "gateway timed out" — it
            // can mean a misbehaving/malicious gateway, or content that was altered after the
            // law's content_hash was committed on-chain. Either way it's worth an operator's
            // attention even though this service still degrades gracefully by ruling without
            // the full text (see doc comment above).
            if e.to_string().contains("hash mismatch") {
                tracing::error!(content_hash = %hex::encode(content_hash), error = %e, "IPFS gateway returned content that does not match the on-chain hash — refusing to use it");
            } else {
                tracing::debug!(content_hash = %hex::encode(content_hash), error = %e, "IPFS content fetch failed — ruling without full law text");
            }
            None
        }
    }
}

/// Does the actual gateway fetch and, unlike the bug this replaced, verifies the fetched
/// content's SHA-256 digest against `content_hash` before returning it — see
/// `ipfs::verify_content_hash`'s doc comment for why a gateway response must never be trusted
/// on a bare 200 status alone. Returns `Err` (not a silently-accepted body) on any failure,
/// including a hash mismatch, so unverified content structurally cannot reach
/// `context::render_case_context` / the Claude prompt.
async fn fetch_and_verify_ipfs_content(content_hash: &[u8; 32]) -> anyhow::Result<String> {
    let cid = ipfs::digest_to_cidv0(content_hash);
    let url = format!("https://ipfs.io/ipfs/{cid}");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("failed to build IPFS gateway HTTP client")?;
    let resp = client.get(&url).send().await.context("IPFS gateway unreachable")?;
    if !resp.status().is_success() {
        anyhow::bail!("IPFS gateway returned HTTP {}", resp.status());
    }
    let bytes = resp.bytes().await.context("failed to read IPFS gateway response body")?;
    ipfs::verify_content_hash(&bytes, content_hash)?;
    String::from_utf8(bytes.to_vec()).context("IPFS content was not valid UTF-8")
}

async fn fetch_u128_value(
    rpc: &RpcClient,
    pallet: &str,
    item: &str,
    key: u32,
) -> anyhow::Result<Option<u128>> {
    let storage_key = map_key_u32(pallet, item, key);
    let Some(bytes) = rpc.get_storage(&storage_key).await? else { return Ok(None) };
    Ok(u128::decode(&mut &bytes[..]).ok())
}

async fn fetch_bool_value(
    rpc: &RpcClient,
    pallet: &str,
    item: &str,
    key: u32,
) -> anyhow::Result<Option<bool>> {
    let storage_key = map_key_u32(pallet, item, key);
    let Some(bytes) = rpc.get_storage(&storage_key).await? else { return Ok(None) };
    Ok(bool::decode(&mut &bytes[..]).ok())
}

/// Reads `TreasuryLedger::DepartmentExpenditures[department_id]` — a `department_id ->
/// expenditure_index` secondary index pallet-treasury-ledger now maintains alongside
/// `ExpenditureLog`, populated by the same `record_expenditure` call that writes the primary
/// log entry (see that pallet's storage doc comment for why the two can never drift apart) —
/// to find which expenditure indices belong to this department, then batch-fetches just those
/// entries from `ExpenditureLog` itself.
///
/// This replaces an earlier full-`ExpenditureLog`-scan-and-filter implementation, whose own doc
/// comment plainly flagged it as "does not scale" — a `/project-review` pass caught it and this
/// is the fix. A `state_getKeysPaged` call scoped to this department's prefix returns only this
/// department's keys server-side (the department id is the double map's first key, so its scoped prefix is
/// exactly `map_key_u32`'s output for `DepartmentExpenditures`), so this now costs O(this
/// department's expenditure count) per case, not O(every expenditure ever recorded on the
/// chain) — the thing that mattered given this runs once per TreasuryDispute case ruling, for
/// however many cases accumulate over the chain's lifetime, not once ever.
async fn fetch_expenditures_for_department(
    rpc: &RpcClient,
    department_id: u32,
) -> anyhow::Result<Vec<(u64, u128, [u8; 32])>> {
    let dept_prefix = map_key_u32("TreasuryLedger", "DepartmentExpenditures", department_id);
    let index_keys = rpc.get_keys_paged(&dept_prefix).await?;
    if index_keys.is_empty() {
        return Ok(vec![]);
    }
    // The double map's second key (`Blake2_128Concat`, like the first) also appends the raw
    // key at the tail, so `decode_u64_map_key` — written for a plain single-key map — still
    // extracts it correctly here.
    let mut indices: Vec<u64> =
        index_keys.iter().filter_map(|k| cases::decode_u64_map_key(k)).collect();
    indices.sort_unstable();

    let log_keys: Vec<String> =
        indices.iter().map(|idx| map_key_u64("TreasuryLedger", "ExpenditureLog", *idx)).collect();
    let values = rpc.query_storage_at(&log_keys).await?;

    let mut out = Vec::new();
    for (index, value_hex) in indices.iter().zip(values.iter()) {
        let Some(value_hex) = value_hex else { continue };
        let Ok(bytes) = hex::decode(value_hex.trim_start_matches("0x")) else { continue };
        let Ok((dept, amount, ipfs_hash)) = <(u32, u128, [u8; 32])>::decode(&mut &bytes[..]) else {
            continue;
        };
        // Defensive, not load-bearing: `index` came from the department-scoped secondary
        // index, so this should always match. A mismatch would mean the index and the primary
        // log drifted on-chain (shouldn't happen — both are written together in
        // `record_expenditure`), and skipping is safer than mis-attributing the entry.
        if dept != department_id {
            continue;
        }
        out.push((*index, amount, ipfs_hash));
    }
    Ok(out)
}

/// Same department-scoped-index approach as `fetch_expenditures_for_department`, over
/// `PalletAudit::AuditLog` via its own secondary index, `PalletAudit::DepartmentAuditEntries`
/// (populated in `on_expenditure`, the only place `AuditLog` entries are created).
async fn fetch_audit_entries_for_department(
    rpc: &RpcClient,
    _config: &Config,
    department_id: u32,
) -> anyhow::Result<Vec<AuditEntry>> {
    let dept_prefix = map_key_u32("PalletAudit", "DepartmentAuditEntries", department_id);
    let index_keys = rpc.get_keys_paged(&dept_prefix).await?;
    if index_keys.is_empty() {
        return Ok(vec![]);
    }
    let mut indices: Vec<u64> =
        index_keys.iter().filter_map(|k| cases::decode_u64_map_key(k)).collect();
    indices.sort_unstable();

    let log_keys: Vec<String> =
        indices.iter().map(|idx| map_key_u64("PalletAudit", "AuditLog", *idx)).collect();
    let values = rpc.query_storage_at(&log_keys).await?;

    let mut out = Vec::new();
    for value_hex in values.iter().flatten() {
        let Ok(bytes) = hex::decode(value_hex.trim_start_matches("0x")) else { continue };
        let Ok(entry) = AuditEntry::decode(&mut &bytes[..]) else { continue };
        if entry.dept_id == department_id {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Builds a full storage key for a `Blake2_128Concat`-hashed map with a `u32` key. Also used to
/// build the department-scoped *prefix* of a `(u32, u64)` double map — a double map's first-key
/// segment is encoded identically to a single-key map's, so this same function serves both.
fn map_key_u32(pallet: &str, item: &str, key: u32) -> String {
    let prefix = rpc::storage_prefix(pallet, item);
    let hash = rpc::blake2_128(&key.encode());
    format!("{prefix}{}{}", hex::encode(hash), hex::encode(key.encode()))
}

/// Same as `map_key_u32`, for a `Blake2_128Concat`-hashed map with a `u64` key
/// (`ExpenditureLog`, `AuditLog`).
fn map_key_u64(pallet: &str, item: &str, key: u64) -> String {
    let prefix = rpc::storage_prefix(pallet, item);
    let hash = rpc::blake2_128(&key.encode());
    format!("{prefix}{}{}", hex::encode(hash), hex::encode(key.encode()))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── should_finalize ──────────────────────────────────────────────────────────────────────

    const APPEAL_WINDOW: u32 = 50_400; // 7 days at this runtime's 12s block time.

    #[test]
    fn does_not_finalize_while_appeal_window_is_still_open() {
        let ruling_block = 1_000;
        let still_inside_window = ruling_block + APPEAL_WINDOW - 1;
        assert!(!should_finalize(
            &CaseStatus::AIRulingIssued,
            Some(ruling_block),
            still_inside_window,
            APPEAL_WINDOW
        ));
    }

    #[test]
    fn does_not_finalize_exactly_at_the_deadline_block() {
        // Mirrors the pallet's own strict `>` check in `finalize_ruling` — the deadline block
        // itself is still within the window, only blocks strictly after it are eligible.
        let ruling_block = 1_000;
        let deadline = ruling_block + APPEAL_WINDOW;
        assert!(!should_finalize(&CaseStatus::AIRulingIssued, Some(ruling_block), deadline, APPEAL_WINDOW));
    }

    #[test]
    fn finalizes_once_the_appeal_window_has_closed_unappealed() {
        let ruling_block = 1_000;
        let one_block_past_deadline = ruling_block + APPEAL_WINDOW + 1;
        assert!(should_finalize(
            &CaseStatus::AIRulingIssued,
            Some(ruling_block),
            one_block_past_deadline,
            APPEAL_WINDOW
        ));
    }

    #[test]
    fn never_finalizes_an_appealed_case_regardless_of_window_state() {
        let ruling_block = 1_000;
        // Well past the deadline — would finalize if status were still AIRulingIssued.
        let long_past_deadline = ruling_block + APPEAL_WINDOW + 100_000;
        for appealed_status in [
            CaseStatus::InJuryAppeal,
            CaseStatus::JurySeated,
            CaseStatus::FinalRuling,
            CaseStatus::Enforced,
        ] {
            assert!(
                !should_finalize(&appealed_status, Some(ruling_block), long_past_deadline, APPEAL_WINDOW),
                "status {appealed_status:?} must never be finalized via the no-appeal path"
            );
        }
    }

    #[test]
    fn never_finalizes_a_case_still_only_filed() {
        assert!(!should_finalize(&CaseStatus::Filed, None, 1_000_000, APPEAL_WINDOW));
    }

    #[test]
    fn does_not_finalize_when_ruling_block_is_missing() {
        // AIRulingIssued but no AIRulingBlock entry — shouldn't happen in practice (both are
        // written together by submit_ai_ruling), but must fail safe (not finalize) rather than
        // guess a deadline.
        assert!(!should_finalize(&CaseStatus::AIRulingIssued, None, 1_000_000, APPEAL_WINDOW));
    }

    // ── map_key_u32 / map_key_u64 — department-scoped secondary-index key math ─────────────
    //
    // `fetch_expenditures_for_department`/`fetch_audit_entries_for_department` now read
    // `TreasuryLedger::DepartmentExpenditures`/`PalletAudit::DepartmentAuditEntries` (double
    // maps) instead of scanning `ExpenditureLog`/`AuditLog` in full. These tests pin the pure
    // key-building math those functions depend on, without needing a live chain.

    #[test]
    fn map_key_u32_is_64_hex_chars_plus_0x_plus_the_encoded_key() {
        let key = map_key_u32("TreasuryLedger", "DepartmentExpenditures", 7u32);
        // "0x" + 32-byte prefix (64 hex chars) + 16-byte blake2_128 hash (32 hex chars) +
        // 4-byte raw u32 key (8 hex chars).
        assert_eq!(key.len(), 2 + 64 + 32 + 8);
        assert!(key.starts_with("0x"));
        // The raw (un-hashed) key is appended verbatim at the tail — Blake2_128Concat's
        // defining property, and what `decode_u64_map_key` relies on elsewhere.
        assert!(key.ends_with(&hex::encode(7u32.encode())));
    }

    #[test]
    fn map_key_u32_differs_for_different_departments_and_different_storage_items() {
        let dept_a = map_key_u32("TreasuryLedger", "DepartmentExpenditures", 1u32);
        let dept_b = map_key_u32("TreasuryLedger", "DepartmentExpenditures", 2u32);
        assert_ne!(dept_a, dept_b, "different departments must scope to different prefixes");

        let audit_dept_a = map_key_u32("PalletAudit", "DepartmentAuditEntries", 1u32);
        assert_ne!(
            dept_a, audit_dept_a,
            "the same department id under a different pallet/item must not collide"
        );
    }

    #[test]
    fn map_key_u64_round_trips_through_decode_u64_map_key() {
        // The full single-key (ExpenditureLog/AuditLog) storage key this crate builds to fetch
        // an entry by index must itself decode back to that same index via the same helper used
        // to extract indices out of the double map's keys — confirming both call sites agree on
        // the "raw key appended at the tail" shape.
        let index: u64 = 424_242;
        let key = map_key_u64("TreasuryLedger", "ExpenditureLog", index);
        assert_eq!(cases::decode_u64_map_key(&key), Some(index));
    }

    #[test]
    fn department_scoped_prefix_is_a_strict_prefix_of_the_full_double_map_key() {
        // Confirms the load-bearing assumption `fetch_expenditures_for_department` relies on:
        // `map_key_u32(pallet, "DepartmentExpenditures", dept)` (used as the
        // `state_getKeysPaged` prefix) is exactly the leading portion of what a full
        // `(dept, index)` double-map key looks like — i.e. scoping by department really does
        // scope to that department's entries and nothing else, because a double map's first-key
        // segment is encoded identically to a single-key map's.
        let dept: u32 = 3;
        let index: u64 = 99;
        let dept_prefix = map_key_u32("TreasuryLedger", "DepartmentExpenditures", dept);

        // Hand-build a full double-map key the same way pallet storage would encode it:
        // prefix(32B) ++ blake2_128(dept)(16B) ++ dept(4B) ++ blake2_128(index)(16B) ++ index(8B).
        let full_prefix = rpc::storage_prefix("TreasuryLedger", "DepartmentExpenditures");
        let dept_hash = rpc::blake2_128(&dept.encode());
        let index_hash = rpc::blake2_128(&index.encode());
        let full_key = format!(
            "{full_prefix}{}{}{}{}",
            hex::encode(dept_hash),
            hex::encode(dept.encode()),
            hex::encode(index_hash),
            hex::encode(index.encode()),
        );

        assert!(
            full_key.starts_with(&dept_prefix),
            "dept_prefix={dept_prefix} must be a strict prefix of full_key={full_key}"
        );
        assert_eq!(cases::decode_u64_map_key(&full_key), Some(index));
    }
}
