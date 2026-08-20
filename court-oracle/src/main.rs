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

mod cases;
mod claude;
mod config;
mod context;
mod extrinsic;
mod ipfs;
mod keystore;
mod rpc;

use cases::{AuditEntry, CaseRecord, CaseStatus, CaseSubject, LawRecord, Verdict};
use config::Config;
use context::SubjectContext;
use rpc::RpcClient;

use anyhow::Context as _;
use codec::{Decode, Encode};
use serde::Serialize;
use sp_core::crypto::{AccountId32, Ss58Codec};
use sp_core::Pair as _;
use std::collections::HashSet;
use std::time::Duration;

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

    let mut already_processed: HashSet<u32> = HashSet::new();
    // Tracks case_ids whose `finalize_ruling` extrinsic has already been submitted this run, so
    // a case doesn't get a second finalize attempt while the first is still pending inclusion
    // (status only leaves `AIRulingIssued` once the extrinsic actually lands) — same rationale
    // as `already_processed` above for `submit_ai_ruling`.
    let mut finalize_processed: HashSet<u32> = HashSet::new();
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
            &mut already_processed,
            &mut finalize_processed,
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
    already_processed: &mut HashSet<u32>,
    finalize_processed: &mut HashSet<u32>,
) -> anyhow::Result<()> {
    let model_version = fetch_current_ai_model_version(rpc).await?;
    if model_version == 0 {
        tracing::warn!("CurrentAIModelVersion is 0 (no AI model has ever been governance-approved) — submit_ai_ruling would reject every call with NoApprovedAIModel; skipping this poll cycle entirely rather than ruling on cases we can't submit for");
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
                if already_processed.contains(&case_id) {
                    continue;
                }
                tracing::info!(case_id, filer = %filer.to_ss58check(), subject = ?subject, "found filed case, ruling");

                match rule_on_case(rpc, config, claude_client, ipfs_client, &filer, case_id, &subject).await {
                    Ok((ruling_hash, verdict)) => {
                        if config.dry_run {
                            tracing::info!(case_id, ruling_hash = %hex::encode(ruling_hash), verdict = ?verdict, "DRY_RUN set — not submitting submit_ai_ruling");
                            already_processed.insert(case_id);
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
                                    already_processed.insert(case_id);
                                }
                                Err(e) => tracing::error!(case_id, error = %e, "author_submitExtrinsic failed"),
                            },
                            Err(e) => tracing::error!(case_id, error = %e, "failed to build/sign extrinsic"),
                        }
                    }
                    Err(e) => {
                        tracing::error!(case_id, error = %e, "failed to produce a ruling for this case — leaving it Filed for a later poll cycle");
                    }
                }
            }
            CaseStatus::AIRulingIssued => {
                if finalize_processed.contains(&case_id) {
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
                    finalize_processed.insert(case_id);
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
                            finalize_processed.insert(case_id);
                        }
                        Err(e) => tracing::error!(case_id, error = %e, "author_submitExtrinsic failed for finalize_ruling"),
                    },
                    Err(e) => tracing::error!(case_id, error = %e, "failed to build/sign finalize_ruling extrinsic"),
                }
            }
            _ => continue,
        }
    }

    Ok(())
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

/// Scans the entire `TreasuryLedger::ExpenditureLog` map and returns entries tagged with
/// `department_id`. **This does not scale** — `ExpenditureLog` is keyed by a monotonic
/// expenditure counter, not by department, so there is no way to query "just this department's
/// entries" without either scanning the whole log (what this does) or the chain adding a
/// department-indexed secondary map. Documented plainly as a real limitation, not silently
/// left to degrade — see README.md.
async fn fetch_expenditures_for_department(
    rpc: &RpcClient,
    department_id: u32,
) -> anyhow::Result<Vec<(u64, u128, [u8; 32])>> {
    let prefix = rpc::storage_prefix("TreasuryLedger", "ExpenditureLog");
    let keys = rpc.get_keys_paged(&prefix).await?;
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = rpc.query_storage_at(&keys).await?;
    let mut out = Vec::new();
    for (key_hex, value_hex) in keys.iter().zip(values.iter()) {
        let Some(value_hex) = value_hex else { continue };
        let Some(index) = cases::decode_u64_map_key(key_hex) else { continue };
        let Ok(bytes) = hex::decode(value_hex.trim_start_matches("0x")) else { continue };
        let Ok((dept, amount, ipfs_hash)) = <(u32, u128, [u8; 32])>::decode(&mut &bytes[..]) else {
            continue;
        };
        if dept == department_id {
            out.push((index, amount, ipfs_hash));
        }
    }
    out.sort_by_key(|(index, _, _)| *index);
    Ok(out)
}

/// Same scan-and-filter approach and the same scaling caveat as
/// `fetch_expenditures_for_department`, over `PalletAudit::AuditLog`.
async fn fetch_audit_entries_for_department(
    rpc: &RpcClient,
    _config: &Config,
    department_id: u32,
) -> anyhow::Result<Vec<AuditEntry>> {
    let prefix = rpc::storage_prefix("PalletAudit", "AuditLog");
    let keys = rpc.get_keys_paged(&prefix).await?;
    if keys.is_empty() {
        return Ok(vec![]);
    }
    let values = rpc.query_storage_at(&keys).await?;
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

/// Builds a full storage key for a `Blake2_128Concat`-hashed map with a `u32` key.
fn map_key_u32(pallet: &str, item: &str, key: u32) -> String {
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
}
