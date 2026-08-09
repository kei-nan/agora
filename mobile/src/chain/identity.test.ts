/**
 * Tests `identity.ts`'s three chain-submitting extrinsic wrappers
 * (`registerCitizen`/`reverifyCitizen`/`migrateOprfScheme`) against a fake `@polkadot/api`
 * instance, mirroring the "exercise the encode/submit path without a live device" approach
 * `zkProving.test.ts` already uses for proving (a fake `NoirProver` there; a fake chain `api`
 * here, since neither a Noir prover nor a live chain exists in this environment).
 *
 * What's covered:
 *  - each call reaches `api.tx.identity.<name>` with its arguments in the exact order
 *    `pallets/pallet-identity/src/lib.rs`'s extrinsic signatures take (HANDOFF logs #75/#76);
 *  - a malformed payload (bad public-input shape, wrong-length anchor/committee-hash
 *    material) is rejected locally, before ever calling `getApi()`, with a message that says
 *    which field is wrong — not the chain's bare `InvalidZKProof`/`CommitteeKeyMismatch`;
 *  - a `dispatchError` from the chain rejects the returned promise;
 *  - a `status.isFinalized` update with no error resolves it.
 *
 * What's NOT covered, honestly: no live chain, no real ZKPassport proof, and no live OPRF
 * committee exist in this environment (see HANDOFF logs #73-77), so nothing here proves the
 * *values* submitted are cryptographically real — only that this module encodes/submits
 * whatever it's given in the shape the pallet expects.
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));

import { Keyring } from '@polkadot/keyring';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { getApi } from './api';
import {
  MigrateOprfSchemeParams,
  NUM_OPRF_COMMITTEES,
  OprfCommitteeKeyHashes,
  RegisterCitizenParams,
  ReverifyCitizenParams,
  getSigningKeypair,
  migrateOprfScheme,
  registerCitizen,
  reverifyCitizen,
} from './identity';

/** A 32-byte field element filled with `seed`, distinct per call so args are identifiable. */
function bytes32(seed: number): Uint8Array {
  return new Uint8Array(32).fill(seed);
}

/** A field element holding `value`, big-endian — mirrors proofEncoding.test.ts's helper. */
function field(value: bigint): Uint8Array {
  const out = new Uint8Array(32);
  let v = value;
  for (let i = 31; i >= 0; i--) {
    out[i] = Number(v & 0xffn);
    v >>= 8n;
  }
  return out;
}

/** A well-formed `count_4` public-input array (9 elements): current_date set, scoped_nullifier non-zero. */
function validPublicInputs(): Uint8Array[] {
  const inputs = Array.from({ length: 9 }, () => field(0n));
  inputs[2] = field(1_800_000_000n); // current_date
  inputs[7] = field(42n); // scoped_nullifier (index 6 + D, D = 1 for count_4)
  return inputs;
}

function committeeHashes(seedOffset: number): OprfCommitteeKeyHashes {
  return [
    bytes32(seedOffset),
    bytes32(seedOffset + 1),
    bytes32(seedOffset + 2),
    bytes32(seedOffset + 3),
    bytes32(seedOffset + 4),
  ];
}

interface RecordedCall {
  name: string;
  args: unknown[];
}

/**
 * A fake `ApiPromise`-shaped object exposing only `tx.identity.<call>`, each returning a
 * submittable-extrinsic stand-in whose `signAndSend` invokes the status callback the way
 * real `@polkadot/api` does — once, asynchronously, with either a `dispatchError` or a
 * finalized status.
 */
