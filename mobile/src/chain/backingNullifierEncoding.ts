/**
 * Encodes/decodes the byte envelope `runtime/src/backing_nullifier_verifier.rs` expects for
 * `pallet_elections::back_delegate`/`remove_backing`'s `zk_proof` argument, plus the matching
 * `public_inputs: [[u8; 32]; 4]` shape.
 *
 * **This file and `runtime/src/backing_nullifier_verifier.rs` are one contract split across two
 * languages**, the same relationship `proofEncoding.ts` has with `runtime/src/verifier.rs` for
 * the ZKPassport outer-proof envelope — see that module's doc comment for the general pattern.
 * This is a genuinely different envelope, not a reuse: distinct magic byte (`0x42`/'B', vs
 * `0x5A`/'Z'), one byte shorter (no `outer_count` field — this circuit has no ZKPassport-style
 * family of variants, exactly one circuit, exactly one VK), so a proof for one can never be
 * misparsed as the other's.
 *
 * # Envelope layout
 *
 * ```text
 * [ 0]      magic           0x42 ('B' for backing-nullifier)
 * [ 1]      format_version  0x01
 * [ 2]      proof_variant   0 = ZK (`bb prove -t evm`), 1 = Plain (`bb prove -t evm-no-zk`)
 * [ 3.. 7]  proof_len       u32, big-endian
 * [ 7..7+L] proof           the raw bytes of bb's `proof` output file, verbatim
 * ```
 *
 * # Public inputs
 *
 * Exactly 4 canonical BN254 field elements, in this fixed order (confirmed against a real
 * `bb prove -t evm` run of the circuit's own fixture — see `backing_nullifier_verifier.rs`'s
 * module docs):
 *
 * ```text
 * index   field
 * -----   ------------------------
 * 0       root
 * 1       delegate_persona_id
 * 2       max_backings_per_citizen
 * 3       backing_nullifier
 * ```
 *
 * Unlike the ZKPassport outer proof, this circuit has no aggregation/pairing-point object to
 * worry about prepending — it is not itself a recursive verifier of anything, so `bb`'s
 * `public_inputs` output file is exactly these 4 words, nothing more.
 */
import {
  BN254_SCALAR_FIELD_MODULUS,
  FIELD_ELEMENT_BYTES,
  fieldToBigInt,
  splitPublicInputs,
  type UltraHonkProofVariant,
} from './proofEncoding';

/** Envelope magic byte — `'B'`, for backing-nullifier. Mirrors `backing_nullifier_verifier.rs::ENVELOPE_MAGIC`. */
export const BACKING_ENVELOPE_MAGIC = 0x42;

/** Envelope format version. Mirrors `backing_nullifier_verifier.rs::ENVELOPE_VERSION`. */
export const BACKING_ENVELOPE_VERSION = 0x01;

/** Fixed envelope header length, in bytes. Mirrors `backing_nullifier_verifier.rs::ENVELOPE_HEADER_LEN`. */
export const BACKING_ENVELOPE_HEADER_LEN = 7;

/** Number of public inputs this circuit exposes. Mirrors `backing_nullifier_verifier.rs::PUBLIC_INPUT_COUNT`. */
export const BACKING_PUBLIC_INPUT_COUNT = 4;

export const BACKING_ROOT_INDEX = 0;
export const BACKING_DELEGATE_PERSONA_ID_INDEX = 1;
export const BACKING_MAX_BACKINGS_PER_CITIZEN_INDEX = 2;
export const BACKING_NULLIFIER_INDEX = 3;

const VARIANT_TO_TAG: Record<UltraHonkProofVariant, number> = { zk: 0, plain: 1 };
const TAG_TO_VARIANT: Record<number, UltraHonkProofVariant> = { 0: 'zk', 1: 'plain' };

export interface BackingNullifierEnvelopeHeader {
  variant: UltraHonkProofVariant;
  /** Length of the raw bb proof, in bytes. */
  proofLength: number;
}

export interface DecodedBackingNullifierProof {
  header: BackingNullifierEnvelopeHeader;
  proof: Uint8Array;
}

/**
 * Wraps a raw bb UltraHonk proof of the `backing-nullifier` circuit in the envelope
 * `backing_nullifier_verifier.rs` expects.
 *
 * @param proof the exact bytes of bb's `proof` output file (or `@aztec/bb.js`'s proof buffer)
 *   — not hex, not JSON, and not re-encoded in any way. Must use the `zk` variant (`-t evm`) for
 *   a real submission — `plain` exists only because the runtime envelope can express it and
 *   round-tripping it should be testable, exactly as `proofEncoding.ts`'s
 *   `encodeUltraHonkProof` documents for the ZKPassport envelope.
 */
