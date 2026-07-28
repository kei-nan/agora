/**
 * Byte-identical cross-check for `compressG1` / `compressG2` /
 * `encodeGroth16Proof` against ground truth produced by:
 *
 *   1. A throwaway Rust tool (`ark_bn254::{G1Affine,G2Affine}` +
 *      `ark_serialize::CanonicalSerialize::serialize_compressed`) — the same
 *      crates `runtime/src/verifier.rs` uses to deserialize/verify proofs.
 *   2. `scripts/convert_vk.py`'s `_compress_g1` / `_compress_g2` (the
 *      already production-validated Python reference for this exact
 *      ark-serialize compressed-point format).
 *
 * Python and Rust were independently run against the identical fixed
 * coordinates below and produced byte-identical output for all 8 vectors
 * (4 G1, 4 G2), confirming both are the same ground truth before comparing
 * the TypeScript port against it. See PR/commit description for the raw
 * three-way run logs.
 *
 * The G1 vectors cover both branches of `_fq_is_negative(y)` (plus the
 * `y == 0` edge case). The G2 vectors cover both the `c1 != 0` and
 * `c1 == 0` branches of the Fq2 tie-break rule, each in turn covering both
 * "negative" and "not negative".
 *
 * Points are NOT required to be on the BN254 curve — this suite tests the
 * serialization byte layout only, not curve/proof validity (mirroring what
 * the Rust ground-truth tool does with `G1Affine::new_unchecked` /
 * `G2Affine::new_unchecked`).
 */
import { compressG1, compressG2, encodeGroth16Proof, SnarkjsG1, SnarkjsG2 } from './proofEncoding';

const P = '21888242871839275222246405745257275088696311157297823662689037894645226208583';

function hexToBytes(hex: string): Uint8Array {
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(hex.substr(i * 2, 2), 16);
  }
  return out;
}

describe('compressG1', () => {
  const cases: Array<{ name: string; point: SnarkjsG1; hex: string }> = [
    {
      name: 'not_negative (y=5)',
      point: ['111', '5', '1'],
      hex: '6f00000000000000000000000000000000000000000000000000000000000000',
    },
    {
      name: 'negative (y=P-5)',
      point: [
        '222',
        '21888242871839275222246405745257275088696311157297823662689037894645226208578',
        '1',
      ],
      hex: 'de00000000000000000000000000000000000000000000000000000000000080',
    },
    {
      name: 'not_negative (y=0) edge case',
      point: ['333', '0', '1'],
      hex: '4d01000000000000000000000000000000000000000000000000000000000000',
    },
    {
      name: 'negative (y=P-1, max element)',
      point: [
        '444',
        '21888242871839275222246405745257275088696311157297823662689037894645226208582',
        '1',
      ],
      hex: 'bc01000000000000000000000000000000000000000000000000000000000080',
    },
  ];

  for (const { name, point, hex } of cases) {
    it(`matches Rust/Python ground truth: ${name}`, () => {
      const got = compressG1(point);
      expect(got.length).toBe(32);
      expect(Buffer.from(got).toString('hex')).toBe(hex);
      expect(got).toEqual(hexToBytes(hex));
    });
  }

  it('rejects a non-affine (z != 1) point', () => {
    expect(() => compressG1(['1', '2', '3'])).toThrow();
  });
});

describe('compressG2', () => {
  const cases: Array<{ name: string; point: SnarkjsG2; hex: string }> = [
    {
      name: 'c1_nonzero__not_negative',
      point: [['1000', '2000'], ['999', '7'], ['1', '0']],
      hex:
        'e803000000000000000000000000000000000000000000000000000000000000' +
        'd007000000000000000000000000000000000000000000000000000000000000',
    },
    {
      name: 'c1_nonzero__negative',
      point: [
        ['1001', '2001'],
        ['999', '21888242871839275222246405745257275088696311157297823662689037894645226208576'],
        ['1', '0'],
      ],
      hex:
        'e903000000000000000000000000000000000000000000000000000000000000' +
        'd107000000000000000000000000000000000000000000000000000000000080',
    },
    {
      name: 'c1_zero__c0_not_negative',
      point: [['1002', '2002'], ['9', '0'], ['1', '0']],
      hex:
        'ea03000000000000000000000000000000000000000000000000000000000000' +
        'd207000000000000000000000000000000000000000000000000000000000000',
    },
    {
      name: 'c1_zero__c0_negative',
      point: [
        ['1003', '2003'],
        ['21888242871839275222246405745257275088696311157297823662689037894645226208574', '0'],
        ['1', '0'],
      ],
      hex:
        'eb03000000000000000000000000000000000000000000000000000000000000' +
        'd307000000000000000000000000000000000000000000000000000000000080',
    },
  ];

  for (const { name, point, hex } of cases) {
    it(`matches Rust/Python ground truth: ${name}`, () => {
      const got = compressG2(point);
      expect(got.length).toBe(64);
      expect(Buffer.from(got).toString('hex')).toBe(hex);
      expect(got).toEqual(hexToBytes(hex));
    });
  }
});

describe('encodeGroth16Proof', () => {
  it('concatenates A || B || C || variant into 129 bytes', () => {
    const proof = {
      pi_a: ['111', '5', '1'] as SnarkjsG1,
      pi_b: [['1000', '2000'], ['999', '7'], ['1', '0']] as SnarkjsG2,
      pi_c: [
        '222',
        '21888242871839275222246405745257275088696311157297823662689037894645226208578',
        '1',
      ] as SnarkjsG1,
    };

    const out0 = encodeGroth16Proof(proof, 0);
    expect(out0.length).toBe(129);
    expect(Buffer.from(out0.slice(0, 32)).toString('hex')).toBe(
      '6f00000000000000000000000000000000000000000000000000000000000000'.slice(0, 64),
    );
    expect(Buffer.from(out0.slice(32, 96)).toString('hex')).toBe(
      'e803000000000000000000000000000000000000000000000000000000000000' +
        'd007000000000000000000000000000000000000000000000000000000000000',
    );
    expect(Buffer.from(out0.slice(96, 128)).toString('hex')).toBe(
      'de00000000000000000000000000000000000000000000000000000000000080',
    );
    expect(out0[128]).toBe(0);

    const out1 = encodeGroth16Proof(proof, 1);
    expect(out1[128]).toBe(1);
    // Everything but the variant byte is identical.
    expect(out1.slice(0, 128)).toEqual(out0.slice(0, 128));
  });
});

describe('sanity: P constant matches convert_vk.py', () => {
  it('BN254_BASE_FIELD_PRIME equals scripts/convert_vk.py P', () => {
    const { BN254_BASE_FIELD_PRIME } = require('./proofEncoding');
    expect(BN254_BASE_FIELD_PRIME.toString()).toBe(P);
  });
});
