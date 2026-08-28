/**
 * Tests `faceMatch.ts`'s bridge to the native `FaceCaptureModule`/
 * `FaceMatchModule` against fake `NativeModules` entries (the manual
 * `__mocks__/react-native.js` mock), mirroring `keystoreSigner.test.ts`'s
 * `loadModule({linked, os})` approach — see that file's doc comment for why
 * the module-under-test must be freshly `require`d after the fake native
 * modules are in place (`faceMatch.ts` captures `NativeModules.X` into a
 * module-scope `const` at import time, same as `nfcPassportReader.ts`/
 * `keystoreSigner.ts`).
 *
 * What's covered: availability reflects platform + *both* modules being
 * linked (not just one), base64 encoding of the `dg2` bytes passed to
 * `matchAgainstPassport`, and every export throwing when unavailable. What's
 * NOT covered, honestly: no real CameraX session, no real ML Kit/TFLite
 * inference, no real device — see `FaceCaptureModule.kt`/`FaceMatchModule.kt`'s
 * doc comments for what remains unverified.
 */
import type * as FaceMatchModuleFile from './faceMatch';

function base64Of(data: Uint8Array): string {
  return Buffer.from(data).toString('base64');
}

function fakeCaptureModule() {
  return {
    hasCameraPermission: jest.fn(async () => true),
    requestCameraPermission: jest.fn(async () => true),
    capturePhoto: jest.fn(async (challenge: string) => ({
      uri: `file:///fake/${challenge}.jpg`,
      leftEyeOpenProbability: 0.9,
      rightEyeOpenProbability: 0.9,
      headEulerAngleY: 2.5,
    })),
    captureFaceAndQr: jest.fn(async () => ({
      uri: 'file:///fake/qrcombined.jpg',
      leftEyeOpenProbability: 0.9,
      rightEyeOpenProbability: 0.9,
      headEulerAngleY: 2.5,
      qrText: 'agora-liveness-v1:0123456789abcdef0123456789abcdef',
    })),
    deleteCaptureFile: jest.fn(async (_uri: string) => undefined),
  };
}

function fakeMatchModule() {
  return {
    matchAgainstPassport: jest.fn(async (_dg2Base64: string, dg2MimeType: string, _capturedUri: string) => {
      if (dg2MimeType !== 'image/jpeg') {
        return { matched: false, skipped: true, reason: `unsupported DG2 image format: ${dg2MimeType}` };
      }
      return { matched: true, skipped: false, score: 0.71 };
    }),
  };
}

/**
 * Sets up `NativeModules`/`Platform` for the mock 'react-native' module,
 * resets the module registry, and re-requires `faceMatch.ts` fresh so its
 * module-scope native-module captures see this state.
 */
function loadModule(opts: {
  captureLinked: boolean;
  matchLinked: boolean;
  os?: string;
}): typeof FaceMatchModuleFile {
  jest.resetModules();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rn = require('react-native');
  rn.Platform.OS = opts.os ?? 'android';
  if (opts.captureLinked) {
    rn.NativeModules.FaceCaptureModule = fakeCaptureModule();
  } else {
    delete rn.NativeModules.FaceCaptureModule;
  }
  if (opts.matchLinked) {
    rn.NativeModules.FaceMatchModule = fakeMatchModule();
  } else {
    delete rn.NativeModules.FaceMatchModule;
  }
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require('./faceMatch');
}

describe('isFaceMatchAvailable', () => {
  it('is false when neither native module is linked', () => {
    const mod = loadModule({ captureLinked: false, matchLinked: false });
    expect(mod.isFaceMatchAvailable()).toBe(false);
  });

  it('is false when only one of the two modules is linked', () => {
    expect(loadModule({ captureLinked: true, matchLinked: false }).isFaceMatchAvailable()).toBe(false);
    expect(loadModule({ captureLinked: false, matchLinked: true }).isFaceMatchAvailable()).toBe(false);
  });

  it('is true only on android with both modules linked', () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true, os: 'android' });
    expect(mod.isFaceMatchAvailable()).toBe(true);
  });

  it('is false on ios even if both are registered under the same names', () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true, os: 'ios' });
    expect(mod.isFaceMatchAvailable()).toBe(false);
  });
});

