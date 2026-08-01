/**
 * Mobile-side half of our own Document Signer Certificate (DSC) registry —
 * built and hosted by us instead of read from a vendor-hosted registry (see
 * HANDOFF.md log #63 for the original "why our own tree" rationale, which
 * carries over unchanged under ZKPassport — confirmed in log #66, this
 * file's `certificate_registry_root` public input is a plain circuit input,
 * not hardwired to any vendor contract). The other half — actually building
 * the depth-16 tree from a set of trusted certificates, which only ever
 * needs to run off-chain as part of the governance-approved
 * certificate-onboarding process, never on a phone — lives in
 * `scripts/certificate-registry/`, which depends directly on
 * `@zkpassport/utils` (ZKPassport's own first-party registry-building code)
 * rather than re-deriving the tree-building logic a second time. This file
 * only has what a phone actually needs at proving time: computing its own
 * DSC's fingerprint (to know which precomputed leaf/proof to fetch) and
 * independently sanity-checking a fetched proof before spending minutes on
 * an expensive local proving attempt with bad inputs.
 *
 * REPLACES the previous Rarimo-era version of this file (depth-80 Poseidon1
 * SMT, key=value=pubkeyHash set-membership design — see HANDOFF.md log #63)
 * per the log #65 decision to drop Rarimo for ZKPassport. Per log #66's
 * research, ZKPassport's registry is a fundamentally different design, not
 * a reparameterization of the old one: Poseidon2 (not Poseidon1), a
 * fixed-depth-16 INDEX-addressed incremental binary tree (not a
 * key-addressed sparse tree), and a structured multi-field leaf (not a bare
 * pubkeyHash). Nothing from the old implementation carries over byte-for-byte.
 *
 * Tree spec (reverse-engineered from the actual circuit source — verified
 * against `common/src/lib.nr` in zkpassport/circuits — and cross-checked
 * against `@zkpassport/utils`' own `registry` module, which independently
 * implements the identical formulas for its own tree-building tool; see
 * this file's test suite for concrete cross-checked vectors captured from
 * `@zkpassport/utils@0.37.3` + `@zkpassport/poseidon2@0.6.2`):
 *
 *  - Hash function: Poseidon2 over the BN254 scalar field (NOT the Poseidon1
 *    the old Rarimo-era tree used). `@zkpassport/poseidon2` is a
 *    zero-dependency, pure-TypeScript implementation (no WASM) — safe to
 *    bundle into React Native/Hermes, unlike `@aztec/bb.js` (WASM-backed,
 *    used only in `scripts/certificate-registry/`'s off-chain tooling where
 *    that's not a constraint). Confirmed byte-identical to Noir's
 *    `Poseidon2::hash` and to `@aztec/bb.js`'s `poseidon2Hash` via a genuine
 *    three-way cross-check (see HANDOFF.md log #66).
 *  - A fixed-depth-16, arity-2, INDEX-addressed incremental Merkle tree —
 *    not a sparse/key-addressed tree like the old design. Leaves are
 *    assigned sequential integer positions (0, 1, 2, ...), not looked up by
 *    key; a leaf's Merkle path is derived purely from its integer index's
 *    bits, least-significant bit first (bit 0 selects the branch at the
 *    leaf's immediate parent; bit 15 selects the branch at the root).
 *    Missing/unfilled positions at any level hash as a precomputed
 *    "zero subtree" value (0 at the leaf level, `Poseidon2(zero, zero)` one
 *    level up, and so on) — there is no sparse/Empty-side branching
 *    ambiguity the old depth-80 SMT design had to account for.
 *  - `@zkpassport/utils`' own tree-builder additionally SORTS all leaf
 *    hashes ascending (as field elements) before assigning them index
 *    positions — this project's own tree-building tool
 *    (`scripts/certificate-registry/`) must do the same for its output to
 *    match what any ZKPassport-circuit-compatible verifier expects.
 *  - Leaf hash (schema v1 — the version this project targets; see
 *    `calculate_certificate_registry_leaf_v1` in `common/src/lib.nr`):
 *      leaf = Poseidon2(
 *        tags[0], tags[1], tags[2],
 *        packBE(cert_type[1 byte] || country[3 bytes] || expiry[4 bytes BE]),
 *        fingerprint,
 *        ...packBE31(public_key_bytes),
 *      )
 *    `packBE` packs an arbitrary byte string into big-endian field limbs of
 *    up to 31 bytes each; a byte string under 31 bytes packs into a single
 *    field. `country` is the 3-character ISO 3166-1 alpha-3 code, encoded as
 *    its raw ASCII bytes. `public_key_bytes` is the RSA modulus for RSA
 *    keys, or `x || y` (raw big-endian coordinates) for EC keys — see
 *    `publicKeyToBytes` in `@zkpassport/utils`.
 *  - Certificate fingerprint: NOT the same packing convention as the rest of
 *    the leaf — `fingerprint = Poseidon2(packLE31(certificate_der_bytes))`,
 *    where `packLE31` packs the *entire* DER-encoded certificate into
 *    LITTLE-endian field limbs of up to 31 bytes each (first byte of each
 *    chunk is the chunk's least-significant byte — the opposite convention
 *    from `packBE` above). Confirmed from `packLeBytesAndHashPoseidon2` in
 *    `@zkpassport/utils` (this is genuinely a different byte-packing
 *    direction than the rest of the leaf; do not "simplify" the two packers
 *    into one without re-checking against the source, this project has been
 *    burned before by silently-wrong shift/endianness assumptions — see
 *    HANDOFF.md log #62's dg15Shift/aaShift mix-up).
 *  - Merkle combine step (`compute_merkle_root` in `common/src/lib.nr`):
 *    walking from the leaf upward, at level `i` (0-indexed from the leaf),
 *    if bit `i` of the leaf's index is 0, the running value is the LEFT
 *    child and the sibling is the RIGHT child; if bit `i` is 1, the running
 *    value is the RIGHT child. Combine as `Poseidon2(left, right)`.
 *
 * Field element byte encoding for chain submission: big-endian 32 bytes,
 * matching `Fr::from_be_bytes_mod_order` in runtime/src/verifier.rs (every
 * other public input in this project's ZK flow uses the same convention).
 */
