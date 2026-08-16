/**
 * Tests `oprfCommittee.ts`'s duty-check/fulfill flow against a fake `@polkadot/api`
 * instance, mirroring `mobile/src/chain/identity.test.ts` and `voting.test.ts`'s
 * approach (a fake `ApiPromise`-shaped object, no real network, no live chain).
 *
 * Covers the two-round migration (`submit_oprf_round1`/`submit_oprf_round2`, call
 * indices 16/17 — the old `submit_oprf_response`/`oprfResponses` this file used to
 * test no longer exist on-chain):
 *  - `getMemberCommitteeSlots` reads `committeeMembers` for every slot and returns
 *    only the ones containing the given address (unchanged by the migration);
 *  - `fetchPendingDuties` cross-references `pendingOprfQueries` against
 *    `committeeMembers` and the new `oprfRound1Commitments`/`oprfRound2Responses`
 *    double-maps plus the `oprfThreshold` constant, producing one duty per pair the
 *    member still owes a round-1 or round-2 submission for, tagged with the right
 *    `phase`;
 *  - `submitRound1` calls the injected `CommitteeCrypto.round1` with the whole
 *    64-byte blinded query, the secret share, and the given seed, then submits
 *    `submitOprfRound1` with the returned five points in argument order;
 *  - `submitRound2` calls the injected `CommitteeCrypto.round2Response` with the
 *    secret share, seed, and the three public aggregation values, then submits
 *    `submitOprfRound2` with the returned `z_i`;
 *  - local validation (bad committee slot, wrong-length blinded query) rejects before
 *    ever calling the crypto stub or the chain.
 *
 * What's NOT covered, honestly: this app still has no real Wasm-loading
 * `CommitteeCrypto` implementation wired at the module level by default (see
 * `CommitteeCrypto.ts`'s doc comment) — this only proves the app-side orchestration
 * matches the real, now-landed pallet interface, not a real crypto core. It also does
 * not (and cannot) exercise real `rhoI`/`lambdaI`/`e` values for round 2 — this app has
 * no aggregation-math implementation to produce them (see `submitRound2`'s doc
 * comment in `oprfCommittee.ts`); tests supply arbitrary fixture bytes for those,
 * proving only that they're passed through to the crypto core and the chain call
 * unchanged.
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));

import { getApi } from './api';
import {
  fetchPendingDuties,
  getMemberCommitteeSlots,
  submitRound1,
  submitRound2,
} from './oprfCommittee';
import type { CommitteeCrypto, OprfRound1Commitment, OprfRound2Response } from '../crypto/CommitteeCrypto';

const ADDRESS = '5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY';
const OTHER_ADDRESS = '5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty';
const THIRD_ADDRESS = '5FLSigC9HGRKVhB9FiEo4Y3koPsNmBmLJbpXg2mp1hXcS59Y';

function bytes(len: number, seed: number): Uint8Array {
  return new Uint8Array(len).fill(seed);
}

function fixtureCrypto(overrides: Partial<CommitteeCrypto> = {}): CommitteeCrypto {
  return {
    evaluateQuery: jest.fn(),
    round1: jest.fn(),
    round2Response: jest.fn(),
    ...overrides,
  };
}

interface RecordedCall {
  name: string;
  args: unknown[];
}

/** `committeeMembers(slot)` fake: returns an object shaped like a codec `Vec<AccountId>` (`.toJSON()`). */
function committeeMembersFake(rosterBySlot: Record<number, string[]>) {
  return (slot: number) => Promise.resolve({ toJSON: () => rosterBySlot[slot] ?? [] });
}

interface FakeQueryRecord {
  queryId: number;
  submitter: string;
  blindedQuery: Uint8Array;
  postedAt: number;
}

/** `pendingOprfQueries.entries()` fake, shaped like real `@polkadot/api` storage-map entries. */
function pendingOprfQueriesFake(records: FakeQueryRecord[]) {
  return {
    entries: () =>
      Promise.resolve(
        records.map((r) => [
          { args: [{ toNumber: () => r.queryId }] },
          {
            isNone: false,
            isSome: true,
            unwrap: () => ({
              submitter: { toString: () => r.submitter },
              blindedQuery: { toU8a: () => r.blindedQuery },
              postedAt: { toNumber: () => r.postedAt },
            }),
          },
        ]),
      ),
  };
}

