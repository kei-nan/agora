/**
 * Tests the DEV-ONLY key storage placeholders: both `devSigningKeypair()` and
 * `devOprfSecretShare()` are deterministic (same value every call, matching the
 * "every install of this build derives the same value" contract their doc comments
 * make), and are distinct from one another (they must never accidentally collapse to
 * the same bytes, since they're supposed to be two independent credentials).
 */
import { devOprfSecretShare, devSigningKeypair } from './keyStorage';

describe('devSigningKeypair', () => {
  it('returns the same address on every call', async () => {
    const a = await devSigningKeypair();
    const b = await devSigningKeypair();
    expect(a.address).toBe(b.address);
  });

  it('is capable of signing (a real KeyringPair, not a placeholder object)', async () => {
    const pair = await devSigningKeypair();
    const signature = pair.sign(new Uint8Array([1, 2, 3]));
    expect(signature).toBeInstanceOf(Uint8Array);
    expect(signature.length).toBeGreaterThan(0);
  });
});

describe('devOprfSecretShare', () => {
  it('returns a 32-byte value, identical on every call', async () => {
    const a = await devOprfSecretShare();
    const b = await devOprfSecretShare();
    expect(a).toHaveLength(32);
    expect(a).toEqual(b);
  });

  it('is distinct from the signing keypair\'s public key', async () => {
    const secretShare = await devOprfSecretShare();
    const pair = await devSigningKeypair();
    expect(Buffer.from(secretShare).equals(Buffer.from(pair.publicKey))).toBe(false);
  });
});
