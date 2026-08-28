/**
 * Tests `playIntegrity.ts`'s bridge to the native `PlayIntegrityModule`
 * against fake `NativeModules` entries — same `loadModule({linked, os})`
 * re-require pattern as `faceMatch.test.ts`/`qrChallenge.test.ts`.
 */
import type * as PlayIntegrityModuleFile from './playIntegrity';

function fakePlayIntegrityModule() {
  return {
    requestIntegrityToken: jest.fn(async (_nonceBase64: string) => 'fake-opaque-integrity-token'),
  };
}

function loadModule(opts: { linked: boolean; os?: string }): typeof PlayIntegrityModuleFile {
  jest.resetModules();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rn = require('react-native');
  rn.Platform.OS = opts.os ?? 'android';
  if (opts.linked) {
    rn.NativeModules.PlayIntegrityModule = fakePlayIntegrityModule();
  } else {
    delete rn.NativeModules.PlayIntegrityModule;
  }
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require('./playIntegrity');
}

describe('isPlayIntegrityAvailable', () => {
  it('is false when the native module is not linked', () => {
    expect(loadModule({ linked: false }).isPlayIntegrityAvailable()).toBe(false);
  });

  it('is true only on android with the module linked', () => {
    expect(loadModule({ linked: true, os: 'android' }).isPlayIntegrityAvailable()).toBe(true);
  });

  it('is false on ios even if the module is registered under the same name', () => {
    expect(loadModule({ linked: true, os: 'ios' }).isPlayIntegrityAvailable()).toBe(false);
  });
});

describe('requestIntegrityToken', () => {
  it('throws instead of calling the native module when unavailable', async () => {
    const mod = loadModule({ linked: false });
    await expect(mod.requestIntegrityToken('abc123')).rejects.toThrow(/not available/);
  });

  it('passes the nonce through and returns the token verbatim', async () => {
    const mod = loadModule({ linked: true });
    const token = await mod.requestIntegrityToken('deadbeef');
    expect(token).toBe('fake-opaque-integrity-token');
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.PlayIntegrityModule;
    expect(native.requestIntegrityToken).toHaveBeenCalledWith('deadbeef');
  });

  it('propagates a native rejection (e.g. Play Services unavailable)', async () => {
    const mod = loadModule({ linked: true });
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.PlayIntegrityModule;
    native.requestIntegrityToken.mockRejectedValueOnce(new Error('PLAY_INTEGRITY_ERROR'));
    await expect(mod.requestIntegrityToken('deadbeef')).rejects.toThrow('PLAY_INTEGRITY_ERROR');
  });
});