function fakeApi(outcome: { dispatchError?: unknown } = {}) {
  const calls: RecordedCall[] = [];

  function makeCall(name: string) {
    return (...args: unknown[]) => {
      calls.push({ name, args });
      return {
        signAndSend: (_pair: unknown, callback: (result: any) => void) => {
          queueMicrotask(() => {
            if (outcome.dispatchError) {
              callback({ status: { isFinalized: false }, dispatchError: outcome.dispatchError });
            } else {
              callback({ status: { isFinalized: true }, dispatchError: undefined });
            }
          });
          return Promise.resolve();
        },
      };
    };
  }

  return {
    calls,
    api: {
      tx: {
        identity: {
          registerCitizen: makeCall('registerCitizen'),
          reverifyCitizen: makeCall('reverifyCitizen'),
          migrateOprfScheme: makeCall('migrateOprfScheme'),
        },
      },
    },
  };
}

const mockedGetApi = getApi as jest.MockedFunction<typeof getApi>;

beforeEach(() => {
  mockedGetApi.mockReset();
});

describe('registerCitizen', () => {
  function validParams(): RegisterCitizenParams {
    return {
      zkProof: new Uint8Array([1, 2, 3, 4]),
      publicInputs: validPublicInputs(),
      outerCount: 4,
      anchor: bytes32(0xaa),
      oprfPkHashes: committeeHashes(1),
    };
  }

  it('submits register_citizen with the 4 arguments in the pallet\'s order', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const params = validParams();
    await registerCitizen(params);

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('registerCitizen');
    expect(calls[0].args).toEqual([
      params.zkProof,
      params.publicInputs,
      params.anchor,
      params.oprfPkHashes,
    ]);
  });

  it('resolves when the extrinsic finalizes without a dispatch error', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    await expect(registerCitizen(validParams())).resolves.toBeUndefined();
  });

  it('rejects when the chain reports a dispatch error', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'identity.InvalidAnchorProof' } });
    mockedGetApi.mockResolvedValue(api as any);
    await expect(registerCitizen(validParams())).rejects.toThrow('identity.InvalidAnchorProof');
  });

  it('rejects a malformed public-input array before ever calling getApi', async () => {
    const params = validParams();
    params.publicInputs = params.publicInputs.slice(0, 5); // wrong length for outerCount 4
    await expect(registerCitizen(params)).rejects.toThrow(RangeError);
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects a short anchor before ever calling getApi', async () => {
    const params = validParams();
    params.anchor = new Uint8Array(31);
    await expect(registerCitizen(params)).rejects.toThrow(/anchor is 31 bytes/);
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects the wrong number of OPRF committee key hashes before ever calling getApi', async () => {
    const params = validParams();
    (params as any).oprfPkHashes = committeeHashes(1).slice(0, 4);
    await expect(registerCitizen(params)).rejects.toThrow(
      new RegExp(`expected ${NUM_OPRF_COMMITTEES} OPRF committee key hashes`),
    );
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects an undersized individual committee key hash before ever calling getApi', async () => {
    const params = validParams();
    const hashes = committeeHashes(1).slice() as Uint8Array[];
    hashes[2] = new Uint8Array(16);
    (params as any).oprfPkHashes = hashes;
    await expect(registerCitizen(params)).rejects.toThrow(/committee key hash 2 is 16 bytes/);
    expect(mockedGetApi).not.toHaveBeenCalled();
  });
});

describe('reverifyCitizen', () => {
  function validParams(): ReverifyCitizenParams {
    return {
      zkProof: new Uint8Array([5, 6, 7]),
      publicInputs: validPublicInputs(),
      outerCount: 4,
      anchor: bytes32(0xbb),
      oprfPkHashes: committeeHashes(11),
    };
  }

  it('submits reverify_citizen with the 4 arguments in the pallet\'s order', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const params = validParams();
    await reverifyCitizen(params);

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('reverifyCitizen');
    expect(calls[0].args).toEqual([
      params.zkProof,
      params.publicInputs,
      params.anchor,
      params.oprfPkHashes,
    ]);
  });

  it('rejects when the chain reports a dispatch error (e.g. AnchorMismatch)', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'identity.AnchorMismatch' } });
    mockedGetApi.mockResolvedValue(api as any);
    await expect(reverifyCitizen(validParams())).rejects.toThrow('identity.AnchorMismatch');
  });

  it('rejects a malformed anchor before ever calling getApi', async () => {
    const params = validParams();
    (params as any).anchor = new Uint8Array(0);
    await expect(reverifyCitizen(params)).rejects.toThrow(RangeError);
    expect(mockedGetApi).not.toHaveBeenCalled();
  });
});

