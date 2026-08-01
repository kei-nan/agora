/**
 * Encodes a ZKPassport UltraHonk proof into the byte envelope
 * `runtime/src/verifier.rs` expects for `pallet_identity_zk::register_citizen`'s
 * `zk_proof` argument, and helpers for the matching `public_inputs` array.
 *
 * Replaces the Rarimo-era Groth16 encoder (snarkjs JSON -> 129 ark-serialize bytes).
 * ZKPassport's circuits are Noir/UltraHonk proved by Barretenberg, so none of that
 * carried over — see `docs/project/changelog/065-068.md` entries 65/66.
 *
 * **This file and `runtime/src/verifier.rs` are one contract split across two
 * languages.** Every constant and every check below mirrors something there. Change
 * neither alone; if you bump `ENVELOPE_VERSION` here, bump it there in the same commit.
 *
 * # Envelope layout
 *
 * ```text
 * [ 0]      magic           0x5A ('Z' for ZKPassport)
 * [ 1]      format_version  0x01
 * [ 2]      outer_count     N, the ZKPassport `main/outer/count_N` variant (4..=13)
 * [ 3]      proof_variant   0 = ZK (`bb prove -t evm`), 1 = Plain (`bb prove -t evm-no-zk`)
 * [ 4.. 8]  proof_len       u32, big-endian
 * [ 8..8+L] proof           the raw bytes of bb's `proof` output file, verbatim
 * ```
 *
 * The proof bytes are passed through untouched. bb already emits a flat array of
 * 32-byte big-endian words; re-serializing it here would only create opportunities to
 * corrupt it. The header carries the two things the proof bytes cannot tell the runtime
 * on their own: which outer circuit produced it, and whether it was proved in ZK or
 * plain mode (the two have different lengths *and* different transcripts).
 *
 * # Public inputs
 *
 * These travel separately, as `register_citizen`'s `public_inputs` argument, matching
 * bb's separate `public_inputs` output file chunked into 32-byte big-endian field
 * elements. The layout is fixed by
 * `src/noir/bin/main/outer/count_N/src/main.nr` (circuits repo at `d3a75ac`):
 *
 * ```text
 * index          field
 * ------------   -----------------------------------------------------------
 * 0              certificate_registry_root
 * 1              circuit_registry_root
 * 2              current_date              (unix seconds, u64)
 * 3              service_scope
 * 4              service_subscope
 * 5 .. 5+D       param_commitments[D]      D = N - 3 (the disclosure-subproof count)
 * 5+D            nullifier_type
 * 6+D            scoped_nullifier
 * 7+D            oprf_pk_hash
 * ```
 *
 * # Status
 *
 * The encoder is real and tested against genuine bb 5.0.0 output. The chain will still
 * reject whatever it produces: no Rust UltraHonk verifier can check bb 5.0.0 proofs yet,
 * so `verifier.rs` is deliberately fail-closed. See its module docs for the evidence.
 */

/** Envelope magic byte — `'Z'`, for ZKPassport. Mirrors `verifier.rs::ENVELOPE_MAGIC`. */
export const ENVELOPE_MAGIC = 0x5a;

/** Envelope format version. Mirrors `verifier.rs::ENVELOPE_VERSION`. */
export const ENVELOPE_VERSION = 0x01;

/** Fixed envelope header length, in bytes. Mirrors `verifier.rs::ENVELOPE_HEADER_LEN`. */
export const ENVELOPE_HEADER_LEN = 8;

/**
 * The three non-disclosure subproofs every ZKPassport outer circuit wraps
 * (`sig-check/dsc`, `sig-check/id-data`, `data-check/integrity`).
 * Mirrors `verifier.rs::BASE_SUBPROOF_COUNT`.
 */
export const BASE_SUBPROOF_COUNT = 3;

/**
 * Public inputs that are always present, i.e. everything except `param_commitments`.
 * Mirrors `verifier.rs::FIXED_PUBLIC_INPUT_COUNT`.
 */
export const FIXED_PUBLIC_INPUT_COUNT = 8;

/** Size of one field element / one bb proof word, in bytes. */
export const FIELD_ELEMENT_BYTES = 32;

/** The `count_N` outer-circuit variants ZKPassport actually ships. */
export const MIN_OUTER_COUNT = 4;
export const MAX_OUTER_COUNT = 13;

