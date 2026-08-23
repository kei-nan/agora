/**
 * Mobile-side client for `pallet-identity`'s on-chain incremental Merkle tree over
 * `backing_commitment` values (`pallets/pallet-identity/src/lib.rs`, "Backing-commitment
 * incremental Merkle tree" section) — the tree the `backing-nullifier` circuit
 * (`circuits/oprf-identity-anchor/backing-nullifier`) proves membership against.
 *
 * # What this module is for
 *
 * A `backing-nullifier` proof needs a fresh authentication path (root + sibling hashes) for the
 * citizen's own fixed leaf position every time it proves — not a value cached once at
 * registration. Unlike `leaf_index`/`backing_root_secret` (stable for the citizen's lifetime),
 * the *siblings* on that path can change over time as unrelated citizens register or get
 * revoked elsewhere in the tree (any leaf sharing an ancestor subtree with ours moves one of
 * our siblings). So this module always reads live chain storage rather than persisting a
 * witness locally — the one thing worth caching client-side is `leaf_index` itself (this
 * module's own `fetchBackingLeafIndexForNullifier` is a plain lookup, safe to call as needed
 * rather than memorized), not the path through it. The pallet's own `BackingRootHistoryWindowBlocks`
 * retention window is what makes this practical: a witness fetched right before proving stays
 * acceptable to the runtime for that whole window (see `is_valid_backing_commitment_root`),
 * comfortably covering the time a real on-device Noir proving run takes.
 *
 * # Hash function
 *
 * Poseidon2 over the BN254 scalar field, via `@zkpassport/poseidon2` — the same
 * zero-dependency, pure-TypeScript implementation `certificateTree.ts` already uses and has
 * independently cross-checked against Noir's `Poseidon2::hash` and `@aztec/bb.js`'s
 * `poseidon2Hash` (see that module's doc comment). `pallets/pallet-identity/src/lib.rs`'s own
 * `poseidon2_bn254::hash_bytes` and `circuits/oprf-identity-anchor/backing-nullifier`'s
 * `Poseidon2::hash` are the same permutation under that established cross-check, so this
 * module's hashes are byte-identical to both the pallet's tree and the circuit's own
 * `backing_tree_node_hash`.
 *
 * # Tree spec (mirrors `pallets/pallet-identity/src/lib.rs` exactly — see that file's
 * "Backing-commitment incremental Merkle tree" section for the authoritative Rust source this
 * was transcribed from)
 *
 *  - Depth 32 ({@link BACKING_TREE_DEPTH}), NOT `certificateTree.ts`'s depth-16 — a different
 *    tree for a different purpose (citizens, not trusted certificates), sized for up to
 *    `2^32` leaves.
 *  - Internal-node hash: `Poseidon2(DS_BACKING_TREE_NODE=210, left, right)` — a 3-element
 *    hash, domain-separated from every other Poseidon2 use in this codebase.
 *  - Empty-leaf placeholder: `Poseidon2(DS_BACKING_TREE_EMPTY_LEAF=211)` — a 1-element hash,
 *    distinct from a bare `0`, so "never registered" can never be confused with a real
 *    `backing_commitment` (astronomically unlikely as that already is under Poseidon2).
 *  - Zero-subtree value at level `L`: the empty-leaf hash, node-hashed with itself `L` times.
 *  - Combine step: walking from the leaf upward, if the running index is even at that level the
 *    current value is the LEFT child and the sibling is RIGHT; if odd, current is RIGHT and
 *    sibling is LEFT. Identical bit convention to `certificateTree.ts`'s
 *    `computeMerkleRoot`/`compute_merkle_root`, just index-parity instead of an explicit bit
 *    shift (this tree's leaf indices can exceed 2^31, outside what JS's 32-bit bitwise
 *    operators can safely handle, so this module uses plain arithmetic (`% 2`, `/2` floored)
 *    instead of `&`/`>>` throughout).
 *
 * Note this module deliberately does NOT attempt to derive `backing_root_secret` or resolve a
 * citizen's own `leaf_index` from a real ZKPassport nullifier — both depend on the same OPRF
 * committee round-trip `oprfCombine.ts`'s `combineCommitteeSlotResponses` documents as an
 * unimplemented stub, and `identity.ts`'s own `getSigningKeypair` documents its `nullifierHash`
 * as a placeholder that will not match `CitizenNullifier`'s real on-chain value. This module's
 * `fetchBackingLeafIndexForNullifier` performs the real chain lookup for whoever eventually has
 * a real nullifier to pass it; nothing here fabricates one.
 */
