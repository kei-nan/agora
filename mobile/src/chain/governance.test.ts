/**
 * Tests `governance.ts`'s pallet-elections delegate-registry wrappers against a fake
 * `@polkadot/api` instance, mirroring `voting.test.ts`'s approach — the motivating case is the
 * same one that file documents: nothing checks `api.tx.palletElections.*` call shapes at
 * compile time, so a call-site/pallet-signature mismatch (like the one commit 786b792
 * introduced on the Rust side, reshaping `register_as_delegate`/`back_delegate`/
 * `remove_backing`) would otherwise only surface as a live chain rejection.
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));

import { getApi } from './api';
import {
  backDelegate,
  BackingProofSubmission,
  fetchDelegatePersonaId,
  fetchMaxBackingsPerCitizen,
  isBackingDelegate,
  registerAsDelegate,
  RegisterAsDelegateParams,
  removeBackingFromDelegate,
} from './governance';
import {
  _resetBackingStateForTests,
  getBackingSlotFor,
  isBackingDelegateLocally,
  recordBacking,
} from './backingState';
import { OprfCommitteeKeyHashes } from './identity';

interface RecordedCall {
  name: string;
  args: unknown[];
}

function fakeApi(options: {
  dispatchError?: unknown;
  delegatePersonaId?: Uint8Array | null;
  maxBackingsPerCitizen?: number;
} = {}) {
  const calls: RecordedCall[] = [];

  function makeCall(name: string) {
    return (...args: unknown[]) => {
      calls.push({ name, args });
      return {
        signAndSend: (_pair: unknown, callback: (result: any) => void) => {
          queueMicrotask(() => {
            if (options.dispatchError) {
              callback({ status: { isFinalized: false }, events: [], dispatchError: options.dispatchError });
            } else {
              callback({ status: { isFinalized: true }, events: [], dispatchError: undefined });
            }
          });
          return Promise.resolve(() => undefined);
        },
      };
    };
  }

  const delegatePersonaId = options.delegatePersonaId;
  return {
    calls,
    api: {
      tx: {
        palletElections: {
          registerAsDelegate: makeCall('registerAsDelegate'),
          backDelegate: makeCall('backDelegate'),
          removeBacking: makeCall('removeBacking'),
        },
      },
      query: {
        palletElections: {
          delegatePersonaIdOf: async (_delegate: string) =>
            delegatePersonaId === undefined || delegatePersonaId === null
              ? { isNone: true }
              : { isNone: false, unwrap: () => ({ toU8a: () => delegatePersonaId }) },
          maxBackingsPerCitizen: async () => ({ toNumber: () => options.maxBackingsPerCitizen ?? 50 }),
        },
      },
    },
  };
}

const mockedGetApi = getApi as jest.MockedFunction<typeof getApi>;

const personaKeypair = { address: '5PersonaAddress' } as any;
const citizenKeypair = { address: '5CitizenAddress' } as any;

beforeEach(() => {
  mockedGetApi.mockReset();
  _resetBackingStateForTests();
});

describe('registerAsDelegate', () => {
  it('submits persona_account, delegate_persona_id, zk_proof, public_inputs, scheme_version, oprf_pk_hashes, display_name, profile_ipfs_hash in that order, signed by the persona keypair', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const delegatePersonaId = new Uint8Array(32).fill(1);
    const zkProof = new Uint8Array([1, 2, 3]);
    const publicInputs = [new Uint8Array(32), new Uint8Array(32)];
    const oprfPkHashes = Array.from({ length: 5 }, () => new Uint8Array(32)) as unknown as OprfCommitteeKeyHashes;
    const profileIpfsHash = new Uint8Array(32).fill(2);

    const params: RegisterAsDelegateParams = {
      delegatePersonaId,
      zkProof,
      publicInputs,
      schemeVersion: 3,
      oprfPkHashes,
      displayName: 'Alice',
      profileIpfsHash,
    };
    await registerAsDelegate(personaKeypair, params);

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('registerAsDelegate');
    expect(calls[0].args).toEqual([
      personaKeypair.address,
      delegatePersonaId,
      zkProof,
      publicInputs,
      3,
      oprfPkHashes,
      'Alice',
      profileIpfsHash,
    ]);
  });

  it('propagates a dispatch error', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'DelegatePersonaAlreadyUsed' } });
    mockedGetApi.mockResolvedValue(api as any);

    await expect(
      registerAsDelegate(personaKeypair, {
        delegatePersonaId: new Uint8Array(32),
        zkProof: new Uint8Array([1]),
        publicInputs: [new Uint8Array(32)],
        schemeVersion: 1,
        oprfPkHashes: Array.from({ length: 5 }, () => new Uint8Array(32)) as unknown as OprfCommitteeKeyHashes,
        displayName: 'Bob',
        profileIpfsHash: new Uint8Array(32),
      }),
    ).rejects.toBeDefined();
  });
});

describe('backDelegate', () => {
  it('submits delegate, zk_proof, public_inputs, and records the local backing slot on success', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const proof: BackingProofSubmission = {
      zkProof: new Uint8Array([4, 5, 6]),
      publicInputs: [new Uint8Array(32), new Uint8Array(32), new Uint8Array(32), new Uint8Array(32)],
    };
    await backDelegate(citizenKeypair, '5DelegateAddr', proof, 2);

    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({ name: 'backDelegate', args: ['5DelegateAddr', proof.zkProof, proof.publicInputs] });
    expect(getBackingSlotFor('5DelegateAddr')).toBe(2);
    expect(isBackingDelegateLocally('5DelegateAddr')).toBe(true);
  });

  it('does not record a local backing slot if the extrinsic fails', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'InvalidBackingProof' } });
    mockedGetApi.mockResolvedValue(api as any);

    const proof: BackingProofSubmission = {
      zkProof: new Uint8Array([1]),
      publicInputs: [new Uint8Array(32), new Uint8Array(32), new Uint8Array(32), new Uint8Array(32)],
    };
    await expect(backDelegate(citizenKeypair, '5DelegateAddr', proof, 0)).rejects.toBeDefined();
    expect(getBackingSlotFor('5DelegateAddr')).toBeNull();
  });
});

describe('removeBackingFromDelegate', () => {
  it('submits delegate, zk_proof, public_inputs, and clears the local backing slot on success', async () => {
    recordBacking('5DelegateAddr', 2);
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const proof: BackingProofSubmission = {
      zkProof: new Uint8Array([7, 8, 9]),
      publicInputs: [new Uint8Array(32), new Uint8Array(32), new Uint8Array(32), new Uint8Array(32)],
    };
    await removeBackingFromDelegate(citizenKeypair, '5DelegateAddr', proof);

    expect(calls).toHaveLength(1);
    expect(calls[0]).toEqual({ name: 'removeBacking', args: ['5DelegateAddr', proof.zkProof, proof.publicInputs] });
    expect(getBackingSlotFor('5DelegateAddr')).toBeNull();
  });
});

describe('isBackingDelegate', () => {
  it('is a purely local lookup, not a chain query', async () => {
    // No getApi mock configured at all — if this queried the chain it would throw.
    expect(await isBackingDelegate('5DelegateAddr')).toBe(false);
    recordBacking('5DelegateAddr', 0);
    expect(await isBackingDelegate('5DelegateAddr')).toBe(true);
  });
});

describe('fetchDelegatePersonaId', () => {
  it('returns null when the delegate has no persona id on file', async () => {
    const { api } = fakeApi({ delegatePersonaId: null });
    mockedGetApi.mockResolvedValue(api as any);
    expect(await fetchDelegatePersonaId('5DelegateAddr')).toBeNull();
  });

  it('returns the on-file persona id', async () => {
    const id = new Uint8Array(32).fill(9);
    const { api } = fakeApi({ delegatePersonaId: id });
    mockedGetApi.mockResolvedValue(api as any);
    expect(await fetchDelegatePersonaId('5DelegateAddr')).toEqual(id);
  });
});

describe('fetchMaxBackingsPerCitizen', () => {
  it('returns the live governance value', async () => {
    const { api } = fakeApi({ maxBackingsPerCitizen: 7 });
    mockedGetApi.mockResolvedValue(api as any);
    expect(await fetchMaxBackingsPerCitizen()).toBe(7);
  });
});
