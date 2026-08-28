/**
 * Tests `qrChallenge.ts`'s bridge to the native `QrChallengeModule` against
 * fake `NativeModules` entries — same `loadModule({linked, os})` re-require
 * pattern as `faceMatch.test.ts` (see that file's doc comment for why).
 */
import type * as QrChallengeModuleFile from './qrChallenge';

function fakeQrChallengeModule() {
  return {
    captureAndDecodeQrCode: jest.fn(async () => 'agora-liveness-v1:0123456789abcdef0123456789abcdef'),
  };
}

function loadModule(opts: { linked: boolean; os?: string }): typeof QrChallengeModuleFile {
  jest.resetModules();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rn = require('react-native');
  rn.Platform.OS = opts.os ?? 'android';
  if (opts.linked) {
    rn.NativeModules.QrChallengeModule = fakeQrChallengeModule();
  } else {
    delete rn.NativeModules.QrChallengeModule;
  }
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require('./qrChallenge');
}

describe('isQrChallengeScanAvailable', () => {
  it('is false when the native module is not linked', () => {
    expect(loadModule({ linked: false }).isQrChallengeScanAvailable()).toBe(false);
  });

  it('is true only on android with the module linked', () => {
    expect(loadModule({ linked: true, os: 'android' }).isQrChallengeScanAvailable()).toBe(true);
  });

  it('is false on ios even if the module is registered under the same name', () => {
    expect(loadModule({ linked: true, os: 'ios' }).isQrChallengeScanAvailable()).toBe(false);
  });
});

describe('captureAndDecodeQrCode', () => {
  it('throws instead of calling the native module when unavailable', async () => {
    const mod = loadModule({ linked: false });
    await expect(mod.captureAndDecodeQrCode()).rejects.toThrow(/not available/);
  });

  it('returns the native result verbatim when available', async () => {
    const mod = loadModule({ linked: true });
    const text = await mod.captureAndDecodeQrCode();
    expect(text).toBe('agora-liveness-v1:0123456789abcdef0123456789abcdef');
  });

  it('propagates a native rejection (e.g. camera not bound yet)', async () => {
    const mod = loadModule({ linked: true });
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.QrChallengeModule;
    native.captureAndDecodeQrCode.mockRejectedValueOnce(new Error('CAMERA_NOT_READY'));
    await expect(mod.captureAndDecodeQrCode()).rejects.toThrow('CAMERA_NOT_READY');
  });
});
