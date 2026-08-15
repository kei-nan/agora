/**
 * Client-side combination of a locked OPRF committee slot's round1/round2 responses into
 * the `OPRFProof` witness the `oprf_identity_anchor` (`anchor`) circuit's `oprf_proofs:
 * [OPRFProof; 5]` parameter needs, one entry per committee slot.
 *
 * # What's real here
 *
 * The types below (`OprfRound1Commitment`, `OprfRound2Response`, `CombinedCommitteeProof`)
 * match confirmed on-chain/circuit shapes, not guesses:
 *
 *  - `OprfRound1Commitment`/`OprfRound2Response` mirror `pallet-identity`'s own
 *    `OprfRound1Commitment<AccountId>`/`OprfRound2Response<AccountId>` structs
 *    (`pallets/pallet-identity/src/lib.rs`, around the "OPRF committee on-chain mailbox"
 *    section) field-for-field: `r_i`/`d_g`/`d_q`/`e_g`/`e_q` are each `[u8; 64]`, `z_i` is
 *    `[u8; 32]`. The `index` field on both is NOT an on-chain field — it's the resolved
 *    1-based DKG roster position `identity.ts`'s `resolveCommitteeMemberIndex` produces
 *    from `CommitteeMembers[slot]`, attached here because the combination math (once it
 *    exists) needs each response tagged with its Lagrange-interpolation party index, and
 *    that index is not itself stored alongside the response on-chain.
 *  - `CombinedCommitteeProof` is based on `utils::types::OPRFProof`'s confirmed field list
 *    — `{ pk, dlog_e, dlog_s, response_blinded, response, beta }` — read directly out of
 *    `oprf-committee-dev/src/prover_toml.rs`'s doc comment ("Renders a `Prover.toml` for
 *    the `oprf_identity_anchor` (`anchor`) circuit's `oprf_proofs: [OPRFProof; 5]`
 *    parameter (`utils::types::OPRFProof { pk, dlog_e, dlog_s, response_blinded, response,
 *    beta }`)") and its `OprfProofWitness` struct, which carries exactly those six fields.
 *    `pk`/`response_blinded`/`response` are BabyJubJub curve points (`{ x: Field; y: Field
 *    }`, per `oprf-committee-dev/src/babyjubjub.rs`'s `Point`); `dlog_e`/`dlog_s`/`beta`
 *    are field/scalar elements. This module represents each as a 32-byte big-endian
 *    encoding, matching the convention `proofEncoding.ts` already uses elsewhere in this
 *    codebase for Noir field elements.
 *
 * # What's NOT real here — and deliberately so
 *
 * `combineCommitteeSlotResponses` below is a stub that throws. It is not a shortcut taken
 * for lack of time; producing real threshold-cryptography code here, unverified, would be
 * actively worse than not producing it — a silently-wrong finite-field reduction or a
 * transcribed constant with one flipped digit fails with no diagnostic that points at the
 * cause. This project has otherwise been careful to keep exactly this kind of gap clearly
 * labeled rather than papered over (`dev-mode`'s passthrough ZK verifiers are explicitly
 * gated out of the default build; `zkProving.ts` documents its own missing native prover
 * seam rather than faking one) — this module follows that same convention.
 *
 * The real math this module would need to perform —
 * `oprf-committee-dev::threshold::combine_evaluations`/`combine_responses` — operates over:
 *
 *  - BabyJubJub curve points, under a ~251-bit prime (`BABYJUBJUB_Fr`) that is NOT BN254's
 *    own scalar-field modulus — a subtle, easy-to-get-wrong reduction detail if ported
 *    casually against the wrong modulus.
 *  - A Poseidon2 parameterization matching TaceoLabs' `t=3`/`t=16` instantiation
 *    (`oprf-committee-dev/src/poseidon2_taceo.rs`). This is NOT the parameterization this
 *    repo's existing `@zkpassport/poseidon2` dependency provides — that package is `t=4`,
 *    a different, incompatible instantiation already in use elsewhere in this codebase.
 *    Reusing it here would silently produce wrong hashes, not a build error.
 *
 * Concrete, ordered list of what would need to happen before this could be implemented for
 * real:
 *
 *  1. Transcribe TaceoLabs' Poseidon2 `t=3`/`t=16` round constants and verify them against
 *     `poseidon2_taceo.rs`'s own `t3_matches_upstream_vector_0_1_2`/
 *     `t16_matches_upstream_vector_0_to_15` Rust test vectors. This environment has no Rust
 *     toolchain to run those tests, so this step cannot be completed here — only flagged.
 *  2. Port BabyJubJub curve arithmetic (point addition, scalar multiplication, the correct
 *     `BABYJUBJUB_Fr` reduction) and validate it against `babyjubjub.rs`'s own tests.
 *  3. Port the Lagrange-interpolation / Chaum-Pedersen combination logic itself
 *     (`combine_evaluations`/`combine_responses`), validated against a freshly-generated
 *     fixture. `oprf-committee-dev/src/threshold.rs` has an `#[ignore]`d
 *     `export_combined_proof_fixture_for_noir_cross_check` helper that could generate one —
 *     but doing so requires a Rust toolchain, unavailable in this environment. That's the
 *     concrete next step for whoever picks this up, not something completable now.
 *  4. Resolve where the raw (non-hashed) committee group public keys come from for mobile
 *     to consume. Only Poseidon2 *hashes* of these are on-chain (`OprfCommitteeKeys`); the
 *     raw points are, per `committee-node/README.md`'s `GROUP_PUBKEY_HEX` note, "a
 *     deployment-time constant every member must be given independently at ceremony time"
 *     — a real, unresolved deployment/data-plumbing gap, not just a porting task.
 */