import { poseidon2Hash } from '@zkpassport/poseidon2';
import {
  decodeAsn1,
  getEcdsaKeyLocation,
  getRsaKeyLocation,
  extractEcdsaPubkey,
  extractRsaPubkey,
  type Pubkey,
} from './sodParser';

export const TREE_DEPTH = 16;

export const CERT_TYPE_CSCA = 1;
export const CERT_TYPE_DSC = 2;

// ---------------------------------------------------------------------------
// Certificate parsing
// ---------------------------------------------------------------------------

/**
 * Extracts the SubjectPublicKeyInfo from a standalone DER-encoded X.509
 * certificate (a DSC or CSCA cert, as opposed to a passport SOD). Reuses
 * sodParser.ts's shape-based DFS helpers, which don't care whether they're
 * walking a SOD's CMS structure or a bare `Certificate ::= SEQUENCE {
 * tbsCertificate, signatureAlgorithm, signatureValue }` — both contain a
 * SubjectPublicKeyInfo with the same recognizable shape. Circuit-agnostic —
 * unaffected by the Rarimo -> ZKPassport switch.
 */
export function extractPubkeyFromCertificate(certDer: Uint8Array): Pubkey {
  const tree = decodeAsn1(certDer);
  if (getEcdsaKeyLocation(tree)) return extractEcdsaPubkey(tree);
  if (getRsaKeyLocation(tree)) return extractRsaPubkey(tree);
  throw new Error('extractPubkeyFromCertificate: no recognizable RSA or ECDSA SubjectPublicKeyInfo found');
}

