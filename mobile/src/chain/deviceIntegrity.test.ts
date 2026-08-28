/**
 * Covers `deviceIntegrity.ts`: nonce generation/encoding (pure) and
 * `captureDeviceIntegritySignal`'s best-effort behavior against a mocked
 * `../native/playIntegrity` bridge — see that file's `playIntegrity.test.ts`
 * for the native-bridge layer itself, and `faceMatch.test.ts` for the
 * `loadModule`-style re-require pattern this borrows for the native mock.
 */
import type * as DeviceIntegrityFile from './deviceIntegrity';

function loadModule(opts: { linked: boolean }): typeof DeviceIntegrityFile {
  jest.resetModules();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rn = require('react-native');
  rn.Platform.OS = 'android';
  if (opts.linked) {
    rn.NativeModules.PlayIntegrityModule = {
      requestIntegrityToken: jest.fn(async (_nonceBase64: string) => 'fake-opaque-integrity-token'),
    };
  } else {
    delete rn.NativeModules.PlayIntegrityModule;
  }
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require('./deviceIntegrity');
}

describe('generateDeviceIntegrityNonce', () => {
  it('generates DEVICE_INTEGRITY_NONCE_BYTES of entropy', () => {
    const mod = loadModule({ linked: true });
    const nonce = mod.generateDeviceIntegrityNonce();
    expect(nonce).toHaveLength(mod.DEVICE_INTEGRITY_NONCE_BYTES);
  });

  it('generates different nonces across calls', () => {
    const mod = loadModule({ linked: true });
    expect(mod.generateDeviceIntegrityNonce()).not.toEqual(mod.generateDeviceIntegrityNonce());
  });
});

describe('nonceToBase64Url', () => {
  it('produces URL-safe base64 with no padding', () => {
    const mod = loadModule({ linked: true });
    // Bytes chosen so the plain-base64 encoding would contain '+', '/', and '=' padding.
    const bytes = new Uint8Array([0xfb, 0xff, 0xbf, 0xff, 0xfe]);
    const plainBase64 = Buffer.from(bytes).toString('base64');
    expect(plainBase64).toContain('+');
    expect(plainBase64).toContain('/');
    const url = mod.nonceToBase64Url(bytes);
    expect(url).not.toContain('+');
    expect(url).not.toContain('/');
    expect(url).not.toContain('=');
  });

  it('is at least 16 chars once encoded for a 32-byte nonce (Play Integrity minimum)', () => {
    const mod = loadModule({ linked: true });
    const nonce = mod.generateDeviceIntegrityNonce();
    expect(mod.nonceToBase64Url(nonce).length).toBeGreaterThanOrEqual(16);
  });
});

describe('captureDeviceIntegritySignal', () => {
  it('never throws — resolves captured:false when the native module is unavailable', async () => {
    const mod = loadModule({ linked: false });
    const result = await mod.captureDeviceIntegritySignal();
    expect(result.captured).toBe(false);
  });

  it('resolves captured:true with the token and nonce when available', async () => {
    const mod = loadModule({ linked: true });
    const nonce = new Uint8Array(32).fill(7);
    const result = await mod.captureDeviceIntegritySignal(nonce);
    expect(result.captured).toBe(true);
    if (result.captured) {
      expect(result.signal.token).toBe('fake-opaque-integrity-token');
      expect(result.signal.nonceBase64).toBe(mod.nonceToBase64Url(nonce));
      expect(typeof result.signal.requestedAtMs).toBe('number');
    }
  });

  it('never throws — resolves captured:false when the native call rejects', async () => {
    const mod = loadModule({ linked: true });
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.PlayIntegrityModule;
    native.requestIntegrityToken.mockRejectedValueOnce(new Error('no Play Services'));
    const result = await mod.captureDeviceIntegritySignal();
    expect(result.captured).toBe(false);
    if (!result.captured) {
      expect(result.reason).toBe('no Play Services');
    }
  });

  it('never throws — resolves captured:false rather than hanging when the native call never settles', async () => {
    jest.useFakeTimers();
    try {
      const mod = loadModule({ linked: true });
      // eslint-disable-next-line @typescript-eslint/no-var-requires
      const native = require('react-native').NativeModules.PlayIntegrityModule;
      native.requestIntegrityToken.mockImplementationOnce(() => new Promise(() => {}));
      const resultPromise = mod.captureDeviceIntegritySignal();
      await jest.advanceTimersByTimeAsync(mod.DEVICE_INTEGRITY_TIMEOUT_MS);
      const result = await resultPromise;
      expect(result.captured).toBe(false);
      if (!result.captured) {
        expect(result.reason).toMatch(/timed out/);
      }
    } finally {
      jest.useRealTimers();
    }
  });
});
