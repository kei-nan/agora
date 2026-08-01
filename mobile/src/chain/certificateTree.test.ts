/**
 * Cross-checks `calculateCertificateLeafHash`, the depth-16 Merkle
 * combine (`computeMerkleRoot`), and the fingerprint packer
 * (`computeCertificateFingerprint`) against ZKPassport's own first-party
 * `@zkpassport/utils@0.37.3` (`registry` module) + `@zkpassport/poseidon2@0.6.2`
 * — not reproduced from memory or re-derived from the Noir source a second
 * time independently. Every `ORACLE_*` constant below was captured by
 * actually running:
 *
 *   import { getCertificateLeafHash, tagsArrayToBitsFlag } from '@zkpassport/utils/registry';
 *   import { poseidon2Hash } from '@zkpassport/poseidon2';
 *   const certA = { tags: ['IC'], country: 'USA',
 *     public_key: { type: 'RSA', modulus: '0x' + 'ab'.repeat(256) },
 *     validity: { not_after: 1893456000 }, fingerprint: '0x' + '11'.repeat(32) };
 *   const certB = { tags: ['DE'], country: 'DEU',
 *     public_key: { type: 'EC', public_key_x: '0x' + 'cd'.repeat(32), public_key_y: '0x' + 'ef'.repeat(32) },
 *     validity: { not_after: 1924992000 }, fingerprint: '0x' + '22'.repeat(32) };
 *   await getCertificateLeafHash(certA, { version: 1, type: 2 }); // -> ORACLE_LEAF_A
 *   await getCertificateLeafHash(certB, { version: 1, type: 2 }); // -> ORACLE_LEAF_B
 *
 * GOTCHA worth recording so nobody re-discovers it the slow way: the
 * certificate's type (CSCA=1 vs DSC=2) is read from the *options* object's
 * `type` field, NOT from the certificate object's own `type` field — a
 * `{ ...cert, type: 2 }` shape (this file's first draft) is silently
 * ignored and the type defaults to CSCA (1). A separate real limitation
 * this surfaced: `buildMerkleTreeFromCerts(certs, version)` has NO way to
 * pass `type` through to the certificates it hashes at all (it always calls
 * the equivalent of `getCertificateLeafHash(cert, { version })`, type
 * omitted) — so this package version cannot build a mixed or all-DSC tree
 * via that entry point; whoever wires up `scripts/certificate-registry/`
 * needs to call `getCertificateLeafHash` per-certificate directly (passing
 * the correct `type` per call) and assemble/sort the tree by hand rather
 * than trusting `buildMerkleTreeFromCerts` to do it for a DSC registry.
 * That is why the tree/proof vectors below are composed by hand from two
 * correctly-typed `getCertificateLeafHash` outputs plus the (leaf-content-
 * independent) zero-subtree chain, rather than taken directly from
 * `buildMerkleTreeFromCerts`'s own output.
 */
import { poseidon2Hash } from '@zkpassport/poseidon2';
import {
  CERT_TYPE_DSC,
  calculateCertificateLeafHash,
  computeCertificateFingerprint,
  computeMerkleRoot,
  computeZeroes,
  fieldElementToBytes32BE,
  packBeBytesIntoField,
  packBeBytesIntoFields,
  packLeBytesIntoFields,
  publicKeyToBytes,
  verifyInclusion,
  type CertificateTreeLeaf,
} from './certificateTree';
import type { Pubkey } from './sodParser';

const TAGS_A: [bigint, bigint, bigint] = [
  0x40000000000000000000000000000000000000000000000000000n,
  0n,
  0n,
];
const TAGS_B: [bigint, bigint, bigint] = [0x400000000000000000000n, 0n, 0n];

const RSA_PUBKEY: Pubkey = { kind: 'rsa', n: 'ab'.repeat(256), exp: '10001' };
const EC_PUBKEY: Pubkey = { kind: 'ecdsa', x: 'cd'.repeat(32), y: 'ef'.repeat(32), param: '' };

const LEAF_A: CertificateTreeLeaf = {
  tags: TAGS_A,
  certType: CERT_TYPE_DSC,
  country: 'USA',
  publicKey: publicKeyToBytes(RSA_PUBKEY),
  expiry: 1893456000,
  fingerprint: BigInt('0x' + '11'.repeat(32)),
};
const LEAF_B: CertificateTreeLeaf = {
  tags: TAGS_B,
  certType: CERT_TYPE_DSC,
  country: 'DEU',
  publicKey: publicKeyToBytes(EC_PUBKEY),
  expiry: 1924992000,
  fingerprint: BigInt('0x' + '22'.repeat(32)),
};

