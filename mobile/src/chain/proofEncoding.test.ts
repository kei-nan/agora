/**
 * Tests the ZKPassport UltraHonk proof envelope against the contract
 * `runtime/src/verifier.rs` implements.
 *
 * The headline test uses a **real bb 5.0.0 UltraHonk proof**
 * (`__fixtures__/ultrahonkProof.fixture.json`), not synthetic filler: bb 5.0.0 (the
 * version ZKPassport pins) was installed via `bbup`, and
 * `bb prove -t evm --write_vk` was run against a trivial Noir circuit
 * (`fn main(a: pub Field, b: pub Field, c: Field) { assert(a * b == c); }`, a=3 b=5 c=15).
 * That fixture is *not* a ZKPassport proof — it cannot be, since no ZKPassport proving
 * inputs exist yet — but it is genuine bb output, so it pins the encoder against real
 * proof lengths and the real 32-byte-word structure rather than against assumptions.
 *
 * The mirroring tests in `runtime/src/verifier.rs`'s `mod tests` cover the same
 * envelope and layout rules from the Rust side. If you change one, change both.
 */
import { Buffer } from 'buffer';
import {
  assertValidPublicInputs,
  BASE_SUBPROOF_COUNT,
  BN254_SCALAR_FIELD_MODULUS,
  decodeUltraHonkProof,
  describePublicInputs,
  disclosureCountFor,
  encodeUltraHonkProof,
  ENVELOPE_HEADER_LEN,
  ENVELOPE_MAGIC,
  ENVELOPE_VERSION,
  expectedPublicInputCount,
  fieldToBigInt,
  publicInputsFromHex,
  splitPublicInputs,
} from './proofEncoding';

const fixture = require('./__fixtures__/ultrahonkProof.fixture.json');

const realProof = new Uint8Array(Buffer.from(fixture.proofBase64, 'base64'));
const realPublicInputs = new Uint8Array(Buffer.from(fixture.publicInputsBase64, 'base64'));

/** A field element holding `value`, big-endian. */
function field(value: bigint): Uint8Array {
  const out = new Uint8Array(32);
  let v = value;
  for (let i = 31; i >= 0; i--) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  if (v !== 0n) {
    throw new RangeError(`field: ${value} does not fit in 32 bytes`);
  }
  return out;
}

/** A valid `count_4` public-input array: 9 elements, canonical, non-zero nullifier. */
function validPublicInputs(outerCount = 4): Uint8Array[] {
  const inputs = Array.from({ length: expectedPublicInputCount(outerCount) }, () => field(0n));
  inputs[2] = field(1_800_000_000n); // current_date, a plausible unix timestamp
  inputs[6 + disclosureCountFor(outerCount)] = field(42n); // scoped_nullifier
  return inputs;
}

describe('the real bb 5.0.0 fixture', () => {
  it('is genuine bb output with the length the toolchain produced', () => {
    expect(fixture.bbVersion).toBe('5.0.0');
    expect(realProof.length).toBe(fixture.proofLength);
    expect(realProof.length).toBe(4544);
    // A flat array of 32-byte words — the assumption the encoder enforces.
    expect(realProof.length % 32).toBe(0);
    // Two public inputs (a and b), 64 bytes.
    expect(realPublicInputs.length).toBe(64);
  });

  it('round-trips through the envelope byte-for-byte', () => {
    const envelope = encodeUltraHonkProof(realProof, { outerCount: 4, variant: 'zk' });
    expect(envelope.length).toBe(ENVELOPE_HEADER_LEN + realProof.length);

    const decoded = decodeUltraHonkProof(envelope);
    expect(decoded.header).toEqual({ outerCount: 4, variant: 'zk', proofLength: 4544 });
    expect(Buffer.from(decoded.proof)).toEqual(Buffer.from(realProof));
  });

  it('splits its public inputs into 32-byte field elements', () => {
    const inputs = splitPublicInputs(realPublicInputs);
    expect(inputs).toHaveLength(2);
    // The circuit was proved with a = 3, b = 5.
    expect(fieldToBigInt(inputs[0])).toBe(3n);
    expect(fieldToBigInt(inputs[1])).toBe(5n);
  });
});

