//! # Identity Pallet
//!
//! Citizen registry and ZKPassport-based identity verification: biometric passport NFC scan
//! feeds a ZKPassport Noir/UltraHonk ZK proof (`register_citizen` verifies it via
//! `T::ZkVerifier`, backed in production by the real bb 5.0.0 UltraHonk pairing check in
//! `runtime/src/verifier.rs` — see that module's docs for the exact public-input layout).
//! Replaces the earlier Rarimo Freedom Tool integration, dropped in favor of ZKPassport
//! (see HANDOFF.md log #65 for why).
//!
//! Nullifier = the ZKPassport outer circuit's own `scoped_nullifier` public output — an
//! identity commitment scoped to `service_scope`/`service_subscope`, stable across passport
//! renewals. This pallet extracts it directly from the verified proof's public inputs
//! (`public_inputs[public_inputs.len() - 2]` in `register_citizen`) and checks it against
//! `NullifierRegistry` for uniqueness; it does not itself compute a nullifier formula (in
//! particular, **not** `Poseidon2(national_id || country_code)` — that describes this
//! pallet's separate OPRF identity-anchor derivation below, not the nullifier).
//!
//! Registration also requires a second, independent Sybil-resistance check: a mandatory OPRF
//! identity-anchor, recomputed and checked via `T::AnchorVerifier` against the
//! governance-approved committee keys in `OprfCommitteeKeys` (see `circuits/oprf-identity-anchor/README.md`
//! and this file's `AnchorProofVerifier` trait docs for the full design). The on-chain OPRF
//! committee mailbox (`submit_oprf_query`/`submit_oprf_round1`/`submit_oprf_round2`) is the
//! two-round, genuine-threshold mechanism committee members use to jointly answer those
//! anchor queries (`docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`
//! — "Option B") — this pallet performs no cryptographic verification of their submissions
//! itself; see `OprfRound1Commitment`'s doc comment for why, and
//! `oprf-committee-dev/src/threshold.rs` for the protocol and its own on-chain-independent
//! verification against a real Noir dependency.
#![cfg_attr(not(feature = "std"), no_std)]
extern crate alloc;
pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {

    use codec::DecodeWithMemTracking;
    use frame_support::pallet_prelude::*;
    use frame_support::traits::{Currency, EnsureOriginWithArg, UnixTime};
    use frame_system::pallet_prelude::*;
    use sp_runtime::traits::{Saturating, Zero};

    /// Computes the domain-separated hash a legislature motion's `call_hash` must equal for
    /// `AdminOrigin` to authorize `tag`'s call with `params`. See
    /// `pallet_constitution::pallet::legislature_call_hash` for the full rationale.
    pub(crate) fn legislature_call_hash(tag: &'static [u8], params: impl Encode) -> [u8; 32] {
        let mut preimage = alloc::vec::Vec::from(tag);
        preimage.extend(params.encode());
        frame_support::Hashable::blake2_256(&preimage)
    }

    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Number of independent OPRF committees (changelog entry 73's 5-committee topology).
    /// Must match `circuits/oprf-identity-anchor/lib/identity-anchor::NUM_COMMITTEES`.
    pub const NUM_COMMITTEES: u8 = 5;

    /// Deterministically assigns which of the `NUM_COMMITTEES` OPRF committees a citizen's
    /// on-chain mailbox traffic (`PendingOprfQueries`/`OprfRound1Commitments`) is routed to
    /// (changelog entry 82's query/response mailbox design): `Poseidon2(date_of_birth) mod
    /// NUM_COMMITTEES`. Keyed on date of birth (rather than the never-on-chain-stored
    /// national ID) so the assignment is stable for a citizen's whole lifetime without
    /// depending on data this pallet doesn't have. Reuses the already-validated
    /// `poseidon2-bn254` port (changelog entry 75 — ported line-for-line from
    /// `noir-lang/noir`'s own blackbox solver) rather than re-deriving Poseidon2.
    ///
    /// `date_of_birth` is encoded the same way `current_date` is elsewhere in this pallet
    /// (see `check_outer_proof_freshness`): the low 8 bytes of a big-endian 32-byte field
    /// element. The hash digest is then reduced mod `NUM_COMMITTEES` via manual big-endian
    /// byte-at-a-time modular reduction (`acc = (acc * 256 + byte) mod NUM_COMMITTEES`) —
    /// this `no_std` pallet has no arbitrary-precision integer dependency, and none is needed
    /// since the modulus is a small constant.
    pub fn committee_slot_for(date_of_birth: u64) -> u8 {
        let mut input = [0u8; 32];
        input[24..32].copy_from_slice(&date_of_birth.to_be_bytes());
        let digest = poseidon2_bn254::hash_bytes(&[input]);
        let modulus = NUM_COMMITTEES as u16;
        let mut acc: u16 = 0;
        for byte in digest {
            acc = (acc * 256 + byte as u16) % modulus;
        }
        acc as u8
    }

    // ── ZKPassport proof-scope domain separation ────────────────────────────────
    //
    // `service_scope`/`service_subscope` (`public_inputs[SERVICE_SCOPE_INDEX]`/
    // `public_inputs[SERVICE_SUBSCOPE_INDEX]`) are ZKPassport's own request-time
    // domain-separation fields: `service_scope` is whatever domain string the calling app
    // requests a proof under (there is no registration/approval process with ZKPassport —
    // it's simply a string the app picks), and `service_subscope` is a second, app-chosen
    // string layered on top of it per use-case, both folded into the proof's public inputs
    // via Poseidon2 hashing together with the passport chip data. Mirrors
    // `pallets/pallet-anticorruption/src/lib.rs`'s `zero_padded_scope_tag`/
    // `WHISTLEBLOWER_REPORT_SERVICE_SCOPE`/`WHISTLEBLOWER_REPORT_SERVICE_SUBSCOPE` pattern
    // exactly, including keeping this pallet's own local copy of the tag-padding helper
    // rather than importing anticorruption's — each ZK-proof-consuming pallet in this
    // codebase defines its own domain-separation constants locally (pallet-elections does
    // the same for `register_as_delegate`, with its own distinct scope entirely, not by
    // importing this pallet's), which avoids a purely-for-a-constant cross-crate dependency
    // and keeps each pallet's proof-scope story self-contained and independently auditable.

    /// Index of `service_scope` in an outer ZKPassport proof's `public_inputs` — fixed
    /// regardless of disclosure-subproof count. See `runtime/src/verifier.rs`'s module docs
    /// for the full public-input layout.
    const SERVICE_SCOPE_INDEX: usize = 3;
    /// Index of `service_subscope` in `public_inputs` — fixed regardless of disclosure count.
    const SERVICE_SUBSCOPE_INDEX: usize = 4;

    /// Zero-pads a 31-byte ASCII tag into a canonical 32-byte big-endian BN254 `Fr` element —
    /// identical construction to, and for the identical reason as,
    /// `pallet_anticorruption::pallet::zero_padded_scope_tag` (see that function's doc
    /// comment for the full canonicality argument).
    const fn zero_padded_scope_tag(tag: &[u8; 31]) -> [u8; 32] {
        let mut out = [0u8; 32];
        let mut i = 0;
        while i < 31 {
            out[i + 1] = tag[i];
            i += 1;
        }
        out
    }

    /// Domain-separation constant every pallet-identity call that verifies an outer
    /// ZKPassport proof (`register_citizen`, `reverify_citizen`, `recover_account`,
    /// `migrate_oprf_scheme` — anything going through `Self::verify_outer_proof`) requires as
    /// the proof's `service_scope`. Shared across all four calls (they are all genuinely the
    /// same ZKPassport-integrated app/domain — only the *use-case*, checked via the
    /// call-specific subscope constants below, differs between them).
    ///
    /// PLACEHOLDER: replace with Agora's real, actually-registered production domain string
    /// before this ever verifies a real ZKPassport proof — there is no approval process, this
    /// just needs to be the true domain the mobile app requests proofs under.
    pub const AGORA_IDENTITY_SERVICE_SCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_IDENTITY_SERVICE_SCOPE_V1");

    /// `service_subscope` required for `register_citizen` specifically. Distinct from the
    /// other three subscopes below on purpose, not merely for tidiness: `zk_proof`/
    /// `public_inputs` are ordinary, permanently-public call arguments once an extrinsic
    /// lands on-chain, so if all four calls accepted the same subscope, an observer could
    /// capture *any* citizen's proof off one call (e.g. a `reverify_citizen` transaction) and
    /// replay the exact same bytes into a *different* call — most dangerously
    /// `recover_account`, whose whole job is to rebind a citizen's identity to a new
    /// `AccountId` given nothing but a proof of the same nullifier. A captured, still-fresh
    /// `reverify_citizen` proof replayed into `recover_account` by a third party would (absent
    /// this check) satisfy every guard that call has today — nullifier match, on-file anchor
    /// match, on-file backing_commitment match — and hijack the account to the replayer's own
    /// address. Giving each call its own subscope closes this the same way
    /// `WHISTLEBLOWER_REPORT_SERVICE_SCOPE` stops a `register_citizen` proof from being
    /// replayed into `submit_whistleblower_report`.
    pub const AGORA_IDENTITY_REGISTER_SUBSCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_IDENTITY_REGISTER_SUB_V01");

    /// `service_subscope` required for `reverify_citizen` specifically. See
    /// [`AGORA_IDENTITY_REGISTER_SUBSCOPE`] for why this must differ from the other three.
    pub const AGORA_IDENTITY_REVERIFY_SUBSCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_IDENTITY_REVERIFY_SUB_V01");

    /// `service_subscope` required for `recover_account` specifically. See
    /// [`AGORA_IDENTITY_REGISTER_SUBSCOPE`] for why this must differ from the other three —
    /// this is the highest-stakes of the four, since a replayed proof accepted here rebinds a
    /// citizen's identity to a different `AccountId`.
    pub const AGORA_IDENTITY_RECOVER_SUBSCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_IDENTITY_RECOVER_SUB_V001");

    /// `service_subscope` required for `migrate_oprf_scheme` specifically. See
    /// [`AGORA_IDENTITY_REGISTER_SUBSCOPE`] for why this must differ from the other three.
    pub const AGORA_IDENTITY_MIGRATE_SUBSCOPE: [u8; 32] =
        zero_padded_scope_tag(b"AGORA_IDENTITY_MIGRATE_SUB_V001");

    // ── Backing-commitment incremental Merkle tree ──────────────────────────────
    //
    // Builds and maintains an on-chain incremental Merkle tree over every citizen's
    // `BackingCommitment` value (see that storage item's doc comment for what a leaf value
    // is and why it's safe to expose as one). This is purely the tree/root-history
    // *infrastructure*: no extrinsic here yet consumes it for a real backing-privacy proof
    // (that consumer is future work, likely in `pallet-voting`/`pallet-elections` — see
    // `BackingCommitment`'s doc comment) — this pallet only maintains it correctly and
    // exposes `current_backing_commitment_root`/`is_valid_backing_commitment_root` for that
    // future consumer to call.

    /// Depth of the on-chain incremental Merkle tree over `BackingCommitment` values. Chosen
    /// to exactly match this pallet's own citizen-count ceiling: `TotalCitizens`/`CitizenIndex`
    /// are already `u32` (`TotalCitizensOverflow` caps registration at `u32::MAX`), so a depth
    /// of 32 gives the tree exactly `2^32` (~4.29 billion) leaf slots — enough that
    /// `NextBackingLeafIndex` (also `u32`, see its own doc comment) never needs a wider index
    /// type, and comfortably above any real national citizenry (the largest, China, is ~1.4
    /// billion — more than 3x headroom) even after accounting for the fact that a revoked
    /// citizen's tombstoned leaf (see `tombstone_backing_leaf`) permanently consumes a slot
    /// that is never reclaimed. Mirrors `certificateTree.ts`'s depth-16 Poseidon2 tree in
    /// spirit (both are Poseidon2 incremental Merkle trees over a per-citizen/per-cert-issuer
    /// value), but sized for a citizenry rather than a much smaller set of trusted CA
    /// certificates — depth 16 (65,536 leaves) would be wildly undersized here.
    pub const BACKING_TREE_DEPTH: u8 = 32;

    /// Domain-separation tags for this tree's own Poseidon2 hashing, kept in a numeric range
    /// (210-212) disjoint from `runtime/src/anchor_verifier.rs`'s proof-type tags (200-202) so
    /// the two tag namespaces can never collide even though both ultimately feed the same
    /// Poseidon2 permutation via the same `poseidon2-bn254` crate.
    const DS_BACKING_TREE_NODE: u8 = 210;
    const DS_BACKING_TREE_EMPTY_LEAF: u8 = 211;
    const DS_BACKING_TREE_TOMBSTONE_LEAF: u8 = 212;

    fn backing_tree_tag_field(tag: u8) -> [u8; 32] {
        let mut field = [0u8; 32];
        field[31] = tag;
        field
    }

    /// 2-to-1 internal-node hash for the backing-commitment tree. Domain-separated
    /// (`DS_BACKING_TREE_NODE`) from this tree's own leaf-placeholder values below, and from
    /// every other Poseidon2 domain in this codebase — necessary so an internal node can never
    /// be reinterpreted as, or collide with, a value from a different domain under Poseidon2's
    /// random-oracle assumption. This is the standard second-preimage defense a Merkle tree's
    /// own hash function needs (the same principle as RFC 6962's leaf/node prefix separation):
    /// without it, an attacker who can choose two leaf values could in principle craft a pair
    /// that hashes to the same value as some existing internal node, forging a shorter fake
    /// path. Domain-separating leaves and internal nodes from each other closes that.
    pub fn backing_tree_node_hash(left: [u8; 32], right: [u8; 32]) -> [u8; 32] {
        poseidon2_bn254::hash_bytes(&[backing_tree_tag_field(DS_BACKING_TREE_NODE), left, right])
    }

    /// The fixed placeholder leaf value for a never-yet-registered position. Deliberately not
    /// a bare `[0u8; 32]` (a value a genuine `backing_commitment` could, however
    /// astronomically unlikely, equal) — a domain-separated hash instead, so "empty" can never
    /// be confused with a real commitment.
    pub fn backing_tree_empty_leaf() -> [u8; 32] {
        poseidon2_bn254::hash_bytes(&[backing_tree_tag_field(DS_BACKING_TREE_EMPTY_LEAF)])
    }

    /// The fixed placeholder a revoked citizen's leaf is overwritten with (see
    /// `tombstone_backing_leaf`). Distinct from `backing_tree_empty_leaf` so "never
    /// registered" and "registered, then revoked" remain distinguishable on-chain; distinct
    /// from any real `backing_commitment` for the same reason as that function.
    pub fn backing_tree_tombstone_leaf() -> [u8; 32] {
        poseidon2_bn254::hash_bytes(&[backing_tree_tag_field(DS_BACKING_TREE_TOMBSTONE_LEAF)])
    }

    /// The all-`backing_tree_empty_leaf` subtree root at `level` (0 = a single empty leaf).
    /// Recomputed on demand rather than cached in a `const`/genesis-seeded table — this
    /// pallet is `no_std` (no `LazyLock`), and this is only ever needed for the sibling of a
    /// not-yet-touched position, so it costs at most `BACKING_TREE_DEPTH` extra Poseidon2
    /// calls in the worst case (a nearly-empty tree), negligible next to the real ZK proof
    /// verification `register_citizen`/`revoke_citizen`'s callers already do earlier in the
    /// same extrinsic.
    pub fn backing_tree_zero_hash(level: u8) -> [u8; 32] {
        let mut current = backing_tree_empty_leaf();
        for _ in 0..level {
            current = backing_tree_node_hash(current, current);
        }
        current
    }

    // ── OPRF committee mailbox data types (changelog entry 82) ──────────────────

    /// A citizen's pending blinded OPRF query, posted to the on-chain mailbox in place of a
    /// relay server (changelog entry 82). `blinded_query` is a BabyJubJub point — 64 bytes,
    /// two 32-byte big-endian field elements (x then y) — safe to be public because blinding
    /// hides the underlying identity input (see `oprf-committee-dev/src/oprf.rs`'s
    /// `blinded_query`). `posted_at` anchors the `OprfQuerySlaBlocks` expiry window checked
    /// by `submit_oprf_round1`.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct OprfQueryRecord<AccountId, BlockNumber> {
        pub submitter: AccountId,
        pub blinded_query: [u8; 64],
        pub posted_at: BlockNumber,
    }

    /// One committee member's round-1 broadcast toward a genuine `t`-of-`n` threshold
    /// evaluation of a pending query — the "Option B" design in
    /// `docs/project/research/oprf-alternatives/11-genuine-threshold-evaluation-design.md`,
    /// replacing the single-response-wins design this struct's doc comment used to describe
    /// (see that document's "The problem this closes" for exactly why: under the old design
    /// any *one* member's server was fully and permanently sufficient to answer for its whole
    /// committee, which is not the threshold property changelog #073 specified).
    ///
    /// `r_i` is member `index`'s **partial** OPRF evaluation `s_i · Q` under their own Feldman
    /// share, not the full secret. `(d_g, d_q)`/`(e_g, e_q)` are two FROST-style nonce
    /// commitment pairs (`d_i·G, d_i·Q` and `e_i·G, e_i·Q`) — two nonces, not one, specifically
    /// to support the round-2 binding factor that defends against the Drijvers et al. rogue-
    /// nonce attack on naive two-round Schnorr-family aggregation (see
    /// `oprf-committee-dev/src/threshold.rs`'s module docs for the full protocol and why a
    /// single-nonce version was rejected). All five fields are BabyJubJub points, 64-byte
    /// x-then-y-big-endian, same layout `OprfQueryRecord::blinded_query` already uses.
    ///
    /// This pallet performs **no cryptographic verification** of these fields — only
    /// structural checks (`submit_oprf_round1`: registered member, no double submission, set
    /// not already at `OprfThreshold`). The combined proof's correctness is checked exactly
    /// where OPRF proofs have always been checked: client-side, when a citizen's device
    /// assembles the `anchor`/`disclosure` circuit witness from a locked set's round-1/round-2
    /// data (via `oprf-committee-dev::threshold`'s combination functions) and the resulting ZK
    /// proof is verified by `register_citizen`. A member submitting fabricated data makes the
    /// eventual combination fail — not individually identified or penalized by this pallet;
    /// "identifiable abort" for threshold Sigma protocols is a real, separate, harder problem
    /// this implementation does not attempt (see doc 11).
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct OprfRound1Commitment<AccountId> {
        pub member: AccountId,
        pub r_i: [u8; 64],
        pub d_g: [u8; 64],
        pub d_q: [u8; 64],
        pub e_g: [u8; 64],
        pub e_q: [u8; 64],
    }

    /// One committee member's round-2 response, submitted only after round 1's qualifying set
    /// has locked (exactly `OprfThreshold` round-1 commitments exist) and only by a member of
    /// that locked set (`submit_oprf_round2`). `z_i` is a single BN254 scalar field element —
    /// `oprf-committee-dev/src/threshold.rs::round2_response`'s output — which combines
    /// (summed across the locked set, `oprf-committee-dev::threshold::combine_responses`) into
    /// the final proof's `dlog_s`. Like round 1, no cryptographic check happens here; see
    /// `OprfRound1Commitment`'s doc comment for why.
    #[derive(Clone, Debug, PartialEq, Encode, Decode, DecodeWithMemTracking, MaxEncodedLen, TypeInfo)]
    pub struct OprfRound2Response<AccountId> {
        pub member: AccountId,
        pub z_i: [u8; 32],
    }

    #[pallet::config]
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        /// Pluggable ZK proof verifier. Implement with the real Rarimo Groth16 verifier.
        /// Use a no-op impl for testing.
        type ZkVerifier: ZkProofVerifier;
        /// The origin permitted to suspend and restore citizen voting rights via the manual
        /// override extrinsics below (the auto-enforcement path from an actual court case
        /// instead calls `suspend_citizen_internal` directly via the `CitizenSuspender` trait,
        /// bypassing this origin entirely). `EnsureOriginWithArg` so each call site must pass
        /// the domain-separated hash of its own parameters (see `legislature_call_hash`, same
        /// binding `AdminOrigin` below uses) — a resolved approval for one call can never be
        /// replayed to authorize a different one. Wired to
        /// `pallet_courts::EnsureOracleCouncilApproved` in the runtime: it only succeeds once
        /// the Oracle Council's M-of-N threshold has approved this exact call, not merely a
        /// single member's say-so (a bare `pallet_courts::EnsureOracle` here would let any one
        /// Oracle Council member suspend any citizen unilaterally — the gap a project review
        /// flagged, since `EnsureOracle` alone only checks membership).
        type SuspensionOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
        /// The origin permitted to manage the trusted issuer Merkle root allowlist
        /// (add/remove country CA certificate roots). Keeping this separate from
        /// SuspensionOrigin lets governance rotate the allowlist without touching
        /// the court-controlled suspension key. Wired to
        /// `pallet_legislature::EnsureLegislatureMotion` in the runtime.
        /// `EnsureOriginWithArg` so each call site must pass the domain-separated hash of
        /// its own parameters (see `legislature_call_hash`); the origin then verifies that
        /// hash against the motion's approved `call_hash`, so a motion passed to authorize
        /// one call can never be replayed to execute another.
        type AdminOrigin: frame_support::traits::EnsureOriginWithArg<Self::RuntimeOrigin, [u8; 32]>;
        /// Pluggable OPRF identity-anchor proof verifier (see `AnchorProofVerifier` below):
        /// registration-anchor proofs, reverification/liveness proofs, and cross-scheme
        /// migration-consistency ("dual evaluation") proofs. The real OPRF committee
        /// cryptography itself is out of scope for this pallet (see HANDOFF log #67/#68) —
        /// use a passthrough impl for dev/testing until the OPRF circuit work lands.
        type AnchorVerifier: AnchorProofVerifier;
        /// Number of blocks a registration/reverification remains valid before
        /// `is_active_citizen` lazily starts treating the citizen as inactive for voting
        /// purposes. A governance-tunable parameter (see HANDOFF log #67: "make the rotation
        /// interval itself a governance parameter... not a hardcoded 4 years" — the same
        /// principle applies here, though log #67 leaves this specific cadence as an
        /// explicitly open question distinct from the OPRF rotation cadence).
        #[pallet::constant]
        type ReverificationPeriod: Get<u32>;
        /// The origin permitted to force an out-of-cycle OPRF scheme-version bump ahead of
        /// the normal 4-year schedule (e.g. a suspected OPRF committee compromise — see
        /// HANDOFF log #67's emergency-rotation mechanics). Distinct from the normal
        /// scheduled-rotation path (`rotate_oprf_scheme`, gated by `AdminOrigin`). Wired in the
        /// runtime to `pallet_emergency_council::EnsureActiveEmergency<Runtime>` (see
        /// `runtime/src/configs/mod.rs`), which requires *both* a `Root` origin *and* a
        /// currently-active, council-declared emergency (`ActiveEmergency` is `Some(..)`) —
        /// not the bare `EnsureRoot` this was originally a placeholder for. In practice this
        /// means emergency committee-key rotation can no longer be forced by root
        /// unilaterally at any time: it requires the Emergency Council to have genuinely
        /// declared an emergency via its own supermajority-vote path
        /// (`vote_declare_emergency`), and that emergency must not yet have been lifted
        /// (`vote_end_emergency`) or auto-sunset-expired (`on_initialize`'s hard-coded
        /// `expires_at` check — no root shortcut exists around that). A real strengthening of
        /// the separation-of-powers story: emergency OPRF rotation now requires the same
        /// checked, time-bounded emergency-declaration process as every other emergency
        /// power, rather than a single root key acting alone.
        type EmergencyRotationOrigin: frame_support::traits::EnsureOrigin<Self::RuntimeOrigin>;
        /// Wall-clock source for the registration-anchor freshness check (HANDOFF log #75):
        /// `register_citizen` rejects an outer proof whose `current_date` public input is
        /// older than `MaxAnchorProofAge` relative to this. Wired to `pallet_timestamp` in
        /// the runtime (which already implements `UnixTime`); the mock provides a trivial
        /// fixed-clock impl.
        type Now: frame_support::traits::UnixTime;
        /// How old (in seconds) an outer proof's `current_date` public input is allowed to be
        /// at the moment `register_citizen` is submitted. Exists so a genuine-at-generation-
        /// time proof cannot be replayed indefinitely long after it was produced — the
        /// in-circuit expiry check (`check_expiry_from_date`, see
        /// `circuits/oprf-identity-anchor/disclosure/src/main.nr`) only constrains
        /// `current_date` against the passport's own expiry, not against "now".
        #[pallet::constant]
        type MaxAnchorProofAge: Get<u64>;
        /// How far into the future (in seconds) an outer proof's `current_date` public input
        /// is allowed to be, relative to `T::Now`, before it's rejected outright. Exists
        /// because `current_date` is a fully prover-controlled public input — the in-circuit
        /// check (`check_expiry_from_date`, see
        /// `circuits/oprf-identity-anchor/disclosure/src/main.nr`) only constrains it against
        /// the passport's own expiry, never against real time. Without this check, a
        /// future-dated `current_date` makes `MaxAnchorProofAge`'s `saturating_sub` clamp to
        /// zero and the freshness check above passes unconditionally, forever — a single proof
        /// generated once with a sufficiently future-dated `current_date` could then be
        /// resubmitted indefinitely to keep a citizen "verified" without ever holding a
        /// currently-valid passport. A small nonzero value (e.g. a few minutes) tolerates
        /// ordinary clock skew between the prover and this chain's `T::Now` without reopening
        /// that gap by more than the tolerance itself.
        #[pallet::constant]
        type MaxAnchorProofClockSkew: Get<u64>;
        /// Maximum number of accounts permitted in a single OPRF committee slot's roster
        /// (`CommitteeMembers`), changelog entry 82's on-chain query/response mailbox design.
        /// Set with headroom above the eventual ~35-member committees (changelog entry 73's
        /// 5-independent-committee governance topology) — e.g. `ConstU32<50>`.
        #[pallet::constant]
        type MaxCommitteeSize: Get<u32>;
        /// Blocks a posted `PendingOprfQueries` entry stays open for committee responses
        /// before `submit_oprf_round1`/`submit_oprf_round2` start rejecting it as expired
        /// (changelog entry
        /// 82's SLA window). Per that entry's "Still open" section, "~5-7 days" is a
        /// placeholder pending real pilot telemetry — human committee-member response time is
        /// fundamentally slower than a server's, compounded by needing enough of each
        /// committee's members to respond across all `NUM_COMMITTEES` committees.
        #[pallet::constant]
        type OprfQuerySlaBlocks: Get<u32>;
        /// The Shamir/Feldman reconstruction threshold `t`: how many distinct committee
        /// members' round-1 commitments lock a query's qualifying set (`submit_oprf_round1`)
        /// before round 2 opens. Doc 11's own placeholder caveat applies here directly:
        /// changelog #073 specified "~12-of-35" for the eventual full-scale committees and
        /// "6-of-7" for founding groups, but no real committee has been sized yet — set this
        /// per deployment, e.g. `ConstU32<3>` for development/testing (matches
        /// `oprf-committee-dev::threshold`'s own test coverage at 3-of-5, 6-of-7, and 2-of-2).
        /// Must be `<= MaxCommitteeSize` for any slot expected to actually reach quorum.
        #[pallet::constant]
        type OprfThreshold: Get<u32>;
        /// Maximum number of `PendingOprfQueries` entries a single citizen may have open
        /// (not yet pruned) at once, tracked via `PendingOprfQueryCountBySubmitter`. Bounds
        /// per-citizen mailbox growth: unlike most extrinsics, an open OPRF query has no
        /// forced resolution besides the SLA-expiry / full-completion prune paths
        /// (`prune_oprf_query`), which someone has to remember to call — without this cap a
        /// single signed account could call `submit_oprf_query` an unbounded number of times
        /// and grow chain state at ordinary transaction-fee cost with no ceiling.
        #[pallet::constant]
        type MaxPendingOprfQueriesPerCitizen: Get<u32>;
        /// Minimum number of blocks that must pass between two successful `recover_account`
        /// calls for the same nullifier (tracked in `LastRecoveryBlock`). `recover_account`
        /// deliberately has no dispute/challenge window (see its doc comment for why: a delay
        /// only helps if the old key is still reachable to contest it, which contradicts the
        /// lost-device scenario recovery exists for), so this cooldown is the only friction
        /// standing between a valid recovery proof and an immediate, unchallengeable account
        /// swap. Set high enough to blunt rapid-fire abuse/griefing without meaningfully
        /// hindering a citizen who has genuinely lost a second device shortly after recovering
        /// from losing a first one.
        #[pallet::constant]
        type MinBlocksBetweenRecoveries: Get<u32>;
        /// How long (in blocks) a `BackingCommitmentRootHistory` entry remains valid/queryable
        /// after it stops being the current backing-commitment tree root, before
        /// `push_backing_root_history` prunes it. See that storage item's doc comment for the
        /// full retention-window rationale: sized to be `>=` the longest permitted voting
        /// epoch (`pallet_voting::Config::MaxEpochDurationBlocks`) so a Merkle authentication
        /// path fetched at the very start of the longest epoch still verifies against *some*
        /// valid root by the time it's used later in that same epoch, even though other
        /// citizens' registrations/revocations have moved the live root in the meantime.
        #[pallet::constant]
        type BackingRootHistoryWindowBlocks: Get<u32>;
        /// Hard cap on how many entries `BackingRootHistoryEntries` may hold at once — a
        /// worst-case storage ceiling independent of `BackingRootHistoryWindowBlocks`, so an
        /// extreme, sustained burst of registrations/revocations degrades the *effective*
        /// window (oldest entries evicted early) rather than growing chain state without
        /// bound. See `BackingRootHistoryEntries`'s doc comment for why this is a `StorageMap`
        /// ring rather than a single bounded vector, which is what makes a generously large
        /// cap here cheap (each push/prune touches O(1) storage keys, not the whole history).
        #[pallet::constant]
        type MaxBackingRootHistoryEntries: Get<u32>;
        /// AGR balance source for `recover_account`'s cross-pallet-orphaning guard (see
        /// `Error::RecoveryBlockedNonzeroBalance`): rejects recovery outright if `old_account`
        /// holds a nonzero free balance that would otherwise be silently stranded on an
        /// account this call is about to fully invalidate. A plain `Currency<AccountId>` bound
        /// — the same trait `pallet_courts::Config::Currency` already uses — rather than a
        /// bespoke trait: nothing was already in this pallet's scope for reading a balance, and
        /// `Currency` is the standard, off-the-shelf primitive for "does this account hold
        /// funds", so no new bespoke coupling is needed beyond the ordinary `pallet_balances`
        /// wiring every other balance-reading pallet in this runtime already has. Wired to
        /// `Balances` in the runtime.
        type Currency: Currency<Self::AccountId>;
        /// See `RecoveryStateChecker`'s doc comment above for the full rationale (the
        /// delegate/legislature-seat/cabinet-role legs of the same `recover_account` guard
        /// `Config::Currency` above covers the balance leg of) and why this is a Runtime-glue
        /// trait rather than a direct provider impl on pallet-elections/pallet-legislature/
        /// pallet-executive's own `Pallet<T>`.
        type RecoveryStateChecker: RecoveryStateChecker<Self::AccountId>;
    }

    /// Trait for verifying Rarimo-style Groth16 ZK passport proofs.
    /// Implement this in the runtime, plugging in the real Rarimo verifier key and circuit.
    pub trait ZkProofVerifier {
        /// Returns true if the proof is valid for the given nullifier and public inputs.
        /// proof_bytes: serialized Groth16 proof (A, B, C points on BN254).
        /// public_inputs: [nullifier_hash, passport_expiry_timestamp, country_code_hash].
        fn verify(proof_bytes: &[u8], public_inputs: &[[u8; 32]]) -> bool;
    }

    /// Trait for verifying OPRF-derived identity-anchor proofs (see HANDOFF log #67 for the
    /// full Sybil-resistance design this implements). Distinct from `ZkProofVerifier`: that
    /// trait authenticates the passport itself and the per-vote nullifier; this one covers the
    /// separate, one-time registration-uniqueness anchor derived from the passport's stable
    /// personal-number field via an OPRF committee. The real OPRF cryptography is out of scope
    /// for this pallet — implement this once the OPRF circuit work lands, and use a
    /// passthrough impl for dev/testing until then.
    pub trait AnchorProofVerifier {
        /// Verifies that `anchor` was correctly derived — via the OPRF committee, under
        /// `scheme_version` — from the registrant's passport personal-number field, without
        /// revealing that field's value.
        ///
        /// `outer_public_inputs` is the *same* ZKPassport outer-proof public-input array
        /// `register_citizen` has just validated via `T::ZkVerifier::verify` (HANDOFF log
        /// #74/#75): the OPRF identity-anchor circuit (`disclosure`, see
        /// `circuits/oprf-identity-anchor/README.md`) rides inside that same outer proof as
        /// a recursively-verified subproof rather than as a second, standalone proof, so
        /// there is no separate anchor SNARK to check here — only a Poseidon2 recomputation
        /// of `param_commitment` from `(anchor, scheme_version, oprf_pk_hashes)`, checked
        /// against the values the outer proof already exposes. `oprf_pk_hashes` are the 5
        /// per-committee key hashes (changelog entry 73's 5-committee topology) the anchor
        /// was derived under; a real implementation checks the recomputation here, and the
        /// caller (`register_citizen`) separately checks each hash against a
        /// governance-approved key (`OprfCommitteeKeys`) — see
        /// `runtime/src/anchor_verifier.rs`'s module docs for why the split is there and not
        /// inside this trait.
        ///
        /// `backing_commitment` is a second, independent public value folded into the same
        /// `disclosure`-circuit `param_commitment` preimage as `anchor`/`scheme_version`/
        /// `oprf_pk_hashes` (widened from 8 to 9 fields, no new proof-type tag — see
        /// `runtime/src/anchor_verifier.rs`'s module docs and
        /// `circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`'s
        /// `derive_backing_root_term`/`derive_backing_commitment`). Unlike `anchor`, it binds
        /// to no specific target/delegate, so no front-running protection is needed for it.
        fn verify_registration_anchor(
            outer_public_inputs: &[[u8; 32]],
            anchor: [u8; 32],
            scheme_version: u32,
            oprf_pk_hashes: [[u8; 32]; 5],
            backing_commitment: [u8; 32],
        ) -> bool;
        /// Verifies a reverification/liveness proof: the citizen currently holds a
        /// still-valid, unexpired passport that recomputes to the same anchor already on
        /// file. `outer_public_inputs` is the outer ZKPassport proof `reverify_citizen` has
        /// just validated via `T::ZkVerifier::verify` (HANDOFF log #76) — same shape and same
        /// rationale as `verify_registration_anchor`'s parameter of the same name (a
        /// standalone anchor proof's `comm_in` is an unauthenticated private witness; riding
        /// inside a fresh outer proof is what proves the passport is *currently* valid, not
        /// just was at original registration). `backing_commitment` is checked the same way —
        /// see `verify_registration_anchor`'s doc comment.
        fn verify_reverification(
            outer_public_inputs: &[[u8; 32]],
            anchor: [u8; 32],
            scheme_version: u32,
            oprf_pk_hashes: [[u8; 32]; 5],
            backing_commitment: [u8; 32],
        ) -> bool;
        /// Verifies a migration consistency ("dual evaluation") proof: `old_anchor` and
        /// `new_anchor` were both derived from the same underlying personal-number value,
        /// under the old and new OPRF scheme versions respectively, without revealing that
        /// value. `outer_public_inputs` is the outer ZKPassport proof `migrate_oprf_scheme`
        /// has just validated via `T::ZkVerifier::verify` (HANDOFF log #76) — same rationale
        /// as above: a standalone `migrate` proof's `comm_in` is unauthenticated, so this
        /// rides inside a fresh outer proof (`circuits/oprf-identity-anchor/migrate-disclosure`)
        /// instead.
        fn verify_migration(
            outer_public_inputs: &[[u8; 32]],
            old_anchor: [u8; 32],
            new_anchor: [u8; 32],
            old_scheme_version: u32,
            new_scheme_version: u32,
            old_oprf_pk_hashes: [[u8; 32]; 5],
            new_oprf_pk_hashes: [[u8; 32]; 5],
        ) -> bool;
    }

    /// Checked by `recover_account` before rebinding a citizen's per-account identity storage
    /// from `old_account` to a fresh `who`, to guard against silently orphaning state that
    /// lives in *other* pallets and is keyed on `old_account`: a registered delegate persona
    /// (pallet-elections), a legislature seat (pallet-legislature), a cabinet role
    /// (pallet-executive), an open (not-yet-finalized) referendum vote, or an already-claimed
    /// current-epoch budget allocation (both pallet-voting). See `recover_account`'s doc
    /// comment for the full rationale — this is a divest-first guard, not a migration: a
    /// citizen who currently holds any of this state must resign/withdraw/wait via that
    /// pallet's own normal mechanism first, then recover. (An AGR balance is checked the same
    /// way but via the separate `Config::Currency` bound below, not this trait — see that
    /// item's doc comment for why. A pallet-anticorruption asset-disclosure check was
    /// deliberately *not* added here: `submit_asset_disclosure` is callable by any account at
    /// any time with no eligibility gate, so a lapsed or missing disclosure under `old_account`
    /// is trivially re-filable under the new account in one call after recovering — unlike a
    /// delegate persona/seat/cabinet role/open vote/unclaimed budget, there is no exclusive
    /// resource or one-shot action to lose or repeat, so blocking on it would just be friction
    /// with no real protection. pallet-voting's liquid-democracy delegation relationships
    /// — as opposed to the direct-vote/budget-claim state guarded here — remain genuinely
    /// unguarded and still silently orphan on a successful recovery, but that orphaning is
    /// passive: an inert delegation from `old_account` doesn't count towards anything once
    /// `old_account`'s citizen status is removed by this call, unlike an open vote or an
    /// unclaimed budget allocation, either of which is directly repeatable under `who` if left
    /// unguarded. See `docs/project/next-steps.md` item 14.)
    ///
    /// The two pallet-voting checks close a real double-vote/double-claim vector, not just a
    /// data-hygiene gap: since only one `ActiveEpoch` can be open network-wide at a time but
    /// `MaxEpochDurationBlocks` (30 days) exceeds `MinBlocksBetweenRecoveries` (7 days), a
    /// citizen could vote (or claim+spend a fiscal-epoch budget) under `old_account`, drain its
    /// balance to zero (already required for recovery), wait out the cooldown, and recover to a
    /// fresh `who` with no record of having voted/claimed — letting them vote again on the same
    /// still-open referendum, or claim a second full budget allocation for the same epoch.
    ///
    /// Implemented by `Runtime` (see `runtime/src/configs/mod.rs`), not directly by
    /// pallet-elections'/pallet-legislature's/pallet-executive's/pallet-voting's own
    /// `Pallet<T>` the way `pallet_elections::DisclosureChecker` is implemented directly on
    /// `pallet_anticorruption::Pallet<T>`: pallet-identity-zk is the foundational identity
    /// crate other pallets already depend on (directly or transitively — pallet-elections
    /// depends on it for `BackingRootChecker`'s provider side, for instance), so those pallets
    /// implementing a pallet-identity-zk trait directly on their own `Pallet<T>` would need
    /// pallet-identity-zk to depend on them in turn, a circular crate dependency. This mirrors
    /// the existing `CitizenSuspender`-style Runtime-glue pattern instead (see `impl
    /// pallet_courts::CitizenSuspender<BlockNumberFor<Runtime>> for Runtime` in
    /// `runtime/src/configs/mod.rs`), just with the call direction reversed: there,
    /// pallet-identity-zk is the provider being called *into*; here it's the consumer calling
    /// *out*.
    pub trait RecoveryStateChecker<AccountId> {
        /// `true` if `who` currently has a registered delegate persona
        /// (`pallet_elections::Delegates`), in any status (Pending/Active/OnBreak).
        fn is_registered_delegate(who: &AccountId) -> bool;
        /// `true` if `who` currently holds a legislature seat (`pallet_legislature::Members`).
        fn holds_legislature_seat(who: &AccountId) -> bool;
        /// `true` if `who` currently holds a cabinet role — Prime Minister or a portfolio
        /// minister (`pallet_executive::PrimeMinister`/`MinisterPortfolio`).
        fn holds_cabinet_role(who: &AccountId) -> bool;
        /// `true` if `who` has cast a direct vote (`pallet_voting::ReferendumHasVoted`) in any
        /// referendum still in `pallet_voting::ReferendumState::Voting`. Checked against
        /// `pallet_voting::OpenReferenda`, a bounded list of currently-open referendum ids
        /// (bounded by `pallet_voting::Config::MaxConcurrentReferenda`) — not an unbounded scan
        /// over every referendum ever created. See that storage item's doc comment for why the
        /// bound is safe in practice and why it fails closed rather than silently overflowing.
        fn has_open_referendum_vote(who: &AccountId) -> bool;
        /// `true` if `who` has already claimed (`pallet_voting::CitizenClaimedEpoch`) their
        /// quadratic budget allocation for the currently active fiscal epoch
        /// (`pallet_voting::FiscalYearEpoch`). O(1) — no iteration involved.
        fn has_unclaimed_current_epoch_budget(who: &AccountId) -> bool;
    }

    /// Maps nullifier hash -> registered AccountId. Prevents double-registration.
    #[pallet::storage]
    pub type NullifierRegistry<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], T::AccountId>;

    /// Maps AccountId -> nullifier hash for reverse lookup.
    #[pallet::storage]
    pub type CitizenNullifier<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32]>;

    /// Dense indexed list of citizens for O(1) random selection by courts.
    /// Index 0..TotalCitizens-1 are always occupied.
    #[pallet::storage]
    pub type CitizenIndex<T: Config> =
        StorageMap<_, Blake2_128Concat, u32, T::AccountId>;

    /// Reverse index: AccountId -> position in CitizenIndex. Used for O(1) swap-and-pop.
    #[pallet::storage]
    pub type CitizenPosition<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32>;

    /// Total number of registered citizens.
    #[pallet::storage]
    pub type TotalCitizens<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Court-ordered voting suspensions: nullifier -> optional block when suspension lifts.
    /// None means suspended indefinitely; Some(block) means suspended until that block.
    /// Key absent means not suspended.
    #[pallet::storage]
    pub type SuspendedNullifiers<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], Option<BlockNumberFor<T>>>;

    /// Whether the *current* entry in `SuspendedNullifiers` (if any) came from a jury-reviewed
    /// conviction — i.e. a `pallet-courts` case that reached `CaseStatus::JurySeated` and was
    /// decided by `cast_jury_vote`'s majority, as opposed to an unappealed `AIRulingIssued`
    /// case finalized by `finalize_ruling`, or the manual `suspend_citizen` extrinsic below
    /// (both oracle-only, no jury ever involved). Absent/false is the safe default. Consumers
    /// that need a higher evidentiary bar before acting on a suspension — see
    /// `pallet-executive`'s conviction-triggered office vacancy sweep, which deliberately does
    /// NOT auto-remove a sitting PM/minister on a bare, unappealed AI ruling — check this
    /// alongside `is_active_citizen` via `is_suspended_by_jury_reviewed_conviction`.
    #[pallet::storage]
    pub type SuspendedByJuryReview<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], bool, ValueQuery>;

    /// Trusted issuer Merkle roots (certificate_registry_root from ZKPassport's outer
    /// circuit, public_inputs[0]). A proof is only accepted if its certificate_registry_root
    /// matches one of these roots. Roots represent trusted sets of country certificate
    /// authorities.
    #[pallet::storage]
    pub type AllowedMerkleRoots<T: Config> = StorageMap<_, Identity, [u8; 32], bool, ValueQuery>;

    /// Governance-approved OPRF committee public-key hashes, keyed by `(scheme_version,
    /// committee_slot)`. `committee_slot` runs `0..NUM_COMMITTEES`. `register_citizen` checks
    /// every one of the 5 `oprf_pk_hashes` submitted with a registration against this map
    /// before accepting the anchor — see HANDOFF log #75 and `runtime/src/anchor_verifier.rs`.
    /// Mirrors `AllowedMerkleRoots`'s admin-gated allowlist pattern: a proof that uses 4
    /// correct committee keys and substitutes an unapproved 5th must still be rejected, or
    /// the "unpredictable if even one committee is honest" property (changelog entry 73)
    /// inverts into "forgeable if even one committee key check is skipped" (see
    /// `circuits/oprf-identity-anchor/README.md`'s "What the Rust verifier still has to
    /// enforce", obligation 1).
    #[pallet::storage]
    pub type OprfCommitteeKeys<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, u8), [u8; 32]>;

    /// Global OPRF anchor-scheme version. Incremented on rotation — scheduled
    /// (`rotate_oprf_scheme`, `AdminOrigin`) or emergency (`emergency_rotate_oprf_scheme`,
    /// `EmergencyRotationOrigin`). New registrations always anchor under this current value.
    /// Bumping this counter does **not** retroactively invalidate anyone: existing citizens
    /// keep whatever version their on-file anchor was registered under until they individually
    /// call `migrate_oprf_scheme` — see HANDOFF log #67's "dual evaluation" window, where both
    /// the outgoing and incoming OPRF committees stay live and trusted side by side for a
    /// transition period.
    #[pallet::storage]
    pub type OprfSchemeVersion<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// Identity-anchor exclusion registry, keyed by (scheme_version, anchor) -> the AccountId
    /// registered against it. Keying on scheme version (rather than a single flat
    /// anchor -> AccountId map) is what makes `migrate_oprf_scheme` possible without a global
    /// stop-the-world cutover: a citizen's outgoing-scheme entry can be atomically retired and
    /// a same-citizen incoming-scheme entry inserted, without either colliding with a
    /// *different* citizen's still-valid entry under the other version.
    ///
    /// Deliberately separate from `NullifierRegistry`/`CitizenNullifier` (the per-vote
    /// nullifier used for casting ballots) — this anchor exists purely as a one-time
    /// registration-time uniqueness gate and must never be exposed anywhere that could let it
    /// be correlated with voting activity (see HANDOFF log #67).
    #[pallet::storage]
    pub type IdentityAnchorRegistry<T: Config> =
        StorageMap<_, Blake2_128Concat, (u32, [u8; 32]), T::AccountId>;

    /// Reverse lookup: AccountId -> the (scheme_version, anchor) pair currently on file.
    /// `migrate_oprf_scheme` uses this to find and retire the caller's own outgoing-scheme
    /// anchor — checking the caller's own on-file entry (rather than trusting the `old_anchor`
    /// argument on its own) is what stops a citizen from "migrating" using someone else's
    /// anchor value.
    #[pallet::storage]
    pub type CitizenAnchor<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, (u32, [u8; 32])>;

    /// A citizen's `backing_commitment` — the public Merkle-tree-leaf-safe value derived (via
    /// `T::AnchorVerifier::verify_registration_anchor`/`verify_reverification`'s Poseidon2
    /// recomputation) from the same 5-committee OPRF evaluation as `anchor`, but through a
    /// distinct domain separator (`DS_BACKING_ROOT`/`DS_BACKING_COMMITMENT`, see
    /// `circuits/oprf-identity-anchor/lib/identity-anchor/src/lib.nr`). Set at
    /// `register_citizen`, kept in sync at `reverify_citizen` (which requires the caller's
    /// submitted value to match — see `Error::BackingCommitmentMismatch` — exactly mirroring
    /// how `CitizenAnchor`'s `anchor` half is already checked for consistency across
    /// reverifications).
    ///
    /// Deliberately a plain per-citizen map, not an exclusion registry keyed by the commitment
    /// value the way `IdentityAnchorRegistry` is for `anchor`: `backing_commitment` is a
    /// deterministic function of the same secret material `anchor` already is, so `anchor`'s
    /// own `AnchorAlreadyUsed` uniqueness check already transitively guarantees
    /// `backing_commitment` uniqueness — a second exclusion registry here would be redundant.
    /// A future Merkle-tree phase that needs to enumerate every citizen's `backing_commitment`
    /// to build/insert tree leaves can iterate this map directly.
    ///
    /// **Known gap, out of this phase's scope**: `migrate_oprf_scheme` does not currently
    /// touch this map. Since `backing_commitment` (like `anchor`) is derived under a specific
    /// `scheme_version`'s committee keys, a citizen who migrates to a new OPRF scheme version
    /// will have a `BackingCommitment` entry that is stale relative to their new `CitizenAnchor`
    /// scheme version, until `migrate`/`migrate-disclosure` are themselves extended to compute
    /// and widen their own `param_commitment` the same way `anchor`/`disclosure` were widened
    /// here. Not fixed in this pass — doing so was out of the scope given (see this pallet's
    /// module docs / the commit that introduced this storage item) and would need real circuit
    /// changes to `migrate`/`migrate-disclosure`, not just Rust plumbing.
    #[pallet::storage]
    pub type BackingCommitment<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32]>;

    // ── Backing-commitment incremental Merkle tree storage ──────────────────────

    /// Sparse on-chain storage of the backing-commitment tree's touched nodes, keyed by
    /// `(level, index at that level)`. Level 0 is leaves, level `BACKING_TREE_DEPTH` is the
    /// (single, index-0) root. Untouched nodes are implicitly `backing_tree_zero_hash(level)`
    /// and are never written — inserting or tombstoning one leaf only ever writes the
    /// `BACKING_TREE_DEPTH + 1` nodes on that leaf's own authentication path, not the whole
    /// tree, keeping this genuinely cheap despite the tree's large nominal depth.
    #[pallet::storage]
    pub type BackingCommitmentTreeNodes<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, u8, Blake2_128Concat, u32, [u8; 32]>;

    /// Next free leaf index in `BackingCommitmentTreeNodes`. Assigned once, permanently, at
    /// `register_citizen` (`insert_backing_leaf`) and **never reused** — not even after
    /// `revoke_citizen` tombstones that leaf (see `tombstone_backing_leaf`). This is what
    /// keeps every *other* citizen's previously-fetched Merkle authentication path stable
    /// regardless of unrelated registrations/revocations elsewhere in the tree. Contrast with
    /// `CitizenIndex`/`CitizenPosition`'s swap-and-pop reuse: that's fine for their O(1)-
    /// random-pick use case (jury selection), where index stability across calls is never
    /// relied on, but would be actively unsafe reused here, since it would silently invalidate
    /// an unrelated citizen's cached Merkle path every time someone else revoked.
    #[pallet::storage]
    pub type NextBackingLeafIndex<T: Config> = StorageValue<_, u32, ValueQuery>;

    /// A citizen's permanent leaf index in the backing-commitment tree, keyed by **nullifier**
    /// (not `AccountId`) so it survives `recover_account`'s account rebind exactly the way
    /// `SuspendedNullifiers` does — `recover_account` never touches `backing_commitment`'s
    /// *value* (it's checked to match the old account's on-file value, not replaced), so the
    /// underlying leaf never needs to move when only the owning `AccountId` changes. Set once
    /// at `register_citizen` and never reassigned; consulted by `revoke_citizen` to know which
    /// leaf to tombstone. Deliberately left on file after revocation (not removed) — a
    /// harmless, small, permanent audit record of which leaf a given nullifier ever occupied.
    #[pallet::storage]
    pub type BackingLeafIndexOf<T: Config> = StorageMap<_, Blake2_128Concat, [u8; 32], u32>;

    /// Current root of the backing-commitment tree (the node at `(BACKING_TREE_DEPTH, 0)`).
    /// `None` before any citizen has ever registered (equivalent to
    /// `backing_tree_zero_hash(BACKING_TREE_DEPTH)` — see `current_backing_commitment_root`,
    /// which is what callers should actually use). Kept in sync by
    /// `recompute_backing_tree_path`, the only function that ever mutates
    /// `BackingCommitmentTreeNodes`.
    #[pallet::storage]
    pub type BackingCommitmentTreeRoot<T: Config> = StorageValue<_, [u8; 32]>;

    /// FIFO history of backing-commitment tree roots, implemented as a ring over a
    /// `StorageMap` (`head`/`tail` cursors into `BackingRootHistoryEntries`) rather than a
    /// single `StorageValue<BoundedVec<..>>` — pushing a new root or pruning an old one then
    /// touches O(1) storage keys instead of re-encoding/re-decoding the entire history on
    /// every `register_citizen`/`revoke_citizen` call, which matters because
    /// `MaxBackingRootHistoryEntries` needs real headroom (see its own doc comment) and a
    /// multi-thousand-entry bounded vector would make every insert/tombstone pay to
    /// SCALE-(de)serialize the whole thing.
    ///
    /// Root history exists because a citizen's locally-cached Merkle authentication path can
    /// lag real-time insertion by other citizens: a future backing-nullifier proof (see
    /// `BackingCommitment`'s doc comment — not built yet) needs to verify membership against
    /// *some* root that was genuinely current at a recent point, not necessarily the very
    /// latest one. See `Config::BackingRootHistoryWindowBlocks` for the retention-window
    /// rationale.
    #[pallet::storage]
    pub type BackingRootHistoryEntries<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, (BlockNumberFor<T>, [u8; 32])>;

    /// Index of the oldest entry still live in `BackingRootHistoryEntries`. The queue is empty
    /// when this equals `BackingRootHistoryTail`.
    #[pallet::storage]
    pub type BackingRootHistoryHead<T: Config> = StorageValue<_, u64, ValueQuery>;

    /// Index the *next* pushed entry will occupy in `BackingRootHistoryEntries`.
    #[pallet::storage]
    pub type BackingRootHistoryTail<T: Config> = StorageValue<_, u64, ValueQuery>;

    /// O(1) membership index mirroring `BackingRootHistoryEntries`'s live contents: a root
    /// value maps to the block number after which it is no longer considered valid (i.e. the
    /// block it was pushed at, plus `BackingRootHistoryWindowBlocks`).
    /// `is_valid_backing_commitment_root` checks this map *and* re-compares against the
    /// current block, rather than trusting bare presence alone — presence here is only
    /// removed when `push_backing_root_history` next runs and happens to prune this exact
    /// entry, so a root can otherwise sit past its nominal expiry, still present, between
    /// prune passes; the current-block comparison is what makes validity correct regardless of
    /// exactly when the next prune happens to run.
    #[pallet::storage]
    pub type BackingRootValidUntil<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], BlockNumberFor<T>>;

    /// Block at which a citizen's registration lapses for voting purposes unless renewed.
    /// Pushed forward by `reverify_citizen`. Checked lazily inside `is_active_citizen`, the
    /// same point-of-use pattern already used for suspension — no separate background sweep.
    /// Always set at registration; a missing entry for an otherwise-registered citizen is
    /// treated as "past deadline" (inactive) rather than an unbounded grace period.
    #[pallet::storage]
    pub type ReverificationDeadline<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>>;

    /// Self-declared attestation, made at registration, that the citizen holds no other
    /// currently-valid passport from this deployment's country. Purely evidentiary: it exists
    /// so that, if later found false, `pallet-courts`' existing `CitizenConduct` case type has
    /// an on-chain record to point to (no new pallet-courts logic needed — see HANDOFF log
    /// #67). Deliberately not cleared on revocation: the attestation is a historical record of
    /// what was claimed at the time, not a live status flag.
    #[pallet::storage]
    pub type SelfDeclaredSingleDocument<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    // ── OPRF committee on-chain mailbox (changelog entry 82) ────────────────────

    /// Roster of which accounts belong to which OPRF committee slot (`0..NUM_COMMITTEES`).
    /// Committee members poll this (off-chain) to confirm they're authorized to answer for a
    /// given slot, and `submit_oprf_round1`/`submit_oprf_round2` check the caller's account
    /// against it directly. Governance-gated by the same `AdminOrigin` already used for
    /// `OprfCommitteeKeys` (see `add_committee_member`/`remove_committee_member`) — mirrors
    /// that storage's admin-controlled-allowlist pattern rather than inventing a new
    /// governance mechanism.
    #[pallet::storage]
    pub type CommitteeMembers<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        u8,
        BoundedVec<T::AccountId, T::MaxCommitteeSize>,
        ValueQuery,
    >;

    /// A citizen's pending blinded OPRF query, keyed by `query_id` (changelog entry 82's
    /// on-chain mailbox, replacing a relay server: a citizen posts here via
    /// `submit_oprf_query`, committee members for the assigned slot
    /// (`committee_slot_for`) poll it off-chain, and answer via
    /// `submit_oprf_round1`/`submit_oprf_round2`).
    #[pallet::storage]
    pub type PendingOprfQueries<T: Config> =
        StorageMap<_, Blake2_128Concat, u64, OprfQueryRecord<T::AccountId, BlockNumberFor<T>>>;

    /// Monotonically increasing counter assigning each `submit_oprf_query` call a fresh
    /// `query_id`.
    #[pallet::storage]
    pub type NextQueryId<T: Config> = StorageValue<_, u64, ValueQuery>;

    /// Round-1 commitments toward a `(query_id, committee_slot)`'s threshold evaluation, in
    /// submission order, `ValueQuery` (empty `BoundedVec` when nothing has been submitted yet
    /// — simpler than `Option` here since "no entry" and "empty vec" mean the same thing and
    /// every read site would otherwise have to unwrap-or-default anyway). Reaching exactly
    /// `T::OprfThreshold` entries **locks** the qualifying set: `submit_oprf_round1` rejects
    /// any further submission for that pair (`OprfRound1SetLocked`), and `submit_oprf_round2`
    /// requires this to already be at that length before accepting anything.
    #[pallet::storage]
    pub type OprfRound1Commitments<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        u64,
        Blake2_128Concat,
        u8,
        BoundedVec<OprfRound1Commitment<T::AccountId>, T::OprfThreshold>,
        ValueQuery,
    >;

    /// Round-2 responses, same shape and locking pattern as
    /// [`OprfRound1Commitments`] — see `submit_oprf_round2`.
    #[pallet::storage]
    pub type OprfRound2Responses<T: Config> = StorageDoubleMap<
        _,
        Blake2_128Concat,
        u64,
        Blake2_128Concat,
        u8,
        BoundedVec<OprfRound2Response<T::AccountId>, T::OprfThreshold>,
        ValueQuery,
    >;

    /// Count of currently-open (posted but not yet pruned) `PendingOprfQueries` entries per
    /// submitter. Incremented in `submit_oprf_query`, decremented in `prune_oprf_query` —
    /// enforces `T::MaxPendingOprfQueriesPerCitizen`.
    #[pallet::storage]
    pub type PendingOprfQueryCountBySubmitter<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// Last block at which a `recover_account` succeeded for this nullifier. The cooldown
    /// guard (`Config::MinBlocksBetweenRecoveries`) checks against this so a single citizen's
    /// identity can't be bounced between accounts in rapid succession. Abuse/griefing
    /// mitigation only, not a security boundary — see `recover_account`'s doc comment for why
    /// no dispute window backs it up instead.
    #[pallet::storage]
    pub type LastRecoveryBlock<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], BlockNumberFor<T>>;

    /// Tombstone left behind on `old_account` by a completed `recover_account` call, recording
    /// the citizen nullifier that was just moved off of it. Unlike `CitizenNullifier` (removed
    /// from `old_account` as part of that same rebind, live-registration lookup only), this
    /// entry is never removed, so cross-pallet code that recorded `old_account` at some earlier
    /// point in time — e.g. `pallet_elections::UsedBackingNullifier`, which stores the
    /// `AccountId` that originally called `back_delegate` — can still resolve which citizen
    /// `old_account` used to belong to, and from there (via `NullifierRegistry`, which always
    /// holds the *current* owner) find that citizen's present-day `AccountId` even after one or
    /// more further recoveries. See `same_citizen`'s doc comment, which is what actually
    /// consumes this.
    ///
    /// Overwritten, not appended, if `old_account` is ever recovered away from a second time
    /// (only possible if it first became a live citizen again in between, e.g. by being the
    /// `who` of some other, unrelated `recover_account` call) — the tombstone always reflects
    /// whichever citizen most recently vacated this account, which is what `same_citizen` needs.
    #[pallet::storage]
    pub type RecoveredAccountNullifier<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, [u8; 32]>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        CitizenRegistered { who: T::AccountId, nullifier: [u8; 32] },
        CitizenRevoked { who: T::AccountId },
        /// Voting rights suspended by court ruling. `until` = None means indefinite.
        CitizenSuspended { nullifier: [u8; 32], until: Option<BlockNumberFor<T>> },
        /// Voting rights restored (sentence served or conviction overturned).
        CitizenRestored { nullifier: [u8; 32] },
        MerkleRootAdded { merkle_root: [u8; 32] },
        MerkleRootRemoved { merkle_root: [u8; 32] },
        /// A citizen's registration/reverification deadline was pushed forward.
        CitizenReverified { who: T::AccountId, deadline: BlockNumberFor<T> },
        /// A citizen's identity was recovered onto a new `AccountId` after losing access to
        /// the original one (`recover_account`). `old_account` is fully invalidated as of this
        /// event — it no longer has any `CitizenNullifier`/`CitizenAnchor` entry and can no
        /// longer act as this citizen. `nullifier_hash` is included for auditability; unlike
        /// `IdentityAnchorRegistry`'s anchor value, the nullifier is already public elsewhere
        /// (`CitizenRegistered`), so including it here carries no additional
        /// vote-correlation risk.
        CitizenAccountRecovered {
            old_account: T::AccountId,
            new_account: T::AccountId,
            nullifier_hash: [u8; 32],
        },
        /// A citizen migrated their identity anchor to a new OPRF scheme version. Anchor
        /// values themselves are never included in events (see `IdentityAnchorRegistry`'s
        /// doc comment) — only the resulting scheme version is public.
        OprfAnchorMigrated { who: T::AccountId, new_scheme_version: u32 },
        /// The OPRF scheme version was bumped on the normal governance-scheduled cadence.
        OprfSchemeRotated { new_version: u32 },
        /// The OPRF scheme version was bumped out-of-cycle via `EmergencyRotationOrigin`.
        OprfSchemeEmergencyRotated { new_version: u32 },
        /// The OPRF scheme version was bumped voluntarily and early via `AdminOrigin` — not on
        /// the normal schedule and not a compromise response (see
        /// `trigger_voluntary_oprf_rotation`). `reason` is an IPFS content hash for the
        /// legislature's public rationale (mirrors `pallet_courts`' reasoning-on-IPFS pattern),
        /// so a voluntary rotation is always distinguishable on-chain from a silent one.
        OprfSchemeVoluntarilyRotated { new_version: u32, reason: [u8; 32] },
        /// A citizen recorded a self-declaration of holding no other valid passport.
        SelfDeclarationRecorded { who: T::AccountId },
        /// A governance-approved OPRF committee key was set for a `(scheme_version, slot)`
        /// pair (HANDOFF log #75).
        OprfCommitteeKeySet { scheme_version: u32, slot: u8 },
        /// A governance-approved OPRF committee key was removed.
        OprfCommitteeKeyRemoved { scheme_version: u32, slot: u8 },
        /// An account was added to an OPRF committee slot's roster (changelog entry 82).
        CommitteeMemberAdded { slot: u8, who: T::AccountId },
        /// An account was removed from an OPRF committee slot's roster.
        CommitteeMemberRemoved { slot: u8, who: T::AccountId },
        /// A citizen posted a blinded OPRF query to the on-chain mailbox.
        OprfQuerySubmitted { query_id: u64, submitter: T::AccountId },
        /// A committee member submitted their round-1 threshold-evaluation commitment for a
        /// pending query (Option B, doc 11).
        OprfRound1Submitted { query_id: u64, committee_slot: u8, member: T::AccountId },
        /// A `(query_id, committee_slot)`'s round-1 qualifying set reached `OprfThreshold`
        /// distinct members and locked — round 2 is now open for exactly this set.
        OprfRound1SetLocked { query_id: u64, committee_slot: u8 },
        /// A member of an already-locked round-1 set submitted their round-2 response.
        OprfRound2Submitted { query_id: u64, committee_slot: u8, member: T::AccountId },
        /// A dead (SLA-expired, never completed) or fully-answered (every committee slot
        /// reached `OprfThreshold` round-2 responses) OPRF mailbox entry was pruned, freeing
        /// `PendingOprfQueries`/`OprfRound1Commitments`/`OprfRound2Responses` state for
        /// `query_id`.
        OprfQueryPruned { query_id: u64, submitter: T::AccountId },
        /// A citizen's `backing_commitment` was inserted as a fresh leaf in the
        /// backing-commitment Merkle tree at `register_citizen` time. `nullifier` carries no
        /// additional vote-correlation risk beyond what's already public via
        /// `CitizenRegistered` (see that event's own doc comment on this point).
        BackingCommitmentLeafInserted { nullifier: [u8; 32], leaf_index: u32, root: [u8; 32] },
        /// A citizen's backing-commitment leaf was overwritten with the tombstone placeholder
        /// at `revoke_citizen` time.
        BackingCommitmentLeafTombstoned { nullifier: [u8; 32], leaf_index: u32, root: [u8; 32] },
    }

    #[pallet::error]
    pub enum Error<T> {
        AlreadyRegistered,
        NullifierAlreadyUsed,
        InvalidZKProof,
        /// The proof's `service_scope`/`service_subscope` public inputs don't match the
        /// domain-separation constants this specific call requires — either a proof generated
        /// for an entirely different ZKPassport-integrated service, or (see
        /// `AGORA_IDENTITY_REGISTER_SUBSCOPE`'s doc comment) a genuine same-app proof captured
        /// off a different pallet-identity call and replayed here.
        InvalidProofScope,
        NotRegistered,
        NotSuspended,
        /// The proof's certificate_registry_root is not in the on-chain allowlist of
        /// trusted issuers.
        IssuerNotAllowed,
        /// Citizen registry is full (u32::MAX citizens).
        TotalCitizensOverflow,
        /// A suspended citizen may not self-revoke to escape an active court ruling.
        CannotRevokeWhileSuspended,
        /// The identity-anchor registration proof failed verification.
        InvalidAnchorProof,
        /// This anchor is already registered under the current OPRF scheme version — either a
        /// genuine duplicate registration attempt, or a citizen who should be using
        /// `migrate_oprf_scheme` instead of `register_citizen`.
        AnchorAlreadyUsed,
        /// The reverification/liveness proof failed verification.
        InvalidReverificationProof,
        /// The anchor submitted with a reverification proof does not match the citizen's
        /// on-file anchor for their current OPRF scheme version.
        AnchorMismatch,
        /// The backing_commitment submitted with a reverification proof does not match the
        /// citizen's on-file backing_commitment.
        BackingCommitmentMismatch,
        /// The proposed new anchor is already registered under the target scheme version.
        NewAnchorAlreadyUsed,
        /// The migration consistency ("dual evaluation") proof failed verification.
        InvalidMigrationProof,
        /// OPRF scheme version counter would overflow u32::MAX.
        OprfSchemeVersionOverflow,
        /// `NextBackingLeafIndex` would overflow u32::MAX — the backing-commitment tree
        /// (`BACKING_TREE_DEPTH` = 32) has no more free leaf slots. Would require ~4.29
        /// billion never-reused leaf assignments (registrations plus tombstoned revocations
        /// combined) over the chain's whole lifetime to ever actually trigger.
        BackingLeafIndexOverflow,
        /// A committee slot outside `0..NUM_COMMITTEES` was given to
        /// `set_oprf_committee_key`/`remove_oprf_committee_key`.
        InvalidCommitteeSlot,
        /// One of the 5 submitted `oprf_pk_hashes` does not match the governance-approved
        /// key on file for that committee slot and OPRF scheme version (or no key has been
        /// set for that slot at all yet).
        CommitteeKeyMismatch,
        /// The outer proof's `current_date` public input is malformed (does not fit in a
        /// `u64`, i.e. is not a well-formed field-encoded timestamp).
        MalformedProofDate,
        /// The outer proof's `current_date` is older than `MaxAnchorProofAge` relative to
        /// the current chain time — an old-but-not-yet-passport-expired proof cannot be
        /// replayed indefinitely.
        AnchorProofStale,
        /// The outer proof's `current_date` is further in the future than
        /// `MaxAnchorProofClockSkew` relative to the current chain time. `current_date` is a
        /// prover-controlled public input; without this check a future-dated value would make
        /// `AnchorProofStale`'s `saturating_sub` clamp to zero and pass unconditionally.
        AnchorProofFuture,
        /// The account is already present in this committee slot's roster.
        CommitteeMemberAlreadyPresent,
        /// The account is not present in this committee slot's roster.
        CommitteeMemberNotPresent,
        /// This committee slot's roster is already at `MaxCommitteeSize`.
        CommitteeRosterFull,
        /// `NextQueryId` counter would overflow u64::MAX.
        QueryIdOverflow,
        /// No pending OPRF query exists for this `query_id` (never submitted, or already
        /// cleaned up).
        QueryNotFound,
        /// This query's `OprfQuerySlaBlocks` response window has passed.
        QueryExpired,
        /// The caller is not a registered member of `CommitteeMembers` for the given
        /// committee slot.
        NotCommitteeMember,
        /// The caller already submitted a round-1 or round-2 message for this
        /// `(query_id, committee_slot)`.
        DuplicateResponse,
        /// `submit_oprf_round1`: this `(query_id, committee_slot)`'s qualifying set already
        /// has `OprfThreshold` members — no further round-1 submissions are accepted (a
        /// correct client should watch for `OprfRound1SetLocked` and retry against a fresh
        /// query if it arrives too late to join).
        OprfRound1SetLocked,
        /// `submit_oprf_round2`: this `(query_id, committee_slot)`'s round-1 set has not yet
        /// reached `OprfThreshold` members — round 2 is not open yet.
        OprfRound1NotLocked,
        /// `submit_oprf_round2`: the caller did not submit an accepted round-1 commitment for
        /// this `(query_id, committee_slot)`, so they are not part of its (now locked)
        /// qualifying set and may not submit a round-2 response for it.
        NotInLockedSet,
        /// The caller already has `T::MaxPendingOprfQueriesPerCitizen` open (un-pruned)
        /// `PendingOprfQueries` entries.
        TooManyPendingOprfQueries,
        /// `prune_oprf_query`: this query is neither past its `OprfQuerySlaBlocks` deadline
        /// nor fully answered (every committee slot at `T::OprfThreshold` round-2 responses),
        /// so it is still potentially live and must not be pruned yet.
        QueryNotPrunable,
        /// `prune_oprf_query`: this query is fully answered but not yet past its
        /// `OprfQuerySlaBlocks` deadline, so only `query.submitter` — the citizen whose
        /// off-chain client still needs to read and combine the round-1/round-2 data — may
        /// prune it. A third party could otherwise delete that data out from under the
        /// submitter's client before it has had a chance to poll it. Once the deadline has
        /// passed, no legitimate read can still be pending, so anyone may prune (see
        /// `expired` handling above this check).
        NotQuerySubmitter,
        /// `recover_account`: no existing citizen is registered under this proof's nullifier —
        /// there is nothing to recover. Use `register_citizen` for a genuinely new identity
        /// instead.
        NoExistingRegistrationForNullifier,
        /// `recover_account`: `Config::MinBlocksBetweenRecoveries` has not yet passed since the
        /// last successful recovery for this nullifier.
        RecoveryCooldownActive,
        /// `recover_account`: `old_account` holds a nonzero AGR balance. Recovery would strand
        /// it on an account this call is about to fully invalidate — transfer it out (e.g. to
        /// the new account) first, then recover.
        RecoveryBlockedNonzeroBalance,
        /// `recover_account`: `old_account` is a registered delegate persona
        /// (`pallet-elections`). Deregister/resign the delegate persona first, then recover.
        RecoveryBlockedActiveDelegate,
        /// `recover_account`: `old_account` currently holds a legislature seat
        /// (`pallet-legislature`). The seat cannot follow an account rebind — resign first,
        /// then recover.
        RecoveryBlockedLegislatureSeat,
        /// `recover_account`: `old_account` currently holds a cabinet role — Prime Minister or
        /// a portfolio minister (`pallet-executive`). Resign the role first, then recover.
        RecoveryBlockedCabinetRole,
        /// `recover_account`: `old_account` has cast a direct vote in a referendum that is
        /// still open (`pallet-voting`, `ReferendumState::Voting`). Rebinding without this
        /// guard would let the same citizen vote again on the same still-open referendum under
        /// the new account. Wait for the referendum to finalize, then recover.
        RecoveryBlockedOpenReferendumVote,
        /// `recover_account`: `old_account` has already claimed its quadratic budget
        /// allocation (`pallet-voting`) for the currently active fiscal epoch. Rebinding
        /// without this guard would let the same citizen claim a second full allocation under
        /// the new account within the same epoch. Wait for the next fiscal epoch, then recover.
        RecoveryBlockedUnclaimedEpochBudget,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Register a new citizen using a ZKPassport passport proof.
        ///
        /// `public_inputs` is ZKPassport's `main/outer/count_N` layout — see
        /// `runtime/src/verifier.rs`'s module docs for the authoritative field-by-field
        /// breakdown (confirmed against the circuits repo source, not assumed). In short:
        /// `certificate_registry_root` is index 0, `scoped_nullifier` is index `len - 2`
        /// (`6 + D`, where `D = outer_count - 3` disclosure subproofs, so the exact index
        /// shifts with which outer-circuit variant produced the proof — deriving it from
        /// `len` rather than hardcoding it is what makes this work across variants).
        /// **Not** Rarimo's old 5-signal layout (`dg15PubKeyHash`, `passportHash`,
        /// `dg1Commitment`, `pkIdentityHash`, `slaveMerkleRoot`) this call used to assume.
        ///
        /// `anchor`/`oprf_pk_hashes`: the mandatory OPRF identity anchor (HANDOFF log #67,
        /// `AnchorProofVerifier`) and the 5 per-committee key hashes it was derived under
        /// (changelog entry 73's 5-committee topology). There is no separate anchor SNARK
        /// proof parameter (unlike the pre-log-#74 design this replaces): the OPRF
        /// identity-anchor circuit (`disclosure`) rides inside `zk_proof` itself as a
        /// recursively-verified subproof, so `zk_proof`/`public_inputs` above are the only
        /// proof this call needs — see `runtime/src/anchor_verifier.rs`'s module docs. The
        /// anchor check is still applied in addition to, and entirely separately from, the
        /// nullifier check: a duplicate anchor is rejected here even if the passport proof
        /// itself is perfectly valid, since the whole point of the anchor is to catch
        /// same-person double registration that a fresh, renewal-stable-nullifier-free
        /// passport proof alone cannot.
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn register_citizen(
            origin: OriginFor<T>,
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            // count_N exposes N + 5 public inputs; count_13 (the largest allowlisted
            // variant) is the ceiling at 18.
            public_inputs: BoundedVec<[u8; 32], ConstU32<18>>,
            anchor: [u8; 32],
            oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES as usize],
            // See `BackingCommitment`'s storage doc comment: a second, independent public
            // value folded into the same `disclosure`-circuit `param_commitment` preimage as
            // `anchor`, checked and stored the same way.
            backing_commitment: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!CitizenNullifier::<T>::contains_key(&who), Error::<T>::AlreadyRegistered);

            // 1-2. Allowlist + proof-scope + ZK proof verification (expensive BN254 pairing) —
            //    see `Self::verify_outer_proof`.
            Self::verify_outer_proof(
                zk_proof.as_slice(),
                public_inputs.as_slice(),
                AGORA_IDENTITY_REGISTER_SUBSCOPE,
            )?;

            // 3. Only after the proof is authenticated, extract and check the nullifier.
            //    Checking nullifier uniqueness before proof verification would let an attacker
            //    learn whether a nullifier is registered without submitting a valid proof.
            //    scoped_nullifier sits at index `6 + D` = `len - 2` (oprf_pk_hash is the
            //    last field, at `len - 1`).
            let nullifier = public_inputs[public_inputs.len() - 2];
            ensure!(
                !NullifierRegistry::<T>::contains_key(nullifier),
                Error::<T>::NullifierAlreadyUsed
            );

            // 4. Freshness (HANDOFF log #75) — see `Self::check_outer_proof_freshness`.
            Self::check_outer_proof_freshness(public_inputs.as_slice())?;

            // 5. Each of the 5 committee key hashes must match the governance-approved key
            //    for its slot under the current scheme version — see
            //    `circuits/oprf-identity-anchor/README.md`'s "What the Rust verifier still
            //    has to enforce", obligation 1. Checked before the (cheaper, but still
            //    proof-independent) Poseidon2 recomputation below so a mismatched committee
            //    key fails on the more specific error.
            let scheme_version = OprfSchemeVersion::<T>::get();
            Self::check_committee_keys(scheme_version, &oprf_pk_hashes)?;

            // 6. Mandatory identity-anchor check (HANDOFF log #67/#75). Same ordering
            //    rationale as the nullifier above: verify the anchor before consulting the
            //    exclusion registry, so an attacker can't probe anchor uniqueness with a
            //    bogus (anchor, oprf_pk_hashes) tuple.
            ensure!(
                T::AnchorVerifier::verify_registration_anchor(
                    public_inputs.as_slice(),
                    anchor,
                    scheme_version,
                    oprf_pk_hashes,
                    backing_commitment,
                ),
                Error::<T>::InvalidAnchorProof
            );
            ensure!(
                !IdentityAnchorRegistry::<T>::contains_key((scheme_version, anchor)),
                Error::<T>::AnchorAlreadyUsed
            );

            let pos = TotalCitizens::<T>::get();
            let new_total = pos.checked_add(1).ok_or(Error::<T>::TotalCitizensOverflow)?;
            CitizenIndex::<T>::insert(pos, &who);
            CitizenPosition::<T>::insert(&who, pos);
            TotalCitizens::<T>::put(new_total);
            CitizenNullifier::<T>::insert(&who, nullifier);
            NullifierRegistry::<T>::insert(nullifier, &who);
            IdentityAnchorRegistry::<T>::insert((scheme_version, anchor), &who);
            CitizenAnchor::<T>::insert(&who, (scheme_version, anchor));
            BackingCommitment::<T>::insert(&who, backing_commitment);
            // Insert this citizen's backing_commitment as a fresh leaf in the on-chain
            // incremental Merkle tree (see the "Backing-commitment incremental Merkle tree"
            // storage section above). Must run after every other uniqueness/anchor check
            // above has already succeeded — a rejected registration must not consume a leaf
            // index.
            Self::insert_backing_leaf(nullifier, backing_commitment)?;
            let deadline = frame_system::Pallet::<T>::block_number()
                .saturating_add(BlockNumberFor::<T>::from(T::ReverificationPeriod::get()));
            ReverificationDeadline::<T>::insert(&who, deadline);
            Self::deposit_event(Event::CitizenRegistered { who, nullifier });
            Ok(())
        }

        /// Revoke a citizen registration (e.g. country removed from allowlist).
        /// Blocked while the citizen has an active or unexpired suspension — a suspended
        /// citizen cannot self-revoke to escape an active court ruling.
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn revoke_citizen(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let nullifier =
                CitizenNullifier::<T>::get(&who).ok_or(Error::<T>::NotRegistered)?;
            // Block self-revocation if the citizen is currently suspended (including
            // time-limited suspensions whose block hasn't passed yet).
            if let Some(entry) = SuspendedNullifiers::<T>::get(nullifier) {
                let is_active_suspension = match entry {
                    None => true, // indefinite
                    Some(until) => frame_system::Pallet::<T>::block_number() <= until,
                };
                ensure!(!is_active_suspension, Error::<T>::CannotRevokeWhileSuspended);
            }
            CitizenNullifier::<T>::remove(&who);
            NullifierRegistry::<T>::remove(nullifier);
            Self::clear_suspension(nullifier);
            ReverificationDeadline::<T>::remove(&who);
            // Retire the citizen's identity anchor too — a fresh registration under a new
            // anchor is expected to go through the normal exclusion check again. Note:
            // `SelfDeclaredSingleDocument` is deliberately NOT cleared here — see its storage
            // doc comment for why (it's a historical attestation record, not a live flag).
            if let Some((version, anchor)) = CitizenAnchor::<T>::take(&who) {
                IdentityAnchorRegistry::<T>::remove((version, anchor));
            }
            BackingCommitment::<T>::remove(&who);
            // Overwrite this citizen's Merkle-tree leaf with the tombstone placeholder — a
            // revoked citizen must no longer be provable as a member of the backing-commitment
            // tree. Suspension (`suspend_citizen`/`restore_citizen_rights`) deliberately does
            // NOT do this: it's reversible and keyed by nullifier independent of
            // `BackingCommitment`, and a merely-suspended citizen is still a registered
            // citizen. Only outright revocation removes `BackingCommitment` (above), so only
            // revocation tombstones the tree leaf.
            Self::tombstone_backing_leaf(nullifier);
            // Swap-and-pop: fill the vacated slot with the last citizen to keep the index dense.
            let pos = CitizenPosition::<T>::take(&who).ok_or(Error::<T>::NotRegistered)?;
            let last = TotalCitizens::<T>::get().saturating_sub(1);
            TotalCitizens::<T>::put(last);
            if pos < last {
                if let Some(swapped) = CitizenIndex::<T>::get(last) {
                    CitizenIndex::<T>::insert(pos, &swapped);
                    CitizenPosition::<T>::insert(&swapped, pos);
                }
            }
            CitizenIndex::<T>::remove(last);
            Self::deposit_event(Event::CitizenRevoked { who });
            Ok(())
        }

        /// Suspend a citizen's voting and budget-allocation rights by court order.
        /// `until`: None = indefinite suspension; Some(block) = suspension lifts at that block.
        /// If the citizen is already suspended, the existing record is replaced (allows courts
        /// to extend or modify an active suspension).
        /// Origin: `SuspensionOrigin` — a manual Oracle Council override requiring the
        /// council's M-of-N approval of this exact call, not a single member (see
        /// `Config::SuspensionOrigin`).
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn suspend_citizen(
            origin: OriginFor<T>,
            nullifier: [u8; 32],
            until: Option<BlockNumberFor<T>>,
        ) -> DispatchResult {
            T::SuspensionOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::suspend_citizen", (nullifier, until)),
            )?;
            ensure!(NullifierRegistry::<T>::contains_key(nullifier), Error::<T>::NotRegistered);
            // Upsert: courts may extend or modify an existing suspension.
            SuspendedNullifiers::<T>::insert(nullifier, until);
            // This extrinsic is `SuspensionOrigin`-gated (the Oracle Council's M-of-N approval,
            // see `Config::SuspensionOrigin`'s doc comment) — no jury is ever involved on this
            // path, so it's never eligible to trigger a higher-bar consequence like automatic
            // office removal.
            SuspendedByJuryReview::<T>::insert(nullifier, false);
            Self::deposit_event(Event::CitizenSuspended { nullifier, until });
            Ok(())
        }

        /// Restore suspended voting rights.
        /// Called when a sentence is served, the waiting period passes, or a conviction is
        /// overturned on appeal. Works on both active and expired-but-not-yet-cleaned-up records.
        /// Origin: `SuspensionOrigin` — a manual Oracle Council override requiring the
        /// council's M-of-N approval of this exact call, not a single member (see
        /// `Config::SuspensionOrigin`).
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn restore_citizen_rights(
            origin: OriginFor<T>,
            nullifier: [u8; 32],
        ) -> DispatchResult {
            T::SuspensionOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::restore_citizen_rights", nullifier),
            )?;
            ensure!(
                SuspendedNullifiers::<T>::contains_key(nullifier),
                Error::<T>::NotSuspended
            );
            Self::clear_suspension(nullifier);
            Self::deposit_event(Event::CitizenRestored { nullifier });
            Ok(())
        }

        /// Add a trusted issuer Merkle root. Proofs with this slaveMerkleRoot will be accepted.
        /// The root represents a specific snapshot of the trusted country CA certificate set.
        #[pallet::call_index(4)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn add_allowed_merkle_root(
            origin: OriginFor<T>,
            merkle_root: [u8; 32],
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::add_allowed_merkle_root", merkle_root),
            )?;
            AllowedMerkleRoots::<T>::insert(merkle_root, true);
            Self::deposit_event(Event::MerkleRootAdded { merkle_root });
            Ok(())
        }

        /// Remove a trusted issuer Merkle root (e.g. after a CA revocation event).
        #[pallet::call_index(5)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn remove_allowed_merkle_root(
            origin: OriginFor<T>,
            merkle_root: [u8; 32],
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::remove_allowed_merkle_root", merkle_root),
            )?;
            AllowedMerkleRoots::<T>::remove(merkle_root);
            Self::deposit_event(Event::MerkleRootRemoved { merkle_root });
            Ok(())
        }

        /// Periodic re-verification (HANDOFF log #67, restructured in log #76): proves the
        /// caller still holds a currently-valid, unexpired passport that recomputes to the
        /// same anchor already on file, and pushes `ReverificationDeadline` forward by
        /// `ReverificationPeriod` blocks from now. Checked lazily inside `is_active_citizen`
        /// rather than via a background sweep, matching this pallet's existing suspension
        /// pattern.
        ///
        /// `zk_proof`/`public_inputs` is a fresh outer ZKPassport proof — the same shape
        /// `register_citizen` takes, and for the same reason (log #76): a standalone
        /// reverification proof's `comm_in` would be an unauthenticated private witness,
        /// unable to prove the passport is *currently* valid rather than merely was at
        /// original registration. `anchor`/`oprf_pk_hashes` must recompute (via
        /// `T::AnchorVerifier::verify_reverification`, riding the `disclosure` subproof
        /// folded into `zk_proof`) to the same anchor already on file — checked directly here
        /// via `CitizenAnchor`, not left to the proof alone, so a citizen cannot "reverify"
        /// into a different anchor than the one their account is registered under.
        #[pallet::call_index(6)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn reverify_citizen(
            origin: OriginFor<T>,
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            public_inputs: BoundedVec<[u8; 32], ConstU32<18>>,
            anchor: [u8; 32],
            oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES as usize],
            // Must match `BackingCommitment`'s on-file value — see
            // `Error::BackingCommitmentMismatch` and that storage item's doc comment. Both
            // `anchor` and `backing_commitment` are deterministic functions of the same
            // committee evaluation, so a genuine reverification proof always recomputes to the
            // exact same values already on file.
            backing_commitment: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let (scheme_version, on_file_anchor) =
                CitizenAnchor::<T>::get(&who).ok_or(Error::<T>::NotRegistered)?;
            ensure!(anchor == on_file_anchor, Error::<T>::AnchorMismatch);
            let on_file_backing_commitment =
                BackingCommitment::<T>::get(&who).ok_or(Error::<T>::NotRegistered)?;
            ensure!(
                backing_commitment == on_file_backing_commitment,
                Error::<T>::BackingCommitmentMismatch
            );

            Self::verify_outer_proof(
                zk_proof.as_slice(),
                public_inputs.as_slice(),
                AGORA_IDENTITY_REVERIFY_SUBSCOPE,
            )?;
            Self::check_outer_proof_freshness(public_inputs.as_slice())?;
            Self::check_committee_keys(scheme_version, &oprf_pk_hashes)?;

            ensure!(
                T::AnchorVerifier::verify_reverification(
                    public_inputs.as_slice(),
                    anchor,
                    scheme_version,
                    oprf_pk_hashes,
                    backing_commitment,
                ),
                Error::<T>::InvalidReverificationProof
            );
            let deadline = frame_system::Pallet::<T>::block_number()
                .saturating_add(BlockNumberFor::<T>::from(T::ReverificationPeriod::get()));
            ReverificationDeadline::<T>::insert(&who, deadline);
            Self::deposit_event(Event::CitizenReverified { who, deadline });
            Ok(())
        }

        /// OPRF scheme migration (HANDOFF log #67's "dual evaluation", restructured in log
        /// #76): moves the caller's own identity anchor from their current on-file scheme
        /// version to the next one (on-file version + 1), given a proof that `old_anchor` and
        /// `new_anchor` were both derived from the same underlying personal-number value.
        /// Requires only the citizen's current passport — no second document, per log #67.
        ///
        /// `zk_proof`/`public_inputs` is a fresh outer ZKPassport proof carrying the
        /// `migrate-disclosure` subproof (log #76 — same rationale as `reverify_citizen`
        /// above: a standalone `migrate` proof's `comm_in` is unauthenticated, and migration
        /// requires proving the passport is currently valid too, not just proving anchor
        /// continuity). `old_anchor` is no longer a caller-supplied parameter — it is read
        /// directly from the citizen's own `CitizenAnchor` entry, which also removes the need
        /// for a separate "does this old_anchor genuinely belong to the caller" check that a
        /// caller-supplied value would have required.
        ///
        /// Note on versioning: the migration target is always the *caller's own* on-file
        /// version plus one, not necessarily the global `OprfSchemeVersion`. This lets citizens
        /// migrate incrementally and in any order during a rotation window rather than all
        /// needing to act atomically with a global cutover; a citizen who has missed more than
        /// one rotation simply needs to call this once per generation to catch up.
        #[pallet::call_index(7)]
        #[pallet::weight(Weight::from_parts(25_000, 0))]
        pub fn migrate_oprf_scheme(
            origin: OriginFor<T>,
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            public_inputs: BoundedVec<[u8; 32], ConstU32<18>>,
            new_anchor: [u8; 32],
            old_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES as usize],
            new_oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES as usize],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let (old_version, old_anchor) =
                CitizenAnchor::<T>::get(&who).ok_or(Error::<T>::NotRegistered)?;
            let new_version =
                old_version.checked_add(1).ok_or(Error::<T>::OprfSchemeVersionOverflow)?;

            // Verify the proof (and committee keys) before consulting the anchor-registry
            // exclusion check below — same ordering rationale as `register_citizen` (see its
            // own doc comment on this point): checking `NewAnchorAlreadyUsed` first would let
            // an attacker submit a bogus `zk_proof` with a guessed `new_anchor` and learn, from
            // the returned error alone and at zero real proof-computation cost, whether that
            // `(new_version, new_anchor)` pair already belongs to another citizen — leaking
            // cross-citizen anchor-registry membership ahead of proof authentication.
            Self::verify_outer_proof(
                zk_proof.as_slice(),
                public_inputs.as_slice(),
                AGORA_IDENTITY_MIGRATE_SUBSCOPE,
            )?;
            Self::check_outer_proof_freshness(public_inputs.as_slice())?;
            Self::check_committee_keys(old_version, &old_oprf_pk_hashes)?;
            Self::check_committee_keys(new_version, &new_oprf_pk_hashes)?;

            ensure!(
                T::AnchorVerifier::verify_migration(
                    public_inputs.as_slice(),
                    old_anchor,
                    new_anchor,
                    old_version,
                    new_version,
                    old_oprf_pk_hashes,
                    new_oprf_pk_hashes,
                ),
                Error::<T>::InvalidMigrationProof
            );

            ensure!(
                !IdentityAnchorRegistry::<T>::contains_key((new_version, new_anchor)),
                Error::<T>::NewAnchorAlreadyUsed
            );

            IdentityAnchorRegistry::<T>::remove((old_version, old_anchor));
            IdentityAnchorRegistry::<T>::insert((new_version, new_anchor), &who);
            CitizenAnchor::<T>::insert(&who, (new_version, new_anchor));
            Self::deposit_event(Event::OprfAnchorMigrated { who, new_scheme_version: new_version });
            Ok(())
        }

        /// Normal, scheduled OPRF scheme-version bump (the 4-year cycle from HANDOFF log #67).
        /// Not explicitly enumerated as a required call in this feature's task description —
        /// added because without *some* non-emergency way to advance `OprfSchemeVersion`, the
        /// scheduled rotation log #67 describes could never actually happen. Gated by the same
        /// `AdminOrigin` already used for this pallet's other governance-controlled action
        /// (the Merkle root allowlist), rather than introducing a new Config field for it.
        #[pallet::call_index(8)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn rotate_oprf_scheme(origin: OriginFor<T>) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::rotate_oprf_scheme", ()),
            )?;
            let new_version = Self::do_bump_scheme_version()?;
            Self::deposit_event(Event::OprfSchemeRotated { new_version });
            Ok(())
        }

        /// Emergency, out-of-cycle OPRF scheme-version bump (HANDOFF log #67): lets
        /// `EmergencyRotationOrigin` (intended to be pallet-emergency-council) force a rotation
        /// ahead of the normal 4-year schedule, e.g. on a suspected OPRF committee compromise.
        #[pallet::call_index(9)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn emergency_rotate_oprf_scheme(origin: OriginFor<T>) -> DispatchResult {
            T::EmergencyRotationOrigin::ensure_origin(origin)?;
            let new_version = Self::do_bump_scheme_version()?;
            Self::deposit_event(Event::OprfSchemeEmergencyRotated { new_version });
            Ok(())
        }

        /// Voluntary, early OPRF scheme-version bump initiated by governance for a reason other
        /// than a suspected compromise (closes the gap changelog entry 82's "Still open" section
        /// flagged: "Neither [existing rotation path] fits 'move from phones to dedicated
        /// servers on our own schedule, nothing's wrong'"). Distinct from both
        /// `rotate_oprf_scheme` (the normal scheduled cadence) and `emergency_rotate_oprf_scheme`
        /// (`EmergencyRotationOrigin`, compromise-triggered): same `AdminOrigin`/`legislature_call_hash`
        /// gating pattern as `rotate_oprf_scheme` (matching how `pallet_voting::open_voting_epoch`
        /// gates its own `LegislatureOrigin`-equivalent action), but its own call, event, and a
        /// mandatory `reason`.
        ///
        /// `reason` is an IPFS content hash for the legislature's public rationale — mirrors
        /// `pallet_courts`' pattern of keeping reasoning off-chain-but-hash-anchored rather than
        /// storing free text in this `no_std` pallet. It is required, not optional, specifically
        /// so a voluntary rotation is always distinguishable after the fact from a silent one:
        /// anyone auditing `OprfSchemeVoluntarilyRotated` can always resolve *why* governance
        /// chose to rotate early, without confusing it for a scheduled or compromise-driven bump.
        #[pallet::call_index(19)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn trigger_voluntary_oprf_rotation(
            origin: OriginFor<T>,
            reason: [u8; 32],
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::trigger_voluntary_oprf_rotation", reason),
            )?;
            let new_version = Self::do_bump_scheme_version()?;
            Self::deposit_event(Event::OprfSchemeVoluntarilyRotated { new_version, reason });
            Ok(())
        }

        /// Self-declaration (HANDOFF log #67): a registered citizen attests, on-chain, that
        /// they hold no other currently-valid passport from this deployment's country.
        /// Deliberately trivial — just an attestation record for later `pallet-courts`
        /// `CitizenConduct` reference if it's ever found false. Idempotent: re-declaring simply
        /// re-affirms the same flag.
        #[pallet::call_index(10)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn declare_no_other_passport(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_citizen(&who), Error::<T>::NotRegistered);
            SelfDeclaredSingleDocument::<T>::insert(&who, true);
            Self::deposit_event(Event::SelfDeclarationRecorded { who });
            Ok(())
        }

        /// Set (or replace) the governance-approved OPRF committee public-key hash for a
        /// `(scheme_version, slot)` pair (HANDOFF log #75). Mirrors
        /// `add_allowed_merkle_root`'s upsert semantics and `AdminOrigin` gating.
        #[pallet::call_index(11)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn set_oprf_committee_key(
            origin: OriginFor<T>,
            scheme_version: u32,
            slot: u8,
            oprf_pk_hash: [u8; 32],
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(
                    b"pallet-identity::set_oprf_committee_key",
                    (scheme_version, slot, oprf_pk_hash),
                ),
            )?;
            ensure!(slot < NUM_COMMITTEES, Error::<T>::InvalidCommitteeSlot);
            OprfCommitteeKeys::<T>::insert((scheme_version, slot), oprf_pk_hash);
            Self::deposit_event(Event::OprfCommitteeKeySet { scheme_version, slot });
            Ok(())
        }

        /// Remove a governance-approved OPRF committee key (e.g. after a suspected
        /// per-committee compromise, ahead of an emergency rotation).
        #[pallet::call_index(12)]
        #[pallet::weight(Weight::from_parts(8_000, 0))]
        pub fn remove_oprf_committee_key(
            origin: OriginFor<T>,
            scheme_version: u32,
            slot: u8,
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(
                    b"pallet-identity::remove_oprf_committee_key",
                    (scheme_version, slot),
                ),
            )?;
            ensure!(slot < NUM_COMMITTEES, Error::<T>::InvalidCommitteeSlot);
            OprfCommitteeKeys::<T>::remove((scheme_version, slot));
            Self::deposit_event(Event::OprfCommitteeKeyRemoved { scheme_version, slot });
            Ok(())
        }

        /// Add an account to an OPRF committee slot's roster (changelog entry 82's on-chain
        /// mailbox design). Mirrors `set_oprf_committee_key`'s `AdminOrigin` gating; rejects
        /// (rather than silently no-oping) a duplicate add or a roster already at
        /// `MaxCommitteeSize`.
        #[pallet::call_index(13)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn add_committee_member(
            origin: OriginFor<T>,
            slot: u8,
            who: T::AccountId,
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::add_committee_member", (slot, who.clone())),
            )?;
            ensure!(slot < NUM_COMMITTEES, Error::<T>::InvalidCommitteeSlot);
            CommitteeMembers::<T>::try_mutate(slot, |members| -> DispatchResult {
                ensure!(!members.contains(&who), Error::<T>::CommitteeMemberAlreadyPresent);
                members.try_push(who.clone()).map_err(|_| Error::<T>::CommitteeRosterFull)?;
                Ok(())
            })?;
            Self::deposit_event(Event::CommitteeMemberAdded { slot, who });
            Ok(())
        }

        /// Remove an account from an OPRF committee slot's roster (e.g. member rotation).
        #[pallet::call_index(14)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn remove_committee_member(
            origin: OriginFor<T>,
            slot: u8,
            who: T::AccountId,
        ) -> DispatchResult {
            T::AdminOrigin::ensure_origin(
                origin,
                &legislature_call_hash(b"pallet-identity::remove_committee_member", (slot, who.clone())),
            )?;
            ensure!(slot < NUM_COMMITTEES, Error::<T>::InvalidCommitteeSlot);
            CommitteeMembers::<T>::try_mutate(slot, |members| -> DispatchResult {
                let pos = members
                    .iter()
                    .position(|m| m == &who)
                    .ok_or(Error::<T>::CommitteeMemberNotPresent)?;
                members.remove(pos);
                Ok(())
            })?;
            Self::deposit_event(Event::CommitteeMemberRemoved { slot, who });
            Ok(())
        }

        /// Post a blinded OPRF query to the on-chain mailbox (changelog entry 82, replacing a
        /// relay server). Any registered citizen may submit — mirrors
        /// `declare_no_other_passport`'s `is_citizen` gate. Assigns and emits (via
        /// `OprfQuerySubmitted`) a fresh `query_id`; committee members for the query's target
        /// slot (derived off-chain via `committee_slot_for`) poll `PendingOprfQueries` and
        /// answer with `submit_oprf_response`.
        #[pallet::call_index(15)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn submit_oprf_query(
            origin: OriginFor<T>,
            blinded_query: [u8; 64],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(Self::is_citizen(&who), Error::<T>::NotRegistered);
            let open_count = PendingOprfQueryCountBySubmitter::<T>::get(&who);
            ensure!(
                open_count < T::MaxPendingOprfQueriesPerCitizen::get(),
                Error::<T>::TooManyPendingOprfQueries
            );
            let query_id = NextQueryId::<T>::get();
            let next_id = query_id.checked_add(1).ok_or(Error::<T>::QueryIdOverflow)?;
            NextQueryId::<T>::put(next_id);
            PendingOprfQueries::<T>::insert(
                query_id,
                OprfQueryRecord {
                    submitter: who.clone(),
                    blinded_query,
                    posted_at: frame_system::Pallet::<T>::block_number(),
                },
            );
            PendingOprfQueryCountBySubmitter::<T>::insert(&who, open_count.saturating_add(1));
            Self::deposit_event(Event::OprfQuerySubmitted { query_id, submitter: who });
            Ok(())
        }

        /// Round 1 of a genuine `t`-of-`n` threshold OPRF evaluation (Option B, doc 11): a
        /// committee member broadcasts their partial evaluation of a pending query plus
        /// FROST-style nonce commitments (see `OprfRound1Commitment`'s doc comment for the
        /// field-by-field meaning and `oprf-committee-dev/src/threshold.rs` for the full
        /// protocol). Checks, in order: slot valid; query exists and hasn't expired; caller is
        /// on file in `CommitteeMembers` for that slot; the qualifying set isn't already at
        /// `OprfThreshold`; the caller hasn't already submitted for this pair. Performs **no**
        /// cryptographic verification of the submitted points — see `OprfRound1Commitment`'s
        /// doc comment for why that's a deliberate, documented scope boundary, not an
        /// oversight. Once this is the `OprfThreshold`-th accepted commitment for this pair,
        /// the qualifying set locks (`OprfRound1SetLocked`) and no further round-1 submissions
        /// are accepted for it.
        #[pallet::call_index(16)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn submit_oprf_round1(
            origin: OriginFor<T>,
            query_id: u64,
            committee_slot: u8,
            r_i: [u8; 64],
            d_g: [u8; 64],
            d_q: [u8; 64],
            e_g: [u8; 64],
            e_q: [u8; 64],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(committee_slot < NUM_COMMITTEES, Error::<T>::InvalidCommitteeSlot);

            let query = PendingOprfQueries::<T>::get(query_id).ok_or(Error::<T>::QueryNotFound)?;
            let deadline = query
                .posted_at
                .saturating_add(BlockNumberFor::<T>::from(T::OprfQuerySlaBlocks::get()));
            ensure!(
                frame_system::Pallet::<T>::block_number() <= deadline,
                Error::<T>::QueryExpired
            );

            let members = CommitteeMembers::<T>::get(committee_slot);
            ensure!(members.contains(&who), Error::<T>::NotCommitteeMember);

            let locked = OprfRound1Commitments::<T>::try_mutate(
                query_id,
                committee_slot,
                |commitments| -> Result<bool, DispatchError> {
                    ensure!(
                        (commitments.len() as u32) < T::OprfThreshold::get(),
                        Error::<T>::OprfRound1SetLocked
                    );
                    ensure!(
                        !commitments.iter().any(|c| c.member == who),
                        Error::<T>::DuplicateResponse
                    );
                    commitments
                        .try_push(OprfRound1Commitment { member: who.clone(), r_i, d_g, d_q, e_g, e_q })
                        .map_err(|_| Error::<T>::OprfRound1SetLocked)?;
                    Ok((commitments.len() as u32) == T::OprfThreshold::get())
                },
            )?;

            Self::deposit_event(Event::OprfRound1Submitted { query_id, committee_slot, member: who });
            if locked {
                Self::deposit_event(Event::OprfRound1SetLocked { query_id, committee_slot });
            }
            Ok(())
        }

        /// Round 2: a member of an already-locked round-1 qualifying set submits their
        /// response scalar (`OprfRound2Response::z_i` —
        /// `oprf-committee-dev::threshold::round2_response`'s output). Requires round 1 to
        /// already be locked (exactly `OprfThreshold` commitments) and the caller to be one of
        /// its members. No cryptographic verification here either — see
        /// `OprfRound1Commitment`'s doc comment; the same scope boundary applies to round 2.
        /// Once every member of the locked set has submitted, the citizen's own client
        /// combines the round-1/round-2 data into the final proof
        /// (`oprf-committee-dev::threshold::combine_evaluations`/`combine_responses`) exactly
        /// as it already assembles every other part of the registration witness — this pallet
        /// does not compute or store that combination itself.
        #[pallet::call_index(17)]
        #[pallet::weight(Weight::from_parts(15_000, 0))]
        pub fn submit_oprf_round2(
            origin: OriginFor<T>,
            query_id: u64,
            committee_slot: u8,
            z_i: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(committee_slot < NUM_COMMITTEES, Error::<T>::InvalidCommitteeSlot);

            let round1 = OprfRound1Commitments::<T>::get(query_id, committee_slot);
            ensure!(
                (round1.len() as u32) == T::OprfThreshold::get(),
                Error::<T>::OprfRound1NotLocked
            );
            ensure!(round1.iter().any(|c| c.member == who), Error::<T>::NotInLockedSet);

            OprfRound2Responses::<T>::try_mutate(
                query_id,
                committee_slot,
                |responses| -> DispatchResult {
                    ensure!(
                        !responses.iter().any(|r| r.member == who),
                        Error::<T>::DuplicateResponse
                    );
                    responses
                        .try_push(OprfRound2Response { member: who.clone(), z_i })
                        .map_err(|_| Error::<T>::OprfRound1SetLocked)?;
                    Ok(())
                },
            )?;

            Self::deposit_event(Event::OprfRound2Submitted { query_id, committee_slot, member: who });
            Ok(())
        }

        /// Prune a dead or fully-answered OPRF mailbox entry, freeing the chain state
        /// `submit_oprf_query`/`submit_oprf_round1`/`submit_oprf_round2` accumulated for
        /// `query_id`. Permissionless — anyone may call it once one of two safe-to-prune
        /// conditions holds (this pallet has no existing per-block sweep hook to piggyback
        /// on, so this follows the pallet's existing style of explicit, unprivileged
        /// maintenance calls rather than adding new unbounded `on_initialize` iteration):
        ///
        /// 1. **Expired and dead**: the query's `OprfQuerySlaBlocks` deadline (from
        ///    `posted_at`) has passed. `submit_oprf_round1` already refuses new round-1
        ///    submissions once that deadline passes (`Error::QueryExpired`), so whatever
        ///    partial round-1/round-2 state exists at that point can never grow further and is
        ///    safe to discard.
        /// 2. **Fully answered**: every one of the `NUM_COMMITTEES` committee slots has
        ///    reached `T::OprfThreshold` round-2 responses for this `query_id`. This mailbox
        ///    is read off-chain (the querying citizen's own client combines the round-1/
        ///    round-2 data into the final anchor proof — see `OprfRound2Response`'s doc
        ///    comment; this pallet never reads it back itself), so pruning does not happen
        ///    automatically the instant this condition is reached: the caller decides the
        ///    timing. Unlike the expired case, this condition can be reached while the
        ///    submitter's client still hasn't read the data yet, so **only `query.submitter`
        ///    may prune here** (`Error::NotQuerySubmitter` otherwise) — permitting any signed
        ///    account to prune a merely-fully-answered-but-not-expired query would let anyone
        ///    race the legitimate submitter's own client and delete the data before it's ever
        ///    read, a cheap unprivileged way to block that citizen's registration indefinitely.
        ///    Once a query is also past its deadline, no legitimate read can still be pending
        ///    (see condition 1), so the submitter-only restriction is lifted and anyone may
        ///    sweep it, same as any other expired-and-dead query.
        ///
        /// Removes `PendingOprfQueries[query_id]` and, for every slot in `0..NUM_COMMITTEES`,
        /// `OprfRound1Commitments[query_id, slot]` and `OprfRound2Responses[query_id, slot]`,
        /// and decrements the submitter's `PendingOprfQueryCountBySubmitter`.
        #[pallet::call_index(18)]
        #[pallet::weight(Weight::from_parts(20_000, 0))]
        pub fn prune_oprf_query(origin: OriginFor<T>, query_id: u64) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let query = PendingOprfQueries::<T>::get(query_id).ok_or(Error::<T>::QueryNotFound)?;

            let deadline = query
                .posted_at
                .saturating_add(BlockNumberFor::<T>::from(T::OprfQuerySlaBlocks::get()));
            let expired = frame_system::Pallet::<T>::block_number() > deadline;

            let fully_answered = (0..NUM_COMMITTEES).all(|slot| {
                (OprfRound2Responses::<T>::get(query_id, slot).len() as u32)
                    == T::OprfThreshold::get()
            });

            ensure!(expired || fully_answered, Error::<T>::QueryNotPrunable);

            // A fully-answered-but-not-yet-expired query may still be mid-read by the
            // submitter's own off-chain client; only they may decide it's safe to prune. An
            // expired query can have no legitimate read still pending, so anyone may sweep it
            // regardless of who submitted it (this is the only permissionless path).
            if !expired {
                ensure!(who == query.submitter, Error::<T>::NotQuerySubmitter);
            }

            PendingOprfQueries::<T>::remove(query_id);
            for slot in 0..NUM_COMMITTEES {
                OprfRound1Commitments::<T>::remove(query_id, slot);
                OprfRound2Responses::<T>::remove(query_id, slot);
            }
            PendingOprfQueryCountBySubmitter::<T>::mutate(&query.submitter, |count| {
                *count = count.saturating_sub(1);
            });

            Self::deposit_event(Event::OprfQueryPruned { query_id, submitter: query.submitter });
            Ok(())
        }

        /// Recover a citizen's identity onto a brand-new `AccountId` after losing access to the
        /// one they originally registered with (e.g. a reinstalled/reset device — see
        /// `mobile/src/chain/keystoreWallet.ts`'s own doc comment on why a lost wallet file
        /// otherwise strands the identity with no way back in). Takes the exact same proof
        /// shape as `register_citizen` (a fresh outer ZKPassport proof plus the OPRF identity
        /// anchor it recomputes to) and verifies it through the same real `T::ZkVerifier`/
        /// `T::AnchorVerifier` path used at original registration — no weakened check and no
        /// dev-mode/passthrough shortcut here. Possessing the passport and passing the same
        /// liveness/face-match bar as original registration is treated as sufficient proof of
        /// identity, exactly as it is for `register_citizen` itself.
        ///
        /// Unlike `register_citizen` (which rejects outright when the proof's nullifier is
        /// already in `NullifierRegistry`), this call treats that as the expected case: it
        /// looks up which existing citizen the nullifier belongs to and, once the fresh proof
        /// re-derives that citizen's on-file anchor (checked the same way `reverify_citizen`
        /// checks it — `AnchorMismatch` on a mismatch, `InvalidReverificationProof` if the
        /// outer proof doesn't actually recompute to it), rebinds every piece of that citizen's
        /// per-account identity storage from the old `AccountId` to the caller's new one:
        /// `NullifierRegistry`, `CitizenNullifier`, `CitizenAnchor`, `IdentityAnchorRegistry`,
        /// `BackingCommitment` (checked the same way, via `BackingCommitmentMismatch`),
        /// `CitizenIndex`/`CitizenPosition` (repointed in place, not swap-and-popped — the
        /// citizen isn't leaving the registry, just changing address), `ReverificationDeadline`
        /// (pushed forward exactly like a successful `reverify_citizen`, since this proof
        /// establishes the same currently-valid-passport fact), and
        /// `SelfDeclaredSingleDocument` (carried over if set — it's a claim about the citizen,
        /// not the device).
        ///
        /// `SuspendedNullifiers`/`SuspendedByJuryReview` are deliberately **not** touched here:
        /// they're keyed by `nullifier`, not `AccountId`, so a suspended citizen's suspension
        /// automatically carries over onto the new account at zero extra code — recovery cannot
        /// be used to silently launder away an active court-ordered suspension.
        ///
        /// The old `AccountId` is fully invalidated as this citizen's identity: after this call
        /// it has no `CitizenNullifier`/`CitizenAnchor` entry, so it can no longer act as this
        /// citizen (vote, reverify, migrate, self-declare) even if it's still reachable —
        /// closing the gap where a merely-misplaced-but-still-accessible old key could act in
        /// parallel with the new one. Recovering onto an already-registered `who` (including
        /// `who == old_account`) is rejected up front (`AlreadyRegistered`), same check
        /// `register_citizen` makes.
        ///
        /// No time-delayed dispute window guards the rebind — deliberate, not an oversight: a
        /// delay only helps if the old key is still reachable to contest it, which contradicts
        /// the lost-device scenario this call exists for, and would add friction with no real
        /// benefit here. The only guard against abuse/griefing is
        /// `Config::MinBlocksBetweenRecoveries`, a per-nullifier cooldown tracked in
        /// `LastRecoveryBlock`. That absence is also a documented, currently-unresolved product
        /// question, not settled by the reasoning above alone: this same "instant, no dispute
        /// window" property is what makes recovery usable as a silent forced identity-transfer
        /// tool under coercion (e.g. compelling someone to re-scan their own passport onto an
        /// attacker-controlled account). See `docs/project/next-steps.md` for the open item.
        ///
        /// **Cross-pallet orphaning — partially guarded, not migrated.** Everything rebound
        /// above is pallet-identity's own per-account storage, and this call still does not
        /// *move* any other pallet's per-account state — there is no cross-pallet
        /// `AccountMigrator`, and building one (genuine, separate design/implementation work
        /// spanning pallet-voting/elections/legislature/executive/anticorruption) is not
        /// attempted here. What this call *does* do instead, since the fixes below landed:
        /// reject outright, before touching any storage, if `old_account` currently holds a
        /// nonzero AGR balance (`Error::RecoveryBlockedNonzeroBalance`, `Config::Currency`), a
        /// registered pallet-elections delegate persona
        /// (`Error::RecoveryBlockedActiveDelegate`), a pallet-legislature seat
        /// (`Error::RecoveryBlockedLegislatureSeat`), a pallet-executive cabinet role
        /// (`Error::RecoveryBlockedCabinetRole`), a still-open pallet-voting referendum vote
        /// (`Error::RecoveryBlockedOpenReferendumVote`), or an already-claimed current-epoch
        /// pallet-voting budget allocation (`Error::RecoveryBlockedUnclaimedEpochBudget`) — see
        /// `RecoveryStateChecker`'s doc comment for the full rationale, including why the last
        /// two close a real double-vote/double-claim vector rather than just orphaned data, and
        /// why a pallet-anticorruption asset-disclosure check was deliberately left out of this
        /// list. This is a divest-first guard, not a migration: a citizen who holds any of this
        /// state must resign/withdraw/wait via that pallet's own normal mechanism (e.g.
        /// transfer the balance, deregister the delegate persona, resign the seat/role, wait for
        /// the open referendum to finalize or the fiscal epoch to roll over) *before*
        /// recovering, rather than either silently abandoning it (the old behavior) or this call
        /// attempting to move it itself. pallet-voting's liquid-democracy delegation
        /// relationships (distinct from the direct-vote/budget-claim state now guarded above)
        /// and pallet-anticorruption's conflict-registry entries remain genuinely unguarded and
        /// still silently orphan on a successful recovery — real, separate future work, tracked
        /// in `docs/project/next-steps.md`. See `docs/project/pallets/identity.md`'s "Known
        /// limitation" section for the fuller accounting. Any UI that calls this extrinsic must
        /// disclose the remaining gap plainly to the citizen before they confirm — see
        /// `mobile/src/screens/RecoverAccountScreen.tsx`.
        #[pallet::call_index(20)]
        #[pallet::weight(Weight::from_parts(50_000, 0))]
        pub fn recover_account(
            origin: OriginFor<T>,
            zk_proof: BoundedVec<u8, ConstU32<4096>>,
            public_inputs: BoundedVec<[u8; 32], ConstU32<18>>,
            anchor: [u8; 32],
            oprf_pk_hashes: [[u8; 32]; NUM_COMMITTEES as usize],
            // Must match the old account's on-file `BackingCommitment` — see
            // `reverify_citizen`'s `backing_commitment` parameter doc comment for why.
            backing_commitment: [u8; 32],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!CitizenNullifier::<T>::contains_key(&who), Error::<T>::AlreadyRegistered);

            // Verify the real proof before touching any registry state keyed on its claimed
            // nullifier — same ordering rationale used throughout this pallet: an
            // unauthenticated caller must never learn anything about registry contents.
            Self::verify_outer_proof(
                zk_proof.as_slice(),
                public_inputs.as_slice(),
                AGORA_IDENTITY_RECOVER_SUBSCOPE,
            )?;
            let nullifier = public_inputs[public_inputs.len() - 2];

            let old_account = NullifierRegistry::<T>::get(nullifier)
                .ok_or(Error::<T>::NoExistingRegistrationForNullifier)?;
            let (scheme_version, on_file_anchor) = CitizenAnchor::<T>::get(&old_account)
                .ok_or(Error::<T>::NoExistingRegistrationForNullifier)?;
            ensure!(anchor == on_file_anchor, Error::<T>::AnchorMismatch);
            let on_file_backing_commitment = BackingCommitment::<T>::get(&old_account)
                .ok_or(Error::<T>::NoExistingRegistrationForNullifier)?;
            ensure!(
                backing_commitment == on_file_backing_commitment,
                Error::<T>::BackingCommitmentMismatch
            );

            Self::check_outer_proof_freshness(public_inputs.as_slice())?;
            Self::check_committee_keys(scheme_version, &oprf_pk_hashes)?;
            ensure!(
                T::AnchorVerifier::verify_reverification(
                    public_inputs.as_slice(),
                    anchor,
                    scheme_version,
                    oprf_pk_hashes,
                    backing_commitment,
                ),
                Error::<T>::InvalidReverificationProof
            );

            let now = frame_system::Pallet::<T>::block_number();
            if let Some(last) = LastRecoveryBlock::<T>::get(nullifier) {
                let earliest_next = last.saturating_add(BlockNumberFor::<T>::from(
                    T::MinBlocksBetweenRecoveries::get(),
                ));
                ensure!(now >= earliest_next, Error::<T>::RecoveryCooldownActive);
            }

            // ---- Guard: reject if old_account currently holds state this rebind would
            // silently orphan in another pallet (see `RecoveryStateChecker`'s doc comment).
            // A divest-first guard, not a migration — checked, and must fail closed, before
            // any storage below is touched. ----
            ensure!(
                T::Currency::free_balance(&old_account).is_zero(),
                Error::<T>::RecoveryBlockedNonzeroBalance
            );
            ensure!(
                !T::RecoveryStateChecker::is_registered_delegate(&old_account),
                Error::<T>::RecoveryBlockedActiveDelegate
            );
            ensure!(
                !T::RecoveryStateChecker::holds_legislature_seat(&old_account),
                Error::<T>::RecoveryBlockedLegislatureSeat
            );
            ensure!(
                !T::RecoveryStateChecker::holds_cabinet_role(&old_account),
                Error::<T>::RecoveryBlockedCabinetRole
            );
            ensure!(
                !T::RecoveryStateChecker::has_open_referendum_vote(&old_account),
                Error::<T>::RecoveryBlockedOpenReferendumVote
            );
            ensure!(
                !T::RecoveryStateChecker::has_unclaimed_current_epoch_budget(&old_account),
                Error::<T>::RecoveryBlockedUnclaimedEpochBudget
            );

            // ---- Rebind every per-account identity storage from old_account to who. ----
            NullifierRegistry::<T>::insert(nullifier, &who);
            CitizenNullifier::<T>::remove(&old_account);
            CitizenNullifier::<T>::insert(&who, nullifier);
            // Tombstone: leave a permanent trace of which citizen just vacated old_account, so
            // `same_citizen` can still resolve it later — see `RecoveredAccountNullifier`'s doc
            // comment.
            RecoveredAccountNullifier::<T>::insert(&old_account, nullifier);

            IdentityAnchorRegistry::<T>::insert((scheme_version, anchor), &who);
            CitizenAnchor::<T>::remove(&old_account);
            CitizenAnchor::<T>::insert(&who, (scheme_version, anchor));
            BackingCommitment::<T>::remove(&old_account);
            BackingCommitment::<T>::insert(&who, backing_commitment);

            if let Some(pos) = CitizenPosition::<T>::take(&old_account) {
                CitizenPosition::<T>::insert(&who, pos);
                CitizenIndex::<T>::insert(pos, &who);
            }

            ReverificationDeadline::<T>::remove(&old_account);
            let deadline =
                now.saturating_add(BlockNumberFor::<T>::from(T::ReverificationPeriod::get()));
            ReverificationDeadline::<T>::insert(&who, deadline);

            if SelfDeclaredSingleDocument::<T>::take(&old_account) {
                SelfDeclaredSingleDocument::<T>::insert(&who, true);
            }

            LastRecoveryBlock::<T>::insert(nullifier, now);

            Self::deposit_event(Event::CitizenAccountRecovered {
                old_account,
                new_account: who,
                nullifier_hash: nullifier,
            });
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        /// Called by pallet-courts via CitizenSuspender runtime trait when a conduct ruling
        /// is finalized. Bypasses the extrinsic origin check — courts are pre-authorized.
        /// Upserts: if a suspension record already exists it is replaced, allowing courts to
        /// extend or modify a citizen's suspension without needing to restore first.
        /// `jury_reviewed` reflects which path pallet-courts' `auto_finalize` took to get
        /// here (jury majority vs. an unappealed AI ruling) — see `SuspendedByJuryReview`'s
        /// doc comment for what it's used for and why the distinction matters.
        pub fn suspend_citizen_internal(
            nullifier: [u8; 32],
            until: Option<BlockNumberFor<T>>,
            jury_reviewed: bool,
        ) -> DispatchResult {
            ensure!(NullifierRegistry::<T>::contains_key(nullifier), Error::<T>::NotRegistered);
            SuspendedNullifiers::<T>::insert(nullifier, until);
            SuspendedByJuryReview::<T>::insert(nullifier, jury_reviewed);
            Self::deposit_event(Event::CitizenSuspended { nullifier, until });
            Ok(())
        }

        /// Clears a suspension record from both `SuspendedNullifiers` and
        /// `SuspendedByJuryReview` together. These two maps must always be written and cleared
        /// as a pair — a bare `SuspendedNullifiers::remove` without the matching
        /// `SuspendedByJuryReview::remove` leaves a stale jury-review flag behind for a
        /// nullifier that's no longer suspended at all (a storage leak; harmless today since
        /// `is_suspended_by_jury_reviewed_conviction` re-checks `SuspendedNullifiers` freshness
        /// before consulting the flag, but not something to rely on). Use this helper at every
        /// clear site instead of removing from either map directly.
        fn clear_suspension(nullifier: [u8; 32]) {
            SuspendedNullifiers::<T>::remove(nullifier);
            SuspendedByJuryReview::<T>::remove(nullifier);
        }

        /// Used by pallet-courts (via a CitizenSelector trait impl in the runtime).
        pub fn citizen_at(index: u32) -> Option<T::AccountId> {
            CitizenIndex::<T>::get(index)
        }

        pub fn total_citizens() -> u32 {
            TotalCitizens::<T>::get()
        }

        /// True if the account is a registered citizen with no active suspension and no missed
        /// reverification deadline. Timed suspensions are lazily removed from storage once
        /// their block has passed, so subsequent is_citizen / suspend calls do not see stale
        /// records — the reverification deadline is checked the same lazy, point-of-use way
        /// (see HANDOFF log #67), though unlike suspension it isn't cleaned up from storage on
        /// expiry: `reverify_citizen` is the only thing that should ever move it forward again.
        pub fn is_active_citizen(who: &T::AccountId) -> bool {
            let Some(nullifier) = CitizenNullifier::<T>::get(who) else { return false; };
            // A missing deadline is treated as "past deadline" rather than an unbounded grace
            // period; register_citizen always sets one, so this should only trip for data that
            // predates this check.
            match ReverificationDeadline::<T>::get(who) {
                Some(deadline) if frame_system::Pallet::<T>::block_number() <= deadline => {}
                _ => return false,
            }
            match SuspendedNullifiers::<T>::get(nullifier) {
                None => true,
                Some(None) => false, // indefinite suspension
                Some(Some(until)) => {
                    if frame_system::Pallet::<T>::block_number() > until {
                        // Lazily clean up the expired record so future suspend/restore
                        // calls don't incorrectly see it as an active suspension. Clears
                        // SuspendedByJuryReview too — see `clear_suspension`'s doc comment.
                        Self::clear_suspension(nullifier);
                        true
                    } else {
                        false
                    }
                }
            }
        }

        /// True if the account is registered (regardless of suspension status).
        pub fn is_citizen(who: &T::AccountId) -> bool {
            CitizenNullifier::<T>::contains_key(who)
        }

        /// Resolves `who` to the citizen nullifier it is, or once was, associated with: a live
        /// `CitizenNullifier` entry if `who` is presently a registered citizen, else whatever
        /// `RecoveredAccountNullifier` tombstone was left behind if `who` was ever the
        /// `old_account` of a completed `recover_account` call. `None` if `who` has never been
        /// either.
        fn nullifier_of_current_or_recovered(who: &T::AccountId) -> Option<[u8; 32]> {
            CitizenNullifier::<T>::get(who).or_else(|| RecoveredAccountNullifier::<T>::get(who))
        }

        /// True if `former` and `current` are, or were, the same citizen — either the same
        /// account, or `former` is an account whose citizen identity has since been moved (via
        /// one or more `recover_account` calls) onto `current`. Consumed cross-pallet by
        /// `pallet_elections::CitizenChecker::same_citizen` (wired in
        /// `runtime/src/configs/mod.rs`) so `remove_backing` can accept a backing nullifier
        /// originally submitted by an account that has since been recovered away from — a
        /// citizen's `backing_nullifier` is derived purely from their `backing_root_secret` and
        /// slot index, not their `AccountId`, so it still recomputes identically after recovery,
        /// but the raw `UsedBackingNullifier::submitter == who` check that used to gate
        /// `remove_backing` could never match the citizen's new account. Resolving through
        /// `NullifierRegistry` (always the *current* owner of a nullifier, however many
        /// recoveries it has been through) makes this correct across any number of hops, not
        /// just one.
        pub fn same_citizen(former: &T::AccountId, current: &T::AccountId) -> bool {
            if former == current {
                return true;
            }
            let Some(nullifier) = Self::nullifier_of_current_or_recovered(former) else {
                return false;
            };
            NullifierRegistry::<T>::get(nullifier).as_ref() == Some(current)
        }

        /// True only if the account is *currently* suspended (not expired — same lazy-expiry
        /// semantics as `is_active_citizen`) AND that suspension came from a jury-reviewed
        /// conviction, not a bare unappealed AI ruling or the manual admin override. See
        /// `SuspendedByJuryReview`'s doc comment for the full rationale.
        pub fn is_suspended_by_jury_reviewed_conviction(who: &T::AccountId) -> bool {
            let Some(nullifier) = CitizenNullifier::<T>::get(who) else { return false; };
            let currently_suspended = match SuspendedNullifiers::<T>::get(nullifier) {
                None => false,
                Some(None) => true, // indefinite
                Some(Some(until)) => {
                    if frame_system::Pallet::<T>::block_number() > until {
                        // Lazily clean up, mirroring is_active_citizen — an expired suspension
                        // is no longer "currently suspended" by either measure.
                        Self::clear_suspension(nullifier);
                        false
                    } else {
                        true
                    }
                }
            };
            currently_suspended && SuspendedByJuryReview::<T>::get(nullifier)
        }

        /// Shared bump logic for `rotate_oprf_scheme` / `emergency_rotate_oprf_scheme`. Only
        /// advances the global counter — it never touches any citizen's own on-file anchor
        /// version (see `migrate_oprf_scheme`'s doc comment for why that's a separate step).
        fn do_bump_scheme_version() -> Result<u32, DispatchError> {
            let new_version = OprfSchemeVersion::<T>::get()
                .checked_add(1)
                .ok_or(Error::<T>::OprfSchemeVersionOverflow)?;
            OprfSchemeVersion::<T>::put(new_version);
            Ok(new_version)
        }

        /// Shared by `register_citizen`/`reverify_citizen`/`recover_account`/
        /// `migrate_oprf_scheme` (HANDOFF log #76): allowlist check (cheap, no proof work yet),
        /// then the domain-separation scope check (also cheap — see
        /// `AGORA_IDENTITY_REGISTER_SUBSCOPE`'s doc comment for why each caller passes a
        /// different `expected_subscope`), then the expensive BN254 pairing check last.
        /// `public_inputs.len() >= 9` is the smallest real outer-circuit variant (count_4: 8
        /// fixed inputs + 1 disclosure subproof) — anything shorter cannot be a genuine
        /// ZKPassport proof, and must be checked before indexing `SERVICE_SCOPE_INDEX`/
        /// `SERVICE_SUBSCOPE_INDEX` (3/4) below.
        fn verify_outer_proof(
            zk_proof: &[u8],
            public_inputs: &[[u8; 32]],
            expected_subscope: [u8; 32],
        ) -> DispatchResult {
            ensure!(public_inputs.len() >= 9, Error::<T>::InvalidZKProof);
            ensure!(AllowedMerkleRoots::<T>::get(public_inputs[0]), Error::<T>::IssuerNotAllowed);
            ensure!(
                public_inputs[SERVICE_SCOPE_INDEX] == AGORA_IDENTITY_SERVICE_SCOPE
                    && public_inputs[SERVICE_SUBSCOPE_INDEX] == expected_subscope,
                Error::<T>::InvalidProofScope
            );
            ensure!(T::ZkVerifier::verify(zk_proof, public_inputs), Error::<T>::InvalidZKProof);
            Ok(())
        }

        /// Shared by `register_citizen`/`reverify_citizen`/`migrate_oprf_scheme` (HANDOFF log
        /// #75/#76): `current_date` is `public_inputs[2]` (see `runtime/src/verifier.rs`'s
        /// module docs for the full outer-circuit public-input table), a u64 encoded as a
        /// field element's low 8 bytes. Rejects a proof whose `current_date` is older than
        /// `MaxAnchorProofAge` relative to `T::Now`, so a genuine-at-generation-time proof
        /// cannot be replayed indefinitely. Also rejects a `current_date` further in the
        /// future than `MaxAnchorProofClockSkew` — `current_date` is a fully prover-controlled
        /// public input (the in-circuit check only constrains it against the passport's own
        /// expiry, never against real time), so without this second check a future-dated value
        /// would make the staleness check's `saturating_sub` clamp to zero and pass forever.
        fn check_outer_proof_freshness(public_inputs: &[[u8; 32]]) -> DispatchResult {
            let current_date_field = public_inputs[2];
            ensure!(
                current_date_field[..24].iter().all(|byte| *byte == 0),
                Error::<T>::MalformedProofDate
            );
            let current_date = u64::from_be_bytes(
                current_date_field[24..32].try_into().expect("slice is exactly 8 bytes"),
            );
            let now = T::Now::now().as_secs();
            ensure!(
                current_date <= now.saturating_add(T::MaxAnchorProofClockSkew::get()),
                Error::<T>::AnchorProofFuture
            );
            ensure!(
                now.saturating_sub(current_date) <= T::MaxAnchorProofAge::get(),
                Error::<T>::AnchorProofStale
            );
            Ok(())
        }

        /// Shared by `register_citizen`/`reverify_citizen`/`migrate_oprf_scheme` (HANDOFF log
        /// #75/#76): each of the `NUM_COMMITTEES` key hashes must match the
        /// governance-approved key for its slot under `scheme_version` — see
        /// `circuits/oprf-identity-anchor/README.md`'s "What the Rust verifier still has to
        /// enforce", obligation 1. `migrate_oprf_scheme` calls this twice (once per scheme
        /// version either side of the rotation).
        fn check_committee_keys(
            scheme_version: u32,
            oprf_pk_hashes: &[[u8; 32]; NUM_COMMITTEES as usize],
        ) -> DispatchResult {
            ensure!(
                Self::are_committee_keys_approved(scheme_version, oprf_pk_hashes),
                Error::<T>::CommitteeKeyMismatch
            );
            Ok(())
        }

        /// The bool-returning, storage-error-free half of [`Self::check_committee_keys`] --
        /// exposed as `pub` (unlike that function) so other pallets that verify a proof riding
        /// in the same outer-ZKPassport-proof family can enforce the identical Sybil-resistance
        /// guarantee without duplicating `OprfCommitteeKeys` storage access or depending on this
        /// pallet's own `Error` type. Currently consumed by `pallet-elections`'s
        /// `register_as_delegate` (via its own `CommitteeKeyChecker` trait, implemented on this
        /// `Pallet<T>` in `runtime/src/configs/mod.rs`) for exactly the same reason
        /// `register_citizen` checks this itself: without it, a prover could fabricate their own
        /// "committee" keys and self-mint an unlimited number of otherwise-valid-looking
        /// delegate personas.
        pub fn are_committee_keys_approved(
            scheme_version: u32,
            oprf_pk_hashes: &[[u8; 32]; NUM_COMMITTEES as usize],
        ) -> bool {
            oprf_pk_hashes.iter().enumerate().all(|(slot, pk_hash)| {
                OprfCommitteeKeys::<T>::get((scheme_version, slot as u8)) == Some(*pk_hash)
            })
        }

        // ── Backing-commitment incremental Merkle tree ──────────────────────────

        /// Reads a tree node, falling back to the deterministic all-empty-subtree value for
        /// any position never yet written.
        fn backing_tree_node(level: u8, index: u32) -> [u8; 32] {
            BackingCommitmentTreeNodes::<T>::get(level, index)
                .unwrap_or_else(|| backing_tree_zero_hash(level))
        }

        /// Writes `leaf_value` at `leaf_index` and recomputes every ancestor up to the root,
        /// storing the new root and returning it. Shared by both a fresh registration
        /// (`leaf_value` = the real `backing_commitment`) and a revocation tombstone
        /// (`leaf_value` = `backing_tree_tombstone_leaf()`) — the update algorithm is
        /// identical either way, only the leaf value differs.
        fn recompute_backing_tree_path(leaf_index: u32, leaf_value: [u8; 32]) -> [u8; 32] {
            BackingCommitmentTreeNodes::<T>::insert(0u8, leaf_index, leaf_value);
            let mut index = leaf_index;
            let mut current = leaf_value;
            for level in 0..BACKING_TREE_DEPTH {
                let sibling = Self::backing_tree_node(level, index ^ 1);
                current = if index % 2 == 0 {
                    backing_tree_node_hash(current, sibling)
                } else {
                    backing_tree_node_hash(sibling, current)
                };
                index /= 2;
                BackingCommitmentTreeNodes::<T>::insert(level + 1, index, current);
            }
            BackingCommitmentTreeRoot::<T>::put(current);
            current
        }

        /// Pushes `root` onto `BackingRootHistoryEntries`'s FIFO ring, first pruning from the
        /// head any entry that is either older than `BackingRootHistoryWindowBlocks` or, if
        /// none have aged out yet, as many oldest entries as needed to respect the hard
        /// `MaxBackingRootHistoryEntries` count cap. See those two storage/config items' doc
        /// comments for the full retention-window rationale and its documented degrade-under-
        /// burst trade-off.
        fn push_backing_root_history(root: [u8; 32]) {
            let now = frame_system::Pallet::<T>::block_number();
            let window = BlockNumberFor::<T>::from(T::BackingRootHistoryWindowBlocks::get());
            let max_entries = T::MaxBackingRootHistoryEntries::get() as u64;

            let mut head = BackingRootHistoryHead::<T>::get();
            let tail = BackingRootHistoryTail::<T>::get();
            while head < tail {
                let len = tail.saturating_sub(head);
                let Some((entry_block, entry_root)) = BackingRootHistoryEntries::<T>::get(head)
                else {
                    // head < tail implies an entry was written here and not yet removed; if
                    // it's somehow missing, don't loop forever on the inconsistency.
                    break;
                };
                let expired = now.saturating_sub(entry_block) > window;
                if !expired && len < max_entries {
                    break;
                }
                BackingRootHistoryEntries::<T>::remove(head);
                BackingRootValidUntil::<T>::remove(entry_root);
                head = head.saturating_add(1);
            }
            BackingRootHistoryHead::<T>::put(head);

            BackingRootHistoryEntries::<T>::insert(tail, (now, root));
            BackingRootValidUntil::<T>::insert(root, now.saturating_add(window));
            BackingRootHistoryTail::<T>::put(tail.saturating_add(1));
        }

        /// Assigns `nullifier` a fresh, permanent leaf index (see `NextBackingLeafIndex`'s doc
        /// comment for why it's never reused) and inserts `backing_commitment` there — called
        /// once, from `register_citizen`, after every other registration check has already
        /// succeeded.
        fn insert_backing_leaf(nullifier: [u8; 32], backing_commitment: [u8; 32]) -> DispatchResult {
            let leaf_index = NextBackingLeafIndex::<T>::get();
            let next_index =
                leaf_index.checked_add(1).ok_or(Error::<T>::BackingLeafIndexOverflow)?;
            NextBackingLeafIndex::<T>::put(next_index);
            BackingLeafIndexOf::<T>::insert(nullifier, leaf_index);
            let root = Self::recompute_backing_tree_path(leaf_index, backing_commitment);
            Self::push_backing_root_history(root);
            Self::deposit_event(Event::BackingCommitmentLeafInserted {
                nullifier,
                leaf_index,
                root,
            });
            Ok(())
        }

        /// Overwrites `nullifier`'s leaf (if it has one) with the tombstone placeholder —
        /// called from `revoke_citizen`. A no-op if `nullifier` has no `BackingLeafIndexOf`
        /// entry; defensive only, since every caller of this function has already confirmed
        /// the nullifier was registered.
        fn tombstone_backing_leaf(nullifier: [u8; 32]) {
            if let Some(leaf_index) = BackingLeafIndexOf::<T>::get(nullifier) {
                let root =
                    Self::recompute_backing_tree_path(leaf_index, backing_tree_tombstone_leaf());
                Self::push_backing_root_history(root);
                Self::deposit_event(Event::BackingCommitmentLeafTombstoned {
                    nullifier,
                    leaf_index,
                    root,
                });
            }
        }

        /// The backing-commitment tree's current root — `backing_tree_zero_hash
        /// (BACKING_TREE_DEPTH)` (the all-empty-tree root) before any citizen has ever
        /// registered, otherwise `BackingCommitmentTreeRoot`'s on-file value. The function a
        /// future backing-nullifier proof's submitter-facing tooling should call to fetch the
        /// live root to build a fresh authentication path against.
        pub fn current_backing_commitment_root() -> [u8; 32] {
            BackingCommitmentTreeRoot::<T>::get()
                .unwrap_or_else(|| backing_tree_zero_hash(BACKING_TREE_DEPTH))
        }

        /// True if `root` is a backing-commitment tree root that was current at some block
        /// within the last `BackingRootHistoryWindowBlocks` (inclusive of the current root
        /// itself). The function a future backing-nullifier proof's on-chain verification
        /// should call instead of comparing only against `current_backing_commitment_root`,
        /// so a proof built against a slightly-stale-but-still-in-window root is still
        /// accepted. See `BackingRootValidUntil`'s doc comment for why this re-compares
        /// against the current block rather than trusting bare map presence.
        pub fn is_valid_backing_commitment_root(root: [u8; 32]) -> bool {
            match BackingRootValidUntil::<T>::get(root) {
                Some(valid_until) => frame_system::Pallet::<T>::block_number() <= valid_until,
                None => false,
            }
        }
    }
}
