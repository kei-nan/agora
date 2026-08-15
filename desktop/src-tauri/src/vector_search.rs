//! Generic, provider-agnostic vector-search client interface, plus one concrete adapter for
//! Redis/RediSearch.
//!
//! Scope note (see CLAUDE.md / the task that created this file): this module is step one of a
//! future "semantic search over delegate profiles" feature. Everything else that feature needs —
//! which embedding-generation provider to use, the ingestion pipeline that watches the chain for
//! delegate registrations and writes embeddings in here, and any UI — is deliberately out of
//! scope. Nothing in `lib.rs`'s `invoke_handler` calls this module yet; it is real, compiled,
//! tested code with no caller yet, the same way `registerCitizen`/`reverifyCitizen` existed on
//! the mobile side before any UI called them.
//!
//! ## Transport: RESP, not HTTP
//!
//! RediSearch's vector-search commands (`FT.CREATE`, `FT.SEARCH ... KNN ...`) are issued over
//! Redis's native RESP wire protocol, not a REST/HTTP API. Standard Redis Cloud / self-hosted
//! Redis Stack only speaks RESP for these commands — there is no plain-HTTP surface for them.
//! (Upstash's *Redis* product does expose a REST-over-HTTP command endpoint as a convenience for
//! serverless/edge runtimes that can't hold a raw TCP connection, and Upstash also sells a
//! separate, HTTP-native "Vector" product — but neither is "RediSearch", and this codebase has no
//! reason to assume the eventual vector-search vendor is Upstash specifically. This adapter
//! targets standard RediSearch over RESP.) This is why `redis` (the `redis-rs` crate) had to be
//! added to `Cargo.toml` as a new dependency — nothing in this codebase's existing `reqwest`-based
//! HTTP code (`commands::chain::fetch_ipfs_content`, `commands::agent::agent_ask`) can speak RESP.
//!
//! `FT.CREATE`/`FT.SEARCH`/the `VECTOR` schema field type are RediSearch-module commands, not part
//! of core Redis, so they aren't in `redis-rs`'s typed `Commands`/`AsyncCommands` trait surface.
//! They're issued through the crate's raw `redis::cmd("FT.SEARCH")...` command-builder API instead
//! (see `build_knn_search_args` / `RedisCommandArgs::into_cmd` below).
//!
//! ## Operational precondition: the RediSearch index is not self-provisioned
//!
//! This module does not silently assume infrastructure that isn't there. `upsert`/`query`/
//! `delete` all require the target RediSearch index to already exist with a `VECTOR` field named
//! [`EMBEDDING_FIELD`]. [`RedisVectorSearchClient::ensure_index`] is provided as an explicit,
//! separate step an operator/caller can run once (analogous to how `court-oracle/README.md`
//! documents its own operational preconditions rather than having the oracle service provision
//! its own chain state) — it is never called implicitly by `upsert`/`query`/`delete`.

// `mod vector_search;` in lib.rs is private, and nothing else in the crate calls into this
// module yet (deliberately — see the scope note above), so rustc's dead_code lint would
// otherwise flag every pub item here as unused. That's expected for a module built ahead of its
// caller, not a bug; silencing the lint here keeps `cargo build` output honest about *real*
// issues elsewhere.
#![allow(dead_code)]

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// One match returned by a vector-similarity query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub id: String,
    pub score: f32,
    pub metadata: serde_json::Value,
}

