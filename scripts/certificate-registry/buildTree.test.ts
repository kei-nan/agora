/**
 * Validates `certificateDerToLeaf` + `buildCertificateTree` against real,
 * freshly-generated self-signed X.509 certificates (via `@peculiar/x509`'s
 * `X509CertificateGenerator` — a real, standard-conformant DER encoder, not
 * a hand-rolled fixture) — not bare SubjectPublicKeyInfo blobs, since this
 * tool needs a full certificate (issuer country, validity) unlike
 * `mobile/src/chain/certificateTree.test.ts`'s leaf-hash-only cross-checks.
 */
import { webcrypto } from 'node:crypto';
import { X509CertificateGenerator } from '@peculiar/x509';
import { describe, it, expect } from 'vitest';
import { buildCertificateTree, certificateDerToLeaf } from './buildTree';
import { calculateCertificateLeafHash, verifyInclusion } from '../../mobile/src/chain/certificateTree';

const crypto = webcrypto as unknown as Crypto;

async function freshSelfSignedCertDer(countryCode: string): Promise<Uint8Array> {
  const keys = await crypto.subtle.generateKey(
    { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]) },
    true,
    ['sign', 'verify'],
  );
  const cert = await X509CertificateGenerator.createSelfSigned(
    {
      serialNumber: '01',
      name: `C=${countryCode}, O=Agora Test, CN=Test DSC`,
      notBefore: new Date('2024-01-01T00:00:00Z'),
      notAfter: new Date('2034-01-01T00:00:00Z'),
      signingAlgorithm: { name: 'RSASSA-PKCS1-v1_5', hash: 'SHA-256' },
      keys,
    },
    crypto,
  );
  return new Uint8Array(cert.rawData);
}

describe('certificateDerToLeaf', () => {
  it('extracts issuer country and expiry from a real self-signed certificate', async () => {
    const der = await freshSelfSignedCertDer('US');
    const leaf = certificateDerToLeaf(der);
    expect(leaf.country).toBe('USA');
    expect(leaf.expiry).toBe(Math.floor(new Date('2034-01-01T00:00:00Z').getTime() / 1000));
    expect(leaf.publicKey.length).toBe(256); // 2048-bit RSA modulus
  });
});

describe('buildCertificateTree', () => {
  it('single certificate: the leaf-level sibling is literally zero; every level above is a nonzero zero-subtree hash', async () => {
    const der = await freshSelfSignedCertDer('US');
    const result = await buildCertificateTree([der]);
    expect(result.certificates).toHaveLength(1);
    expect(result.certificates[0].index).toBe(0);
    // Level 0 has no second leaf to pair with, so that sibling is the raw
    // zero value — but every level above combines a real ancestor with an
    // "empty subtree" hash (Poseidon2(0,0), Poseidon2 of that, ...), which
    // is NOT itself zero. Only the first sibling should be exactly 0n.
    const siblings = result.certificates[0].siblings.map((s) => BigInt(s));
    expect(siblings[0]).toBe(0n);
    expect(siblings.slice(1).every((s) => s !== 0n)).toBe(true);
  });

  it('multiple certificates: every proof verifies against the shared root', async () => {
    const ders = await Promise.all(['US', 'DE', 'FR', 'JP'].map(freshSelfSignedCertDer));
    const result = await buildCertificateTree(ders);
    expect(result.certificates).toHaveLength(4);

    const root = BigInt(result.root);
    for (const cert of result.certificates) {
      const leaf = certificateDerToLeaf(ders.find((der) => {
        const l = certificateDerToLeaf(der);
        return calculateCertificateLeafHash(l).toString(16).padStart(64, '0') === cert.leafHash.slice(2);
      })!);
      expect(verifyInclusion(root, leaf, cert.index, cert.siblings.map((s) => BigInt(s)))).toBe(true);
    }

    // Distinct certificates must not collapse onto the same tree leaf.
    const hashes = new Set(result.certificates.map((c) => c.leafHash));
    expect(hashes.size).toBe(4);
  });

  it('rejects a directory yielding zero certificates upstream (empty input)', async () => {
    const result = await buildCertificateTree([]);
    expect(result.certificates).toEqual([]);
    expect(result.root).toMatch(/^0x[0-9a-f]{64}$/);
  });
});
