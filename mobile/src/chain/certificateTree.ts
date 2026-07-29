/**
 * Mobile-side half of our own Document Signer Certificate (DSC) registry —
 * built and hosted by us instead of read from Rarimo's own hosted
 * `CertificatesSMT`/`StateKeeper` contracts (see
 * docs.rarimo.com/zk-passport/contracts/). The other half — actually
 * building the depth-80 tree from a set of trusted certificates, which only
 * ever needs to run off-chain as part of the governance-approved
 * certificate-onboarding process, never on a phone — lives in
 * `scripts/certificate-registry/` using `@iden3/js-merkletree` (see that
 * tool's own doc comment for why the tree-building itself isn't
 * hand-rolled here). This file only has what a phone actually needs at
 * proving time: computing its own DSC's fingerprint (to know which
 * precomputed inclusion proof to fetch) and independently sanity-checking
 * a fetched proof before spending minutes on an expensive local proving
 * attempt with bad inputs.
 *
 * Why any of this exists: `RegisterIdentityBuilder`'s `slaveMerkleRoot` /
 * `slaveMerkleInclusionBranches` inputs (see sodParser.ts's doc comment,
 * and pallets/pallet-identity/src/lib.rs's `AllowedMerkleRoots`) prove a
 * passport's signing certificate is ICAO-trusted, by Merkle-inclusion
 * against a root. Rarimo's own deployment reads that root from their own
 * hosted registry — a live third-party service. Depending on it would make
 * *this chain's* citizen registration depend on infrastructure this project
 * doesn't govern, which is exactly the dependency `AllowedMerkleRoots`
 * already exists to avoid (it's `AdminOrigin`/legislature-gated — trust
 * anchors decided by our own governance, not a vendor's server; see
 * HANDOFF.md item 8's Full-vs-Light circuit rationale for the same
 * principle applied elsewhere). The certificate *data* itself isn't
 * Rarimo's — CSCA/DSC certificates are ICAO-standardized, exchanged via
 * ICAO's PKD and countries' own publications — so nothing stops us from
 * building an equivalent tree ourselves and registering our own root via
 * `add_allowed_merkle_root`.
 *
 * Tree spec (reverse-engineered from the actual circuit source, not
 * documentation — verified against
 * circuits/merkleTree/SMTVerifier.circom and
 * circuits/passportVerification/passportVerificationBuilder.circom in
 * rarimo/passport-zk-circuits, and cross-checked against both that repo's
 * own test/process_passport.js#getFakeIdenData (whose "fake" single-leaf
 * root computation is not fake at all for the pubkey-hash formula, only
 * for skipping real Merkle siblings) and iden3/js-merkletree's own
 * `leafKey`/`NodeMiddle.getKey` source, which computes the identical
 * formula independently):
 *
 *  - Standard iden3-style Poseidon Sparse Merkle Tree, depth 80.
 *  - Leaf hash: SMTHash1(key, value) = Poseidon([key, value, 1]).
 *  - Internal node hash: SMTHash2(L, R) = Poseidon([L, R]).
 *  - Each leaf's key AND value are both the certificate's `pubkeyHash` —
 *    this is a set-membership tree (insertion order doesn't matter, and
 *    there's nothing to look up beyond "is this cert in the set").
 *  - Root-to-leaf path bit order: bit 0 (LSB) of `key` selects the branch
 *    at the top (root) level; bit 79 selects the branch at the deepest
 *    level. The tree branches per-bit along any shared key prefix (real
 *    `NodeMiddle`s can have one Empty side — this is genuinely how
 *    `@iden3/js-merkletree` stores it, confirmed from source, not
 *    assumed), so a *real* sibling can legitimately be zero partway down.
 *    Only the *trailing* zeros — deeper than the last level the tree ever
 *    branched at — are padding; see `verifyInclusion`'s doc comment for
 *    why that boundary is "last nonzero index + 1", not "first zero seen".
 *  - `pubkeyHash` itself, per SIGNATURE_TYPE:
 *      RSA/RSA-PSS: only the modulus's low 960 bits are hashed — take the
 *        low 15 64-bit limbs of the modulus (little-endian), regroup into
 *        5 groups of 3 limbs as `limb[3i]*2^128 + limb[3i+1]*2^64 +
 *        limb[3i+2]`, Poseidon(5) over those. (The signature itself is
 *        separately verified against the *full* modulus elsewhere in the
 *        circuit; this truncation only affects which-cert-is-this
 *        identification, not signature security.)
 *      ECDSA: Poseidon([x mod 2^248, y mod 2^248]) over the full-precision
 *        curve point coordinates.
 *
 * Field element byte encoding for chain submission: big-endian 32 bytes,
 * matching `Fr::from_be_bytes_mod_order` in runtime/src/verifier.rs (every
 * other public input in this project's ZK flow uses the same convention).
 */
import { poseidon } from './poseidon';
import {
  decodeAsn1,
  getEcdsaKeyLocation,
  getRsaKeyLocation,
  extractEcdsaPubkey,
  extractRsaPubkey,
  type Pubkey,
} from './sodParser';