/**
 * Standard RSA modulus sizes (bits) actually seen on ICAO DSCs/CSCAs. A
 * proper RSA modulus N = p*q always has a bit-length of exactly `keySize`
 * or `keySize - 1` — never meaningfully less — so rounding the value's own
 * bit-length UP to the nearest entry here recovers the true key byte-width
 * even when `sodParser.ts`'s `BigInt(...).toString(16)` has silently
 * dropped a DER-mandated leading 0x00 byte (added whenever the modulus's
 * top bit is set, to keep the ASN.1 INTEGER non-negative — true for
 * roughly half of all real RSA moduli). Without this, `publicKeyToBytes`
 * would be off by one byte for ~50% of real certificates, shifting every
 * subsequent packed field limb and silently producing a wrong leaf hash.
 */
const STANDARD_RSA_KEY_BITS = [1024, 2048, 3072, 4096, 6144, 8192];

function rsaModulusByteLength(n: bigint): number {
  const bitLength = n.toString(2).length;
  const keyBits = STANDARD_RSA_KEY_BITS.find((size) => size >= bitLength);
  if (keyBits === undefined) {
    throw new Error(`rsaModulusByteLength: modulus bit-length ${bitLength} exceeds all known standard RSA key sizes`);
  }
  return keyBits / 8;
}

/** Serializes a parsed public key to the raw bytes the leaf hash packs — RSA: modulus (padded to its true key byte-width, see `rsaModulusByteLength`); EC: x || y. Matches `publicKeyToBytes` in `@zkpassport/utils`. */
export function publicKeyToBytes(pubkey: Pubkey): Uint8Array {
  if (pubkey.kind === 'rsa') {
    const n = BigInt('0x' + pubkey.n);
    const byteLength = rsaModulusByteLength(n);
    const bytes = hexToBytes(pubkey.n);
    if (bytes.length === byteLength) return bytes;
    const padded = new Uint8Array(byteLength);
    padded.set(bytes, byteLength - bytes.length);
    return padded;
  }
  return concatBytes(padToFieldWidth(hexToBytes(pubkey.x)), padToFieldWidth(hexToBytes(pubkey.y)));
}

/**
 * EC coordinates suffer the same leading-zero-byte loss `sodParser.ts`'s
 * `BigInt(...).toString(16)` inflicts on RSA moduli above, just from a
 * different cause (a curve field element's fixed-width encoding, not DER's
 * sign-byte rule) — a coordinate whose top byte happens to be zero loses
 * that byte on the round-trip through a hex string. Standard field byte
 * widths for every curve ZKPassport's DSC signature-check circuits support
 * (`sig-check/dsc/**\/ecdsa/{nist,brainpool}/*`); rounding a coordinate's
 * own byte-length up to the nearest one recovers the true width.
 */
const STANDARD_EC_FIELD_BYTES = [24, 28, 32, 40, 48, 64, 66];

function padToFieldWidth(bytes: Uint8Array): Uint8Array {
  const width = STANDARD_EC_FIELD_BYTES.find((size) => size >= bytes.length);
  if (width === undefined) {
    throw new Error(`padToFieldWidth: coordinate byte-length ${bytes.length} exceeds all known standard EC field sizes`);
  }
  if (bytes.length === width) return bytes;
  const padded = new Uint8Array(width);
  padded.set(bytes, width - bytes.length);
  return padded;
}

// ---------------------------------------------------------------------------
// Byte/field packing
// ---------------------------------------------------------------------------

function hexToBytes(hex: string): Uint8Array {
  const clean = hex.length % 2 === 0 ? hex : '0' + hex;
  const out = new Uint8Array(clean.length / 2);
  for (let i = 0; i < out.length; i++) out[i] = parseInt(clean.slice(i * 2, i * 2 + 2), 16);
  return out;
}

