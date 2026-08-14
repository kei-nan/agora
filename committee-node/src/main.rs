//! Committee-node orchestration loop: connects to the chain over raw JSON-RPC, polls
//! `PendingOprfQueries` for work, and drives a genuine `t`-of-`n` threshold evaluation
//! (`docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`'s
//! "Option B") to completion — round 1 (`submit_oprf_round1`), then, once this committee's
//! qualifying set locks, round 2 (`submit_oprf_round2`). See README.md before running this
//! anywhere real: several pieces remain unverified against a live chain (no chain is running
//! in this environment), same caveat this component has always carried honestly.
//!
//! ## The two-round shape, in one paragraph
//!
//! Round 1 needs no coordination: compute this member's partial evaluation and FROST-style
//! nonce commitments from its own share, submit, done. Round 2 needs the *whole* locked
//! qualifying set's round-1 data to compute this member's binding factor, Lagrange
//! coefficient, and the shared challenge — all public-data math
//! (`oprf-committee-dev::threshold`, a native dependency of this crate, deliberately not
//! behind the wasm boundary `wasm_host.rs` uses for the secret-touching computation; see
//! `oprf-committee-dev/src/ffi.rs`'s module docs for why). This file has no reliable way to
//! know when the set has actually locked (`OprfThreshold` is a pallet constant, not a storage
//! value, and reading runtime metadata to get it is more machinery than this component
//! currently has) — so it just **attempts** round 2 once it can see its own round-1 entry, and
//! treats the pallet's own rejection (`OprfRound1NotLocked`) as "not ready, retry next poll",
//! the identical pattern round 1 already uses for `OprfRound1SetLocked`.

mod config;
mod extrinsic;
mod keystore;
mod rpc;
mod wasm_host;

use codec::{Decode, Encode};
use config::Config;
use oprf_committee_dev::babyjubjub::Point as ThresholdPoint;
use oprf_committee_dev::ffi as core_ffi;
use oprf_committee_dev::scalar as core_scalar;
use oprf_committee_dev::threshold;
use rpc::RpcClient;
use sp_core::crypto::{AccountId32, Ss58Codec};
use sp_core::Pair as _;
use std::collections::HashMap;
use std::time::Duration;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Mirrors `PendingOprfQueries`'s real value shape:
/// `query_id -> { submitter: AccountId, blinded_query: [u8; 64], posted_at: BlockNumber }`.
/// `#[derive(Decode)]` relies on field order matching the real storage struct's encoding —
/// SCALE encodes struct fields positionally, not by name.
#[derive(Decode)]
struct PendingQuery {
    submitter: AccountId32,
    blinded_query: [u8; 64],
    posted_at: u32,
}

/// Per-query progress this node has made toward its own committee slot's round-1/round-2
/// contribution. Held in memory only — lost on restart (a query already in `Round1Submitted`
/// state is simply re-attempted at round 1 again after a restart, which the pallet rejects as
/// `DuplicateResponse`; this node then has no seed to retry round 2 with for that query until
/// the query itself expires and a fresh one is posted). Same accepted, already-documented
/// limitation the original `already_responded: HashSet<u64>` carried — see
/// `docs/project/research/oprf-alternatives/09-cloud-security-hardening.md`'s Tier 3 item 30.
///
/// `Round1Submitted`'s `seed` is live nonce material for as long as it's held — see
/// `wasm_host.rs`'s module docs on why round 2 must replay the exact same seed round 1 used,
/// and why that makes this value as sensitive as the secret share until the query completes.
/// `ZeroizeOnDrop` wipes `seed` the moment an entry is dropped — whether that's an explicit
/// `HashMap::remove`, or (the actual path this code takes) an overwriting `insert` replacing
/// `Round1Submitted` with `Done` once round 2 succeeds.
#[derive(Zeroize, ZeroizeOnDrop)]
enum QueryProgress {
    Round1Submitted { seed: [u8; 32] },
    Done,
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
        member_index = config.member_index,
        pallet_index = config.pallet_index,
        allow_stub_submission = config.allow_stub_submission,
        "starting committee-node — two-round threshold protocol, see README.md's Option B section"
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

    let mut progress: HashMap<u64, QueryProgress> = HashMap::new();
    let mut interval = tokio::time::interval(Duration::from_secs(config.poll_interval_secs));

    loop {
        interval.tick().await;
        if let Err(e) = poll_once(&rpc, &config, &mut crypto_core, &oprf_secret_key, &seed, &mut progress).await {
            tracing::error!(error = %e, "poll cycle failed, will retry next interval");
        }
    }
}