describe('encodeUltraHonkProof', () => {
  const proof = new Uint8Array(64).fill(0xab);

  it('writes the header verifier.rs parses', () => {
    const envelope = encodeUltraHonkProof(proof, { outerCount: 7, variant: 'plain' });
    expect(envelope[0]).toBe(ENVELOPE_MAGIC);
    expect(envelope[0]).toBe(0x5a);
    expect(envelope[1]).toBe(ENVELOPE_VERSION);
    expect(envelope[2]).toBe(7);
    expect(envelope[3]).toBe(1); // plain
    // proof_len, big-endian u32
    expect(Array.from(envelope.slice(4, 8))).toEqual([0, 0, 0, 64]);
    expect(Array.from(envelope.slice(8))).toEqual(Array.from(proof));
  });

  it('tags the ZK variant as 0 and the plain variant as 1', () => {
    expect(encodeUltraHonkProof(proof, { outerCount: 4, variant: 'zk' })[3]).toBe(0);
    expect(encodeUltraHonkProof(proof, { outerCount: 4, variant: 'plain' })[3]).toBe(1);
  });

  it('rejects an outer count ZKPassport does not ship', () => {
    for (const outerCount of [0, 3, 14, 255, 4.5]) {
      expect(() => encodeUltraHonkProof(proof, { outerCount, variant: 'zk' })).toThrow(RangeError);
    }
  });

  it('rejects proof bytes that are not whole 32-byte words', () => {
    expect(() => encodeUltraHonkProof(new Uint8Array(0), { outerCount: 4, variant: 'zk' })).toThrow(
      RangeError,
    );
    expect(() => encodeUltraHonkProof(new Uint8Array(65), { outerCount: 4, variant: 'zk' })).toThrow(
      RangeError,
    );
  });

  it('works on a proof stored at a non-zero byte offset in its buffer', () => {
    // A Uint8Array from `subarray`/RNFS can share a larger ArrayBuffer; the DataView in
    // the encoder must respect byteOffset or it silently writes into someone else's
    // bytes. Regression guard, not hypothetical.
    const backing = new Uint8Array(200).fill(0x11);
    const view = backing.subarray(100, 164);
    const envelope = encodeUltraHonkProof(view, { outerCount: 4, variant: 'zk' });
    expect(Array.from(envelope.slice(4, 8))).toEqual([0, 0, 0, 64]);
    expect(decodeUltraHonkProof(envelope).proof.every((b) => b === 0x11)).toBe(true);
    // The backing buffer was not written through.
    expect(backing.every((b) => b === 0x11)).toBe(true);
  });
});

describe('decodeUltraHonkProof', () => {
  const envelope = () => encodeUltraHonkProof(new Uint8Array(64).fill(7), { outerCount: 4, variant: 'zk' });

  it('rejects a bad magic byte', () => {
    const bad = envelope();
    bad[0] = 0x00;
    expect(() => decodeUltraHonkProof(bad)).toThrow(/magic/);
  });

  it('rejects an unsupported format version', () => {
    const bad = envelope();
    bad[1] = 0x02;
    expect(() => decodeUltraHonkProof(bad)).toThrow(/version/);
  });

  it('rejects an unknown proof-variant tag', () => {
    const bad = envelope();
    bad[3] = 2;
    expect(() => decodeUltraHonkProof(bad)).toThrow(/variant/);
  });

  it('rejects truncation and trailing padding alike', () => {
    const full = envelope();
    expect(() => decodeUltraHonkProof(full.slice(0, full.length - 1))).toThrow(RangeError);

    const padded = new Uint8Array(full.length + 1);
    padded.set(full);
    expect(() => decodeUltraHonkProof(padded)).toThrow(RangeError);
  });

  it('rejects a header shorter than the header', () => {
    expect(() => decodeUltraHonkProof(new Uint8Array(4))).toThrow(RangeError);
  });
});

