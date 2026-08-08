//! Court-oracle orchestration loop (see README.md): polls `Courts::Cases` for `CaseStatus::Filed`
//! entries, builds a case-appropriate context from other on-chain storage, asks Claude for a
//! Level-0 AI ruling, publishes the full reasoning document to IPFS, and submits
//! `submit_ai_ruling` signed by the configured oracle account.
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

use cases::{AuditEntry, CaseRecord, CaseStatus, CaseSubject, LawRecord};
use config::Config;
use context::SubjectContext;
use rpc::RpcClient;

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
        "this account must be registered on-chain via `Courts::set_oracle_account` (root-only) \
         before submit_ai_ruling calls signed by it will be accepted — this service does not do \
         that itself, see README.md"
    );

    let rpc = RpcClient::new(config.node_rpc_url.clone());
    let claude_client = claude::ClaudeClient::new(claude_api_key, config.claude_model.clone());
    let ipfs_client = ipfs::IpfsClient::new(config.ipfs_api_url.clone());

    let mut already_processed: HashSet<u32> = HashSet::new();
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
) -> anyhow::Result<()> {
    let model_version = fetch_current_ai_model_version(rpc).await?;
    if model_version == 0 {
        tracing::warn!("CurrentAIModelVersion is 0 (no AI model has ever been governance-approved) — submit_ai_ruling would reject every call with NoApprovedAIModel; skipping this poll cycle entirely rather than ruling on cases we can't submit for");
        return Ok(());
    }

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
        if already_processed.contains(&case_id) {
            continue;
        }
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
        if status != CaseStatus::Filed {
            continue;
        }

        tracing::info!(case_id, filer = %filer.to_ss58check(), subject = ?subject, "found filed case, ruling");

        match rule_on_case(rpc, config, claude_client, ipfs_client, &filer, case_id, &subject).await {
            Ok(ruling_hash) => {
                if config.dry_run {
                    tracing::info!(case_id, ruling_hash = %hex::encode(ruling_hash), "DRY_RUN set — not submitting submit_ai_ruling");
                    already_processed.insert(case_id);
                    continue;
                }
                let call = extrinsic::SubmitAiRuling { case_id, ruling_hash, model_version };
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

    Ok(())
}

/// Builds case context, asks Claude, publishes the reasoning document to IPFS, and returns the
/// `ruling_hash` to submit on-chain. Does not itself submit the extrinsic — that's `poll_once`'s
/// job, so this function stays testable-by-construction (every side effect is delegated to an
/// injected client) even though it isn't unit-tested directly here (it does real I/O throughout
/// — see this crate's README on what actually has test coverage).
async fn rule_on_case(
    rpc: &RpcClient,
    config: &Config,
    claude_client: &claude::ClaudeClient,
    ipfs_client: &ipfs::IpfsClient,
    filer: &AccountId32,
    case_id: u32,
    subject: &CaseSubject,
) -> anyhow::Result<[u8; 32]> {
    let subject_context = build_subject_context(rpc, config, subject).await?;
    let case_context = context::render_case_context(case_id, &filer.to_ss58check(), &subject_context);

    let ruling = claude_client.rule(case_id, &case_context).await?;

    let verdict_str = match ruling.verdict {
        claude::Verdict::Upheld => "Upheld",
        claude::Verdict::Overturned => "Overturned",
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

    Ok(ruling_hash)
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
/// `None` on any failure (unreachable gateway, non-200, non-UTF8 body) rather than propagating
/// an error — a missing law text should not abort ruling on the case, just leave that context
/// honestly marked unavailable (see `context::render_case_context`).
async fn fetch_ipfs_gateway_content(content_hash: &[u8; 32]) -> Option<String> {
    let cid = ipfs::digest_to_cidv0(content_hash);
    let url = format!("https://ipfs.io/ipfs/{cid}");
    let client = reqwest::Client::builder().timeout(Duration::from_secs(30)).build().ok()?;
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.text().await.ok()
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