import { poseidon2Hash } from '@zkpassport/poseidon2';
import { getApi } from './api';
import { fieldElementToBytes32BE } from './certificateTree';
import { fieldToBigInt } from './proofEncoding';

/** Must match `pallets/pallet-identity/src/lib.rs`'s `BACKING_TREE_DEPTH`. */
export const BACKING_TREE_DEPTH = 32;

const DS_BACKING_TREE_NODE = 210n;
const DS_BACKING_TREE_EMPTY_LEAF = 211n;

/** `Poseidon2(DS_BACKING_TREE_NODE, left, right)` — mirrors `backing_tree_node_hash` exactly. */
export function backingTreeNodeHash(left: bigint, right: bigint): bigint {
  return poseidon2Hash([DS_BACKING_TREE_NODE, left, right]);
}

/** `Poseidon2(DS_BACKING_TREE_EMPTY_LEAF)` — mirrors `backing_tree_empty_leaf` exactly. */
export function backingTreeEmptyLeafHash(): bigint {
  return poseidon2Hash([DS_BACKING_TREE_EMPTY_LEAF]);
}

// Memoized level-by-level, same spirit as certificateTree.ts's computeZeroes but built
// incrementally on demand rather than all at once — a witness fetch only ever needs the levels
// actually missing from chain storage, and 32 extra Poseidon2 calls in the worst case (a
// nearly-empty tree) is cheap regardless.
let _zeroHashes: bigint[] | null = null;

/** The all-empty-subtree hash at `level` (0 = a single empty leaf). Mirrors `backing_tree_zero_hash`. */
export function backingTreeZeroHash(level: number): bigint {
  if (!Number.isInteger(level) || level < 0 || level > BACKING_TREE_DEPTH) {
    throw new RangeError(`backingTreeZeroHash: level must be in 0..=${BACKING_TREE_DEPTH}, got ${level}`);
  }
  if (_zeroHashes === null) {
    _zeroHashes = [backingTreeEmptyLeafHash()];
  }
  while (_zeroHashes.length <= level) {
    const prev = _zeroHashes[_zeroHashes.length - 1];
    _zeroHashes.push(backingTreeNodeHash(prev, prev));
  }
  return _zeroHashes[level];
}

/**
 * Re-derives a root from a leaf hash + its integer leaf index + sibling path, exactly the way
 * `pallets/pallet-identity/src/lib.rs`'s `recompute_backing_tree_path` does (and, from the
 * proving side, `circuits/oprf-identity-anchor/backing-nullifier`'s `main.nr`). `leafIndex` may
 * exceed `2^31`, so this walks it with plain arithmetic rather than bitwise operators — see this
 * module's doc comment.
 */
export function computeBackingMerkleRoot(
  leafHash: bigint,
  leafIndex: number,
  siblings: readonly bigint[],
): bigint {
  if (!Number.isInteger(leafIndex) || leafIndex < 0 || leafIndex >= 2 ** BACKING_TREE_DEPTH) {
    throw new RangeError(`computeBackingMerkleRoot: leafIndex out of range: ${leafIndex}`);
  }
  if (siblings.length !== BACKING_TREE_DEPTH) {
    throw new RangeError(
      `computeBackingMerkleRoot: expected ${BACKING_TREE_DEPTH} siblings, got ${siblings.length}`,
    );
  }
  let current = leafHash;
  let index = leafIndex;
  for (let level = 0; level < BACKING_TREE_DEPTH; level++) {
    const isLeft = index % 2 === 0;
    current = isLeft ? backingTreeNodeHash(current, siblings[level]) : backingTreeNodeHash(siblings[level], current);
    index = Math.floor(index / 2);
  }
  return current;
}

/** Reads one raw 32-byte tree-node value from `pallet-identity`'s `BackingCommitmentTreeNodes`, `None` (untouched) mapped to `null`. */
async function readTreeNodeRaw(level: number, index: number): Promise<bigint | null> {
  const api = await getApi();
  const entry = await api.query.identity.backingCommitmentTreeNodes(level, index);
  if ((entry as any).isNone) return null;
  const bytes = (entry as any).unwrap().toU8a() as Uint8Array;
  return fieldToBigInt(bytes.length === 32 ? bytes : bytes.slice(-32));
}

