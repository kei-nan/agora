/**
 * Tests `buildCircuitInputs` against the same fixture SOD the Rarimo-era suite used —
 * deliberately the same one, because the point of this file's ZKPassport rework is that
 * the *extraction* layer did not change, only the encoding of its output.
 *
 * `sodParser.fixture.json` is a synthetic but structurally real CMS SignedData SOD: real
 * ASN.1, a real RSA-2048 keypair + self-signed certificate, and a real
 * RSASSA-PKCS1v1.5-SHA256 signature (Node's `crypto.sign`), hand-assembled to have exactly
 * the two signed attributes (contentType, messageDigest) an ICAO SOD actually has.
 *
 * The fixture's `referencePubkeyChunks` / `referenceSignatureChunks` / `reference*Bits`
 * fields captured the *Rarimo* reference script's circom-shaped output. Those are no
 * longer the target encoding, so they are no longer asserted against — but the fields
 * they were derived from are, in their new byte form, and the fixture's independent
 * ground truth (`expectedPubkeyN`, `expectedPubkeyExp`, `expectedSignatureHex`) is now
 * load-bearing rather than incidental. The headline test still verifies a real RSA
 * signature over the extracted bytes, which is ground truth independent of both this file
 * and any vendor's reference script.
 */
import { createPublicKey, createVerify } from 'crypto';
import {
  bigintToBeBytes,
  buildCircuitInputs,
  classifySignatureAlgorithm,
  DG1_LENGTH,
  ECONTENT_LENGTH,
  hashAlgorithmFor,
  hexToBeBytes,
  padToFixedLength,
  SIGNED_ATTRS_LENGTH,
  tbsBucketFor,
} from './sodParser';
import fixture from './__fixtures__/sodParser.fixture.json';

function hexToBytes(hex: string): Uint8Array {
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i++) bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  return bytes;
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