const TREE_DEPTH = 80;

// ---------------------------------------------------------------------------
// Certificate parsing
// ---------------------------------------------------------------------------

/**
 * Extracts the SubjectPublicKeyInfo from a standalone DER-encoded X.509
 * certificate (a DSC or CSCA cert, as opposed to a passport SOD). Reuses
 * sodParser.ts's shape-based DFS helpers, which don't care whether they're
 * walking a SOD's CMS structure or a bare `Certificate ::= SEQUENCE {
 * tbsCertificate, signatureAlgorithm, signatureValue }` — both contain a
 * SubjectPublicKeyInfo with the same recognizable shape.
 */
export function extractPubkeyFromCertificate(certDer: Uint8Array): Pubkey {
  const tree = decodeAsn1(certDer);
  if (getEcdsaKeyLocation(tree)) return extractEcdsaPubkey(tree);
  if (getRsaKeyLocation(tree)) return extractRsaPubkey(tree);
  throw new Error('extractPubkeyFromCertificate: no recognizable RSA or ECDSA SubjectPublicKeyInfo found');
}

// ---------------------------------------------------------------------------
// pubkeyHash (circuit's certificate fingerprint / SMT key)
// ---------------------------------------------------------------------------

/** Splits `x` into `k` little-endian limbs of `nBits` bits each, as bigints. */
function bigintToLimbs(nBits: number, k: number, x: bigint): bigint[] {
  const mod = 1n << BigInt(nBits);
  const out: bigint[] = [];
  let rem = x;
  for (let i = 0; i < k; i++) {
    out.push(rem % mod);
    rem /= mod;
  }
  return out;
}

const TWO_POW_248 = 1n << 248n;

/** Computes the exact `pubkeyHash` the circuit derives from a DSC/CSCA's public key — see this module's doc comment for the per-algorithm formula. */
export function computeDscPubkeyHash(pubkey: Pubkey): bigint {
  if (pubkey.kind === 'rsa') {
    const n = BigInt('0x' + pubkey.n);
    const limbs = bigintToLimbs(64, 15, n);
    const grouped = Array.from({ length: 5 }, (_, i) => limbs[3 * i] * 2n ** 128n + limbs[3 * i + 1] * 2n ** 64n + limbs[3 * i + 2]);
    return poseidon(grouped);
  }
  const x = BigInt('0x' + pubkey.x) % TWO_POW_248;
  const y = BigInt('0x' + pubkey.y) % TWO_POW_248;
  return poseidon([x, y]);
}

// ---------------------------------------------------------------------------
// Byte encoding
// ---------------------------------------------------------------------------

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
// Proof verification (sanity-check only — building the tree happens
// off-chain; see scripts/certificate-registry/)
// ---------------------------------------------------------------------------

function bitAt(x: bigint, i: number): 0 | 1 {
  return ((x >> BigInt(i)) & 1n) === 1n ? 1 : 0;
}

/**
 * Re-derives a root from a leaf + sibling path exactly the way
 * `SMTVerifier.circom` does — used by this module's own tests, and usable
 * by anyone wanting to double-check a proof before trusting it.
 *
 * Levels are only combined with `SMTHash2` down to where the leaf's own
 * subtree starts (i.e. while walking from the root, the levels the tree
 * actually branched through) — everything deeper is zero-padding that must
 * be skipped outright, not folded in as `hash(x, 0)`. The circuit encodes
 * this via its `levIns`/top/`inew` state machine (see this module's doc
 * comment); this is the same logic without the circuit's need for
 * data-independent control flow.
 *
 * Finding that boundary is NOT "count leading nonzero entries": the tree
 * genuinely branches per-bit along a shared key prefix (this is how
 * `@iden3/js-merkletree` — scripts/certificate-registry's tree engine —
 * actually stores things internally, confirmed from its own source, not
 * assumed), so an interior level can legitimately have a real,
 * *non-padding* zero sibling whenever the other side of that branch is a
 * not-yet-diverged Empty subtree. Only *trailing* zeros (deeper than the
 * last level the tree ever branched at) are padding. The boundary is
 * therefore "one past the deepest index with a nonzero sibling" — scan
 * from the bottom, not the top. (An earlier version of this function
 * scanned from the top and broke on exactly this case — a multi-leaf tree
 * whose first branch happens to have an Empty side.)
 */
export function verifyInclusion(root: bigint, key: bigint, value: bigint, siblings: bigint[]): boolean {
  if (siblings.length !== TREE_DEPTH) return false;
  let insertionDepth = TREE_DEPTH;
  while (insertionDepth > 0 && siblings[insertionDepth - 1] === 0n) insertionDepth--;

  let acc = poseidon([key, value, 1n]);
  for (let i = insertionDepth - 1; i >= 0; i--) {
    acc = bitAt(key, i) === 0 ? poseidon([acc, siblings[i]]) : poseidon([siblings[i], acc]);
  }
  return acc === root;
}