describe('capturePhoto', () => {
  it('throws instead of calling the native module when unavailable', async () => {
    const mod = loadModule({ captureLinked: false, matchLinked: false });
    await expect(mod.capturePhoto('baseline')).rejects.toThrow(/not available/);
  });

  it('passes the challenge label through and returns the native result verbatim', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    const result = await mod.capturePhoto('blink');
    expect(result.uri).toBe('file:///fake/blink.jpg');
    expect(result.headEulerAngleY).toBe(2.5);
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.FaceCaptureModule;
    expect(native.capturePhoto).toHaveBeenCalledWith('blink');
  });
});

describe('captureFaceAndQr', () => {
  it('throws instead of calling the native module when unavailable', async () => {
    const mod = loadModule({ captureLinked: false, matchLinked: false });
    await expect(mod.captureFaceAndQr()).rejects.toThrow(/not available/);
  });

  it('returns the native result verbatim, including the decoded QR text', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    const result = await mod.captureFaceAndQr();
    expect(result).toEqual({
      uri: 'file:///fake/qrcombined.jpg',
      leftEyeOpenProbability: 0.9,
      rightEyeOpenProbability: 0.9,
      headEulerAngleY: 2.5,
      qrText: 'agora-liveness-v1:0123456789abcdef0123456789abcdef',
    });
  });

  it('surfaces a null qrText when the native side found no code, without throwing', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.FaceCaptureModule;
    native.captureFaceAndQr.mockResolvedValueOnce({
      uri: 'file:///fake/qrcombined-2.jpg',
      leftEyeOpenProbability: -1,
      rightEyeOpenProbability: -1,
      headEulerAngleY: 0,
      qrText: null,
    });
    const result = await mod.captureFaceAndQr();
    expect(result.qrText).toBeNull();
    expect(result.leftEyeOpenProbability).toBe(-1);
  });

  it('propagates a native rejection (e.g. camera not bound yet)', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.FaceCaptureModule;
    native.captureFaceAndQr.mockRejectedValueOnce(new Error('CAMERA_NOT_READY'));
    await expect(mod.captureFaceAndQr()).rejects.toThrow('CAMERA_NOT_READY');
  });
});

describe('deleteCaptureFile', () => {
  it('does not throw when unavailable — resolves silently instead', async () => {
    const mod = loadModule({ captureLinked: false, matchLinked: false });
    await expect(mod.deleteCaptureFile('file:///fake/baseline.jpg')).resolves.toBeUndefined();
  });

  it('passes the uri through to the native module when available', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    await mod.deleteCaptureFile('file:///fake/baseline-123.jpg');
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.FaceCaptureModule;
    expect(native.deleteCaptureFile).toHaveBeenCalledWith('file:///fake/baseline-123.jpg');
  });

  it('swallows a native-side rejection rather than throwing', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.FaceCaptureModule;
    native.deleteCaptureFile.mockRejectedValueOnce(new Error('delete failed'));
    await expect(mod.deleteCaptureFile('file:///fake/baseline-123.jpg')).resolves.toBeUndefined();
  });
});

describe('matchAgainstPassport', () => {
  it('throws instead of calling the native module when unavailable', async () => {
    const mod = loadModule({ captureLinked: false, matchLinked: false });
    await expect(mod.matchAgainstPassport(new Uint8Array([1, 2, 3]), 'image/jpeg', 'file:///x.jpg')).rejects.toThrow(
      /not available/,
    );
  });

  it('base64-encodes the dg2 bytes before calling the native module', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    const dg2 = new Uint8Array([9, 8, 7, 6]);
    await mod.matchAgainstPassport(dg2, 'image/jpeg', 'file:///selfie.jpg');
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const native = require('react-native').NativeModules.FaceMatchModule;
    expect(native.matchAgainstPassport).toHaveBeenCalledWith(base64Of(dg2), 'image/jpeg', 'file:///selfie.jpg');
  });

  it('surfaces a skipped result (e.g. unsupported DG2 format) rather than throwing', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    const result = await mod.matchAgainstPassport(new Uint8Array([1]), 'image/jp2', 'file:///selfie.jpg');
    expect(result).toEqual({ matched: false, skipped: true, reason: 'unsupported DG2 image format: image/jp2' });
  });

  it('returns a real match verdict for a decodable format', async () => {
    const mod = loadModule({ captureLinked: true, matchLinked: true });
    const result = await mod.matchAgainstPassport(new Uint8Array([1]), 'image/jpeg', 'file:///selfie.jpg');
    expect(result).toEqual({ matched: true, skipped: false, score: 0.71 });
  });
});
