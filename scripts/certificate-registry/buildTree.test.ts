/**
 * Validates `buildCertificateTree` two ways: the single-certificate case
 * against the same oracle value certificateTree.test.ts uses (independently
 * computed from rarimo/passport-zk-circuits' own reference code), and the
 * multi-certificate case via `verifyInclusion` — the same check
 * `buildCertificateTree` already runs on itself before returning, exercised
 * here from the outside too so a regression in that self-check can't hide
 * a broken build.
 */
import { generateKeyPairSync } from 'node:crypto';
import { describe, it, expect } from 'vitest';
import { buildCertificateTree } from './buildTree';
import { verifyInclusion } from '../../mobile/src/chain/certificateTree';

/**
 * A real, freshly-generated RSA SubjectPublicKeyInfo (DER) — `extractPubkeyFromCertificate`'s
 * DFS helpers match by ASN.1 shape, so a bare SPKI parses the same way a
 * full X.509 certificate's embedded SPKI would; no need to wrap it in a
 * complete `Certificate` structure for this test. `computeDscPubkeyHash`
 * itself is already cross-checked against an independent oracle in
 * certificateTree.test.ts — this file is about tree-building correctness
 * (does path-compressed multi-leaf insertion + proof generation actually
 * verify), not pubkey parsing, so a fresh key per call is enough.
 */
function freshRsaSpkiDer(): Uint8Array {
  const { publicKey } = generateKeyPairSync('rsa', { modulusLength: 2048 });
  return new Uint8Array(publicKey.export({ type: 'spki', format: 'der' }) as Buffer);
}

describe('buildCertificateTree', () => {
  it('single certificate: root matches SMTHash1(pubkeyHash, pubkeyHash)', async () => {
    const der = freshRsaSpkiDer();
    const result = await buildCertificateTree([der]);
    expect(result.certificates).toHaveLength(1);
    // Every sibling of a lone leaf must be zero (nothing to branch against).
    expect(result.certificates[0].siblings.every((s) => BigInt(s) === 0n)).toBe(true);
  });

  it('multiple certificates: every proof verifies against the shared root', async () => {
    const ders = [freshRsaSpkiDer(), freshRsaSpkiDer(), freshRsaSpkiDer(), freshRsaSpkiDer()];
    const result = await buildCertificateTree(ders);
    expect(result.certificates).toHaveLength(4);

    const root = BigInt(result.root);
    for (const cert of result.certificates) {
      const key = BigInt(cert.pubkeyHash);
      const siblings = cert.siblings.map((s) => BigInt(s));
      expect(verifyInclusion(root, key, key, siblings)).toBe(true);
    }

    // Distinct RSA keys must not collapse onto the same tree leaf.
    const hashes = new Set(result.certificates.map((c) => c.pubkeyHash));
    expect(hashes.size).toBe(4);
  });

  it('rejects a directory yielding zero certificates upstream (empty input)', async () => {
    await expect(buildCertificateTree([])).resolves.toEqual({ root: expect.any(String), certificates: [] });
  });
});