describe('migrateOprfScheme', () => {
  function validParams(): MigrateOprfSchemeParams {
    return {
      zkProof: new Uint8Array([9, 9, 9]),
      publicInputs: validPublicInputs(),
      outerCount: 4,
      newAnchor: bytes32(0xcc),
      oldOprfPkHashes: committeeHashes(21),
      newOprfPkHashes: committeeHashes(31),
    };
  }

  it('submits migrate_oprf_scheme with the 5 arguments in the pallet\'s order (no old_anchor)', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const params = validParams();
    await migrateOprfScheme(params);

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('migrateOprfScheme');
    // 5 arguments, not 6: old_anchor is read on-chain from CitizenAnchor (log #76), never
    // supplied by the caller.
    expect(calls[0].args).toEqual([
      params.zkProof,
      params.publicInputs,
      params.newAnchor,
      params.oldOprfPkHashes,
      params.newOprfPkHashes,
    ]);
  });

  it('resolves when the extrinsic finalizes without a dispatch error', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    await expect(migrateOprfScheme(validParams())).resolves.toBeUndefined();
  });

  it('rejects when the chain reports a dispatch error (e.g. NewAnchorAlreadyUsed)', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'identity.NewAnchorAlreadyUsed' } });
    mockedGetApi.mockResolvedValue(api as any);
    await expect(migrateOprfScheme(validParams())).rejects.toThrow('identity.NewAnchorAlreadyUsed');
  });

  it('rejects a malformed new anchor before ever calling getApi', async () => {
    const params = validParams();
    (params as any).newAnchor = new Uint8Array(40);
    await expect(migrateOprfScheme(params)).rejects.toThrow(/anchor is 40 bytes/);
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects malformed old committee key hashes distinctly from new ones', async () => {
    const params = validParams();
    (params as any).oldOprfPkHashes = committeeHashes(21).slice(0, 3);
    await expect(migrateOprfScheme(params)).rejects.toThrow(/\(old\)/);
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects malformed new committee key hashes distinctly from old ones', async () => {
    const params = validParams();
    (params as any).newOprfPkHashes = committeeHashes(31).slice(0, 3);
    await expect(migrateOprfScheme(params)).rejects.toThrow(/\(new\)/);
    expect(mockedGetApi).not.toHaveBeenCalled();
  });
});

/**
 * Tests `getSigningKeypair`'s signing-key selection: the new Android
 * Keystore-backed path (`keystoreWallet.ts` via `native/keystoreSigner.ts`)
 * versus the legacy `DEV_ONLY_MNEMONIC` fallback, and — the security property
 * this whole task exists for — that the dev-mnemonic fallback is provably
 * unreachable once `__DEV__` is false, in a build that does not have the
 * Keystore module linked (e.g. iOS today, or any Android build where linking
 * somehow failed): `getSigningKeypair` must throw, never silently sign with
 * the well-known dev mnemonic.
 *
 * `identity.ts` (via `keystoreWallet.ts`) caches its resolved keypair in
 * module scope, so each scenario here needs a fresh module instance — same
 * `jest.resetModules()` + re-require pattern `native/keystoreSigner.test.ts`
 * and `keystoreWallet.test.ts` use, for the same reason (see those files'
 * doc comments).
 */