/// Fetches and decodes `CommitteeMembers[slot]`. Returns an empty `Vec` (not an error) if the
/// key is genuinely absent — the same "nothing there yet" pattern the rest of this file uses
/// for `ValueQuery`-shaped state; a value that exists but fails to decode is still a real
/// error. Roster **order** matters beyond membership here: position `i` (0-based) in this
/// `Vec` is DKG party index `i + 1` — see `config.rs`'s `member_index` doc comment for why that
/// convention, not this function, owns the definition.
async fn fetch_committee_roster(rpc: &RpcClient, slot: u8) -> anyhow::Result<Vec<AccountId32>> {
    let prefix = rpc::storage_prefix("Identity", "CommitteeMembers");
    let key_suffix = rpc::blake2_128(&slot.encode());
    let key = format!("{prefix}{}{}", hex::encode(key_suffix), hex::encode(slot.encode()));

    match rpc.get_storage(&key).await? {
        Some(bytes) => Vec::<AccountId32>::decode(&mut &bytes[..])
            .map_err(|e| anyhow::anyhow!("could not decode CommitteeMembers[{slot}] value: {e}")),
        None => Ok(Vec::new()),
    }
}

/// Logs (non-fatally) whether this node's account appears in `CommitteeMembers[committee_slot]`
/// and, if so, whether it sits at the position `MEMBER_INDEX` claims — a wrong `MEMBER_INDEX`
/// is otherwise silent, per that field's own doc comment, so this is the one place a
/// misconfiguration can be caught early rather than surfacing only as every combined proof
/// this node contributes to quietly failing to verify.
async fn sanity_check_committee_roster(rpc: &RpcClient, config: &Config, account_id: &AccountId32) {
    match fetch_committee_roster(rpc, config.committee_slot).await {
        Ok(roster) if roster.is_empty() => tracing::warn!(
            "CommitteeMembers is empty/absent for slot {} — this node will not be able to \
             submit until governance registers it",
            config.committee_slot
        ),
        Ok(roster) => match roster.iter().position(|m| m == account_id) {
            Some(pos) => {
                let real_index = (pos + 1) as u64;
                tracing::info!(
                    real_index,
                    configured_index = config.member_index,
                    "confirmed: this account is on the CommitteeMembers roster for our slot"
                );
                if real_index != config.member_index {
                    tracing::error!(
                        real_index,
                        configured_index = config.member_index,
                        "MEMBER_INDEX does NOT match this account's real roster position — every \
                         round-2 response this node computes will use the wrong Lagrange \
                         coefficient and silently fail to combine. Fix MEMBER_INDEX before this \
                         node does real work."
                    );
                }
            }
            None => tracing::warn!(
                "this account is NOT on the CommitteeMembers roster for slot {} — round-1/round-2 \
                 submissions will be rejected as NotCommitteeMember",
                config.committee_slot
            ),
        },
        Err(e) => tracing::warn!(error = %e, "could not query CommitteeMembers (chain unreachable?)"),
    }
}

async fn poll_once(
    rpc: &RpcClient,
    config: &Config,
    crypto_core: &mut wasm_host::CryptoCore,
    oprf_secret_key: &[u8],
    chain_seed: &[u8; 32],
    progress: &mut HashMap<u64, QueryProgress>,
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
        if matches!(progress.get(&query_id), Some(QueryProgress::Done)) {
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
                tracing::warn!(query_id, error = ?e, "could not decode PendingOprfQueries value");
                continue;
            }
        };

        let already_in_progress = progress.get(&query_id).map(|p| match p {
            QueryProgress::Round1Submitted { seed } => *seed,
            QueryProgress::Done => unreachable!("filtered above"),
        });

        let result = match already_in_progress {
            None => {
                tracing::info!(
                    query_id,
                    posted_at = query.posted_at,
                    submitter = %query.submitter.to_ss58check(),
                    "found pending OPRF query — attempting round 1"
                );
                try_round1(rpc, config, crypto_core, oprf_secret_key, chain_seed, query_id, &query, progress).await
            }
            Some(round1_seed) => {
                try_round2(
                    rpc, config, crypto_core, oprf_secret_key, chain_seed, query_id, &query, round1_seed, progress,
                )
                .await
            }
        };
        if let Err(e) = result {
            tracing::warn!(query_id, error = %e, "this query's step failed this cycle, will retry next interval");
        }
    }

    Ok(())
}