/**
 * `oprfRound1Commitments(queryId, slot)` / `oprfRound2Responses(queryId, slot)` fake —
 * both are `StorageDoubleMap<(query_id, committee_slot), BoundedVec<{ member, ... }>>`,
 * `ValueQuery` (absent key decodes as an empty vec). `membersByPair` maps
 * `"${queryId}:${slot}"` to the list of member addresses who have submitted for that
 * pair.
 */
function roundStorageFake(membersByPair: Record<string, string[]>) {
  return (queryId: number, slot: number) => {
    const members = membersByPair[`${queryId}:${slot}`] ?? [];
    return Promise.resolve({ toJSON: () => members.map((member) => ({ member })) });
  };
}

function fakeApi(options: {
  rosterBySlot?: Record<number, string[]>;
  queries?: FakeQueryRecord[];
  round1ByPair?: Record<string, string[]>;
  round2ByPair?: Record<string, string[]>;
  threshold?: number;
  dispatchError?: unknown;
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

  return {
    calls,
    api: {
      query: {
        identity: {
          committeeMembers: committeeMembersFake(options.rosterBySlot ?? {}),
          pendingOprfQueries: pendingOprfQueriesFake(options.queries ?? []),
          oprfRound1Commitments: roundStorageFake(options.round1ByPair ?? {}),
          oprfRound2Responses: roundStorageFake(options.round2ByPair ?? {}),
        },
      },
      consts: {
        identity: {
          oprfThreshold: { toNumber: () => options.threshold ?? 3 },
        },
      },
      tx: {
        identity: {
          submitOprfRound1: makeCall('submitOprfRound1'),
          submitOprfRound2: makeCall('submitOprfRound2'),
        },
      },
    },
  };
}

const mockedGetApi = getApi as jest.MockedFunction<typeof getApi>;

beforeEach(() => {
  mockedGetApi.mockReset();
});

describe('getMemberCommitteeSlots', () => {
  it('returns only the slots whose roster contains the address', async () => {
    const { api } = fakeApi({
      rosterBySlot: {
        0: [OTHER_ADDRESS],
        1: [ADDRESS, OTHER_ADDRESS],
        2: [],
        3: [ADDRESS],
        4: [OTHER_ADDRESS],
      },
    });
    mockedGetApi.mockResolvedValue(api as any);

    const slots = await getMemberCommitteeSlots(ADDRESS);
    expect(slots).toEqual([1, 3]);
  });

  it('returns an empty array when the address is on no committee', async () => {
    const { api } = fakeApi({ rosterBySlot: { 0: [OTHER_ADDRESS] } });
    mockedGetApi.mockResolvedValue(api as any);

    expect(await getMemberCommitteeSlots(ADDRESS)).toEqual([]);
  });
});

describe('fetchPendingDuties', () => {
  it('returns nothing when the member is on no committee, without reading pendingOprfQueries', async () => {
    const { api } = fakeApi({ rosterBySlot: {} });
    mockedGetApi.mockResolvedValue(api as any);

    expect(await fetchPendingDuties(ADDRESS)).toEqual([]);
  });

  it('reports phase "round1" when the member has not yet submitted round 1 and the set is still open', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 2: [ADDRESS] },
      queries: [
        { queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100 },
        { queryId: 2, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x22), postedAt: 200 },
      ],
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);

    const duties = await fetchPendingDuties(ADDRESS);
    expect(duties).toEqual([
      { queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100, committeeSlot: 2, phase: 'round1' },
      { queryId: 2, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x22), postedAt: 200, committeeSlot: 2, phase: 'round1' },
    ]);
  });

  it('omits a pair whose round-1 set already locked without this member', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 2: [ADDRESS] },
      queries: [{ queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100 }],
      round1ByPair: { '1:2': [OTHER_ADDRESS, THIRD_ADDRESS, '5SomeoneElse'] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);

    expect(await fetchPendingDuties(ADDRESS)).toEqual([]);
  });

  it('omits a pair the member already submitted round 1 for when the set has not locked yet', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 2: [ADDRESS] },
      queries: [{ queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100 }],
      round1ByPair: { '1:2': [ADDRESS, OTHER_ADDRESS] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);

    expect(await fetchPendingDuties(ADDRESS)).toEqual([]);
  });

  it('reports phase "round2" once round 1 is locked and this member is in the set but has not submitted round 2', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 2: [ADDRESS] },
      queries: [{ queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100 }],
      round1ByPair: { '1:2': [ADDRESS, OTHER_ADDRESS, THIRD_ADDRESS] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);

    const duties = await fetchPendingDuties(ADDRESS);
    expect(duties).toEqual([
      { queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100, committeeSlot: 2, phase: 'round2' },
    ]);
  });

  it('omits a pair where this member has already submitted both rounds', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 2: [ADDRESS] },
      queries: [{ queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100 }],
      round1ByPair: { '1:2': [ADDRESS, OTHER_ADDRESS, THIRD_ADDRESS] },
      round2ByPair: { '1:2': [ADDRESS] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);

    expect(await fetchPendingDuties(ADDRESS)).toEqual([]);
  });

  it('produces a separate duty per committee slot the member serves on', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 0: [ADDRESS], 3: [ADDRESS] },
      queries: [{ queryId: 5, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x33), postedAt: 300 }],
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);

    const duties = await fetchPendingDuties(ADDRESS);
    expect(duties.map((d) => d.committeeSlot)).toEqual([0, 3]);
    expect(duties.every((d) => d.phase === 'round1')).toBe(true);
  });
});