/**
 * BN254 scalar field modulus `r`. Public inputs are canonical field elements, so every
 * one is strictly below this. Mirrors `verifier.rs::BN254_FR_MODULUS_BE`.
 *
 * Note this is the *scalar* field, not the base field the Rarimo-era encoder used —
 * different constant, easy to confuse.
 */
export const BN254_SCALAR_FIELD_MODULUS = BigInt(
  '21888242871839275222246405745257275088548364400416034343698204186575808495617',
);

/**
 * Which `bb` proving mode produced a proof.
 * `zk` = `bb prove -t evm`, `plain` = `bb prove -t evm-no-zk`. Both use a keccak
 * transcript; only `zk` gives witness privacy, so `zk` is the only correct choice for a
 * real passport proof — `plain` exists here because the runtime envelope can express it
 * and round-tripping it should be testable.
 */
export type UltraHonkProofVariant = 'zk' | 'plain';

const VARIANT_TO_TAG: Record<UltraHonkProofVariant, number> = { zk: 0, plain: 1 };
const TAG_TO_VARIANT: Record<number, UltraHonkProofVariant> = { 0: 'zk', 1: 'plain' };

/** The parsed envelope header. */
export interface UltraHonkEnvelopeHeader {
  /** The `main/outer/count_N` variant that produced the proof. */
  outerCount: number;
  variant: UltraHonkProofVariant;
  /** Length of the raw bb proof, in bytes. */
  proofLength: number;
}

/** A decoded envelope: its header plus the raw bb proof bytes. */
export interface DecodedUltraHonkProof {
  header: UltraHonkEnvelopeHeader;
  proof: Uint8Array;
}

/**
 * Number of disclosure subproofs — and hence of `param_commitments` public inputs — a
 * `count_N` outer circuit carries.
 */
export function disclosureCountFor(outerCount: number): number {
  assertValidOuterCount(outerCount);
  return outerCount - BASE_SUBPROOF_COUNT;
}

/**
 * Total number of public inputs a `count_N` outer circuit exposes: `N + 5`
 * (9 for `count_4`).
 */
export function expectedPublicInputCount(outerCount: number): number {
  return FIXED_PUBLIC_INPUT_COUNT + disclosureCountFor(outerCount);
}

function assertValidOuterCount(outerCount: number): void {
  if (
    !Number.isInteger(outerCount) ||
    outerCount < MIN_OUTER_COUNT ||
    outerCount > MAX_OUTER_COUNT
  ) {
    throw new RangeError(
      `outerCount must be an integer in ${MIN_OUTER_COUNT}..${MAX_OUTER_COUNT} ` +
        `(ZKPassport ships main/outer/count_${MIN_OUTER_COUNT}..count_${MAX_OUTER_COUNT}), got ${outerCount}`,
    );
  }
}

/**
 * Wraps a raw bb UltraHonk proof in the envelope `verifier.rs` expects.
 *
 * @param proof the exact bytes of bb's `proof` output file (or `@aztec/bb.js`'s proof
 *   buffer) — not hex, not JSON, and not re-encoded in any way
 * @param options which outer circuit and proving mode produced it
 */
export function encodeUltraHonkProof(
  proof: Uint8Array,
  options: { outerCount: number; variant: UltraHonkProofVariant },
): Uint8Array {
  const { outerCount, variant } = options;
  assertValidOuterCount(outerCount);

  if (proof.length === 0) {
    throw new RangeError('encodeUltraHonkProof: proof is empty');
  }
  // bb's UltraHonk proof is a flat array of 32-byte words. A length that isn't a
  // multiple of 32 means the bytes were mangled somewhere upstream (a hex/utf-8
  // round-trip, most likely) — `verifier.rs` rejects it too, so fail here where the
  // error is actually debuggable.
  if (proof.length % FIELD_ELEMENT_BYTES !== 0) {
    throw new RangeError(
      `encodeUltraHonkProof: proof length ${proof.length} is not a multiple of ${FIELD_ELEMENT_BYTES}; ` +
        'this is not raw bb proof output',
    );
  }
  if (proof.length > 0xffffffff) {
    throw new RangeError(`encodeUltraHonkProof: proof length ${proof.length} exceeds a u32`);
  }

  const out = new Uint8Array(ENVELOPE_HEADER_LEN + proof.length);
  out[0] = ENVELOPE_MAGIC;
  out[1] = ENVELOPE_VERSION;
  out[2] = outerCount;
  out[3] = VARIANT_TO_TAG[variant];
  new DataView(out.buffer, out.byteOffset).setUint32(4, proof.length, false); // big-endian
  out.set(proof, ENVELOPE_HEADER_LEN);
  return out;
}