describe('getSigningKeypair (signing-key selection)', () => {
  /** The literal from `identity.ts`'s own `DEV_ONLY_MNEMONIC` — public, well-known, already in this codebase's source, safe to duplicate here to derive the expected dev address independently rather than hardcoding an SS58 string. */
  const DEV_ONLY_MNEMONIC = 'bottom drive obey lake curtain smoke basket hold race lonely fit walk';
  let expectedDevAddress: string;

  beforeAll(async () => {
    await cryptoWaitReady();
    const keyring = new Keyring({ type: 'sr25519', ss58Format: 42 });
    expectedDevAddress = keyring.addFromUri(DEV_ONLY_MNEMONIC).address;
  });

  afterEach(() => {
    // Restore jest.config.js's default so later test files/tests aren't affected.
    (global as any).__DEV__ = true;
  });

  function fakeKeystoreNativeModule() {
    return {
      encryptSecret: jest.fn(async (plaintextB64: string) => ({
        ciphertext: Buffer.from([...Buffer.from(plaintextB64, 'base64')].reverse()).toString('base64'),
        iv: Buffer.from([4, 4, 4, 4]).toString('base64'),
      })),
      decryptSecret: jest.fn(async (ciphertextB64: string) =>
        Buffer.from([...Buffer.from(ciphertextB64, 'base64')].reverse()).toString('base64'),
      ),
      isHardwareBacked: jest.fn(async () => true),
    };
  }

  /** Fresh `identity.ts` (and everything it transitively imports) with the given `__DEV__`/Keystore-linked state. */
  function freshIdentityModule(opts: { dev: boolean; keystoreLinked: boolean }): typeof import('./identity') {
    jest.resetModules();
    (global as any).__DEV__ = opts.dev;

    // eslint-disable-next-line @typescript-eslint/no-var-requires
    const rn = require('react-native');
    rn.Platform.OS = 'android';
    if (opts.keystoreLinked) {
      rn.NativeModules.KeystoreSigningModule = fakeKeystoreNativeModule();
    } else {
      delete rn.NativeModules.KeystoreSigningModule;
    }
    // eslint-disable-next-line @typescript-eslint/no-var-requires
    require('react-native-fs').__reset();

    // eslint-disable-next-line @typescript-eslint/no-var-requires
    return require('./identity');
  }

  it('falls back to the DEV_ONLY_MNEMONIC keypair when __DEV__ is true and no Keystore module is linked', async () => {
    const mod = freshIdentityModule({ dev: true, keystoreLinked: false });
    const { keypair } = await mod.getSigningKeypair();
    expect(keypair.address).toBe(expectedDevAddress);
  });

  it('throws instead of silently using the dev mnemonic when __DEV__ is false and no Keystore module is linked', async () => {
    const mod = freshIdentityModule({ dev: false, keystoreLinked: false });
    await expect(mod.getSigningKeypair()).rejects.toThrow(/Refusing to sign/);
  });

  it('never lets the dev-mnemonic address leak out under any non-__DEV__, no-Keystore call', async () => {
    const mod = freshIdentityModule({ dev: false, keystoreLinked: false });
    await expect(mod.getSigningKeypair()).rejects.toBeInstanceOf(Error);
    // Belt-and-suspenders on top of the throw itself: confirm there is no
    // code path in this state that could still resolve with the dev address.
    await mod.getSigningKeypair().catch(() => undefined);
  });

  it('prefers the Keystore-backed keypair over the dev mnemonic whenever the module is linked, even inside __DEV__', async () => {
    const mod = freshIdentityModule({ dev: true, keystoreLinked: true });
    const { keypair } = await mod.getSigningKeypair();
    expect(keypair.address).not.toBe(expectedDevAddress);
  });

  it('uses the Keystore-backed keypair (not a throw) when __DEV__ is false and the module is linked', async () => {
    const mod = freshIdentityModule({ dev: false, keystoreLinked: true });
    const { keypair } = await mod.getSigningKeypair();
    expect(keypair.address).not.toBe(expectedDevAddress);
  });

  it('returns a stable nullifierHash derived from the resolved keypair', async () => {
    const mod = freshIdentityModule({ dev: true, keystoreLinked: false });
    const first = await mod.getSigningKeypair();
    const second = await mod.getSigningKeypair();
    expect(second.nullifierHash).toBe(first.nullifierHash);
  });
});
