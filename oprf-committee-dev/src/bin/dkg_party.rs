//! **DEV/TEST-ONLY founding-ceremony simulator — one OS process per committee member.**
//!
//! Run this binary once per simulated party (`--party-index 1`, `--party-index 2`, ... up to
//! `--num-parties`), each as its own separate process, all pointed at the same
//! `--ceremony-dir`. The parties coordinate the Joint-Feldman DKG protocol
//! (`src/dkg.rs` — read that module's doc comment first for the actual cryptography and its
//! real, disclosed limitations) purely through files in that directory: no shared memory, no
//! shared RNG, no function call that hands one party's secret to another. This is the
//! "genuine architectural step up" changelog entries 73/82 asked for over `committee.rs`'s
//! single-process, single-RNG, single-flat-secret dev simulator — see this crate's README for
//! how the two relate (this covers changelog entry 82's still-open "DKG ceremony mechanics"
//! question; `committee.rs` remains what closes changelog entries 78/81's circuit-proving
//! sessions and is untouched by this file).
//!
//! ## Why this is still emphatically not a real ceremony — read before reusing any of this
//!
//! This binary genuinely enforces *process* separation (each party is a real OS process with
//! its own address space; a party's secret share and polynomial coefficients never leave that
//! process except as the specific, individually-verifiable values the protocol actually
//! requires to be sent). It does **not**, and cannot, provide:
//!
//! - **Real network/geographic separation.** Every party in a test run shares one filesystem
//!   (one machine, one disk). The "private" per-recipient share files under `shares/` and the
//!   `private/` directory this binary writes its own final share into are visible to anyone
//!   with read access to `--ceremony-dir` on that machine — that's fine for a test harness
//!   verifying the protocol's math (see `tests/dkg_ceremony.rs`) after the fact, but a REAL
//!   ceremony must never run this way. A real deployment needs an actual point-to-point
//!   authenticated+confidential channel per pair of members (e.g. mutually authenticated
//!   TLS/Noise over the internet between geographically distributed members' own devices) —
//!   the shared-directory file exchange here is a stand-in for that channel's *protocol
//!   shape* (who sends what to whom, in what order, with what public verifiability), not an
//!   implementation of the channel's security properties.
//! - **Real hardware key custody.** `private/party_<i>.secret_share` is a plaintext file. A
//!   real ceremony needs the final share to be generated and held inside real secure hardware
//!   (a phone's Secure Enclave/Keystore, an HSM, or at minimum an encrypted-at-rest keystore
//!   with access control) on that member's own device — per changelog entry 82's own "Still
//!   open" list, this remains unsolved project-wide, not something this file changes.
//! - **Sybil-resistant member vetting/selection.** This binary trusts whatever `--account-id`
//!   string is passed on the command line; it has no notion of *who* is allowed to be "party
//!   3." Real founding-group selection (per changelog entry 73: named institutional seats —
//!   Elections Commission, Judiciary, Audit Office, academic/NGO, international monitoring)
//!   and real sortition-committee eligibility (per entry 73's `committee_slot`/anchor-based
//!   selection) both happen entirely outside this tool.
//! - **Handling a member going offline mid-ceremony beyond "detect and abort."** Every
//!   round-barrier wait ([`wait_for_files`]) has a timeout and, on expiry, reports exactly
//!   which party indices are missing and exits nonzero — a real, tested failure mode (see
//!   `tests/dkg_ceremony.rs`'s `ceremony_reports_missing_party_and_times_out_rather_than_hanging`),
//!   not a silent hang. But there is no resume/retry/replace-the-missing-member logic: a timed-
//!   out ceremony must be re-run from a fresh `--ceremony-dir` with a full roster. Real GJKR
//!   DKG's complaint/justification round (which can disqualify one bad/absent party and
//!   *continue* with the rest) is not implemented — see `src/dkg.rs`'s doc comment.
//! - **Real entropy hygiene.** [`os_seeded_rng`] mixes OS-derived bytes (via
//!   `std::collections::hash_map::RandomState`, which itself draws on the OS's CSPRNG on every
//!   platform the Rust standard library supports) with the process ID and a timestamp. This is
//!   adequate for a dev/test ceremony simulation but is not a substitute for a real member's
//!   own audited hardware RNG — the same caveat `ffi.rs`'s module doc already states for the
//!   query-answering path applies here too.
//!
//! ## Protocol / file layout
//!
//! ```text
//! <ceremony-dir>/
//!   parties/party_<i>.id                 — party i's claimed account id (informational only)
//!   commitments/party_<i>.commitments    — round 1 (public): party i's t Feldman commitments
//!   shares/from_<i>_to_<j>.share         — round 2: party i's share of party j (see the
//!                                           in-file warning above: NOT a secure channel here)
//!   private/party_<i>.secret_share       — party i's own final combined share (test-inspection
//!                                           only; a real deployment never writes this to a
//!                                           shared location — see above)
//!   final/party_<i>_group_key.txt        — party i's own independently-computed copy of the
//!                                           group public key (every party computes this from
//!                                           purely public round-1 data; the test harness
//!                                           confirms all n copies agree byte-for-byte, so no
//!                                           single party is trusted as "the combiner")
//!   final/roster_party_<i>.txt           — party i's own roster entry ("<index> <account-id>")
//!   ready/party_<i>.ready                — sentinel: party i finished successfully
//! ```
//!
//! Each round is a barrier: every party waits (polling, with a timeout) for every other
//! party's file from the previous round to exist before proceeding — the same "poll chain/
//! ceremony state rather than being pushed to" pattern this project already uses for jury duty
//! (`pallet-courts`) and OPRF query answering (changelog entry 82's mailbox design).

