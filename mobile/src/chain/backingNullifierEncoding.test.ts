/**
 * Tests the `backing-nullifier` proof envelope against the contract
 * `runtime/src/backing_nullifier_verifier.rs` implements. Purely synthetic byte-shape
 * fixtures (no real bb proof exists for this circuit's own fixture here — the real
 * cross-checked round trip lives on the Rust side, `cargo test -p agora-runtime --lib
 * backing_nullifier_verifier`) — this test suite only pins the JS-side encoder/decoder and
 * public-input validation against that documented byte layout.
 */
import {
  assertValidBackingPublicInputs,
  BACKING_DELEGATE_PERSONA_ID_INDEX,
  BACKING_ENVELOPE_HEADER_LEN,
  BACKING_ENVELOPE_MAGIC,
  BACKING_ENVELOPE_VERSION,
  BACKING_MAX_BACKINGS_PER_CITIZEN_INDEX,
  BACKING_NULLIFIER_INDEX,
  BACKING_PUBLIC_INPUT_COUNT,
  BACKING_ROOT_INDEX,
  decodeBackingNullifierProof,
  encodeBackingNullifierProof,
} from './backingNullifierEncoding';

/** A field element holding `value`, big-endian. */
function field(value: bigint): Uint8Array {
  const out = new Uint8Array(32);
  let v = value;
  for (let i = 31; i >= 0; i--) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

/** `words` 32-byte words concatenated, as a fake `bb prove -t evm` proof output. */
function fakeProof(words: number): Uint8Array {
  const out = new Uint8Array(words * 32);
  out.fill(0xab);
  return out;
}

describe('encodeBackingNullifierProof / decodeBackingNullifierProof', () => {
  it('round-trips a well-formed proof', () => {
    const proof = fakeProof(61); // roughly this circuit's real bb-reported gate-adjacent size
    const envelope = encodeBackingNullifierProof(proof, 'zk');
    expect(envelope.length).toBe(BACKING_ENVELOPE_HEADER_LEN + proof.length);
    expect(envelope[0]).toBe(BACKING_ENVELOPE_MAGIC);
    expect(envelope[1]).toBe(BACKING_ENVELOPE_VERSION);

    const decoded = decodeBackingNullifierProof(envelope);
    expect(decoded.header.variant).toBe('zk');
    expect(decoded.header.proofLength).toBe(proof.length);
    expect(decoded.proof).toEqual(proof);
  });

  it('encodes the plain variant with a distinct tag', () => {
    const envelope = encodeBackingNullifierProof(fakeProof(2), 'plain');
    expect(decodeBackingNullifierProof(envelope).header.variant).toBe('plain');
  });

  it('uses a distinct magic byte from the ZKPassport outer-proof envelope', () => {
    // 0x5A is proofEncoding.ts's ENVELOPE_MAGIC — this envelope must never collide with it.
    expect(BACKING_ENVELOPE_MAGIC).toBe(0x42);
    expect(BACKING_ENVELOPE_MAGIC).not.toBe(0x5a);
  });

  it('has no outer_count byte — one byte shorter than the ZKPassport envelope header', () => {
    expect(BACKING_ENVELOPE_HEADER_LEN).toBe(7);
  });

  it('rejects an empty proof', () => {
    expect(() => encodeBackingNullifierProof(new Uint8Array(0), 'zk')).toThrow(/empty/);
  });

  it('rejects a proof whose length is not a multiple of 32', () => {
    expect(() => encodeBackingNullifierProof(new Uint8Array(33), 'zk')).toThrow(/multiple of 32/);
  });

  it('rejects a truncated envelope', () => {
    const envelope = encodeBackingNullifierProof(fakeProof(2), 'zk');
    expect(() => decodeBackingNullifierProof(envelope.slice(0, -1))).toThrow(/holds/);
  });

  it('rejects trailing padding after the declared proof length', () => {
    const envelope = encodeBackingNullifierProof(fakeProof(2), 'zk');
    const padded = new Uint8Array(envelope.length + 32);
    padded.set(envelope);
    expect(() => decodeBackingNullifierProof(padded)).toThrow(/holds/);
  });

  it('rejects a bad magic byte', () => {
    const envelope = encodeBackingNullifierProof(fakeProof(2), 'zk');
    envelope[0] = 0x99;
    expect(() => decodeBackingNullifierProof(envelope)).toThrow(/bad magic/);
  });

  it('rejects an unknown format version', () => {
    const envelope = encodeBackingNullifierProof(fakeProof(2), 'zk');
    envelope[1] = 0x02;
    expect(() => decodeBackingNullifierProof(envelope)).toThrow(/format version/);
  });

  it('rejects an unknown proof-variant tag', () => {
    const envelope = encodeBackingNullifierProof(fakeProof(2), 'zk');
    envelope[2] = 0x07;
    expect(() => decodeBackingNullifierProof(envelope)).toThrow(/unknown proof variant/);
  });

  it('rejects a header too short to hold the 7-byte header', () => {
    expect(() => decodeBackingNullifierProof(new Uint8Array(3))).toThrow(/shorter than/);
  });
});

describe('assertValidBackingPublicInputs', () => {
  function validInputs(): Uint8Array[] {
    return [field(1n), field(2n), field(3n), field(4n)];
  }

  it('accepts a well-formed 4-field array', () => {
    expect(() => assertValidBackingPublicInputs(validInputs())).not.toThrow();
  });

  it('has the documented field order', () => {
    expect(BACKING_ROOT_INDEX).toBe(0);
    expect(BACKING_DELEGATE_PERSONA_ID_INDEX).toBe(1);
    expect(BACKING_MAX_BACKINGS_PER_CITIZEN_INDEX).toBe(2);
    expect(BACKING_NULLIFIER_INDEX).toBe(3);
    expect(BACKING_PUBLIC_INPUT_COUNT).toBe(4);
  });

  it('rejects too few public inputs', () => {
    expect(() => assertValidBackingPublicInputs(validInputs().slice(0, 3))).toThrow(/expected 4/);
  });

  it('rejects too many public inputs', () => {
    expect(() => assertValidBackingPublicInputs([...validInputs(), field(5n)])).toThrow(/expected 4/);
  });

  it('rejects a mis-sized field element', () => {
    const inputs = validInputs();
    inputs[1] = new Uint8Array(31);
    expect(() => assertValidBackingPublicInputs(inputs)).toThrow(/32/);
  });

  it('rejects a non-canonical field element', () => {
    const inputs = validInputs();
    inputs[0] = new Uint8Array(32).fill(0xff); // far above the BN254 modulus
    expect(() => assertValidBackingPublicInputs(inputs)).toThrow(/canonical/);
  });
});
