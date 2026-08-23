/**
 * Tests `backingTree.ts` — the pure Merkle math (cross-checked to be internally consistent,
 * since no Rust toolchain is available in the harness that runs these tests to produce a
 * genuine cross-language vector the way `pallets/pallet-identity/src/tests.rs`'s own backing-
 * tree tests were cross-checked against `circuits/oprf-identity-anchor/backing-nullifier`'s
 * `nargo test` suite — see commit e31257a) plus the chain-storage-reading functions, against a
 * fake `@polkadot/api` instance mirroring `voting.test.ts`'s approach.
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));

import { Buffer } from 'buffer';
import { getApi } from './api';
import { fieldElementToBytes32BE } from './certificateTree';
import {
  BACKING_TREE_DEPTH,
  backingTreeEmptyLeafHash,
  backingTreeNodeHash,
  backingTreeZeroHash,
  computeBackingMerkleRoot,
  fetchBackingLeafIndexForNullifier,
  fetchBackingMerkleWitness,
  fetchCurrentBackingRoot,
  isBackingRootCurrentlyValid,
} from './backingTree';

const mockedGetApi = getApi as jest.MockedFunction<typeof getApi>;

beforeEach(() => {
  mockedGetApi.mockReset();
});

// ---------------------------------------------------------------------------
// Pure math
// ---------------------------------------------------------------------------

describe('backingTreeNodeHash / backingTreeZeroHash', () => {
  it('is deterministic', () => {
    expect(backingTreeNodeHash(1n, 2n)).toBe(backingTreeNodeHash(1n, 2n));
  });

  it('is not commutative (left/right matter)', () => {
    expect(backingTreeNodeHash(1n, 2n)).not.toBe(backingTreeNodeHash(2n, 1n));
  });

  it('level-0 zero hash is the empty-leaf hash', () => {
    expect(backingTreeZeroHash(0)).toBe(backingTreeEmptyLeafHash());
  });

  it('level-L zero hash is the empty leaf node-hashed with itself L times', () => {
    let expected = backingTreeEmptyLeafHash();
    for (let i = 0; i < 5; i++) expected = backingTreeNodeHash(expected, expected);
    expect(backingTreeZeroHash(5)).toBe(expected);
  });

  it('rejects an out-of-range level', () => {
    expect(() => backingTreeZeroHash(-1)).toThrow(/level/);
    expect(() => backingTreeZeroHash(BACKING_TREE_DEPTH + 1)).toThrow(/level/);
  });
});

describe('computeBackingMerkleRoot', () => {
  it('the empty leaf, walked against an all-zero-subtree path, reproduces the all-empty-tree root', () => {
    const siblings = Array.from({ length: BACKING_TREE_DEPTH }, (_, level) => backingTreeZeroHash(level));
    const root = computeBackingMerkleRoot(backingTreeEmptyLeafHash(), 0, siblings);
    expect(root).toBe(backingTreeZeroHash(BACKING_TREE_DEPTH));
  });

  it('matches a manual walk respecting left/right parity', () => {
    const leaf = 123456789n;
    const siblings = Array.from({ length: BACKING_TREE_DEPTH }, (_, i) => BigInt(i + 1));
    // leafIndex = 5 = 0b101 -> level0 odd (leaf is RIGHT child), level1 even (LEFT), level2 odd (RIGHT), then even thereafter.
    const leafIndex = 5;
    let expected = leaf;
    let idx = leafIndex;
    for (let level = 0; level < BACKING_TREE_DEPTH; level++) {
      expected = idx % 2 === 0
        ? backingTreeNodeHash(expected, siblings[level])
        : backingTreeNodeHash(siblings[level], expected);
      idx = Math.floor(idx / 2);
    }
    expect(computeBackingMerkleRoot(leaf, leafIndex, siblings)).toBe(expected);
  });

  it('rejects the wrong number of siblings', () => {
    expect(() => computeBackingMerkleRoot(1n, 0, [1n, 2n])).toThrow(/siblings/);
  });

  it('rejects a negative or too-large leaf index', () => {
    const siblings = Array.from({ length: BACKING_TREE_DEPTH }, () => 0n);
    expect(() => computeBackingMerkleRoot(1n, -1, siblings)).toThrow(/leafIndex/);
    expect(() => computeBackingMerkleRoot(1n, 2 ** BACKING_TREE_DEPTH, siblings)).toThrow(/leafIndex/);
  });
});

// ---------------------------------------------------------------------------
// Chain-storage reads
// ---------------------------------------------------------------------------

/** A fake `[u8;32]`-ish codec value with `.isNone`/`.unwrap().toU8a()`, matching this module's usage. */
function someBytes(value: bigint) {
  const bytes = fieldElementToBytes32BE(value);
  return { isNone: false, unwrap: () => ({ toU8a: () => bytes }) };
}
const none = { isNone: true, unwrap: () => { throw new Error('unwrap on None'); } };