use std::collections::hash_map::RandomState;
use std::fs;
use std::hash::{BuildHasher, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use num_bigint::BigUint;
use oprf_committee_dev::babyjubjub::Point;
use oprf_committee_dev::dkg;
use oprf_committee_dev::ffi::{field_from_be, field_to_be};
use rand::SeedableRng;

struct Args {
    ceremony_dir: PathBuf,
    party_index: u64,
    num_parties: u64,
    threshold: u64,
    account_id: String,
    timeout_secs: u64,
}

fn parse_args() -> Args {
    let mut ceremony_dir = None;
    let mut party_index = None;
    let mut num_parties = None;
    let mut threshold = None;
    let mut account_id = None;
    let mut timeout_secs = 60u64;

    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let val = it
            .next()
            .unwrap_or_else(|| panic!("dkg_party: missing value for flag {flag}"));
        match flag.as_str() {
            "--ceremony-dir" => ceremony_dir = Some(PathBuf::from(val)),
            "--party-index" => {
                party_index = Some(val.parse().unwrap_or_else(|_| panic!("--party-index must be a positive integer")))
            }
            "--num-parties" => {
                num_parties = Some(val.parse().unwrap_or_else(|_| panic!("--num-parties must be a positive integer")))
            }
            "--threshold" => {
                threshold = Some(val.parse().unwrap_or_else(|_| panic!("--threshold must be a positive integer")))
            }
            "--account-id" => account_id = Some(val),
            "--timeout-secs" => {
                timeout_secs = val.parse().unwrap_or_else(|_| panic!("--timeout-secs must be a positive integer"))
            }
            other => panic!("dkg_party: unknown flag {other}"),
        }
    }

    let party_index: u64 = party_index.expect("--party-index is required");
    let num_parties: u64 = num_parties.expect("--num-parties is required");
    let threshold: u64 = threshold.expect("--threshold is required");
    assert!(num_parties >= 1, "--num-parties must be at least 1");
    assert!(
        (1..=num_parties).contains(&threshold),
        "--threshold must be in [1, num_parties]"
    );
    assert!(
        (1..=num_parties).contains(&party_index),
        "--party-index must be in [1, num_parties]"
    );

    Args {
        ceremony_dir: ceremony_dir.expect("--ceremony-dir is required"),
        party_index,
        num_parties,
        threshold,
        account_id: account_id.expect("--account-id is required"),
        timeout_secs,
    }
}

