/**
 * Tests `registrationState.ts` against the manual `__mocks__/react-native-fs.js`
 * mock (see that file's doc comment for why `react-native` alone can't be
 * mocked and `react-native-fs` skipped) — same approach as
 * `keystoreWallet.test.ts`, including the "restart" pattern of
 * `jest.resetModules()` + re-require while deliberately NOT resetting the
 * fake filesystem's backing `Map` in between, so the persisted file really
 * does survive the simulated restart the same way a real one would.
 *
 * What's covered: no file on disk reads back `null`; a write/read round-trip;
 * survival across a simulated app restart; two addresses coexisting
 * independently in the same file, with `clearRegistrationStatus` on one
 * leaving the other untouched; a corrupt file treated as absent rather than
 * thrown; `queryId` round-tripping as an exact decimal string with no
 * `Number()` coercion; and the write serializer not losing an update when two
 * writes for different addresses race.
 */
import type * as RegistrationStateModule from './registrationState';

const STATE_FILE_PATH = '/mock-documents/agora-registration-status.json';

function loadModule(): typeof RegistrationStateModule {
  jest.resetModules();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  require('react-native-fs').__reset();
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  return require('./registrationState');
}

/** Reads back whatever `registrationState.ts` actually wrote to the fake filesystem. */
async function readStateFile(): Promise<any> {
  // eslint-disable-next-line @typescript-eslint/no-var-requires
  const rnfs = require('react-native-fs');
  const raw = await rnfs.readFile(STATE_FILE_PATH, 'utf8');
  return JSON.parse(raw);
}

describe('readRegistrationStatus', () => {
  it('returns null when no file exists on disk', async () => {
    const mod = loadModule();
    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toBeNull();
  });

  it('returns null for an address with no entry, even when the file exists for another address', async () => {
    const mod = loadModule();
    await mod.writeRegistrationStatus('5Addr1', { stage: 'PassportScanned' });
    await expect(mod.readRegistrationStatus('5Addr2')).resolves.toBeNull();
  });
});

describe('writeRegistrationStatus / readRegistrationStatus round-trip', () => {
  it('round-trips a PassportScanned record', async () => {
    const mod = loadModule();
    await mod.writeRegistrationStatus('5Addr1', { stage: 'PassportScanned' });
    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toEqual({ stage: 'PassportScanned' });
  });

  it('round-trips a LivenessVerified record', async () => {
    const mod = loadModule();
    const status: RegistrationStateModule.PersistableStatus = {
      stage: 'LivenessVerified',
      faceMatched: true,
    };
    await mod.writeRegistrationStatus('5Addr1', status);
    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toEqual(status);
  });

  it('round-trips a LivenessVerified record with a skipped-match reason', async () => {
    const mod = loadModule();
    const status: RegistrationStateModule.PersistableStatus = {
      stage: 'LivenessVerified',
      faceMatched: false,
      matchSkippedReason: 'unsupported DG2 image format: image/jp2',
    };
    await mod.writeRegistrationStatus('5Addr1', status);
    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toEqual(status);
  });

  it('round-trips a record with OPRF tracking fields', async () => {
    const mod = loadModule();
    const status: RegistrationStateModule.PersistableStatus = {
      stage: 'AwaitingCommitteeRound1',
      queryId: '12345',
      committeeSlots: [0, 1, 2, 3, 4],
      submittedAtBlock: 100,
      slaExpiresAtBlock: 250,
    };
    await mod.writeRegistrationStatus('5Addr1', status);
    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toEqual(status);
  });

  it('persists version 1 and the correct on-disk shape', async () => {
    const mod = loadModule();
    await mod.writeRegistrationStatus('5Addr1', { stage: 'ProofReady' });
    const file = await readStateFile();
    expect(file.version).toBe(1);
    expect(file.byAddress['5Addr1'].status).toEqual({ stage: 'ProofReady' });
    expect(typeof file.byAddress['5Addr1'].updatedAtMs).toBe('number');
  });
});

describe('survival across a simulated app restart', () => {
  it('reads back a record written before a jest.resetModules() restart, without resetting the fake filesystem', async () => {
    const first = loadModule();
    await first.writeRegistrationStatus('5Addr1', { stage: 'ProofMaterialAssembled' });

    // Simulate an app restart: reset the module registry but deliberately do
    // NOT call the mock's __reset() — the persisted file must still be there,
    // exactly as a real restart would clear JS memory but not disk.
    jest.resetModules();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const second: typeof RegistrationStateModule = require('./registrationState');

    await expect(second.readRegistrationStatus('5Addr1')).resolves.toEqual({
      stage: 'ProofMaterialAssembled',
    });
  });
});