describe('buildCircuitInputs', () => {
  const dg1 = hexToBytes(fixture.dg1Hex);
  const sod = hexToBytes(fixture.sodHex);
  const dg15 = new Uint8Array(0);

  const { variant, idData, integrity, activeAuthentication, unresolved } = buildCircuitInputs(
    dg1,
    dg15,
    sod,
  );

  it('selects real ZKPassport circuit paths for this passport', () => {
    // The fixture is RSA-2048 / PKCS#1 v1.5 / SHA-256 throughout, so these are the
    // circuits ZKPassport actually ships for it — all three paths exist under
    // `src/noir/bin/` in the circuits repo.
    expect(variant.signature).toEqual({
      kind: 'rsa',
      path: 'rsa/pkcs/2048/sha256',
      byteWidth: 256,
    });
    expect(variant.signedAttrsHash).toBe('sha256');
    expect(variant.dataGroupHash).toBe('sha256');
    expect(variant.idDataCircuit).toBe(`sig-check/id-data/tbs_${variant.tbsBucket}/rsa/pkcs/2048/sha256`);
    expect(variant.dscCircuit).toBe(`sig-check/dsc/tbs_${variant.tbsBucket}/rsa/pkcs/2048/sha256`);
    expect(variant.integrityCircuit).toBe('data-check/integrity/sa_sha256/dg_sha256');
    // RegisterScreen.tsx interpolates this; keep it a non-empty identifying string.
    expect(variant.name).toBe(variant.idDataCircuit);
  });

  it('is no longer the Rarimo variant name', () => {
    // Guards against a half-finished revert: the old name was
    // `registerIdentity_1_256_1_2_336_248_NA`.
    expect(variant.name).not.toBe(fixture.expectedVariantName);
    expect(variant.name).not.toMatch(/^registerIdentity_/);
  });

  it('pads each byte array to the size its Noir parameter declares', () => {
    expect(idData.dg1).toHaveLength(DG1_LENGTH);
    expect(idData.signedAttributes).toHaveLength(SIGNED_ATTRS_LENGTH);
    expect(idData.eContent).toHaveLength(ECONTENT_LENGTH);
    expect(idData.tbsCertificate).toHaveLength(variant.tbsBucket);

    // Padding is zero-fill on the right, and the real length is recorded separately.
    expect(Array.from(idData.dg1.slice(0, dg1.length))).toEqual(Array.from(dg1));
    expect(idData.dg1Size).toBe(dg1.length);
    expect(idData.dg1.slice(dg1.length).every((b) => b === 0)).toBe(true);
    expect(idData.signedAttributes.slice(idData.signedAttributesSize).every((b) => b === 0)).toBe(true);
    expect(idData.eContent.slice(idData.eContentSize).every((b) => b === 0)).toBe(true);
  });

  it('emits the DSC public key as raw big-endian bytes, not circom limbs', () => {
    expect(idData.dscPubkey.kind).toBe('rsa');
    if (idData.dscPubkey.kind !== 'rsa') throw new Error('unreachable');
    expect(Buffer.from(idData.dscPubkey.modulus).toString('hex')).toBe(fixture.expectedPubkeyN);
    expect(idData.dscPubkey.modulus).toHaveLength(256);
    expect(idData.dscPubkey.exponent).toBe(65537);
  });

  it('emits the SOD signature as raw big-endian bytes', () => {
    expect(Buffer.from(idData.sodSignature).toString('hex')).toBe(fixture.expectedSignatureHex);
    expect(idData.sodSignature).toHaveLength(256);
  });

  it("extracts the DSC's TBSCertificate as well-formed DER", () => {
    const tbs = Buffer.from(idData.tbsCertificate.subarray(0, idData.tbsCertificateSize));
    // A TBSCertificate is a SEQUENCE, and its self-describing DER length must account for
    // exactly the bytes extracted — a good check that the ASN.1 walk found a real element
    // boundary rather than a plausible-looking substring.
    expect(tbs[0]).toBe(0x30);
    expect(derTlvLength(tbs)).toBe(idData.tbsCertificateSize);
    expect(idData.tbsCertificateSize).toBeGreaterThan(0);
    expect(idData.tbsCertificateSize).toBeLessThanOrEqual(variant.tbsBucket);
  });

  it('reports datagroup-hash offsets in bytes', () => {
    // The DG1 hash really is where the offset says it is.
    const { createHash } = require('crypto');
    const dg1Digest = createHash('sha256').update(Buffer.from(dg1)).digest();
    const eContent = Buffer.from(idData.eContent.subarray(0, idData.eContentSize));
    expect(
      eContent.subarray(integrity.dg1HashOffset, integrity.dg1HashOffset + 32).equals(dg1Digest),
    ).toBe(true);

    // Likewise the e-content hash inside the signed attributes.
    const ecDigest = createHash('sha256').update(eContent).digest();
    const signedAttrs = Buffer.from(idData.signedAttributes.subarray(0, idData.signedAttributesSize));
    expect(
      signedAttrs
        .subarray(integrity.eContentHashOffset, integrity.eContentHashOffset + 32)
        .equals(ecDigest),
    ).toBe(true);
  });

  it('reports no Active Authentication for a passport without DG15', () => {
    expect(activeAuthentication).toBeNull();
    expect(integrity.dg15HashOffset).toBeNull();
  });

  it('names every input it cannot derive from the chip', () => {
    expect(unresolved).toEqual({
      cscCertificate: null,
      certificateTreeProof: null,
      commitmentSalts: null,
      serviceScope: null,
    });
  });

  // Strongest possible check: not "matches some reference script's output", but "the
  // extracted signedAttributes bytes are the actual bytes a real RSA signature over this
  // actual key verifies against". Independent of this test file and of any vendor.
  it("extracts signedAttributes bytes that cryptographically verify against the fixture's real RSA signature", () => {
    const saBytes = Buffer.from(idData.signedAttributes.subarray(0, idData.signedAttributesSize));
    // Self-consistency: the recorded size must be the DER element's own declared length.
    expect(derTlvLength(saBytes)).toBe(saBytes.length);

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

describe('byte-encoding helpers (replacing the circom bit/limb ones)', () => {
  it('bigintToBeBytes writes big-endian and keeps leading zeros', () => {
    expect(Array.from(bigintToBeBytes(1n, 4))).toEqual([0, 0, 0, 1]);
    expect(Array.from(bigintToBeBytes(0x0102n, 2))).toEqual([1, 2]);
  });

  it('bigintToBeBytes refuses to silently truncate', () => {
    expect(() => bigintToBeBytes(0x10000n, 2)).toThrow(RangeError);
    expect(() => bigintToBeBytes(-1n, 4)).toThrow(RangeError);
  });

  it('hexToBeBytes left-pads rather than losing leading zero bytes', () => {
    // The exact hazard `certificateTree.ts` documents: a BigInt round-trip drops leading
    // zeros, and a modulus missing one is a modulus the circuit rejects.
    expect(Array.from(hexToBeBytes('01', 4))).toEqual([0, 0, 0, 1]);
  });

  it('padToFixedLength zero-fills on the right and refuses to truncate', () => {
    expect(Array.from(padToFixedLength(new Uint8Array([1, 2]), 4, 'x'))).toEqual([1, 2, 0, 0]);
    expect(() => padToFixedLength(new Uint8Array(5), 4, 'x')).toThrow(RangeError);
  });
});

describe('ZKPassport circuit selection', () => {
  it('maps digest lengths onto the algorithm names the circuit paths use', () => {
    expect(hashAlgorithmFor(20, 'x')).toBe('sha1');
    expect(hashAlgorithmFor(28, 'x')).toBe('sha224');
    expect(hashAlgorithmFor(32, 'x')).toBe('sha256');
    expect(hashAlgorithmFor(48, 'x')).toBe('sha384');
    expect(hashAlgorithmFor(64, 'x')).toBe('sha512');
    expect(() => hashAlgorithmFor(16, 'x')).toThrow();
  });

  it('picks the smallest tbs bucket that fits', () => {
    expect(tbsBucketFor(1)).toBe(700);
    expect(tbsBucketFor(700)).toBe(700);
    expect(tbsBucketFor(701)).toBe(1000);
    expect(tbsBucketFor(1600)).toBe(1600);
    expect(() => tbsBucketFor(1601)).toThrow();
  });

  it('distinguishes PKCS#1 v1.5 from PSS by the salt length', () => {
    const pubkey = { kind: 'rsa' as const, n: 'ab'.repeat(256), exp: '10001' };
    expect(
      classifySignatureAlgorithm(pubkey, { kind: 'rsa', n: 'cd'.repeat(256), salt: 0 }, 'sha256').path,
    ).toBe('rsa/pkcs/2048/sha256');
    expect(
      classifySignatureAlgorithm(pubkey, { kind: 'rsa', n: 'cd'.repeat(256), salt: '32' }, 'sha256').path,
    ).toBe('rsa/pss/2048/sha256');
  });

  it('rejects an RSA key size ZKPassport has no circuit for', () => {
    const pubkey = { kind: 'rsa' as const, n: 'ab'.repeat(320), exp: '10001' }; // 2560-bit
    expect(() =>
      classifySignatureAlgorithm(pubkey, { kind: 'rsa', n: 'cd'.repeat(320), salt: 0 }, 'sha256'),
    ).toThrow(/2560-bit/);
  });

  it('maps known ECDSA curve parameters onto ZKPassport curve directories', () => {
    const p256 = 'FFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFC';
    const pubkey = { kind: 'ecdsa' as const, x: 'aa'.repeat(32), y: 'bb'.repeat(32), param: p256 };
    const sig = { kind: 'ecdsa' as const, r: '11'.repeat(32), s: '22'.repeat(32) };
    expect(classifySignatureAlgorithm(pubkey, sig, 'sha256')).toEqual({
      kind: 'ecdsa',
      path: 'ecdsa/nist/p256/sha256',
      byteWidth: 32,
    });
  });

  it('rejects an unknown curve rather than guessing', () => {
    const pubkey = { kind: 'ecdsa' as const, x: 'aa', y: 'bb', param: 'DEADBEEF' };
    const sig = { kind: 'ecdsa' as const, r: '11', s: '22' };
    expect(() => classifySignatureAlgorithm(pubkey, sig, 'sha256')).toThrow(/unrecognized/);
  });

  it('rejects a pubkey/signature algorithm mismatch', () => {
    const pubkey = { kind: 'rsa' as const, n: 'ab'.repeat(256), exp: '10001' };
    const sig = { kind: 'ecdsa' as const, r: '11', s: '22' };
    expect(() => classifySignatureAlgorithm(pubkey, sig, 'sha256')).toThrow(/disagree/);
  });
});
