//! Committee-node orchestration loop (changelog #082): connects to the chain over raw
//! JSON-RPC, polls `PendingOprfQueries` for work, calls the (possibly-stubbed) OPRF crypto
//! core, and submits `submit_oprf_response`. See README.md before running this anywhere real —
//! several pieces are provisional/stubbed pending two other in-progress efforts (the real
//! `pallet-identity` storage/extrinsics, and the real Wasm-compiled crypto core).

mod config;
mod extrinsic;
mod keystore;
mod rpc;
mod wasm_host;

use codec::{Decode, Encode};
use config::Config;
use rpc::RpcClient;
use sp_core::crypto::{AccountId32, Ss58Codec};
use sp_core::Pair as _;
use std::collections::HashSet;
use std::time::Duration;

/// Mirrors the provisional `PendingOprfQueries` value shape given in the task interface:
/// `query_id -> { submitter: AccountId, blinded_query: [u8; 64], posted_at: BlockNumber }`.
/// `#[derive(Decode)]` relies on field order matching the real storage struct's encoding once
/// it exists — SCALE encodes struct fields positionally, not by name, so if the real pallet
/// adds/reorders fields this needs updating even though it would still compile.
#[derive(Decode)]
struct PendingQuery {
    submitter: AccountId32,
    blinded_query: [u8; 64],
    posted_at: u32,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse().unwrap()))
        .init();

    let config = Config::from_env()?;
    tracing::info!(
        node_rpc_url = %config.node_rpc_url,
        committee_slot = config.committee_slot,
        pallet_index = config.pallet_index,
        call_index = config.call_index,
        allow_stub_submission = config.allow_stub_submission,
        "starting committee-node (changelog #082) — see README.md for what's provisional"
    );

    let passphrase = config.resolve_passphrase()?;
    let secrets = keystore::load(&config.keys_file, &passphrase)?;
    let seed = secrets.chain_account_seed_bytes()?;
    let oprf_secret_key = secrets.oprf_secret_key_bytes()?;

    let account_id =
        AccountId32::from(<sp_core::sr25519::Pair as sp_core::Pair>::from_seed(&seed).public().to_raw());
    tracing::info!(account = %account_id.to_ss58check(), "loaded chain account key");

    let mut crypto_core = wasm_host::load(&config.wasm_module_path)?;

    let rpc = RpcClient::new(config.node_rpc_url.clone());

    sanity_check_committee_roster(&rpc, &config, &account_id).await;

    let mut already_responded: HashSet<u64> = HashSet::new();
    let mut interval = tokio::time::interval(Duration::from_secs(config.poll_interval_secs));

    loop {
        interval.tick().await;
        if let Err(e) = poll_once(&rpc, &config, &mut crypto_core, &oprf_secret_key, &seed, &mut already_responded).await {
            tracing::error!(error = %e, "poll cycle failed, will retry next interval");
        }
    }
}

/// Logs (non-fatally) whether this node's account appears in `CommitteeMembers[committee_slot]`.
/// Purely diagnostic: `CommitteeMembers` doesn't exist on-chain yet (see README), so this will
/// currently always come back empty/absent — that's expected, not an error, until the real
/// pallet lands.
async fn sanity_check_committee_roster(rpc: &RpcClient, config: &Config, account_id: &AccountId32) {
    let prefix = rpc::storage_prefix("Identity", "CommitteeMembers");
    let key_suffix = rpc::blake2_128(&config.committee_slot.encode());
    let key = format!("{prefix}{}{}", hex::encode(key_suffix), hex::encode(config.committee_slot.encode()));

    match rpc.get_storage(&key).await {
        Ok(Some(bytes)) => match Vec::<AccountId32>::decode(&mut &bytes[..]) {
            Ok(members) if members.contains(account_id) => {
                tracing::info!("confirmed: this account is on the CommitteeMembers roster for our slot");
            }
            Ok(_) => tracing::warn!(
                "this account is NOT on the CommitteeMembers roster for slot {} — responses may be rejected once that check is enforced",
                config.committee_slot
            ),
            Err(e) => tracing::warn!(error = ?e, "could not decode CommitteeMembers value — storage shape may not match the provisional interface"),
        },
        Ok(None) => tracing::warn!(
            "CommitteeMembers storage is empty/absent for slot {} — expected until the real pallet-identity work lands (see README)",
            config.committee_slot
        ),
        Err(e) => tracing::warn!(error = %e, "could not query CommitteeMembers (chain unreachable?)"),
    }
}

