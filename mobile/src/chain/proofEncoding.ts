/**
 * Convert a Groth16 proof from snarkjs JSON output format (what
 * `@iden3/react-native-rapidsnark`'s `groth16Prove()` returns) into the
 * 129-byte binary format `runtime/src/verifier.rs` expects for
 * `pallet_identity_zk::register_citizen`'s `zk_proof` argument.
 *
 * This is a direct TypeScript port of `_compress_g1` / `_compress_g2` from
 * `scripts/convert_vk.py` — that script is the authoritative, already
 * production-validated reference for this exact ark-serialize compressed
 * point format (it is used to build the `runtime/assets/vk_sha256.bin` /
 * `vk_sha1.bin` files that pallet-identity-zk verifies proofs against).
 *
 * # Output format (129 bytes), see `runtime/src/verifier.rs` module doc:
 * ```text
 * [  0.. 32]  A   G1 compressed  (ark-serialize LE, flags in byte 31)
 * [ 32.. 96]  B   G2 compressed  (ark-serialize LE, flags in byte 63)
 * [ 96..128]  C   G1 compressed
 * [128]       variant  0 = SHA-256 passport circuit, 1 = SHA-1 circuit
 * ```
 *
 * # Compression format (ark-serialize LE with flags in high bits of last byte)
 *   G1 (32 bytes):
 *     bytes 0-31 = x in little-endian (254-bit Fq, top 2 bits always 0 for valid points)
 *     byte 31 bit 7 (0x80) = YIsNegative flag  (set when y * 2 >= P)
 *     byte 31 bit 6 (0x40) = Infinity flag      (not set for regular points)
 *
 *   G2 (64 bytes):
 *     bytes  0-31 = x.c0 in little-endian
 *     bytes 32-63 = x.c1 in little-endian   (x = c0 + c1*u in Fq2)
 *     byte  63 bit 7 (0x80) = YIsNegative flag
 *     byte  63 bit 6 (0x40) = Infinity flag
 *     is_negative for Fq2: c1 != 0 ? c1*2 >= P : c0*2 >= P
 */

/** BN254 / alt_bn128 base-field prime — same constant as `scripts/convert_vk.py`'s `P`. */
export const BN254_BASE_FIELD_PRIME = BigInt(
  '21888242871839275222246405745257275088696311157297823662689037894645226208583',
);

/** A G1 point in snarkjs JSON shape: `[x_str, y_str, "1"]` (affine, z = "1"). */
export type SnarkjsG1 = [string, string, string];

/**
 * A G2 point in snarkjs JSON shape: `[[x0_str, x1_str], [y0_str, y1_str], ["1", "0"]]`
 * (affine, z = ["1", "0"]).
 */
export type SnarkjsG2 = [[string, string], [string, string], [string, string]];

/** A full snarkjs Groth16 proof, as returned by `groth16Prove()`. */
export interface SnarkjsGroth16Proof {
  pi_a: SnarkjsG1;
  pi_b: SnarkjsG2;
  pi_c: SnarkjsG1;
}

/**
 * True when `n` is in the "upper half" of Fq: `n * 2 >= P` (ark-serialize
 * convention). Mirrors `_fq_is_negative` in `scripts/convert_vk.py`.
 */
function fqIsNegative(n: bigint): boolean {
  return n * 2n >= BN254_BASE_FIELD_PRIME;
}

/**
 * Serialize a non-negative bigint (< 2^256) into `length` little-endian bytes.
 * Equivalent to Python's `int.to_bytes(length, 'little')`.
 */
function toLeBytes(n: bigint, length: number): Uint8Array {
  if (n < 0n) {
    throw new RangeError(`toLeBytes: value must be non-negative, got ${n}`);
  }
  const out = new Uint8Array(length);
  let v = n;
  for (let i = 0; i < length; i++) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  if (v !== 0n) {
    throw new RangeError(`toLeBytes: value ${n} does not fit in ${length} bytes`);
  }
  return out;
}

/**
 * Compress a G1 affine point `[x_str, y_str, "1"]` to 32 bytes (ark-serialize
 * format). snarkjs encodes projective coords with z="1" meaning the point is
 * already affine. Port of `_compress_g1` in `scripts/convert_vk.py`.
 */
export function compressG1(point: SnarkjsG1): Uint8Array {
  const x = BigInt(point[0]);
  const y = BigInt(point[1]);
  if (BigInt(point[2]) !== 1n) {
    throw new Error('compressG1: expected affine (z=1) snarkjs point');
  }

  const b = toLeBytes(x, 32);
  b[31] &= 0x3f; // clear top 2 flag bits
  if (fqIsNegative(y)) {
    b[31] |= 0x80; // YIsNegative
  }
  return b;
}

/**
 * Compress a G2 affine point `[[x0,x1],[y0,y1],["1","0"]]` to 64 bytes. Fq2
 * element: x = x0 + x1*u, serialized as c0 (32 LE bytes) || c1 (32 LE
 * bytes). Port of `_compress_g2` in `scripts/convert_vk.py`.
 */
export function compressG2(point: SnarkjsG2): Uint8Array {
  const x0 = BigInt(point[0][0]);
  const x1 = BigInt(point[0][1]);
  const y0 = BigInt(point[1][0]);
  const y1 = BigInt(point[1][1]);

  // is_negative for Fq2 (ark-ff convention): check c1 first, then c0 if c1 == 0
  const yIsNeg = y1 !== 0n ? fqIsNegative(y1) : fqIsNegative(y0);

  const b = new Uint8Array(64);
  b.set(toLeBytes(x0, 32), 0);
  b.set(toLeBytes(x1, 32), 32);
  b[63] &= 0x3f;
  if (yIsNeg) {
    b[63] |= 0x80;
  }
  return b;
}

/**
 * Encode a snarkjs Groth16 proof (`pi_a`/`pi_b`/`pi_c`) plus a circuit
 * variant tag into the 129-byte format `pallet_identity_zk::register_citizen`
 * expects for `zk_proof`.
 *
 * @param snarkjsProof the proof object as returned by `groth16Prove()`
 * @param variant 0 = SHA-256 passport circuit, 1 = SHA-1 circuit
 */
export function encodeGroth16Proof(
  snarkjsProof: SnarkjsGroth16Proof,
  variant: 0 | 1,
): Uint8Array {
  const a = compressG1(snarkjsProof.pi_a);
  const b = compressG2(snarkjsProof.pi_b);
  const c = compressG1(snarkjsProof.pi_c);

  const out = new Uint8Array(129);
  out.set(a, 0);
  out.set(b, 32);
  out.set(c, 96);
  out[128] = variant;
  return out;
}