const ORACLE_LEAF_A = 0x2d1fc05fe01ca69dbe5004a2d1aaa3bdd426a623e2df96f4aefa39651aaecd00n;
const ORACLE_LEAF_B = 0x135a48063ef073c2cc56975a768749a27d7c3a9a66361abc8503fd5ed0d67977n;
const ORACLE_ROOT = 0x5f4ef5a31d685e88474a1ef02081dbea54b26668e36db1e520c87face161144n;

// The real tree-builder sorts leaves ascending before assigning index
// positions. Numerically, LEAF_B's hash < LEAF_A's hash here, so B lands at
// index 0 and A at index 1 — the reverse of these constants' alphabetic
// order, which is exactly the kind of assumption worth a comment.
const ZERO_SIBLINGS_TAIL = [
  0xb63a53787021a4a962a452c2921b3663aff1ffd8d5510540f8e659e782956f1n,
  0xe34ac2c09f45a503d2908bcb12f1cbae5fa4065759c88d501c097506a8b2290n,
  0x21f9172d72fdcdafc312eee05cf5092980dda821da5b760a9fb8dbdf607c8a20n,
  0x2373ea368857ec7af97e7b470d705848e2bf93ed7bef142a490f2119bcf82d8en,
  0x120157cfaaa49ce3da30f8b47879114977c24b266d58b0ac18b325d878aafddfn,
  0x1c28fe1059ae0237b72334700697bdf465e03df03986fe05200cadeda66bd76n,
  0x2d78ed82f93b61ba718b17c2dfe5b52375b4d37cbbed6f1fc98b47614b0cf21bn,
  0x67243231eddf4222f3911defbba7705aff06ed45960b27f6f91319196ef97e1n,
  0x1849b85f3c693693e732dfc4577217acc18295193bede09ce8b97ad910310972n,
  0x2a775ea761d20435b31fa2c33ff07663e24542ffb9e7b293dfce3042eb104686n,
  0xf320b0703439a8114f81593de99cd0b8f3b9bf854601abb5b2ea0e8a3dda4a7n,
  0xd07f6e7a8a0e9199d6d92801fff867002ff5b4808962f9da2ba5ce1bdd26a73n,
  0x1c4954081e324939350febc2b918a293ebcdaead01be95ec02fcbe8d2c1635d1n,
  0x197f2171ef99c2d053ee1fb5ff5ab288d56b9b41b4716c9214a4d97facc4c4an,
  0x2b9cdd484c5ba1e4d6efcc3f18734b5ac4c4a0b9102e2aeb48521a661d3feee9n,
];
const ORACLE_SIBLINGS_FOR_A = [ORACLE_LEAF_B, ...ZERO_SIBLINGS_TAIL];
const ORACLE_SIBLINGS_FOR_B = [ORACLE_LEAF_A, ...ZERO_SIBLINGS_TAIL];

describe('poseidon2Hash', () => {
  it('matches a value independently computed by @aztec/bb.js (BarretenbergSync.poseidon2Hash) and by executing Noir\'s own Poseidon2::hash via nargo', () => {
    expect(poseidon2Hash([4n, 8n])).toBe(
      0x2bcaeb6d58bb38baf753d58c3c96618fea82163345295eb40e88344eeb0ce2a1n,
    );
  });
});

describe('packBeBytesIntoFields / packBeBytesIntoField', () => {
  it('packs a short byte string into a single big-endian field', () => {
    expect(packBeBytesIntoField(new Uint8Array([0x01, 0x02, 0x03]))).toBe(0x010203n);
  });

  it('splits a 256-byte RSA modulus into 9 big-endian limbs (ceil(256/31) = 9), least-significant limb first — matches the internal `m()` helper in @zkpassport/utils', () => {
    const modulus = new Uint8Array(256).fill(0xab);
    const limbs = packBeBytesIntoFields(modulus, 31);
    expect(limbs).toHaveLength(9);
    // 256 = 8*31 + 8, so the first (most-significant, last array entry)
    // chunk is 8 bytes; every other chunk is a full 31 bytes of 0xab.
    expect(limbs[8]).toBe(BigInt('0x' + 'ab'.repeat(8)));
    expect(limbs[0]).toBe(BigInt('0x' + 'ab'.repeat(31)));
  });
});

describe('packLeBytesIntoFields', () => {
  it('matches @zkpassport/utils packLeBytesAndHashPoseidon2\'s packing for a 45-byte buffer', () => {
    const bytes = new Uint8Array(Array.from({ length: 45 }, (_, i) => i + 1));
    const limbs = packLeBytesIntoFields(bytes, 31);
    expect(poseidon2Hash(limbs)).toBe(
      0x280b2974fa169fd137ecbf5e55c5c7954b1ef7da51abcd0fbb5a44a94d671c77n,
    );
  });
});