function concatBytes(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length);
  out.set(a, 0);
  out.set(b, a.length);
  return out;
}

/** Packs `bytes` into big-endian field limbs of up to `chunkSize` bytes each. Matches Noir's `pack_be_bytes_into_field(s)` / `@zkpassport/utils`' internal `m()` helper: the FIRST bytes of the input become the shortest (possibly partial) chunk, placed LAST in the returned array — i.e. limb 0 is the least-significant (final `chunkSize` bytes), matching the order the leaf hash expects to feed straight into Poseidon2. */
export function packBeBytesIntoFields(bytes: Uint8Array, chunkSize = 31): bigint[] {
  if (bytes.length === 0) return [];
  const numChunks = Math.ceil(bytes.length / chunkSize);
  const firstChunkSize = bytes.length % chunkSize || chunkSize;
  const out: bigint[] = new Array(numChunks);
  let offset = 0;
  for (let chunkIndex = numChunks - 1; chunkIndex >= 0; chunkIndex--) {
    const size = chunkIndex === numChunks - 1 ? firstChunkSize : chunkSize;
    let value = 0n;
    for (let j = 0; j < size; j++) value = (value << 8n) | BigInt(bytes[offset++]);
    out[chunkIndex] = value;
  }
  return out;
}

/** Packs `bytes` (expected to fit in a single field, i.e. <= 31 bytes) into one big-endian field element. */
export function packBeBytesIntoField(bytes: Uint8Array): bigint {
  let value = 0n;
  for (const byte of bytes) value = (value << 8n) | BigInt(byte);
  return value;
}

/** Packs `bytes` into little-endian field limbs of up to `chunkSize` bytes each — the DIFFERENT convention `packLeBytesAndHashPoseidon2` uses for certificate fingerprints only. Matches `@zkpassport/utils`' `packLeBytesIntoFields`: chunks are taken front-to-back from the input (chunk 0 = first `chunkSize` bytes, in original order), and within each chunk the FIRST byte is the LEAST-significant byte of the resulting field. */
export function packLeBytesIntoFields(bytes: Uint8Array, chunkSize = 31): bigint[] {
  if (bytes.length === 0) return [];
  const numChunks = Math.ceil(bytes.length / chunkSize);
  const out: bigint[] = new Array(numChunks);
  let offset = 0;
  for (let chunkIndex = 0; chunkIndex < numChunks; chunkIndex++) {
    const size = Math.min(chunkSize, bytes.length - offset);
    let value = 0n;
    for (let j = size - 1; j >= 0; j--) value = (value << 8n) | BigInt(bytes[offset + j]);
    offset += size;
    out[chunkIndex] = value;
  }
  return out;
}

/** Encodes a BN254 scalar field element as 32 big-endian bytes — matches `Fr::from_be_bytes_mod_order` in runtime/src/verifier.rs. */
export function fieldElementToBytes32BE(x: bigint): Uint8Array {
  const out = new Uint8Array(32);
  let rem = x;
  for (let i = 31; i >= 0; i--) {
    out[i] = Number(rem & 0xffn);
    rem >>= 8n;
  }
  return out;
}

// ---------------------------------------------------------------------------
// Certificate fingerprint
// ---------------------------------------------------------------------------

/** Computes a certificate's fingerprint — `Poseidon2(packLE31(der_bytes))` — matching `packLeBytesAndHashPoseidon2` in `@zkpassport/utils`. Used both as a leaf field and, independently, as a way for the phone to name/identify its own DSC without depending on any particular tree it may or may not already have a proof for. */
export function computeCertificateFingerprint(certDer: Uint8Array): bigint {
  return poseidon2Hash(packLeBytesIntoFields(certDer, 31));
}

// ---------------------------------------------------------------------------
// Certificate tree leaf hash (schema v1)
// ---------------------------------------------------------------------------