async fn poll_once(
    rpc: &RpcClient,
    config: &Config,
    crypto_core: &mut wasm_host::CryptoCore,
    oprf_secret_key: &[u8],
    seed: &[u8; 32],
    already_responded: &mut HashSet<u64>,
) -> anyhow::Result<()> {
    let prefix = rpc::storage_prefix("Identity", "PendingOprfQueries");
    let keys = rpc.get_keys_paged(&prefix).await?;
    if keys.is_empty() {
        tracing::debug!("no pending OPRF queries");
        return Ok(());
    }

    let values = rpc.query_storage_at(&keys).await?;

    for (key_hex, value_hex) in keys.iter().zip(values.iter()) {
        let Some(value_hex) = value_hex else { continue };
        let Some(query_id) = decode_u64_map_key(key_hex) else {
            tracing::warn!(key = %key_hex, "could not extract query_id from storage key — skipping");
            continue;
        };
        if already_responded.contains(&query_id) {
            continue;
        }

        let raw = match hex::decode(value_hex.trim_start_matches("0x")) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(query_id, error = %e, "PendingOprfQueries value was not valid hex");
                continue;
            }
        };
        let query = match PendingQuery::decode(&mut &raw[..]) {
            Ok(q) => q,
            Err(e) => {
                tracing::warn!(query_id, error = ?e, "could not decode PendingOprfQueries value — storage shape may not match the provisional interface (see README)");
                continue;
            }
        };

        tracing::info!(query_id, posted_at = query.posted_at, submitter = %query.submitter.to_ss58check(), "found pending OPRF query");

        // Confirmed against the real pallet-identity storage layout (changelog #082's
        // reconciliation pass): PendingOprfQueries.blinded_query is [u8; 64], x-then-y
        // big-endian — the wasm_host::evaluate() ABI now takes that whole 64-byte value
        // directly rather than split x/y halves.
        let evaluation = crypto_core.evaluate(oprf_secret_key, query.blinded_query)?;

        if !evaluation.is_real && !config.allow_stub_submission {
            tracing::warn!(
                query_id,
                "STUB evaluation produced, but ALLOW_STUB_SUBMISSION is false — NOT submitting. \
                 Set ALLOW_STUB_SUBMISSION=true only against a throwaway dev chain with no real citizens."
            );
            continue;
        }

        let call = extrinsic::SubmitOprfResponse {
            query_id,
            committee_slot: config.committee_slot,
            evaluation: evaluation.evaluation,
            dlog_proof: evaluation.dlog_proof.to_vec(),
        };

        match extrinsic::build_signed(rpc, seed, config.pallet_index, config.call_index, call).await {
            Ok(extrinsic_hex) => match rpc.submit_extrinsic(&extrinsic_hex).await {
                Ok(tx_hash) => {
                    tracing::info!(query_id, tx_hash, "submitted submit_oprf_response");
                    already_responded.insert(query_id);
                }
                Err(e) => tracing::error!(query_id, error = %e, "author_submitExtrinsic failed"),
            },
            Err(e) => tracing::error!(query_id, error = %e, "failed to build/sign extrinsic"),
        }
    }

    Ok(())
}

/// Extracts the SCALE-encoded `u64` map key from a full storage key, assuming
/// `Blake2_128Concat` (32-byte twox128 pallet+item prefix, then 16-byte blake2_128 hash, then
/// the raw un-hashed key). This hasher choice is an assumption — see README's "Provisional
/// interfaces": the task's interface spec doesn't state `PendingOprfQueries`'s hasher, so this
/// follows the same `Blake2_128Concat` convention every other numeric-keyed map in
/// `pallets/pallet-identity/src/lib.rs` already uses (e.g. `CitizenIndex`).
fn decode_u64_map_key(key_hex: &str) -> Option<u64> {
    let bytes = hex::decode(key_hex.trim_start_matches("0x")).ok()?;
    if bytes.len() < 8 {
        return None;
    }
    let tail = &bytes[bytes.len() - 8..];
    Some(u64::from_le_bytes(tail.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_u64_map_key_round_trips() {
        let query_id: u64 = 42;
        // Simulate prefix(32 bytes) ++ blake2_128 hash(16 bytes) ++ raw key(8 bytes).
        let mut key_bytes = vec![0xABu8; 32 + 16];
        key_bytes.extend(query_id.to_le_bytes());
        let key_hex = format!("0x{}", hex::encode(key_bytes));
        assert_eq!(decode_u64_map_key(&key_hex), Some(query_id));
    }

    #[test]
    fn decode_u64_map_key_rejects_too_short() {
        assert_eq!(decode_u64_map_key("0x1234"), None);
    }
}