/**
 * Inverse of {@link encodeUltraHonkProof}. Applies exactly the checks `verifier.rs`'s
 * `parse_envelope` applies, so anything this accepts the runtime will parse too (it may
 * still reject it later, on the VK lookup or the pairing check).
 */
export function decodeUltraHonkProof(envelope: Uint8Array): DecodedUltraHonkProof {
  if (envelope.length < ENVELOPE_HEADER_LEN) {
    throw new RangeError(
      `decodeUltraHonkProof: envelope is ${envelope.length} bytes, shorter than the ${ENVELOPE_HEADER_LEN}-byte header`,
    );
  }
  if (envelope[0] !== ENVELOPE_MAGIC) {
    throw new Error(
      `decodeUltraHonkProof: bad magic 0x${envelope[0].toString(16)}, expected 0x${ENVELOPE_MAGIC.toString(16)}`,
    );
  }
  if (envelope[1] !== ENVELOPE_VERSION) {
    throw new Error(
      `decodeUltraHonkProof: envelope format version ${envelope[1]} is not the supported ${ENVELOPE_VERSION}`,
    );
  }

  const outerCount = envelope[2];
  const variant = TAG_TO_VARIANT[envelope[3]];
  if (variant === undefined) {
    throw new Error(`decodeUltraHonkProof: unknown proof variant tag ${envelope[3]}`);
  }

  const proofLength = new DataView(envelope.buffer, envelope.byteOffset).getUint32(4, false);
  // Exact-length match, same as the runtime: reject truncation *and* trailing padding.
  if (envelope.length !== ENVELOPE_HEADER_LEN + proofLength) {
    throw new RangeError(
      `decodeUltraHonkProof: header declares a ${proofLength}-byte proof but the envelope holds ` +
        `${envelope.length - ENVELOPE_HEADER_LEN} bytes`,
    );
  }
  if (proofLength === 0 || proofLength % FIELD_ELEMENT_BYTES !== 0) {
    throw new RangeError(
      `decodeUltraHonkProof: proof length ${proofLength} is not a non-zero multiple of ${FIELD_ELEMENT_BYTES}`,
    );
  }

  return {
    header: { outerCount, variant, proofLength },
    proof: envelope.slice(ENVELOPE_HEADER_LEN),
  };
}

/**
 * Splits bb's `public_inputs` output file into 32-byte big-endian field elements — the
 * shape `register_citizen`'s `public_inputs: Vec<[u8; 32]>` takes.
 */
export function splitPublicInputs(bytes: Uint8Array): Uint8Array[] {
  if (bytes.length % FIELD_ELEMENT_BYTES !== 0) {
    throw new RangeError(
      `splitPublicInputs: ${bytes.length} bytes is not a multiple of ${FIELD_ELEMENT_BYTES}`,
    );
  }
  const out: Uint8Array[] = [];
  for (let i = 0; i < bytes.length; i += FIELD_ELEMENT_BYTES) {
    out.push(bytes.slice(i, i + FIELD_ELEMENT_BYTES));
  }
  return out;
}

/**
 * Same, for the `0x`-prefixed hex strings `@aztec/bb.js` hands back instead of a buffer.
 * Each string must be a full 32-byte word; short hex is rejected rather than
 * left-padded, since a silently-padded field element is a very quiet way to produce a
 * proof that never verifies.
 */
export function publicInputsFromHex(hex: readonly string[]): Uint8Array[] {
  return hex.map((value, index) => {
    const digits = value.startsWith('0x') || value.startsWith('0X') ? value.slice(2) : value;
    if (digits.length !== FIELD_ELEMENT_BYTES * 2) {
      throw new RangeError(
        `publicInputsFromHex: element ${index} has ${digits.length} hex digits, expected ${FIELD_ELEMENT_BYTES * 2}`,
      );
    }
    if (!/^[0-9a-fA-F]+$/.test(digits)) {
      throw new RangeError(`publicInputsFromHex: element ${index} is not hexadecimal`);
    }
    const out = new Uint8Array(FIELD_ELEMENT_BYTES);
    for (let i = 0; i < FIELD_ELEMENT_BYTES; i++) {
      out[i] = parseInt(digits.substr(i * 2, 2), 16);
    }
    return out;
  });
}