// ── Directory layout ─────────────────────────────────────────────────────────

fn parties_dir(dir: &Path) -> PathBuf {
    dir.join("parties")
}
fn commitments_dir(dir: &Path) -> PathBuf {
    dir.join("commitments")
}
fn shares_dir(dir: &Path) -> PathBuf {
    dir.join("shares")
}
fn private_dir(dir: &Path) -> PathBuf {
    dir.join("private")
}
fn final_dir(dir: &Path) -> PathBuf {
    dir.join("final")
}
fn ready_dir(dir: &Path) -> PathBuf {
    dir.join("ready")
}

fn party_id_path(dir: &Path, i: u64) -> PathBuf {
    parties_dir(dir).join(format!("party_{i}.id"))
}
fn commitments_path(dir: &Path, i: u64) -> PathBuf {
    commitments_dir(dir).join(format!("party_{i}.commitments"))
}
fn share_path(dir: &Path, from: u64, to: u64) -> PathBuf {
    shares_dir(dir).join(format!("from_{from}_to_{to}.share"))
}
fn private_share_path(dir: &Path, i: u64) -> PathBuf {
    private_dir(dir).join(format!("party_{i}.secret_share"))
}
fn final_group_key_path(dir: &Path, i: u64) -> PathBuf {
    final_dir(dir).join(format!("party_{i}_group_key.txt"))
}
fn final_roster_path(dir: &Path, i: u64) -> PathBuf {
    final_dir(dir).join(format!("roster_party_{i}.txt"))
}
fn ready_path(dir: &Path, i: u64) -> PathBuf {
    ready_dir(dir).join(format!("party_{i}.ready"))
}

/// Writes `contents` to `path` via a write-to-temp-then-rename, so any other party polling for
/// `path` with a plain [`Path::exists`] check either sees nothing or sees the complete file —
/// never a partial write. `rename` is atomic within the same filesystem on both POSIX and
/// Windows for a same-volume destination, which every path here is (`--ceremony-dir` and its
/// subdirectories).
fn write_atomic(path: &Path, contents: &str) {
    let tmp = path.with_extension(format!(
        "tmp{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos()
    ));
    fs::write(&tmp, contents).unwrap_or_else(|e| panic!("write {}: {e}", tmp.display()));
    fs::rename(&tmp, path)
        .unwrap_or_else(|e| panic!("rename {} -> {}: {e}", tmp.display(), path.display()));
}