/// Round 1: compute and submit this member's partial evaluation and nonce commitments. No
/// coordination needed — every willing member can do this independently and asynchronously.
/// On success, records the seed used (needed, unchanged, for round 2). A rejection (most
/// commonly `OprfRound1SetLocked`, if `OprfThreshold` other members already filled the set
/// first) is treated as "this query wasn't ours to answer" — logged by the caller, not
/// retried, since retrying the same query forever would just collect the same rejection.
async fn try_round1(
    rpc: &RpcClient,
    config: &Config,
    crypto_core: &mut wasm_host::CryptoCore,
    oprf_secret_key: &[u8],
    chain_seed: &[u8; 32],
    query_id: u64,
    query: &PendingQuery,
    progress: &mut HashMap<u64, QueryProgress>,
) -> anyhow::Result<()> {
    let mut round1_seed = [0u8; 32];
    {
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut round1_seed);
    }

    let commitment = crypto_core.round1(oprf_secret_key, query.blinded_query, round1_seed)?;
    if !commitment.is_real && !config.allow_stub_submission {
        tracing::warn!(
            query_id,
            "STUB round1 produced, but ALLOW_STUB_SUBMISSION is false — NOT submitting. \
             Set ALLOW_STUB_SUBMISSION=true only against a throwaway dev chain with no real citizens."
        );
        return Ok(());
    }

    let call = extrinsic::SubmitOprfRound1 {
        query_id,
        committee_slot: config.committee_slot,
        r_i: commitment.r_i,
        d_g: commitment.d_g,
        d_q: commitment.d_q,
        e_g: commitment.e_g,
        e_q: commitment.e_q,
    };
    let extrinsic_hex = extrinsic::build_signed_round1(
        rpc,
        chain_seed,
        config.pallet_index,
        extrinsic::ROUND1_CALL_INDEX,
        call,
    )
    .await?;

    // As with the pre-rewrite version of this loop: `submit_extrinsic` succeeding means the
    // node's transaction pool accepted it, not that on-chain execution will. If this
    // extrinsic is actually rejected on-chain (e.g. the set locked between our read and our
    // submission), the next poll cycle's round-2 attempt for this query will itself fail
    // (`NotInLockedSet`) and get logged — this degrades to a retry, not a silent wrong state.
    let tx_hash = rpc.submit_extrinsic(&extrinsic_hex).await?;
    tracing::info!(query_id, tx_hash, "submitted submit_oprf_round1");
    progress.insert(query_id, QueryProgress::Round1Submitted { seed: round1_seed });
    Ok(())
}

