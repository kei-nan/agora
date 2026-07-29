/**
 * Cross-checks `computeDscPubkeyHash` against a value independently
 * computed by requiring rarimo/passport-zk-circuits' own `test/poseidon.js`
 * + `test/process_passport.js#getFakeIdenData` against the same RSA
 * modulus already used by sodParser.test.ts's fixture (see that fixture's
 * own doc comment for provenance) — not reproduced from memory.
 *
 * `verifyInclusion` is exercised against a small tree built *by hand* here
 * (not via `@iden3/js-merkletree`, which is scripts/certificate-registry's
 * job and a heavier dependency than this file should need) using the exact
 * SMTHash1/SMTHash2 formulas from this module's own doc comment. The
 * two-leaf case deliberately picks keys that share bit 0 (forcing a real,
 * non-padding zero sibling at depth 0) so it would have caught the bug an
 * earlier version of `verifyInclusion` actually shipped with — see that
 * function's doc comment for the full story.
 */
import { poseidon } from './poseidon';
import { computeDscPubkeyHash, fieldElementToBytes32BE, verifyInclusion } from './certificateTree';
import type { Pubkey } from './sodParser';
import fixture from './__fixtures__/sodParser.fixture.json';

// Computed once via:
//   node -e 'const {poseidon}=require("./test/poseidon.js"); ...'
// against a clone of rarimo/passport-zk-circuits, using fixture.expectedPubkeyN
// as the RSA modulus — see this file's doc comment.
const ORACLE_PK_HASH = 4228862645996229144674977447983286239012301908651443861407052046600929457056n;
const ORACLE_SINGLE_LEAF_ROOT = 9881918629624367122786109866856189475574581976908846030740366624468694806948n;

describe('computeDscPubkeyHash', () => {
  it('matches the reference implementation for an RSA modulus', () => {
    const pubkey: Pubkey = { kind: 'rsa', n: fixture.expectedPubkeyN, exp: fixture.expectedPubkeyExp };
    expect(computeDscPubkeyHash(pubkey)).toBe(ORACLE_PK_HASH);
  });
});

describe('verifyInclusion', () => {
  const TREE_DEPTH = 80;
  const zeros = (): bigint[] => new Array(TREE_DEPTH).fill(0n);

  it('verifies a single-leaf tree against the oracle root, with all-zero siblings', () => {
    const siblings = zeros();
    expect(verifyInclusion(ORACLE_SINGLE_LEAF_ROOT, ORACLE_PK_HASH, ORACLE_PK_HASH, siblings)).toBe(true);
  });

  it('rejects a tampered root', () => {
    expect(verifyInclusion(ORACLE_SINGLE_LEAF_ROOT + 1n, ORACLE_PK_HASH, ORACLE_PK_HASH, zeros())).toBe(false);
  });

  it('rejects a proof of the wrong length', () => {
    expect(verifyInclusion(ORACLE_SINGLE_LEAF_ROOT, ORACLE_PK_HASH, ORACLE_PK_HASH, zeros().slice(1))).toBe(false);
  });

  it('verifies a two-leaf tree whose keys share bit 0 (a real, non-padding zero sibling at depth 0)', () => {
    const key1 = 4n; // bits: bit0=0, bit1=0
    const key2 = 6n; // bits: bit0=0, bit1=1 — diverges from key1 at depth 1
    const leaf1 = poseidon([key1, key1, 1n]);
    const leaf2 = poseidon([key2, key2, 1n]);
    const depth1Node = poseidon([leaf1, leaf2]); // key1 (bit1=0) left, key2 (bit1=1) right
    const root = poseidon([depth1Node, 0n]); // both keys (bit0=0) went left; right side at depth 0 is genuinely Empty

    const siblingsFor1 = [0n, leaf2, ...new Array(TREE_DEPTH - 2).fill(0n)];
    const siblingsFor2 = [0n, leaf1, ...new Array(TREE_DEPTH - 2).fill(0n)];

    expect(verifyInclusion(root, key1, key1, siblingsFor1)).toBe(true);
    expect(verifyInclusion(root, key2, key2, siblingsFor2)).toBe(true);

    // Swapping which sibling goes with which key must not also verify.
    expect(verifyInclusion(root, key1, key1, siblingsFor2)).toBe(false);
    expect(verifyInclusion(root, key2, key2, siblingsFor1)).toBe(false);
  });
});

describe('fieldElementToBytes32BE', () => {
  it('encodes as 32 big-endian bytes', () => {
    const bytes = fieldElementToBytes32BE(1n);
    expect(bytes).toHaveLength(32);
    expect(bytes[31]).toBe(1);
    expect(bytes.slice(0, 31).every((b) => b === 0)).toBe(true);
  });

  it('round-trips the oracle pubkeyHash', () => {
    const bytes = fieldElementToBytes32BE(ORACLE_PK_HASH);
    const reconstructed = bytes.reduce((acc, b) => (acc << 8n) | BigInt(b), 0n);
    expect(reconstructed).toBe(ORACLE_PK_HASH);
  });
});
