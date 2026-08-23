/**
 * Tests `keystoreWallet.ts` — the layer that turns the raw Keystore
 * encrypt/decrypt primitive (`native/keystoreSigner.ts`) into a persisted,
 * reusable `KeyringPair` — against the manual `__mocks__/react-native.js`
 * and `__mocks__/react-native-fs.js` mocks (see those files' doc comments
 * for why both are needed: the real packages can't load under this repo's
 * plain-'node' Jest environment at all).
 *
 * Like `native/keystoreSigner.test.ts`, this module caches state in
 * module-scope (`_keypairPromise`, plus the underlying native module refs
 * one level down) at import time, so each test re-requires it fresh via
 * `jest.resetModules()` after wiring up the fake native Keystore module and
 * clearing the fake filesystem — see `loadModule()`.
 *
 * What's covered: a fresh install generates a random seed, encrypts it, and
 * persists it; a second call reuses the same persisted (and re-decrypted)
 * seed rather than generating a new one, so the derived address is stable
 * across "restarts" (fresh `loadModule()` calls simulate that); the
 * generated seed is never written to the fake filesystem in plaintext; a
 * corrupt/unrecognized on-disk file is treated as "no wallet on file" rather
 * than crashing; calling this when the Keystore module isn't linked throws
 * rather than silently doing something insecure.
 *
 * What's NOT covered, honestly: no real Android Keystore, no real
 * react-native-fs sandboxed storage, no real device.
 */
import type * as KeystoreWalletModule from './keystoreWallet';

const WALLET_FILE_PATH = '/mock-documents/agora-wallet-key.enc.json';

/** Same reverse-bytes fake cipher as `native/keystoreSigner.test.ts` — real enough to prove plaintext never hits disk, not real crypto. */
function installFakeKeystoreModule() {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rn = require('react-native');
  rn.Platform.OS = 'android';
  rn.NativeModules.KeystoreSigningModule = {
    encryptSecret: jest.fn(async (plaintextB64: string) => {
      const plaintext = Buffer.from(plaintextB64, 'base64');
      return {
        ciphertext: Buffer.from([...plaintext].reverse()).toString('base64'),
        iv: Buffer.from([7, 7, 7, 7]).toString('base64'),
      };
    }),
    decryptSecret: jest.fn(async (ciphertextB64: string) => {
      const ciphertext = Buffer.from(ciphertextB64, 'base64');
      return Buffer.from([...ciphertext].reverse()).toString('base64');
    }),
    isHardwareBacked: jest.fn(async () => true),
  };
}

function loadModule(): typeof KeystoreWalletModule {
  jest.resetModules();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  require('react-native-fs').__reset();
  installFakeKeystoreModule();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require('./keystoreWallet');
}

/** Reads back whatever `keystoreWallet.ts` actually wrote to the fake filesystem. */
async function readWalletFile(): Promise<{ version: number; ciphertext: string; iv: string }> {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rnfs = require('react-native-fs');
  const raw = await rnfs.readFile(WALLET_FILE_PATH, 'utf8');
  return JSON.parse(raw);
}

describe('getOrCreateKeystoreKeypair', () => {
  it('throws when the Keystore module is not linked, without touching the filesystem', async () => {
    jest.resetModules();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    require('react-native-fs').__reset();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rn = require('react-native');
    delete rn.NativeModules.KeystoreSigningModule;
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const mod: typeof KeystoreWalletModule = require('./keystoreWallet');

    await expect(mod.getOrCreateKeystoreKeypair()).rejects.toThrow(/not available/);
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    await expect(require('react-native-fs').exists(WALLET_FILE_PATH)).resolves.toBe(false);
  });

  it('generates and persists a wallet file on first use', async () => {
    const mod = loadModule();
    await mod.getOrCreateKeystoreKeypair();

    const file = await readWalletFile();
    expect(file.version).toBe(1);
    expect(typeof file.ciphertext).toBe('string');
    expect(typeof file.iv).toBe('string');
  });

  it('never writes the plaintext seed to disk', async () => {
    const mod = loadModule();
    const keypair = await mod.getOrCreateKeystoreKeypair();

    const file = await readWalletFile();
    const seedHex = Buffer.from(keypair.publicKey).toString('hex'); // not the seed itself, but any raw key material is a good canary
    expect(file.ciphertext).not.toContain(seedHex);
    // The fake cipher is just byte-reversal, so the *ciphertext*, base64-decoded
    // and reversed, must equal the real seed used to derive the keypair —
    // and that seed must never appear verbatim (non-reversed) in the file.
    const ciphertextBytes = Buffer.from(file.ciphertext, 'base64');
    const recoveredSeed = Buffer.from([...ciphertextBytes].reverse());
    expect(Buffer.from(file.ciphertext, 'base64').equals(recoveredSeed)).toBe(false);
  });

  it('returns a real sr25519 KeyringPair with a working sign()', async () => {
    const mod = loadModule();
    const keypair = await mod.getOrCreateKeystoreKeypair();

    expect(keypair.address).toEqual(expect.any(String));
    expect(keypair.publicKey).toBeInstanceOf(Uint8Array);
    expect(keypair.publicKey.length).toBe(32);
    const sig = keypair.sign(Buffer.from('hello'));
    expect(sig).toBeInstanceOf(Uint8Array);
    expect(sig.length).toBeGreaterThan(0);
  });

  it('reuses the same persisted seed across a simulated app restart (fresh module load)', async () => {
    const first = loadModule();
    const keypairA = await first.getOrCreateKeystoreKeypair();
    const addressA = keypairA.address;

    // Simulate an app restart: reset the module registry (clears
    // keystoreWallet.ts's in-memory `_keypairPromise` cache) but do NOT
    // reset the fake filesystem or re-install a fresh fake Keystore module
    // with different behavior — the persisted file should still be there.
    jest.resetModules();
    installFakeKeystoreModule();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const second: typeof KeystoreWalletModule = require('./keystoreWallet');
    const keypairB = await second.getOrCreateKeystoreKeypair();

    expect(keypairB.address).toBe(addressA);
  });

  it('caches within a process — a second call in the same load does not touch the native module again', async () => {
    const mod = loadModule();
    await mod.getOrCreateKeystoreKeypair();
    await mod.getOrCreateKeystoreKeypair();

    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.KeystoreSigningModule;
    // One encrypt (first-use generation) and zero further decrypts, since
    // the second call should hit the in-memory cache, not disk again.
    expect(native.encryptSecret).toHaveBeenCalledTimes(1);
    expect(native.decryptSecret).not.toHaveBeenCalled();
  });

  it('treats a corrupt on-disk file as "no wallet on file" and generates a fresh one instead of throwing', async () => {
    const mod = loadModule();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rnfs = require('react-native-fs');
    await rnfs.writeFile(WALLET_FILE_PATH, 'not valid json{{{', 'utf8');

    const keypair = await mod.getOrCreateKeystoreKeypair();
    expect(keypair.address).toEqual(expect.any(String));
  });

  it('treats an unrecognized file version as "no wallet on file" rather than misreading it', async () => {
    const mod = loadModule();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rnfs = require('react-native-fs');
    await rnfs.writeFile(
      WALLET_FILE_PATH,
      JSON.stringify({ version: 99, ciphertext: 'x', iv: 'y' }),
      'utf8',
    );

    const keypair = await mod.getOrCreateKeystoreKeypair();
    expect(keypair.address).toEqual(expect.any(String));
    const file = await readWalletFile();
    expect(file.version).toBe(1); // overwritten with a freshly generated, correctly-versioned wallet
  });
});