describe('computeCertificateFingerprint', () => {
  it('matches @zkpassport/utils packLeBytesAndHashPoseidon2 for the same 45-byte buffer', () => {
    const bytes = new Uint8Array(Array.from({ length: 45 }, (_, i) => i + 1));
    expect(computeCertificateFingerprint(bytes)).toBe(
      0x280b2974fa169fd137ecbf5e55c5c7954b1ef7da51abcd0fbb5a44a94d671c77n,
    );
  });
});

describe('publicKeyToBytes', () => {
  it('recovers the full 256-byte width of a 2048-bit RSA modulus even when the hex string lost a leading zero byte', () => {
    // 0x00ab...ab (top byte zero) -> BigInt(...).toString(16) drops it,
    // leaving what looks like a 255-byte value.
    const n = '00' + 'ab'.repeat(255);
    const bytes = publicKeyToBytes({ kind: 'rsa', n: BigInt('0x' + n).toString(16), exp: '10001' });
    expect(bytes).toHaveLength(256);
    expect(bytes[0]).toBe(0);
  });

  it('recovers a 32-byte P-256 coordinate even when its hex string lost a leading zero byte', () => {
    const x = BigInt('0x' + '00' + 'cd'.repeat(31)).toString(16);
    const bytes = publicKeyToBytes({ kind: 'ecdsa', x, y: 'ef'.repeat(32), param: '' });
    expect(bytes).toHaveLength(64);
    expect(bytes[0]).toBe(0);
  });
});

describe('calculateCertificateLeafHash', () => {
  it('matches @zkpassport/utils getCertificateLeafHash for an RSA DSC leaf', () => {
    expect(calculateCertificateLeafHash(LEAF_A)).toBe(ORACLE_LEAF_A);
  });

  it('matches @zkpassport/utils getCertificateLeafHash for an EC DSC leaf', () => {
    expect(calculateCertificateLeafHash(LEAF_B)).toBe(ORACLE_LEAF_B);
  });

  it('rejects a country code that is not exactly 3 characters', () => {
    expect(() => calculateCertificateLeafHash({ ...LEAF_A, country: 'US' })).toThrow();
  });
});

describe('computeZeroes', () => {
  it('matches the zero-subtree values baked into the oracle two-leaf tree\'s own proof (levels 1..15, where no second subtree exists)', () => {
    const zeroes = computeZeroes(16);
    expect(zeroes[0]).toBe(0n);
    for (let level = 1; level < 16; level++) {
      expect(zeroes[level]).toBe(ZERO_SIBLINGS_TAIL[level - 1]);
    }
  });
});

describe('computeMerkleRoot / verifyInclusion', () => {
  it('reproduces the oracle root for leaf A at index 1', () => {
    expect(computeMerkleRoot(ORACLE_LEAF_A, 1, ORACLE_SIBLINGS_FOR_A)).toBe(ORACLE_ROOT);
  });

  it('reproduces the oracle root for leaf B at index 0', () => {
    expect(computeMerkleRoot(ORACLE_LEAF_B, 0, ORACLE_SIBLINGS_FOR_B)).toBe(ORACLE_ROOT);
  });

  it('verifyInclusion accepts both real leaves against the oracle root', () => {
    expect(verifyInclusion(ORACLE_ROOT, LEAF_A, 1, ORACLE_SIBLINGS_FOR_A)).toBe(true);
    expect(verifyInclusion(ORACLE_ROOT, LEAF_B, 0, ORACLE_SIBLINGS_FOR_B)).toBe(true);
  });

  it('rejects a tampered root', () => {
    expect(verifyInclusion(ORACLE_ROOT + 1n, LEAF_A, 1, ORACLE_SIBLINGS_FOR_A)).toBe(false);
  });

  it('rejects the wrong index (bit pattern no longer matches the sibling path)', () => {
    expect(verifyInclusion(ORACLE_ROOT, LEAF_A, 0, ORACLE_SIBLINGS_FOR_A)).toBe(false);
  });

  it('rejects a proof of the wrong depth', () => {
    expect(() => computeMerkleRoot(ORACLE_LEAF_A, 1, ORACLE_SIBLINGS_FOR_A.slice(1))).toThrow();
  });
});

describe('fieldElementToBytes32BE', () => {
  it('encodes as 32 big-endian bytes', () => {
    const bytes = fieldElementToBytes32BE(1n);
    expect(bytes).toHaveLength(32);
    expect(bytes[31]).toBe(1);
    expect(bytes.slice(0, 31).every((b) => b === 0)).toBe(true);
  });

  it('round-trips the oracle root', () => {
    const bytes = fieldElementToBytes32BE(ORACLE_ROOT);
    const reconstructed = bytes.reduce((acc, b) => (acc << 8n) | BigInt(b), 0n);
    expect(reconstructed).toBe(ORACLE_ROOT);
  });
});