/// A minimal, provider-agnostic vector-search backend: store an embedding with an id and
/// arbitrary metadata, query by nearest neighbors, delete by id. No Redis-specific (or any other
/// vendor-specific) concept appears in this trait's signature — swapping the backend a future
/// caller uses is meant to be "write a new adapter", never "touch the calling code".
///
/// ## Why `#[async_trait]` instead of native async-fn-in-trait
///
/// This toolchain (Rust 1.96 per CLAUDE.md) supports native `async fn` directly in traits, and
/// for a trait only ever used behind compile-time generics (`fn f<C: VectorSearchClient>(c: &C)`)
/// that would be enough — no new dependency needed. But native async-fn-in-trait is *not*
/// `dyn`-compatible (the returned `impl Future` has no fixed size), so a trait defined that way
/// cannot back a `Box<dyn VectorSearchClient>` / `Arc<dyn VectorSearchClient>` without extra
/// machinery.
///
/// This module deliberately chooses real `dyn` polymorphism, because "swapping vendor means
/// writing a new adapter, not touching calling code" is a *runtime construction-site* property,
/// not just a type-parameter property: the eventual caller (the ingestion pipeline, or a
/// Tauri-managed app state) is expected to construct exactly one concrete adapter at startup
/// (picked by config/env var) and hand every downstream function a `dyn VectorSearchClient` —
/// generics would instead push a concrete type parameter through every function signature along
/// the way, which *is* touching calling code the moment there's more than one call site. Rather
/// than fight native-async-fn's dyn-incompatibility by hand (manual `Box<dyn Future>` boxing,
/// `Pin`, lifetime plumbing), this uses the well-known `async-trait` crate, which does exactly
/// that boxing via a macro and is the standard solution for this exact situation.
#[async_trait]
pub trait VectorSearchClient: Send + Sync {
    /// Inserts or overwrites the embedding + metadata stored under `id`.
    async fn upsert(&self, id: &str, embedding: &[f32], metadata: serde_json::Value) -> Result<(), String>;

    /// Returns the `top_k` nearest neighbors to `embedding`, best match first.
    async fn query(&self, embedding: &[f32], top_k: usize) -> Result<Vec<SearchResult>, String>;

    /// Removes the entry stored under `id`, if any.
    async fn delete(&self, id: &str) -> Result<(), String>;
}

// ── Redis / RediSearch adapter ────────────────────────────────────────────────

/// Environment variable holding the Redis connection URL (e.g.
/// `redis://default:<password>@<host>:<port>`), read fresh on every connection attempt —
/// mirrors `commands::agent::agent_ask`'s `CLAUDE_API_KEY` convention exactly: no credential
/// caching in this struct, no Tauri config entry, no UI input.
const REDIS_VECTOR_SEARCH_URL_VAR: &str = "REDIS_VECTOR_SEARCH_URL";

/// RediSearch hash field holding the raw FLOAT32 vector bytes.
const EMBEDDING_FIELD: &str = "embedding";
/// RediSearch hash field holding the JSON-serialized metadata blob.
const METADATA_FIELD: &str = "metadata";
/// RediSearch hash field echoing the logical id back (also encoded in the key itself, but kept
/// as its own field so `FT.SEARCH ... RETURN` can hand it back without a second round trip).
const ID_FIELD: &str = "id";
/// Alias `FT.SEARCH`'s KNN clause assigns to the computed distance, used both when building the
/// query and when parsing its reply.
const SCORE_ALIAS: &str = "vector_score";

/// A [`VectorSearchClient`] backed by a RediSearch index over RESP (see module docs for why RESP
/// rather than HTTP). Does **not** provision the index itself — see module docs' "operational
/// precondition" section and [`ensure_index`](RedisVectorSearchClient::ensure_index).
pub struct RedisVectorSearchClient {
    /// Name of the RediSearch index (and the `{index_name}:` key prefix its documents live
    /// under). Not a credential — just a logical namespace — so it's a plain constructor
    /// argument rather than something read from the environment.
    index_name: String,
}

impl RedisVectorSearchClient {
    pub fn new(index_name: impl Into<String>) -> Self {
        Self { index_name: index_name.into() }
    }

    /// Opens a fresh connection using `REDIS_VECTOR_SEARCH_URL`, read from the environment on
    /// every call (never cached on `self`) — matching `agent.rs`'s `CLAUDE_API_KEY` handling.
    async fn connect(&self) -> Result<redis::aio::MultiplexedConnection, String> {
        let url = std::env::var(REDIS_VECTOR_SEARCH_URL_VAR).map_err(|_| {
            format!(
                "{REDIS_VECTOR_SEARCH_URL_VAR} not configured. Set it in your environment to enable vector search."
            )
        })?;
        let client = redis::Client::open(url)
            .map_err(|e| format!("invalid redis connection string: {e}"))?;
        client
            .get_multiplexed_async_connection()
            .await
            .map_err(|e| format!("redis connection failed: {e}"))
    }