describe('submitRound1', () => {
  const duty = {
    queryId: 7,
    committeeSlot: 2,
    blindedQuery: (() => {
      const b = new Uint8Array(64);
      b.set(bytes(32, 0xaa), 0);
      b.set(bytes(32, 0xbb), 32);
      return b;
    })(),
  };

  const commitment: OprfRound1Commitment = {
    rI: bytes(64, 0x01),
    dG: bytes(64, 0x02),
    dQ: bytes(64, 0x03),
    eG: bytes(64, 0x04),
    eQ: bytes(64, 0x05),
  };

  it('calls the injected crypto\'s round1 with secret share, blinded query, and seed', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto({ round1: jest.fn().mockResolvedValue(commitment) });
    const secretShareBytes = bytes(32, 0x99);
    const seed = bytes(32, 0x77);

    await submitRound1({ duty, secretShareBytes, pair: {} as any, crypto, seed });

    expect(crypto.round1).toHaveBeenCalledWith(secretShareBytes, duty.blindedQuery, seed);
  });

  it('submits submitOprfRound1 with query id, slot, and the five points in argument order', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto({ round1: jest.fn().mockResolvedValue(commitment) });

    await submitRound1({ duty, secretShareBytes: bytes(32, 0x99), pair: {} as any, crypto, seed: bytes(32, 0x77) });

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('submitOprfRound1');
    const [queryId, committeeSlot, rI, dG, dQ, eG, eQ] = calls[0].args as [
      number,
      number,
      Uint8Array,
      Uint8Array,
      Uint8Array,
      Uint8Array,
      Uint8Array,
    ];
    expect(queryId).toBe(7);
    expect(committeeSlot).toBe(2);
    expect(rI).toEqual(commitment.rI);
    expect(dG).toEqual(commitment.dG);
    expect(dQ).toEqual(commitment.dQ);
    expect(eG).toEqual(commitment.eG);
    expect(eQ).toEqual(commitment.eQ);
  });

  it('rejects an out-of-range committee slot before calling the crypto stub or the chain', async () => {
    const crypto = fixtureCrypto({ round1: jest.fn().mockResolvedValue(commitment) });
    await expect(
      submitRound1({
        duty: { ...duty, committeeSlot: 5 },
        secretShareBytes: bytes(32, 0x99),
        pair: {} as any,
        crypto,
        seed: bytes(32, 0x77),
      }),
    ).rejects.toThrow(RangeError);
    expect(crypto.round1).not.toHaveBeenCalled();
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects a wrong-length blinded query before calling the crypto stub or the chain', async () => {
    const crypto = fixtureCrypto({ round1: jest.fn().mockResolvedValue(commitment) });
    await expect(
      submitRound1({
        duty: { ...duty, blindedQuery: new Uint8Array(10) },
        secretShareBytes: bytes(32, 0x99),
        pair: {} as any,
        crypto,
        seed: bytes(32, 0x77),
      }),
    ).rejects.toThrow(/blindedQuery is 10 bytes/);
    expect(crypto.round1).not.toHaveBeenCalled();
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects when the chain reports a dispatch error', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'identity.NotCommitteeMember' } });
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto({ round1: jest.fn().mockResolvedValue(commitment) });

    await expect(
      submitRound1({ duty, secretShareBytes: bytes(32, 0x99), pair: {} as any, crypto, seed: bytes(32, 0x77) }),
    ).rejects.toThrow('identity.NotCommitteeMember');
  });

  it('propagates the crypto stub\'s "not implemented" error when no crypto override is given', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    await expect(
      submitRound1({ duty, secretShareBytes: bytes(32, 0x99), pair: {} as any, seed: bytes(32, 0x77) }),
    ).rejects.toThrow(/not implemented/);
  });
});