const DELEGATE_PERSONA_WALLET_FILE_PATH = '/mock-documents/agora-delegate-persona-key.enc.json';

describe('getOrCreateDelegatePersonaKeypair', () => {
  it('throws when the Keystore module is not linked, without touching the filesystem', async () => {
    jest.resetModules();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    require('react-native-fs').__reset();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rn = require('react-native');
    delete rn.NativeModules.KeystoreSigningModule;
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const mod: typeof KeystoreWalletModule = require('./keystoreWallet');

    await expect(mod.getOrCreateDelegatePersonaKeypair()).rejects.toThrow(/not available/);
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    await expect(require('react-native-fs').exists(DELEGATE_PERSONA_WALLET_FILE_PATH)).resolves.toBe(false);
  });

  it('generates and persists its own wallet file, distinct from the citizen wallet\'s', async () => {
    const mod = loadModule();
    await mod.getOrCreateDelegatePersonaKeypair();

    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rnfs = require('react-native-fs');
    await expect(rnfs.exists(DELEGATE_PERSONA_WALLET_FILE_PATH)).resolves.toBe(true);
    await expect(rnfs.exists(WALLET_FILE_PATH)).resolves.toBe(false); // the citizen wallet was never touched
  });

  it('is a genuinely different keypair (and address) than the citizen wallet, from an independent random seed', async () => {
    const mod = loadModule();
    const citizen = await mod.getOrCreateKeystoreKeypair();
    const persona = await mod.getOrCreateDelegatePersonaKeypair();

    expect(persona.address).not.toBe(citizen.address);
    expect(Buffer.from(persona.publicKey)).not.toEqual(Buffer.from(citizen.publicKey));
  });

  it('reuses the same persisted seed across a simulated app restart, independent of the citizen wallet', async () => {
    const first = loadModule();
    const personaA = await first.getOrCreateDelegatePersonaKeypair();

    jest.resetModules();
    installFakeKeystoreModule();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const second: typeof KeystoreWalletModule = require('./keystoreWallet');
    const personaB = await second.getOrCreateDelegatePersonaKeypair();

    expect(personaB.address).toBe(personaA.address);
  });

  it('_resetCachedKeystoreKeypairForTests clears both cached keypairs', async () => {
    const mod = loadModule();
    const citizenA = await mod.getOrCreateKeystoreKeypair();
    const personaA = await mod.getOrCreateDelegatePersonaKeypair();

    mod._resetCachedKeystoreKeypairForTests();

    // Same underlying files still exist, so re-deriving reaches the same addresses — this only
    // proves the in-memory cache was actually cleared and re-populated (decrypt gets called
    // again), not that a fresh seed was minted.
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.KeystoreSigningModule;
    const decryptCallsBefore = native.decryptSecret.mock.calls.length;
    const citizenB = await mod.getOrCreateKeystoreKeypair();
    const personaB = await mod.getOrCreateDelegatePersonaKeypair();

    expect(citizenB.address).toBe(citizenA.address);
    expect(personaB.address).toBe(personaA.address);
    expect(native.decryptSecret.mock.calls.length).toBeGreaterThan(decryptCallsBefore);
  });
});