describe('multiple addresses coexisting', () => {
  it('keeps two different addresses\' records independent in the same file', async () => {
    const mod = loadModule();
    await mod.writeRegistrationStatus('5Addr1', { stage: 'PassportScanned' });
    await mod.writeRegistrationStatus('5Addr2', { stage: 'ProofReady' });

    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toEqual({ stage: 'PassportScanned' });
    await expect(mod.readRegistrationStatus('5Addr2')).resolves.toEqual({ stage: 'ProofReady' });
  });

  it('clearRegistrationStatus on one address leaves the other untouched', async () => {
    const mod = loadModule();
    await mod.writeRegistrationStatus('5Addr1', { stage: 'PassportScanned' });
    await mod.writeRegistrationStatus('5Addr2', { stage: 'ProofReady' });

    await mod.clearRegistrationStatus('5Addr1');

    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toBeNull();
    await expect(mod.readRegistrationStatus('5Addr2')).resolves.toEqual({ stage: 'ProofReady' });
  });
});

describe('corrupt file handling', () => {
  it('treats a corrupt/garbage on-disk file as absent, returning null rather than throwing', async () => {
    const mod = loadModule();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rnfs = require('react-native-fs');
    await rnfs.writeFile(STATE_FILE_PATH, 'not valid json{{{', 'utf8');

    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toBeNull();
  });

  it('treats an unrecognized file version as absent rather than misreading it', async () => {
    const mod = loadModule();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rnfs = require('react-native-fs');
    await rnfs.writeFile(
      STATE_FILE_PATH,
      JSON.stringify({ version: 99, byAddress: { '5Addr1': { status: { stage: 'ProofReady' }, updatedAtMs: 1 } } }),
      'utf8',
    );

    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toBeNull();
  });

  it('overwrites a corrupt file cleanly on the next write', async () => {
    const mod = loadModule();
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rnfs = require('react-native-fs');
    await rnfs.writeFile(STATE_FILE_PATH, 'garbage', 'utf8');

    await mod.writeRegistrationStatus('5Addr1', { stage: 'PassportScanned' });
    const file = await readStateFile();
    expect(file.version).toBe(1);
    expect(file.byAddress['5Addr1'].status).toEqual({ stage: 'PassportScanned' });
  });
});

describe('queryId precision', () => {
  it('round-trips a u64-range queryId as an exact decimal string, never coerced through Number()', async () => {
    const mod = loadModule();
    // 2^64 - 1 — well past Number.MAX_SAFE_INTEGER (2^53 - 1); if this were
    // ever coerced through Number() and back to a string, it would come back
    // rounded, not as this exact literal.
    const bigQueryId = '18446744073709551615';
    const status: RegistrationStateModule.PersistableStatus = {
      stage: 'OprfQuerySubmitted',
      queryId: bigQueryId,
      committeeSlots: [0, 1, 2, 3, 4],
      submittedAtBlock: 1,
      slaExpiresAtBlock: 2,
    };
    await mod.writeRegistrationStatus('5Addr1', status);

    const read = await mod.readRegistrationStatus('5Addr1');
    expect(read).not.toBeNull();
    if (read && read.stage === 'OprfQuerySubmitted') {
      expect(read.queryId).toBe(bigQueryId);
      expect(typeof read.queryId).toBe('string');
    } else {
      throw new Error('expected OprfQuerySubmitted stage');
    }
  });
});

describe('write serialization', () => {
  it('lands both updates when two concurrent writes for different addresses race', async () => {
    const mod = loadModule();
    await Promise.all([
      mod.writeRegistrationStatus('5Addr1', { stage: 'PassportScanned' }),
      mod.writeRegistrationStatus('5Addr2', { stage: 'ProofReady' }),
    ]);

    await expect(mod.readRegistrationStatus('5Addr1')).resolves.toEqual({ stage: 'PassportScanned' });
    await expect(mod.readRegistrationStatus('5Addr2')).resolves.toEqual({ stage: 'ProofReady' });

    const file = await readStateFile();
    expect(Object.keys(file.byAddress).sort()).toEqual(['5Addr1', '5Addr2']);
  });
});