export interface CertificateTreeLeaf {
  /** Up to 3 bit-flag tag fields — see `tagsArrayToBitsFlag` in `@zkpassport/utils`. Pass `[0n, 0n, 0n]` if untagged. */
  tags: [bigint, bigint, bigint];
  certType: typeof CERT_TYPE_CSCA | typeof CERT_TYPE_DSC;
  /** ISO 3166-1 alpha-3 issuing country code, e.g. "USA". */
  country: string;
  /** RSA modulus, or EC `x || y` — see `publicKeyToBytes`. */
  publicKey: Uint8Array;
  /** Certificate `validity.notAfter`, as a Unix timestamp (seconds). */
  expiry: number;
  fingerprint: bigint;
}

/** Computes a certificate tree leaf's hash — see this module's doc comment for the exact formula. */
export function calculateCertificateLeafHash(leaf: CertificateTreeLeaf): bigint {
  if (leaf.country.length !== 3) {
    throw new Error(`calculateCertificateLeafHash: country must be a 3-character alpha-3 code, got "${leaf.country}"`);
  }
  const countryBytes = new TextEncoder().encode(leaf.country);
  const metaBytes = new Uint8Array(8);
  metaBytes[0] = leaf.certType;
  metaBytes.set(countryBytes, 1);
  metaBytes[4] = (leaf.expiry >>> 24) & 0xff;
  metaBytes[5] = (leaf.expiry >>> 16) & 0xff;
  metaBytes[6] = (leaf.expiry >>> 8) & 0xff;
  metaBytes[7] = leaf.expiry & 0xff;
  const packedMeta = packBeBytesIntoField(metaBytes);
  const packedPubkey = packBeBytesIntoFields(leaf.publicKey, 31);
  return poseidon2Hash([...leaf.tags, packedMeta, leaf.fingerprint, ...packedPubkey]);
}

// ---------------------------------------------------------------------------
// Merkle tree (depth-16, index-addressed, Poseidon2)
// ---------------------------------------------------------------------------

/** Precomputed "empty subtree" hash at each level, from the leaf level (index 0, value 0) up to (but not including) the root. Level `i`'s zero value is `Poseidon2(zero_{i-1}, zero_{i-1})`, seeded with `zero_0 = 0`. */
export function computeZeroes(depth: number): bigint[] {
  const zeroes: bigint[] = [0n];
  for (let i = 1; i < depth; i++) {
    zeroes.push(poseidon2Hash([zeroes[i - 1], zeroes[i - 1]]));
  }
  return zeroes;
}

/**
 * Re-derives a root from a leaf hash + its integer index + sibling path,
 * exactly the way `compute_merkle_root` does in `common/src/lib.nr`. Used
 * both by this module's own tests and by `scripts/certificate-registry/` to
 * self-check a freshly built tree's proofs before publishing them.
 */
export function computeMerkleRoot(leafHash: bigint, index: number, hashPath: bigint[]): bigint {
  if (hashPath.length !== TREE_DEPTH) {
    throw new Error(`computeMerkleRoot: expected ${TREE_DEPTH} siblings, got ${hashPath.length}`);
  }
  let current = leafHash;
  for (let i = 0; i < TREE_DEPTH; i++) {
    const bit = (index >> i) & 1;
    current = bit === 0 ? poseidon2Hash([current, hashPath[i]]) : poseidon2Hash([hashPath[i], current]);
  }
  return current;
}

/**
 * Sanity-checks a fetched certificate + inclusion proof before spending
 * minutes on an expensive local proving attempt with bad inputs. Verifies
 * both that the claimed leaf hashes to what's expected AND that the Merkle
 * path actually reaches `root`.
 */
export function verifyInclusion(root: bigint, leaf: CertificateTreeLeaf, index: number, hashPath: bigint[]): boolean {
  const leafHash = calculateCertificateLeafHash(leaf);
  return computeMerkleRoot(leafHash, index, hashPath) === root;
}