/// Polls (100ms interval) until every path in `paths` exists, or `timeout` elapses. On timeout,
/// returns an error naming exactly which paths never appeared — the concrete "a member went
/// offline mid-ceremony" failure mode named in this project's own open-questions list
/// (changelog entry 82), surfaced as a clear, fail-safe error rather than an indefinite hang.
fn wait_for_files(paths: &[PathBuf], timeout: Duration, what: &str) -> Result<(), String> {
    let start = Instant::now();
    loop {
        let missing: Vec<&PathBuf> = paths.iter().filter(|p| !p.exists()).collect();
        if missing.is_empty() {
            return Ok(());
        }
        if start.elapsed() > timeout {
            let missing_str: Vec<String> =
                missing.iter().map(|p| p.display().to_string()).collect();
            return Err(format!(
                "timed out after {:?} waiting for {what}; still missing: {}",
                timeout,
                missing_str.join(", ")
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A dev/test-quality seed mixing OS-derived entropy (`RandomState`, which the Rust standard
/// library itself seeds from the platform CSPRNG — `getrandom`/`/dev/urandom` on Linux,
/// `BCryptGenRandom` on Windows, `SecRandomCopyBytes` on macOS — on every call) with the
/// process ID and a timestamp, so concurrently-started sibling processes on the same machine
/// don't collide even if the underlying `RandomState` seeding were ever coarser than
/// per-call-fresh. **Not** a claim of hardware-RNG quality or auditable entropy hygiene — see
/// this file's module doc, "Real entropy hygiene."
fn os_seeded_rng() -> rand::rngs::StdRng {
    let mut seed = [0u8; 32];
    for chunk in seed.chunks_mut(8) {
        let v = RandomState::new().build_hasher().finish();
        let b = v.to_le_bytes();
        chunk.copy_from_slice(&b[..chunk.len()]);
    }
    let pid = std::process::id().to_le_bytes();
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().subsec_nanos().to_le_bytes();
    for (i, b) in pid.iter().enumerate() {
        seed[i] ^= b;
    }
    for (i, b) in nanos.iter().enumerate() {
        seed[8 + i] ^= b;
    }
    rand::rngs::StdRng::from_seed(seed)
}

// ── Wire encoding (text, hex — human-inspectable, matches this being a ceremony a real
//    operator could audit by eye, not just a machine-to-machine binary protocol) ──────────────

fn encode_point(p: &Point) -> String {
    format!("{} {}", hex::encode(field_to_be(&p.x)), hex::encode(field_to_be(&p.y)))
}

fn decode_point(line: &str) -> Point {
    let mut it = line.split_whitespace();
    let x_hex = it.next().unwrap_or_else(|| panic!("malformed point line: {line:?}"));
    let y_hex = it.next().unwrap_or_else(|| panic!("malformed point line: {line:?}"));
    let x = field_from_be(&hex::decode(x_hex).unwrap_or_else(|e| panic!("bad hex in {line:?}: {e}")));
    let y = field_from_be(&hex::decode(y_hex).unwrap_or_else(|e| panic!("bad hex in {line:?}: {e}")));
    Point::new(x, y)
}

fn encode_points(points: &[Point]) -> String {
    points.iter().map(encode_point).collect::<Vec<_>>().join("\n")
}

fn decode_points(text: &str) -> Vec<Point> {
    text.lines().filter(|l| !l.trim().is_empty()).map(decode_point).collect()
}

/// 32-byte big-endian hex, left-padded — `BABYJUBJUB_Fr` scalars (shares, and the polynomial
/// coefficients they come from) are ~251 bits, always fitting in 32 bytes.
fn scalar_to_hex(x: &BigUint) -> String {
    let bytes = x.to_bytes_be();
    assert!(bytes.len() <= 32, "scalar unexpectedly wider than 32 bytes");
    let mut buf = [0u8; 32];
    buf[32 - bytes.len()..].copy_from_slice(&bytes);
    hex::encode(buf)
}

fn scalar_from_hex(s: &str) -> BigUint {
    BigUint::from_bytes_be(&hex::decode(s.trim()).unwrap_or_else(|e| panic!("bad scalar hex {s:?}: {e}")))
}

fn log(i: u64, msg: &str) {
    eprintln!("[party {i}] {msg}");
}

fn main() {
    let args = parse_args();
    let dir = &args.ceremony_dir;
    let i = args.party_index;
    let n = args.num_parties;
    let t = args.threshold;
    let timeout = Duration::from_secs(args.timeout_secs);

    for d in [
        parties_dir(dir),
        commitments_dir(dir),
        shares_dir(dir),
        private_dir(dir),
        final_dir(dir),
        ready_dir(dir),
    ] {
        fs::create_dir_all(&d).unwrap_or_else(|e| panic!("create {}: {e}", d.display()));
    }

    log(i, &format!("starting DKG ceremony: n={n} t={t} dir={}", dir.display()));

    write_atomic(&party_id_path(dir, i), &args.account_id);

    // ── Round 1: sample this party's polynomial, publish Feldman commitments ──
    let mut rng = os_seeded_rng();
    let poly = dkg::Polynomial::random(t as usize, &mut rng);
    let commitments = dkg::commit(&poly);
    write_atomic(&commitments_path(dir, i), &encode_points(&commitments));
    log(i, "posted round-1 Feldman commitments");

    let all_commitment_paths: Vec<PathBuf> = (1..=n).map(|j| commitments_path(dir, j)).collect();
    if let Err(e) = wait_for_files(&all_commitment_paths, timeout, "round-1 commitments from every party") {
        log(i, &format!("FATAL: {e}"));
        std::process::exit(1);
    }
    let all_commitments: Vec<Vec<Point>> = (1..=n)
        .map(|j| decode_points(&fs::read_to_string(commitments_path(dir, j)).unwrap()))
        .collect();
    log(i, "received round-1 commitments from every party");

    // ── Round 2: evaluate this party's polynomial at every party's index, send each its own
    //    share via a per-recipient file. See the module doc: on real hardware this must be a
    //    real point-to-point secure channel, not a shared directory. ──
    for j in 1..=n {
        let share = poly.eval(j);
        write_atomic(&share_path(dir, i, j), &scalar_to_hex(&share));
    }
    log(i, "posted round-2 shares for every party");

    let incoming_paths: Vec<PathBuf> = (1..=n).map(|sender| share_path(dir, sender, i)).collect();
    if let Err(e) = wait_for_files(&incoming_paths, timeout, "round-2 shares addressed to me") {
        log(i, &format!("FATAL: {e}"));
        std::process::exit(1);
    }

    // ── Verify every received share against its sender's public commitments, then combine.
    //    A verification failure aborts the whole ceremony (see module doc: no complaint/
    //    justification round is implemented — a known, disclosed gap vs. full GJKR). ──
    let mut received_shares = Vec::with_capacity(n as usize);
    for sender in 1..=n {
        let raw = fs::read_to_string(share_path(dir, sender, i))
            .unwrap_or_else(|e| panic!("read share from {sender}: {e}"));
        let share = scalar_from_hex(&raw);
        let sender_commitments = &all_commitments[(sender - 1) as usize];
        if !dkg::verify_share(&share, i, sender_commitments) {
            log(
                i,
                &format!(
                    "FATAL: share from party {sender} failed Feldman verification — aborting \
                     (a real ceremony would raise a complaint and could disqualify party \
                     {sender} and continue without it; this simulator does not implement that \
                     — see module doc)"
                ),
            );
            std::process::exit(2);
        }
        received_shares.push(share);
    }
    log(i, "verified every received share against its sender's public commitments");

    let my_final_share = dkg::combine_received_shares(&received_shares);

    // Written to a shared directory ONLY because this is a test harness that needs to inspect
    // it after all processes exit to check the ceremony's math (see tests/dkg_ceremony.rs). A
    // real deployment writes this straight into that member's own secure local storage and
    // NEVER to anywhere another party (or this test harness) could read it. See module doc.
    write_atomic(&private_share_path(dir, i), &scalar_to_hex(&my_final_share));

    // ── Final public outputs: every party independently computes and publishes its own copy
    //    of the group public key from purely public round-1 data, and its own roster entry.
    //    No single party is trusted as "the combiner" — the test harness (or, in a real
    //    deployment, any independent observer) confirms all n copies agree. ──
    let first_commitments: Vec<Point> = all_commitments.iter().map(|c| c[0]).collect();
    let y = dkg::group_public_key(&first_commitments);
    write_atomic(&final_group_key_path(dir, i), &encode_point(&y));
    write_atomic(&final_roster_path(dir, i), &format!("{i} {}", args.account_id));

    write_atomic(&ready_path(dir, i), "ok");

    log(
        i,
        &format!(
            "ceremony complete. group public key (x,y hex) = {}",
            encode_point(&y)
        ),
    );
}