/** One committee member's round-1 FROST-style commitment, `pallet-identity`'s `OprfRound1Commitment<AccountId>`. */
export interface OprfRound1Commitment {
  /** SS58 address of the submitting committee member. */
  member: string;
  /**
   * This member's 1-based DKG roster position — `resolveCommitteeMemberIndex`'s output, NOT
   * an on-chain field of `OprfRound1Commitment` itself. See this module's doc comment.
   */
  index: number;
  /** 64 bytes. */
  rI: Uint8Array;
  /** 64 bytes. */
  dG: Uint8Array;
  /** 64 bytes. */
  dQ: Uint8Array;
  /** 64 bytes. */
  eG: Uint8Array;
  /** 64 bytes. */
  eQ: Uint8Array;
}

/** One committee member's round-2 response scalar, `pallet-identity`'s `OprfRound2Response<AccountId>`. */
export interface OprfRound2Response {
  /** SS58 address of the submitting committee member. */
  member: string;
  /** This member's 1-based DKG roster position — see {@link OprfRound1Commitment.index}. */
  index: number;
  /** 32 bytes. */
  zI: Uint8Array;
}

/** A BabyJubJub curve point, 32-byte big-endian field-element coordinates. */
export interface BabyJubJubPoint {
  x: Uint8Array;
  y: Uint8Array;
}

/**
 * One committee slot's combined OPRF proof — the shape `utils::types::OPRFProof` takes
 * (confirmed field list, see this module's doc comment), ready to feed one entry of the
 * `oprf_identity_anchor` circuit's `oprf_proofs: [OPRFProof; 5]` parameter. All scalar
 * fields are 32-byte big-endian encodings, matching `proofEncoding.ts`'s convention.
 */
export interface CombinedCommitteeProof {
  pk: BabyJubJubPoint;
  dlogE: Uint8Array;
  dlogS: Uint8Array;
  responseBlinded: BabyJubJubPoint;
  response: BabyJubJubPoint;
  beta: Uint8Array;
}

/** Thrown by {@link combineCommitteeSlotResponses} — see this module's doc comment for the full accounting. */
export class OprfCombinationUnimplementedError extends Error {
  constructor() {
    super(
      'combineCommitteeSlotResponses is not implemented: the BabyJubJub/Poseidon2-t3-t16 ' +
        'threshold-combination math has not been ported and verified yet. See oprfCombine.ts\'s ' +
        'module doc comment for exactly what is missing and why this is a deliberate stub, not an ' +
        'oversight.',
    );
    this.name = 'OprfCombinationUnimplementedError';
  }
}

/**
 * Would combine one committee slot's locked round1/round2 responses (`OprfThreshold` of
 * each, per `pallet-identity`) plus that committee's group public key into the
 * `OPRFProof` witness the `anchor` circuit needs for that slot.
 *
 * Always throws {@link OprfCombinationUnimplementedError} — see this module's top-of-file
 * doc comment for the full explanation of what's missing (Poseidon2 t3/t16 round
 * constants, BabyJubJub curve arithmetic, the Lagrange-interpolation combination logic
 * itself, and sourcing the raw committee group public key) and why it cannot be
 * implemented here without a Rust toolchain to cross-check against `oprf-committee-dev`'s
 * own test vectors.
 */
export async function combineCommitteeSlotResponses(
  round1: OprfRound1Commitment[],
  round2: OprfRound2Response[],
  groupPublicKey: Uint8Array,
): Promise<CombinedCommitteeProof> {
  throw new OprfCombinationUnimplementedError();
}