/** Reads a 32-byte big-endian field element as a bigint. */
export function fieldToBigInt(value: Uint8Array): bigint {
  if (value.length !== FIELD_ELEMENT_BYTES) {
    throw new RangeError(`fieldToBigInt: expected ${FIELD_ELEMENT_BYTES} bytes, got ${value.length}`);
  }
  let out = 0n;
  for (const byte of value) {
    out = (out << 8n) | BigInt(byte);
  }
  return out;
}

/** The ZKPassport outer circuit's public inputs, named. */
export interface ZkPassportOuterPublicInputs {
  certificateRegistryRoot: Uint8Array;
  circuitRegistryRoot: Uint8Array;
  /** Unix seconds. */
  currentDate: bigint;
  serviceScope: Uint8Array;
  serviceSubscope: Uint8Array;
  paramCommitments: Uint8Array[];
  nullifierType: Uint8Array;
  /** `H(private_nullifier, service_scope, service_subscope)` — the on-chain nullifier. */
  scopedNullifier: Uint8Array;
  oprfPkHash: Uint8Array;
}

/**
 * Validates a public-input array against the `count_N` layout, applying exactly the
 * checks `verifier.rs::check_public_inputs` applies. Throws on the first failure.
 *
 * Worth calling before submitting: the runtime's rejection is a bare
 * `Error::InvalidZKProof` with no detail, whereas this says which field is wrong.
 */
export function assertValidPublicInputs(
  outerCount: number,
  publicInputs: readonly Uint8Array[],
): void {
  const expected = expectedPublicInputCount(outerCount);
  if (publicInputs.length !== expected) {
    throw new RangeError(
      `assertValidPublicInputs: count_${outerCount} exposes ${expected} public inputs, got ${publicInputs.length}`,
    );
  }

  publicInputs.forEach((value, index) => {
    if (value.length !== FIELD_ELEMENT_BYTES) {
      throw new RangeError(
        `assertValidPublicInputs: public input ${index} is ${value.length} bytes, expected ${FIELD_ELEMENT_BYTES}`,
      );
    }
    if (fieldToBigInt(value) >= BN254_SCALAR_FIELD_MODULUS) {
      throw new RangeError(
        `assertValidPublicInputs: public input ${index} is not a canonical BN254 field element`,
      );
    }
  });

  // `current_date` is a u64 in the circuit, so it must fit in the low 8 bytes. Catching
  // this here is how a public-input array still built in Rarimo's old order gets caught
  // before it wastes an on-chain call.
  const currentDate = fieldToBigInt(publicInputs[2]);
  if (currentDate >= 1n << 64n) {
    throw new RangeError(
      'assertValidPublicInputs: current_date (index 2) does not fit in a u64 — is this array still in the old Rarimo order?',
    );
  }

  const disclosureCount = disclosureCountFor(outerCount);
  if (fieldToBigInt(publicInputs[6 + disclosureCount]) === 0n) {
    throw new RangeError(
      `assertValidPublicInputs: scoped_nullifier (index ${6 + disclosureCount}) is zero; the circuit asserts it is not`,
    );
  }
}

/**
 * Names a validated public-input array. Validates first, so a mis-ordered array is
 * rejected rather than silently mislabelled.
 */
export function describePublicInputs(
  outerCount: number,
  publicInputs: readonly Uint8Array[],
): ZkPassportOuterPublicInputs {
  assertValidPublicInputs(outerCount, publicInputs);
  const disclosureCount = disclosureCountFor(outerCount);
  return {
    certificateRegistryRoot: publicInputs[0],
    circuitRegistryRoot: publicInputs[1],
    currentDate: fieldToBigInt(publicInputs[2]),
    serviceScope: publicInputs[3],
    serviceSubscope: publicInputs[4],
    paramCommitments: publicInputs.slice(5, 5 + disclosureCount) as Uint8Array[],
    nullifierType: publicInputs[5 + disclosureCount],
    scopedNullifier: publicInputs[6 + disclosureCount],
    oprfPkHash: publicInputs[7 + disclosureCount],
  };
}