    /// Explicit, opt-in index-provisioning step (`FT.CREATE ... SCHEMA ... VECTOR ...`). Not
    /// called implicitly by `upsert`/`query`/`delete` — see module docs. Safe to call against an
    /// index that already exists with the same shape (RediSearch's "Index already exists" error
    /// is treated as success); a *different*-shaped existing index will surface as a real error
    /// from `FT.CREATE` (e.g. a dimension mismatch), which this deliberately does not paper over.
    pub async fn ensure_index(&self, embedding_dim: usize) -> Result<(), String> {
        let mut conn = self.connect().await?;
        let cmd = build_create_index_args(&self.index_name, embedding_dim).into_cmd();
        let result: redis::RedisResult<redis::Value> = cmd.query_async(&mut conn).await;
        match result {
            Ok(_) => Ok(()),
            Err(e) if e.to_string().contains("Index already exists") => Ok(()),
            Err(e) => Err(format!("failed to create RediSearch index {:?}: {e}", self.index_name)),
        }
    }
}

#[async_trait]
impl VectorSearchClient for RedisVectorSearchClient {
    async fn upsert(&self, id: &str, embedding: &[f32], metadata: serde_json::Value) -> Result<(), String> {
        let mut conn = self.connect().await?;
        let cmd = build_upsert_args(&self.index_name, id, embedding, &metadata)?.into_cmd();
        cmd.query_async::<()>(&mut conn)
            .await
            .map_err(|e| format!("redis upsert failed: {e}"))
    }

    async fn query(&self, embedding: &[f32], top_k: usize) -> Result<Vec<SearchResult>, String> {
        let mut conn = self.connect().await?;
        let cmd = build_knn_search_args(&self.index_name, embedding, top_k).into_cmd();
        let reply: redis::Value = cmd
            .query_async(&mut conn)
            .await
            .map_err(|e| format!("redis query failed: {e}"))?;
        parse_search_reply(&reply)
    }

    async fn delete(&self, id: &str) -> Result<(), String> {
        let mut conn = self.connect().await?;
        let cmd = build_delete_args(&self.index_name, id).into_cmd();
        cmd.query_async::<()>(&mut conn)
            .await
            .map_err(|e| format!("redis delete failed: {e}"))
    }
}

// ── Pure command-building / reply-parsing logic (independently testable) ─────────────────────
//
// Everything below builds or parses plain data (`Vec<Vec<u8>>` argument lists, `redis::Value`
// replies) with no I/O, so it's unit-tested directly without a live Redis/RediSearch instance.
// `RedisCommandArgs` exists specifically to keep that logic decoupled from `redis::Cmd`'s
// internal representation — tests assert on the argument bytes this module itself produced,
// not on how the `redis` crate happens to store them.

/// A command name plus its ordered argument bytes — a thin, crate-independent stand-in for
/// `redis::Cmd` used so the *building* of a command is testable as plain data.
struct RedisCommandArgs {
    name: &'static str,
    args: Vec<Vec<u8>>,
}

impl RedisCommandArgs {
    fn new(name: &'static str) -> Self {
        Self { name, args: Vec::new() }
    }

    fn arg(mut self, value: impl Into<Vec<u8>>) -> Self {
        self.args.push(value.into());
        self
    }

    fn into_cmd(self) -> redis::Cmd {
        let mut cmd = redis::cmd(self.name);
        for a in self.args {
            cmd.arg(a);
        }
        cmd
    }
}

fn redis_key(index_name: &str, id: &str) -> String {
    format!("{index_name}:{id}")
}