describe('the count_N public-input layout', () => {
  it('exposes N + 5 public inputs, of which N - 3 are param commitments', () => {
    for (let outerCount = 4; outerCount <= 13; outerCount++) {
      expect(expectedPublicInputCount(outerCount)).toBe(outerCount + 5);
      expect(disclosureCountFor(outerCount)).toBe(outerCount - BASE_SUBPROOF_COUNT);
    }
    // The registration shape.
    expect(expectedPublicInputCount(4)).toBe(9);
  });

  it('accepts a well-formed count_4 array', () => {
    expect(() => assertValidPublicInputs(4, validPublicInputs())).not.toThrow();
  });

  it('rejects the old Rarimo 5-signal array', () => {
    // The exact shape pallet-identity was built for. It must not pass as a ZKPassport one.
    expect(() => assertValidPublicInputs(4, Array.from({ length: 5 }, () => field(0n)))).toThrow(
      RangeError,
    );
  });

  it('rejects a non-canonical field element', () => {
    const inputs = validPublicInputs();
    inputs[0] = field(BN254_SCALAR_FIELD_MODULUS);
    expect(() => assertValidPublicInputs(4, inputs)).toThrow(/canonical/);
  });

  it('accepts the largest canonical field element', () => {
    const inputs = validPublicInputs();
    inputs[0] = field(BN254_SCALAR_FIELD_MODULUS - 1n);
    expect(() => assertValidPublicInputs(4, inputs)).not.toThrow();
  });

  it('rejects a current_date that does not fit in a u64', () => {
    const inputs = validPublicInputs();
    inputs[2] = field(1n << 64n);
    expect(() => assertValidPublicInputs(4, inputs)).toThrow(/current_date/);
  });

  it('rejects a zero scoped_nullifier, which the circuit itself forbids', () => {
    const inputs = validPublicInputs();
    inputs[7] = field(0n);
    expect(() => assertValidPublicInputs(4, inputs)).toThrow(/scoped_nullifier/);
  });

  it('names each field at the index the circuit puts it', () => {
    const inputs = validPublicInputs(5); // D = 2, so 10 public inputs
    inputs[0] = field(0x11n);
    inputs[1] = field(0x22n);
    inputs[3] = field(0x33n);
    inputs[4] = field(0x44n);
    inputs[5] = field(0x55n);
    inputs[6] = field(0x66n);
    inputs[7] = field(0x77n); // nullifier_type at 5 + D
    inputs[8] = field(0x88n); // scoped_nullifier at 6 + D
    inputs[9] = field(0x99n); // oprf_pk_hash at 7 + D

    const named = describePublicInputs(5, inputs);
    expect(fieldToBigInt(named.certificateRegistryRoot)).toBe(0x11n);
    expect(fieldToBigInt(named.circuitRegistryRoot)).toBe(0x22n);
    expect(named.currentDate).toBe(1_800_000_000n);
    expect(fieldToBigInt(named.serviceScope)).toBe(0x33n);
    expect(fieldToBigInt(named.serviceSubscope)).toBe(0x44n);
    expect(named.paramCommitments.map(fieldToBigInt)).toEqual([0x55n, 0x66n]);
    expect(fieldToBigInt(named.nullifierType)).toBe(0x77n);
    expect(fieldToBigInt(named.scopedNullifier)).toBe(0x88n);
    expect(fieldToBigInt(named.oprfPkHash)).toBe(0x99n);
  });
});

describe('publicInputsFromHex', () => {
  it('parses full-width 0x-prefixed words', () => {
    const [value] = publicInputsFromHex([`0x${'00'.repeat(31)}2a`]);
    expect(fieldToBigInt(value)).toBe(42n);
  });

  it('accepts unprefixed hex too', () => {
    const [value] = publicInputsFromHex(['00'.repeat(31) + 'ff']);
    expect(fieldToBigInt(value)).toBe(255n);
  });

  it('rejects short hex rather than left-padding it', () => {
    expect(() => publicInputsFromHex(['0x2a'])).toThrow(RangeError);
  });

  it('rejects non-hexadecimal input', () => {
    expect(() => publicInputsFromHex([`0x${'zz'.repeat(32)}`])).toThrow(RangeError);
  });
});

describe('splitPublicInputs', () => {
  it('rejects a length that is not a whole number of field elements', () => {
    expect(() => splitPublicInputs(new Uint8Array(33))).toThrow(RangeError);
  });

  it('returns an empty array for empty input', () => {
    expect(splitPublicInputs(new Uint8Array(0))).toEqual([]);
  });
});