/// Round 2: fetch the current qualifying set, compute this member's binding factor, Lagrange
/// coefficient, and the shared challenge (all public-data math,
/// `oprf-committee-dev::threshold`, native — no secret material involved), then compute and
/// submit this member's response scalar. See this file's module docs for why this is
/// attempted unconditionally rather than gated on a known "is it locked" check.
#[allow(clippy::too_many_arguments)]
async fn try_round2(
    rpc: &RpcClient,
    config: &Config,
    crypto_core: &mut wasm_host::CryptoCore,
    oprf_secret_key: &[u8],
    chain_seed: &[u8; 32],
    query_id: u64,
    query: &PendingQuery,
    round1_seed: [u8; 32],
    progress: &mut HashMap<u64, QueryProgress>,
) -> anyhow::Result<()> {
    let roster = fetch_committee_roster(rpc, config.committee_slot).await?;
    let round1_set = rpc.get_oprf_round1_commitments(query_id, config.committee_slot).await?;

    // Translate the chain's AccountId-keyed round-1 records into the index-keyed shape
    // `oprf-committee-dev::threshold` operates on, via the roster-position convention
    // (`config.rs`'s `member_index` doc comment owns this definition).
    let mut commitments = Vec::with_capacity(round1_set.len());
    for c in &round1_set {
        let Some(pos) = roster.iter().position(|m| m == &c.member) else {
            tracing::warn!(
                query_id,
                member = %c.member.to_ss58check(),
                "a round-1 participant for this query is not on the current CommitteeMembers \
                 roster (removed after submitting?) — skipping this query for now"
            );
            return Ok(());
        };
        commitments.push(threshold::Round1Commitment {
            index: (pos + 1) as u64,
            r_i: point_from_bytes(&c.r_i),
            d_g: point_from_bytes(&c.d_g),
            d_q: point_from_bytes(&c.d_q),
            e_g: point_from_bytes(&c.e_g),
            e_q: point_from_bytes(&c.e_q),
        });
    }

    let my_index = config.member_index;
    if !commitments.iter().any(|c| c.index == my_index) {
        // Our own round-1 submission isn't visible in this read yet (RPC lag / not yet
        // finalized) — nothing to do this cycle, not an error.
        return Ok(());
    }

    let q = point_from_bytes(&query.blinded_query);
    let indices: Vec<u64> = commitments.iter().map(|c| c.index).collect();
    let rhos: Vec<(u64, ark_bn254::Fr)> = indices
        .iter()
        .map(|&i| (i, threshold::binding_factor(i, &commitments, &q)))
        .collect();
    let (d_g, d_q) = threshold::aggregate_nonce_commitments(&commitments, &rhos);
    let r = threshold::combine_evaluations(&commitments);
    let ds_dlog = core_ffi::field_from_be(&wasm_host::DS_DLOG_BE);
    let group_pk = point_from_bytes(&config.group_pubkey);
    let e = threshold::combined_challenge(&group_pk, &q, &r, &d_g, &d_q, ds_dlog);

    let my_rho = rhos
        .iter()
        .find(|(i, _)| *i == my_index)
        .expect("checked above that our index is present")
        .1;
    let my_lambda = threshold::lagrange_coefficient(my_index, &indices);

    let response = crypto_core.round2_response(
        oprf_secret_key,
        round1_seed,
        core_ffi::field_to_be(&my_rho),
        core_ffi::field_to_be(&core_scalar::to_field(&my_lambda)),
        core_ffi::field_to_be(&e),
    )?;
    if !response.is_real && !config.allow_stub_submission {
        tracing::warn!(
            query_id,
            "STUB round2 produced, but ALLOW_STUB_SUBMISSION is false — NOT submitting."
        );
        return Ok(());
    }

    let call = extrinsic::SubmitOprfRound2 { query_id, committee_slot: config.committee_slot, z_i: response.z_i };
    let extrinsic_hex = extrinsic::build_signed_round2(
        rpc,
        chain_seed,
        config.pallet_index,
        extrinsic::ROUND2_CALL_INDEX,
        call,
    )
    .await?;

    match rpc.submit_extrinsic(&extrinsic_hex).await {
        Ok(tx_hash) => {
            tracing::info!(query_id, tx_hash, "submitted submit_oprf_round2");
            progress.insert(query_id, QueryProgress::Done);
        }
        Err(e) => {
            // Most likely OprfRound1NotLocked (the set we can see isn't full yet — a fresher
            // read next cycle may find it locked) or NotInLockedSet (our round-1 landed after
            // the set had already locked without us) — either way, retry, don't crash the loop.
            tracing::warn!(query_id, error = %e, "submit_oprf_round2 rejected, will retry next cycle");
        }
    }
    Ok(())
}

/// Decodes a 64-byte `x(32) || y(32)` big-endian point into `oprf-committee-dev`'s native
/// `Point` type, for feeding into `threshold`'s public-data aggregation functions. Does no
/// validation (on-curve, subgroup) of its own — every input here is either this node's own
/// freshly-computed output or already-accepted on-chain data, and `threshold`'s functions are
/// pure arithmetic with no validation of their own either; a malformed point here just
/// produces a combined proof that fails to verify downstream, the same failure mode as any
/// other input error in this protocol.
fn point_from_bytes(b: &[u8; 64]) -> ThresholdPoint {
    ThresholdPoint::new(core_ffi::field_from_be(&b[0..32]), core_ffi::field_from_be(&b[32..64]))
}

/// Extracts the SCALE-encoded `u64` map key from a full storage key, assuming
/// `Blake2_128Concat` (32-byte twox128 pallet+item prefix, then 16-byte blake2_128 hash, then
/// the raw un-hashed key) — the same hasher every numeric-keyed map in
/// `pallets/pallet-identity/src/lib.rs` uses (e.g. `CitizenIndex`, `PendingOprfQueries`).
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

    /// `point_from_bytes` round-trips the BabyJubJub generator — cheap sanity check that the
    /// x/y split lands at the offsets `threshold`'s functions expect (the same fixture
    /// `wasm_host.rs`'s own tests use, so a mismatch here would mean the two files disagree
    /// about the 64-byte point convention they're both supposed to share).
    #[test]
    fn point_from_bytes_matches_the_babyjubjub_generator() {
        let generator = oprf_committee_dev::babyjubjub::Point::generator();
        let mut bytes = [0u8; 64];
        bytes[0..32].copy_from_slice(&core_ffi::field_to_be(&generator.x));
        bytes[32..64].copy_from_slice(&core_ffi::field_to_be(&generator.y));
        let round_tripped = point_from_bytes(&bytes);
        assert_eq!(round_tripped.x, generator.x);
        assert_eq!(round_tripped.y, generator.y);
    }
}
