/**
 * Cross-checks `buildCircuitInputs` against the reference implementation
 * (`process_passport.js` from rarimo/passport-zk-circuits) on a synthetic
 * but structurally real CMS SignedData SOD — real ASN.1, a real RSA-2048
 * keypair + self-signed certificate, and a real RSASSA-PKCS1v1.5-SHA256
 * signature (Node's `crypto.sign`), hand-assembled to have exactly the two
 * signed attributes (contentType, messageDigest) an ICAO SOD actually has
 * (unlike `openssl cms -sign`'s default output, which also adds
 * `signingTime`/`smimeCapabilities` — attributes real SODs don't carry, and
 * whose presence broke `getZero`'s "messageDigest is the SET's last
 * attribute" assumption in an earlier draft of this fixture; see
 * HANDOFF.md's log entry for this work).
 *
 * `sodParser.fixture.json` bundles: the raw dg1/sod bytes (hex), the RSA
 * public key's real (n, exp), the real signature bytes, and this exact
 * fixture's actual output from running the *unmodified* reference
 * `processPassport()` end-to-end (variant name + padded bit arrays + limb-
 * chunked pubkey/signature) — captured once by cloning
 * rarimo/passport-zk-circuits and running it directly, not reproduced from
 * memory. Regenerating this fixture (new keypair, new random DG1) is
 * possible by re-running that same process; not scripted into this repo
 * since it needs a clone of the upstream circuits repo + openssl, which are
 * one-off tooling, not project dependencies.
 */
import { createPublicKey, createVerify } from 'crypto';
import { buildCircuitInputs } from './sodParser';
import fixture from './__fixtures__/sodParser.fixture.json';

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return bytes;
}

describe('buildCircuitInputs', () => {
  const dg1 = hexToBytes(fixture.dg1Hex);
  const sod = hexToBytes(fixture.sodHex);
  const dg15 = new Uint8Array(0);

  const { variant, inputs } = buildCircuitInputs(dg1, dg15, sod);

  it('identifies the same circuit variant as the reference implementation', () => {
    expect(variant.name).toBe(fixture.expectedVariantName);
  });

  it('extracts pubkey limbs identical to the reference implementation', () => {
    expect(inputs.pubkey).toEqual(fixture.referencePubkeyChunks);
  });

  it('extracts signature limbs identical to the reference implementation', () => {
    expect(inputs.signature).toEqual(fixture.referenceSignatureChunks);
  });

  it('pads dg1 identically to the reference implementation', () => {
    expect(inputs.dg1.join('')).toBe(fixture.referenceDg1Bits);
  });

  it('pads encapsulatedContent identically to the reference implementation', () => {
    expect(inputs.encapsulatedContent.join('')).toBe(fixture.referenceEncapsulatedContentBits);
  });

  it('pads signedAttributes identically to the reference implementation', () => {
    expect(inputs.signedAttributes.join('')).toBe(fixture.referenceSignedAttributesBits);
  });

  it('produces no dg15 fields for an _NA (no Active Authentication) passport', () => {
    expect(inputs.dg15).toEqual([]);
    expect(variant.dg15SigAlgo).toBe(0);
    expect(variant.dg15Blocks).toBe(0);
  });

  // Strongest possible check: not just "matches the reference script's
  // output", but "the extracted signedAttributes bytes are the actual bytes
  // a real RSA signature over this actual key verifies against" — exercises
  // the module against ground truth independent of both this test file and
  // process_passport.js.
  it("extracts signedAttributes bytes that cryptographically verify against the fixture's real RSA signature", () => {
    // padBits' output is SHA-padded (0x80 + zero-fill + bit-length suffix,
    // to a 512/1024-bit block boundary); the original DER content is a
    // prefix of it, whose exact length is self-describing via its own DER
    // length prefix — decode that directly rather than needing it out of
    // band.
    const saBytesPadded = Buffer.from(bitsToBytes(inputs.signedAttributes.join('')));
    const saBytes = saBytesPadded.subarray(0, derTlvLength(saBytesPadded));

    const publicKey = createPublicKey({
      key: {
        kty: 'RSA',
        n: Buffer.from(fixture.expectedPubkeyN, 'hex').toString('base64url'),
        e: Buffer.from(fixture.expectedPubkeyExp, 'hex').toString('base64url'),
      },
      format: 'jwk',
    });
    const verifier = createVerify('RSA-SHA256');
    verifier.update(saBytes);
    expect(verifier.verify(publicKey, Buffer.from(fixture.expectedSignatureHex, 'hex'))).toBe(true);
  });
});

function bitsToBytes(bits: string): Uint8Array {
  const out = new Uint8Array(bits.length / 8);
  for (let i = 0; i < out.length; i++) {
    out[i] = parseInt(bits.slice(i * 8, i * 8 + 8), 2);
  }
  return out;
}

/** Reads a DER TLV's total encoded length (tag + length-field + value bytes) from its start. */
function derTlvLength(buf: Buffer): number {
  let pos = 1; // tag byte
  const first = buf[pos];
  pos += 1;
  let len: number;
  if ((first & 0x80) === 0) {
    len = first;
  } else {
    const n = first & 0x7f;
    len = 0;
    for (let i = 0; i < n; i++) len = (len << 8) | buf[pos + i];
    pos += n;
  }
  return pos + len;
}