export function encodeBackingNullifierProof(
  proof: Uint8Array,
  variant: UltraHonkProofVariant,
): Uint8Array {
  if (proof.length === 0) {
    throw new RangeError('encodeBackingNullifierProof: proof is empty');
  }
  if (proof.length % FIELD_ELEMENT_BYTES !== 0) {
    throw new RangeError(
      `encodeBackingNullifierProof: proof length ${proof.length} is not a multiple of ${FIELD_ELEMENT_BYTES}; ` +
        'this is not raw bb proof output',
    );
  }
  if (proof.length > 0xffffffff) {
    throw new RangeError(`encodeBackingNullifierProof: proof length ${proof.length} exceeds a u32`);
  }

  const out = new Uint8Array(BACKING_ENVELOPE_HEADER_LEN + proof.length);
  out[0] = BACKING_ENVELOPE_MAGIC;
  out[1] = BACKING_ENVELOPE_VERSION;
  out[2] = VARIANT_TO_TAG[variant];
  new DataView(out.buffer, out.byteOffset).setUint32(3, proof.length, false); // big-endian
  out.set(proof, BACKING_ENVELOPE_HEADER_LEN);
  return out;
}

/**
 * Inverse of {@link encodeBackingNullifierProof}. Applies exactly the checks
 * `backing_nullifier_verifier.rs`'s `parse_envelope` applies, so anything this accepts the
 * runtime will parse too (it may still reject it later, on the pairing check).
 */
export function decodeBackingNullifierProof(envelope: Uint8Array): DecodedBackingNullifierProof {
  if (envelope.length < BACKING_ENVELOPE_HEADER_LEN) {
    throw new RangeError(
      `decodeBackingNullifierProof: envelope is ${envelope.length} bytes, shorter than the ` +
        `${BACKING_ENVELOPE_HEADER_LEN}-byte header`,
    );
  }
  if (envelope[0] !== BACKING_ENVELOPE_MAGIC) {
    throw new Error(
      `decodeBackingNullifierProof: bad magic 0x${envelope[0].toString(16)}, expected 0x${BACKING_ENVELOPE_MAGIC.toString(16)}`,
    );
  }
  if (envelope[1] !== BACKING_ENVELOPE_VERSION) {
    throw new Error(
      `decodeBackingNullifierProof: envelope format version ${envelope[1]} is not the supported ${BACKING_ENVELOPE_VERSION}`,
    );
  }

  const variant = TAG_TO_VARIANT[envelope[2]];
  if (variant === undefined) {
    throw new Error(`decodeBackingNullifierProof: unknown proof variant tag ${envelope[2]}`);
  }

  const proofLength = new DataView(envelope.buffer, envelope.byteOffset).getUint32(3, false);
  if (envelope.length !== BACKING_ENVELOPE_HEADER_LEN + proofLength) {
    throw new RangeError(
      `decodeBackingNullifierProof: header declares a ${proofLength}-byte proof but the envelope holds ` +
        `${envelope.length - BACKING_ENVELOPE_HEADER_LEN} bytes`,
    );
  }
  if (proofLength === 0 || proofLength % FIELD_ELEMENT_BYTES !== 0) {
    throw new RangeError(
      `decodeBackingNullifierProof: proof length ${proofLength} is not a non-zero multiple of ${FIELD_ELEMENT_BYTES}`,
    );
  }

  return {
    header: { variant, proofLength },
    proof: envelope.slice(BACKING_ENVELOPE_HEADER_LEN),
  };
}

/**
 * Validates a public-input array against the fixed 4-field `backing-nullifier` layout, applying
 * exactly the checks `backing_nullifier_verifier.rs::check_public_inputs` applies (structural
 * only — canonical field elements, exact count; the semantic checks against live chain storage
 * are `pallet-elections`' `verify_backing_proof`'s job, not this function's or the runtime
 * verifier's — see that module's "What this module does not check" docs).
 */
export function assertValidBackingPublicInputs(publicInputs: readonly Uint8Array[]): void {
  if (publicInputs.length !== BACKING_PUBLIC_INPUT_COUNT) {
    throw new RangeError(
      `assertValidBackingPublicInputs: expected ${BACKING_PUBLIC_INPUT_COUNT} public inputs, got ${publicInputs.length}`,
    );
  }
  publicInputs.forEach((value, index) => {
    if (value.length !== FIELD_ELEMENT_BYTES) {
      throw new RangeError(
        `assertValidBackingPublicInputs: public input ${index} is ${value.length} bytes, expected ${FIELD_ELEMENT_BYTES}`,
      );
    }
    if (fieldToBigInt(value) >= BN254_SCALAR_FIELD_MODULUS) {
      throw new RangeError(
        `assertValidBackingPublicInputs: public input ${index} is not a canonical BN254 field element`,
      );
    }
  });
}

export { splitPublicInputs };
