/**
 * Tests `keystoreSigner.ts`'s bridge to the native `KeystoreSigningModule`
 * against a fake `NativeModules.KeystoreSigningModule` (the manual
 * `__mocks__/react-native.js` mock — see that file's doc comment for why the
 * real 'react-native' package can't load under this repo's Jest config at
 * all). Mirrors the "exercise the JS-side encode/decode contract without a
 * real native module" approach `identity.test.ts` already uses for the
 * chain `api` (a fake `ApiPromise`) — here it's a fake native bridge module.
 *
 * `keystoreSigner.ts` reads `NativeModules.KeystoreSigningModule` into a
 * module-scope `const` at import time — the same pattern
 * `native/nfcPassportReader.ts` already uses, and a reasonable one in the
 * real app (native modules are registered before any JS runs). That means a
 * test can't just mutate `NativeModules.KeystoreSigningModule` *after*
 * importing this module and expect it to notice — the module under test
 * must be freshly `require`d (via `jest.resetModules()`) *after* the fake
 * native module is in place, mirroring the real load order. `loadModule()`
 * below does exactly that.
 *
 * What's covered: base64<->Uint8Array conversion round-trips correctly in
 * both directions, `isKeystoreSigningAvailable()` reflects platform + module
 * presence, and every export throws/returns a safe default when the module
 * isn't there. What's NOT covered, honestly: no real Android Keystore, no
 * real AES-GCM, no real device — see `KeystoreSigningModule.kt`'s doc
 * comment and this task's final report for what remains unverified.
 */
import type * as KeystoreSignerModule from './keystoreSigner';

function bytes(...values: number[]): Uint8Array {
  return new Uint8Array(values);
}

function base64Of(data: Uint8Array): string {
  return Buffer.from(data).toString('base64');
}

/** A fake native module: "encrypts" by reversing byte order, "decrypts" by reversing back — enough to prove the JS wrapper threads bytes through correctly, not real crypto. */
function fakeNativeModule() {
  return {
    encryptSecret: jest.fn(async (plaintextB64: string) => {
      const plaintext = Buffer.from(plaintextB64, 'base64');
      const ciphertext = Buffer.from([...plaintext].reverse());
      return { ciphertext: ciphertext.toString('base64'), iv: base64Of(bytes(9, 9, 9)) };
    }),
    decryptSecret: jest.fn(async (ciphertextB64: string, ivB64: string) => {
      if (ivB64 !== base64Of(bytes(9, 9, 9))) {
        throw new Error('bad iv');
      }
      const ciphertext = Buffer.from(ciphertextB64, 'base64');
      const plaintext = Buffer.from([...ciphertext].reverse());
      return plaintext.toString('base64');
    }),
    isHardwareBacked: jest.fn(async () => true),
  };
}

/**
 * Sets up `NativeModules`/`Platform` for the mock 'react-native' module,
 * resets the module registry, and re-requires `keystoreSigner.ts` fresh so
 * its module-scope `const KeystoreSigningModule = NativeModules...` capture
 * sees this state — see module doc comment above for why that ordering
 * matters.
 */
function loadModule(opts: { linked: boolean; os?: string }): typeof KeystoreSignerModule {
  jest.resetModules();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rn = require('react-native');
  rn.Platform.OS = opts.os ?? 'android';
  if (opts.linked) {
    rn.NativeModules.KeystoreSigningModule = fakeNativeModule();
  } else {
    delete rn.NativeModules.KeystoreSigningModule;
  }
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require('./keystoreSigner');
}

describe('isKeystoreSigningAvailable', () => {
  it('is false when no native module is linked', () => {
    const mod = loadModule({ linked: false });
    expect(mod.isKeystoreSigningAvailable()).toBe(false);
  });

  it('is true only on android with the module linked', () => {
    const mod = loadModule({ linked: true, os: 'android' });
    expect(mod.isKeystoreSigningAvailable()).toBe(true);
  });

  it('is false on ios even if something is registered under the same name', () => {
    const mod = loadModule({ linked: true, os: 'ios' });
    expect(mod.isKeystoreSigningAvailable()).toBe(false);
  });
});

describe('encryptSecret / decryptSecret', () => {
  it('round-trips arbitrary bytes through encrypt then decrypt', async () => {
    const mod = loadModule({ linked: true });
    const plaintext = bytes(1, 2, 3, 250, 0, 255, 128);
    const { ciphertext, iv } = await mod.encryptSecret(plaintext);
    const roundTripped = await mod.decryptSecret(ciphertext, iv);
    expect(roundTripped).toEqual(plaintext);
  });

  it('passes plaintext to the native module as base64, not raw bytes', async () => {
    const mod = loadModule({ linked: true });
    const plaintext = bytes(0, 1, 2, 3);
    await mod.encryptSecret(plaintext);
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.KeystoreSigningModule;
    expect(native.encryptSecret).toHaveBeenCalledWith(base64Of(plaintext));
  });

  it('throws instead of calling the native module when unavailable', async () => {
    const mod = loadModule({ linked: false });
    await expect(mod.encryptSecret(bytes(1))).rejects.toThrow(/not available/);
  });

  it('surfaces a native decrypt rejection (e.g. tampered ciphertext) rather than swallowing it', async () => {
    const mod = loadModule({ linked: true });
    const plaintext = bytes(5, 6, 7);
    const { ciphertext } = await mod.encryptSecret(plaintext);
    await expect(mod.decryptSecret(ciphertext, bytes(0, 0, 0))).rejects.toThrow('bad iv');
  });
});

describe('isHardwareBacked', () => {
  it('returns false (not a throw) when the native module is unavailable', async () => {
    const mod = loadModule({ linked: false });
    await expect(mod.isHardwareBacked()).resolves.toBe(false);
  });

  it('delegates to the native module when available', async () => {
    const mod = loadModule({ linked: true });
    await expect(mod.isHardwareBacked()).resolves.toBe(true);
  });
});