describe('submitRound2', () => {
  const duty = { queryId: 7, committeeSlot: 2 };
  const response: OprfRound2Response = { zI: bytes(32, 0x0a) };

  it('calls the injected crypto\'s round2Response with secret share, seed, and the three public values', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto({ round2Response: jest.fn().mockResolvedValue(response) });
    const secretShareBytes = bytes(32, 0x99);
    const seed = bytes(32, 0x77);
    const rhoI = bytes(32, 0x11);
    const lambdaI = bytes(32, 0x22);
    const e = bytes(32, 0x33);

    await submitRound2({ duty, secretShareBytes, pair: {} as any, crypto, seed, rhoI, lambdaI, e });

    expect(crypto.round2Response).toHaveBeenCalledWith(secretShareBytes, seed, rhoI, lambdaI, e);
  });

  it('submits submitOprfRound2 with query id, slot, and z_i unchanged', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto({ round2Response: jest.fn().mockResolvedValue(response) });

    await submitRound2({
      duty,
      secretShareBytes: bytes(32, 0x99),
      pair: {} as any,
      crypto,
      seed: bytes(32, 0x77),
      rhoI: bytes(32, 0x11),
      lambdaI: bytes(32, 0x22),
      e: bytes(32, 0x33),
    });

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('submitOprfRound2');
    const [queryId, committeeSlot, zI] = calls[0].args as [number, number, Uint8Array];
    expect(queryId).toBe(7);
    expect(committeeSlot).toBe(2);
    expect(zI).toEqual(response.zI);
  });

  it('rejects an out-of-range committee slot before calling the crypto stub or the chain', async () => {
    const crypto = fixtureCrypto({ round2Response: jest.fn().mockResolvedValue(response) });
    await expect(
      submitRound2({
        duty: { ...duty, committeeSlot: 5 },
        secretShareBytes: bytes(32, 0x99),
        pair: {} as any,
        crypto,
        seed: bytes(32, 0x77),
        rhoI: bytes(32, 0x11),
        lambdaI: bytes(32, 0x22),
        e: bytes(32, 0x33),
      }),
    ).rejects.toThrow(RangeError);
    expect(crypto.round2Response).not.toHaveBeenCalled();
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects when the chain reports a dispatch error', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'identity.OprfRound1NotLocked' } });
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto({ round2Response: jest.fn().mockResolvedValue(response) });

    await expect(
      submitRound2({
        duty,
        secretShareBytes: bytes(32, 0x99),
        pair: {} as any,
        crypto,
        seed: bytes(32, 0x77),
        rhoI: bytes(32, 0x11),
        lambdaI: bytes(32, 0x22),
        e: bytes(32, 0x33),
      }),
    ).rejects.toThrow('identity.OprfRound1NotLocked');
  });

  it('propagates the crypto stub\'s "not implemented" error when no crypto override is given', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    await expect(
      submitRound2({
        duty,
        secretShareBytes: bytes(32, 0x99),
        pair: {} as any,
        seed: bytes(32, 0x77),
        rhoI: bytes(32, 0x11),
        lambdaI: bytes(32, 0x22),
        e: bytes(32, 0x33),
      }),
    ).rejects.toThrow(/not implemented/);
  });
});