/** `readTreeNodeRaw`, falling back to the deterministic zero-subtree value — mirrors `Pallet::backing_tree_node`. */
async function readTreeNode(level: number, index: number): Promise<bigint> {
  const value = await readTreeNodeRaw(level, index);
  return value ?? backingTreeZeroHash(level);
}

/**
 * Fetches a fresh authentication path (siblings) for `leafIndex`, plus the root that path
 * currently proves membership against — read live from chain storage, level by level, exactly
 * mirroring `pallet-identity`'s own `backing_tree_node`/`recompute_backing_tree_path` walk.
 * Deliberately does not accept a cached witness parameter: see this module's doc comment for
 * why a witness is only ever meaningful as of the moment it's read.
 */
export async function fetchBackingMerkleWitness(
  leafIndex: number,
): Promise<{ siblings: bigint[]; root: bigint }> {
  if (!Number.isInteger(leafIndex) || leafIndex < 0 || leafIndex >= 2 ** BACKING_TREE_DEPTH) {
    throw new RangeError(`fetchBackingMerkleWitness: leafIndex out of range: ${leafIndex}`);
  }
  const siblings: bigint[] = [];
  let index = leafIndex;
  let current = await readTreeNode(0, leafIndex);
  for (let level = 0; level < BACKING_TREE_DEPTH; level++) {
    const siblingIndex = index % 2 === 0 ? index + 1 : index - 1;
    const sibling = await readTreeNode(level, siblingIndex);
    siblings.push(sibling);
    current = index % 2 === 0 ? backingTreeNodeHash(current, sibling) : backingTreeNodeHash(sibling, current);
    index = Math.floor(index / 2);
  }
  return { siblings, root: current };
}

/** Current root of the backing-commitment tree — mirrors `current_backing_commitment_root`. */
export async function fetchCurrentBackingRoot(): Promise<bigint> {
  const api = await getApi();
  const entry = await api.query.identity.backingCommitmentTreeRoot();
  if ((entry as any).isNone) return backingTreeZeroHash(BACKING_TREE_DEPTH);
  const bytes = (entry as any).unwrap().toU8a() as Uint8Array;
  return fieldToBigInt(bytes.length === 32 ? bytes : bytes.slice(-32));
}

/**
 * Whether `root` was a genuinely current backing-commitment tree root at some block within
 * `pallet-identity`'s `BackingRootHistoryWindowBlocks` retention window — the same check
 * `is_valid_backing_commitment_root` performs on-chain (reading `BackingRootValidUntil` and
 * comparing to the current block, not just checking bare presence). A `backing-nullifier` proof
 * built against a `root` this returns `false` for will be rejected by
 * `pallet-elections`' `verify_backing_proof` regardless of how sound the proof itself is.
 */
export async function isBackingRootCurrentlyValid(root: bigint): Promise<boolean> {
  const api = await getApi();
  const rootBytes = fieldElementToBytes32BE(root);
  const [entry, header] = await Promise.all([
    api.query.identity.backingRootValidUntil(rootBytes),
    api.rpc.chain.getHeader(),
  ]);
  if ((entry as any).isNone) return false;
  const validUntil = (entry as any).unwrap().toNumber();
  return header.number.toNumber() <= validUntil;
}

/**
 * A citizen's permanent leaf index in the backing-commitment tree, keyed by their real
 * ZKPassport nullifier (`pallet-identity`'s `BackingLeafIndexOf`, a `StorageMap<[u8;32], u32>`
 * keyed by nullifier rather than `AccountId` — see that storage item's doc comment for why:
 * it survives `recover_account`'s account rebind). Returns `null` if this nullifier has never
 * registered a backing-commitment leaf.
 *
 * `nullifier` must be the citizen's real, on-chain `CitizenNullifier` value — this function
 * performs no derivation of its own. As of this writing nothing in `mobile/src/chain/` produces
 * a real one yet (see this module's doc comment); it exists so a future caller with a real
 * nullifier in hand has a correct lookup ready to call, rather than reinventing one.
 */
export async function fetchBackingLeafIndexForNullifier(nullifier: Uint8Array): Promise<number | null> {
  if (nullifier.length !== 32) {
    throw new RangeError(`fetchBackingLeafIndexForNullifier: nullifier is ${nullifier.length} bytes, expected 32`);
  }
  const api = await getApi();
  const entry = await api.query.identity.backingLeafIndexOf(nullifier);
  if ((entry as any).isNone) return null;
  return (entry as any).unwrap().toNumber();
}