/// Packs an embedding as RediSearch expects a `FLOAT32` vector: raw little-endian bytes,
/// 4 bytes per dimension, no length prefix or separators.
fn embedding_to_bytes(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn build_upsert_args(
    index_name: &str,
    id: &str,
    embedding: &[f32],
    metadata: &serde_json::Value,
) -> Result<RedisCommandArgs, String> {
    let metadata_json =
        serde_json::to_string(metadata).map_err(|e| format!("failed to serialize metadata: {e}"))?;
    Ok(RedisCommandArgs::new("HSET")
        .arg(redis_key(index_name, id))
        .arg(ID_FIELD.as_bytes().to_vec())
        .arg(id.as_bytes().to_vec())
        .arg(EMBEDDING_FIELD.as_bytes().to_vec())
        .arg(embedding_to_bytes(embedding))
        .arg(METADATA_FIELD.as_bytes().to_vec())
        .arg(metadata_json.into_bytes()))
}

fn build_delete_args(index_name: &str, id: &str) -> RedisCommandArgs {
    RedisCommandArgs::new("DEL").arg(redis_key(index_name, id))
}

/// Builds `FT.SEARCH <index> "*=>[KNN <top_k> @embedding $vec AS vector_score]" PARAMS 2 vec
/// <bytes> SORTBY vector_score RETURN 3 id metadata vector_score DIALECT 2`.
fn build_knn_search_args(index_name: &str, embedding: &[f32], top_k: usize) -> RedisCommandArgs {
    RedisCommandArgs::new("FT.SEARCH")
        .arg(index_name.as_bytes().to_vec())
        .arg(format!("*=>[KNN {top_k} @{EMBEDDING_FIELD} $vec AS {SCORE_ALIAS}]").into_bytes())
        .arg(b"PARAMS".to_vec())
        .arg(b"2".to_vec())
        .arg(b"vec".to_vec())
        .arg(embedding_to_bytes(embedding))
        .arg(b"SORTBY".to_vec())
        .arg(SCORE_ALIAS.as_bytes().to_vec())
        .arg(b"RETURN".to_vec())
        .arg(b"3".to_vec())
        .arg(ID_FIELD.as_bytes().to_vec())
        .arg(METADATA_FIELD.as_bytes().to_vec())
        .arg(SCORE_ALIAS.as_bytes().to_vec())
        .arg(b"DIALECT".to_vec())
        .arg(b"2".to_vec())
}

fn build_create_index_args(index_name: &str, embedding_dim: usize) -> RedisCommandArgs {
    RedisCommandArgs::new("FT.CREATE")
        .arg(index_name.as_bytes().to_vec())
        .arg(b"ON".to_vec())
        .arg(b"HASH".to_vec())
        .arg(b"PREFIX".to_vec())
        .arg(b"1".to_vec())
        .arg(redis_key(index_name, "").into_bytes())
        .arg(b"SCHEMA".to_vec())
        .arg(ID_FIELD.as_bytes().to_vec())
        .arg(b"TEXT".to_vec())
        .arg(METADATA_FIELD.as_bytes().to_vec())
        .arg(b"TEXT".to_vec())
        .arg(EMBEDDING_FIELD.as_bytes().to_vec())
        .arg(b"VECTOR".to_vec())
        .arg(b"HNSW".to_vec())
        .arg(b"6".to_vec())
        .arg(b"TYPE".to_vec())
        .arg(b"FLOAT32".to_vec())
        .arg(b"DIM".to_vec())
        .arg(embedding_dim.to_string().into_bytes())
        .arg(b"DISTANCE_METRIC".to_vec())
        .arg(b"COSINE".to_vec())
}

/// Best-effort conversion of a scalar `redis::Value` reply element to a UTF-8 string. Returns
/// `None` for shapes that can't reasonably be read as text (nested arrays, `Nil`, etc.).
fn value_to_string(value: &redis::Value) -> Option<String> {
    match value {
        redis::Value::BulkString(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        redis::Value::SimpleString(s) => Some(s.clone()),
        redis::Value::Int(n) => Some(n.to_string()),
        redis::Value::Double(d) => Some(d.to_string()),
        _ => None,
    }
}

/// Parses an `FT.SEARCH` reply (built with `RETURN 3 id metadata vector_score`, RESP2 shape)
/// into `SearchResult`s: `[total_count, key1, [field, value, field, value, ...], key2, ...]`.
fn parse_search_reply(reply: &redis::Value) -> Result<Vec<SearchResult>, String> {
    let redis::Value::Array(top) = reply else {
        return Err("unexpected FT.SEARCH reply shape: expected a top-level array".to_string());
    };
    if top.is_empty() {
        return Ok(Vec::new());
    }

    let mut results = Vec::new();
    // top[0] is the total result count; entries alternate (key, fields-array) from there.
    let mut i = 1;
    while i + 1 < top.len() {
        if let redis::Value::Array(fields) = &top[i + 1] {
            let mut id = String::new();
            let mut score: f32 = 0.0;
            let mut metadata = serde_json::Value::Null;

            let mut j = 0;
            while j + 1 < fields.len() {
                let field_name = value_to_string(&fields[j]).unwrap_or_default();
                let field_value = &fields[j + 1];
                if field_name == ID_FIELD {
                    id = value_to_string(field_value).unwrap_or_default();
                } else if field_name == SCORE_ALIAS {
                    score = value_to_string(field_value)
                        .and_then(|s| s.parse::<f32>().ok())
                        .unwrap_or(0.0);
                } else if field_name == METADATA_FIELD {
                    let raw = value_to_string(field_value).unwrap_or_default();
                    metadata = serde_json::from_str(&raw).unwrap_or(serde_json::Value::Null);
                }
                j += 2;
            }
            results.push(SearchResult { id, score, metadata });
        }
        i += 2;
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Pure command-building tests ───────────────────────────────────────────

    #[test]
    fn knn_search_args_carry_index_name_top_k_and_correctly_sized_vector_bytes() {
        let embedding = vec![1.0f32, -2.5, 0.0, 3.25];
        let cmd = build_knn_search_args("agora-delegates", &embedding, 7);

        assert_eq!(cmd.name, "FT.SEARCH");
        assert_eq!(cmd.args[0], b"agora-delegates".to_vec());

        let knn_clause = String::from_utf8(cmd.args[1].clone()).unwrap();
        assert!(knn_clause.contains("KNN 7 "), "clause was: {knn_clause}");
        assert!(knn_clause.contains(EMBEDDING_FIELD));
        assert!(knn_clause.contains(SCORE_ALIAS));

        // The vector arg (index 5: index_name, clause, PARAMS, "2", "vec", <vector>) must be
        // exactly 4 bytes per f32 dimension, little-endian FLOAT32 as RediSearch requires.
        let vector_bytes = &cmd.args[5];
        assert_eq!(vector_bytes.len(), embedding.len() * 4);
        assert_eq!(&vector_bytes[0..4], &1.0f32.to_le_bytes()[..]);
        assert_eq!(&vector_bytes[4..8], &(-2.5f32).to_le_bytes()[..]);

        assert!(cmd.args.iter().any(|a| *a == b"DIALECT".to_vec()));
    }

    #[test]
    fn upsert_args_json_encode_metadata_and_pack_embedding_bytes() {
        let metadata = serde_json::json!({"displayName": "Alice", "topics": ["housing"]});
        let cmd = build_upsert_args("agora-delegates", "delegate-42", &[0.5, 0.25], &metadata)
            .expect("metadata is valid JSON, must serialize");

        assert_eq!(cmd.name, "HSET");
        assert_eq!(cmd.args[0], b"agora-delegates:delegate-42".to_vec());

        // Field/value pairs: id, embedding, metadata (order fixed by build_upsert_args).
        let metadata_bytes = &cmd.args[6];
        let round_tripped: serde_json::Value = serde_json::from_slice(metadata_bytes).unwrap();
        assert_eq!(round_tripped, metadata);

        let embedding_bytes = &cmd.args[4];
        assert_eq!(embedding_bytes.len(), 8); // 2 dims * 4 bytes
    }

    #[test]
    fn delete_args_target_the_same_key_shape_upsert_writes() {
        let cmd = build_delete_args("agora-delegates", "delegate-42");
        assert_eq!(cmd.name, "DEL");
        assert_eq!(cmd.args, vec![b"agora-delegates:delegate-42".to_vec()]);
    }

    #[test]
    fn create_index_args_declare_a_float32_vector_field_with_the_requested_dimension() {
        let cmd = build_create_index_args("agora-delegates", 384);
        assert_eq!(cmd.name, "FT.CREATE");
        assert!(cmd.args.iter().any(|a| *a == b"VECTOR".to_vec()));
        assert!(cmd.args.iter().any(|a| *a == b"FLOAT32".to_vec()));
        assert!(cmd.args.iter().any(|a| *a == b"384".to_vec()));
        // PREFIX must match the exact key shape upsert/delete use, or FT.CREATE would index an
        // empty set of documents.
        assert!(cmd.args.iter().any(|a| *a == b"agora-delegates:".to_vec()));
    }

    // ── Pure reply-parsing tests ──────────────────────────────────────────────

    #[test]
    fn parse_search_reply_extracts_id_score_and_metadata_from_a_well_formed_reply() {
        let reply = redis::Value::Array(vec![
            redis::Value::Int(1),
            redis::Value::BulkString(b"agora-delegates:delegate-42".to_vec()),
            redis::Value::Array(vec![
                redis::Value::BulkString(ID_FIELD.as_bytes().to_vec()),
                redis::Value::BulkString(b"delegate-42".to_vec()),
                redis::Value::BulkString(METADATA_FIELD.as_bytes().to_vec()),
                redis::Value::BulkString(br#"{"displayName":"Alice"}"#.to_vec()),
                redis::Value::BulkString(SCORE_ALIAS.as_bytes().to_vec()),
                redis::Value::BulkString(b"0.1234".to_vec()),
            ]),
        ]);

        let results = parse_search_reply(&reply).expect("well-formed reply must parse");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "delegate-42");
        assert!((results[0].score - 0.1234).abs() < 1e-6);
        assert_eq!(results[0].metadata, serde_json::json!({"displayName": "Alice"}));
    }

    #[test]
    fn parse_search_reply_on_zero_results_returns_empty_vec_not_an_error() {
        let reply = redis::Value::Array(vec![redis::Value::Int(0)]);
        let results = parse_search_reply(&reply).expect("zero-result reply is not an error");
        assert!(results.is_empty());
    }

    #[test]
    fn parse_search_reply_rejects_a_reply_that_is_not_a_top_level_array() {
        let reply = redis::Value::Int(42);
        let err = parse_search_reply(&reply).expect_err("non-array top level must be rejected");
        assert!(err.contains("unexpected FT.SEARCH reply shape"), "{err}");
    }

    // ── Env-var / connection-failure paths ────────────────────────────────────
    //
    // No live Redis/RediSearch instance exists to test against (see module docs and CLAUDE.md —
    // there is no provisioned Redis Cloud account). Following chain.rs's
    // `chain_status_surfaces_disconnected_state_not_fake_success` precedent: exercise the real
    // failure paths (missing config, unreachable host) honestly, and do not write a test that
    // pretends to validate a successful round trip against nothing.
    //
    // Both cases live in one #[tokio::test] because they mutate the process-global
    // REDIS_VECTOR_SEARCH_URL env var, which Rust's default parallel-threaded test runner would
    // otherwise race across separate test functions.
    #[tokio::test]
    async fn missing_then_unreachable_env_config_produce_distinct_honest_errors() {
        // `env::set_var`/`remove_var` are `unsafe fn` as of Rust 1.82 (not thread-safe on all
        // platforms if another thread reads/writes env vars concurrently) — sound here because
        // this whole scenario runs single-threaded within one test function (see comment above)
        // and no other test in this module touches process env vars.
        unsafe {
            std::env::remove_var(REDIS_VECTOR_SEARCH_URL_VAR);
        }
        let client = RedisVectorSearchClient::new("agora-delegates-test");

        let missing_err = client
            .query(&[0.1, 0.2, 0.3], 5)
            .await
            .expect_err("must error when REDIS_VECTOR_SEARCH_URL is unset");
        assert!(missing_err.contains("REDIS_VECTOR_SEARCH_URL"), "{missing_err}");
        assert!(missing_err.contains("not configured"), "{missing_err}");

        // Port 1 is reserved/unused; connections to it are refused immediately and
        // deterministically, mirroring chain.rs's `fetch_chain_status("http://127.0.0.1:1")`
        // technique for exercising a real connection failure without depending on anything
        // actually running locally.
        unsafe {
            std::env::set_var(REDIS_VECTOR_SEARCH_URL_VAR, "redis://127.0.0.1:1");
        }
        let unreachable_err = client
            .query(&[0.1, 0.2, 0.3], 5)
            .await
            .expect_err("must error when redis is unreachable, not silently succeed");
        assert!(!unreachable_err.is_empty());
        assert!(
            unreachable_err.contains("redis"),
            "expected a redis-context-prefixed error, got: {unreachable_err}"
        );

        unsafe {
            std::env::remove_var(REDIS_VECTOR_SEARCH_URL_VAR);
        }
    }
}