function fakeApi(options: {
  nodes?: Map<string, bigint>;
  root?: bigint | null;
  validUntil?: Map<string, number>;
  blockNumber?: number;
  leafIndexOf?: Map<string, number>;
} = {}) {
  const nodes = options.nodes ?? new Map<string, bigint>();
  const validUntil = options.validUntil ?? new Map<string, number>();
  const leafIndexOf = options.leafIndexOf ?? new Map<string, number>();

  return {
    query: {
      identity: {
        backingCommitmentTreeNodes: async (level: number, index: number) => {
          const key = `${level}:${index}`;
          return nodes.has(key) ? someBytes(nodes.get(key)!) : none;
        },
        backingCommitmentTreeRoot: async () =>
          options.root === undefined || options.root === null ? none : someBytes(options.root),
        backingRootValidUntil: async (rootBytes: Uint8Array) => {
          const key = Buffer.from(rootBytes).toString('hex');
          return validUntil.has(key)
            ? { isNone: false, unwrap: () => ({ toNumber: () => validUntil.get(key) }) }
            : none;
        },
        backingLeafIndexOf: async (nullifier: Uint8Array) => {
          const key = Buffer.from(nullifier).toString('hex');
          return leafIndexOf.has(key)
            ? { isNone: false, unwrap: () => ({ toNumber: () => leafIndexOf.get(key) }) }
            : none;
        },
      },
    },
    rpc: {
      chain: {
        getHeader: async () => ({ number: { toNumber: () => options.blockNumber ?? 0 } }),
      },
    },
  };
}

describe('fetchBackingMerkleWitness', () => {
  it('an entirely empty tree witnesses against the all-empty-tree root', async () => {
    mockedGetApi.mockResolvedValue(fakeApi() as any);
    const { siblings, root } = await fetchBackingMerkleWitness(0);
    expect(siblings).toHaveLength(BACKING_TREE_DEPTH);
    siblings.forEach((sibling, level) => expect(sibling).toBe(backingTreeZeroHash(level)));
    expect(root).toBe(backingTreeZeroHash(BACKING_TREE_DEPTH));
  });

  it('picks up a real leaf value written at level 0', async () => {
    const leafValue = 999n;
    const nodes = new Map<string, bigint>([['0:0', leafValue]]);
    mockedGetApi.mockResolvedValue(fakeApi({ nodes }) as any);
    const { root } = await fetchBackingMerkleWitness(0);
    const siblings = Array.from({ length: BACKING_TREE_DEPTH }, (_, level) => backingTreeZeroHash(level));
    expect(root).toBe(computeBackingMerkleRoot(leafValue, 0, siblings));
  });

  it('rejects an out-of-range leaf index', async () => {
    mockedGetApi.mockResolvedValue(fakeApi() as any);
    await expect(fetchBackingMerkleWitness(-1)).rejects.toThrow(/leafIndex/);
  });
});

describe('fetchCurrentBackingRoot', () => {
  it('returns the all-empty-tree root when nothing has ever registered', async () => {
    mockedGetApi.mockResolvedValue(fakeApi({ root: null }) as any);
    expect(await fetchCurrentBackingRoot()).toBe(backingTreeZeroHash(BACKING_TREE_DEPTH));
  });

  it('returns the on-file root when set', async () => {
    mockedGetApi.mockResolvedValue(fakeApi({ root: 42n }) as any);
    expect(await fetchCurrentBackingRoot()).toBe(42n);
  });
});

describe('isBackingRootCurrentlyValid', () => {
  it('is false for a root with no BackingRootValidUntil entry', async () => {
    mockedGetApi.mockResolvedValue(fakeApi({ blockNumber: 100 }) as any);
    expect(await isBackingRootCurrentlyValid(7n)).toBe(false);
  });

  it('is true while the current block is at or before validUntil', async () => {
    const rootBytesHex = Buffer.from(fieldElementToBytes32BE(7n)).toString('hex');
    const validUntil = new Map([[rootBytesHex, 100]]);
    mockedGetApi.mockResolvedValue(fakeApi({ validUntil, blockNumber: 100 }) as any);
    expect(await isBackingRootCurrentlyValid(7n)).toBe(true);
  });

  it('is false once the current block passes validUntil', async () => {
    const rootBytesHex = Buffer.from(fieldElementToBytes32BE(7n)).toString('hex');
    const validUntil = new Map([[rootBytesHex, 100]]);
    mockedGetApi.mockResolvedValue(fakeApi({ validUntil, blockNumber: 101 }) as any);
    expect(await isBackingRootCurrentlyValid(7n)).toBe(false);
  });
});

describe('fetchBackingLeafIndexForNullifier', () => {
  it('returns null for an unregistered nullifier', async () => {
    mockedGetApi.mockResolvedValue(fakeApi() as any);
    expect(await fetchBackingLeafIndexForNullifier(new Uint8Array(32))).toBeNull();
  });

  it('returns the recorded leaf index', async () => {
    const nullifier = new Uint8Array(32).fill(0xab);
    const leafIndexOf = new Map([[Buffer.from(nullifier).toString('hex'), 17]]);
    mockedGetApi.mockResolvedValue(fakeApi({ leafIndexOf }) as any);
    expect(await fetchBackingLeafIndexForNullifier(nullifier)).toBe(17);
  });

  it('rejects a nullifier that is not 32 bytes', async () => {
    await expect(fetchBackingLeafIndexForNullifier(new Uint8Array(31))).rejects.toThrow(/32/);
  });
});
