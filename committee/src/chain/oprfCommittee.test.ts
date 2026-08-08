/**
 * Tests `oprfCommittee.ts`'s duty-check/fulfill flow against a fake `@polkadot/api`
 * instance, mirroring `mobile/src/chain/identity.test.ts` and `voting.test.ts`'s
 * approach (a fake `ApiPromise`-shaped object, no real network, no live chain).
 *
 * Covers:
 *  - `getMemberCommitteeSlots` reads `committeeMembers` for every slot and returns
 *    only the ones containing the given address;
 *  - `fetchPendingDuties` cross-references `pendingOprfQueries` against
 *    `committeeMembers` and `oprfResponses` (a presence check per `(query, slot)`
 *    pair — see `oprfCommittee.ts`'s doc comment on why this isn't per-address),
 *    producing one duty per outstanding pair and skipping already-answered ones;
 *  - `fulfillDuty` calls the injected `CommitteeCrypto` with the whole 64-byte
 *    blinded query and this member's secret key, then submits `submitOprfResponse`
 *    with the returned evaluation and proof unchanged, in the pallet's argument
 *    order;
 *  - local validation (bad committee slot, wrong-length blinded query) rejects before
 *    ever calling the crypto stub or the chain.
 *
 * What's NOT covered, honestly: this app still has no real Wasm-loading
 * `CommitteeCrypto` implementation (see `CommitteeCrypto.ts`'s doc comment) — this
 * only proves the app-side orchestration matches the real, now-landed pallet
 * interface, not a real crypto core.
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));

import { getApi } from './api';
import { fetchPendingDuties, fulfillDuty, getMemberCommitteeSlots } from './oprfCommittee';
import type { CommitteeCrypto, OprfEvaluationResult } from '../crypto/CommitteeCrypto';

const ADDRESS = '5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY';
const OTHER_ADDRESS = '5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty';

function bytes(len: number, seed: number): Uint8Array {
  return new Uint8Array(len).fill(seed);
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
 * `oprfResponses(queryId, slot)` fake — `OprfResponses` is a `StorageDoubleMap` holding
 * at most one record per `(query, slot)` pair (real pallet shape, confirmed in
 * `oprfCommittee.ts`'s doc comment), so this fake takes a simple set of "pairs that
 * already have a response", not per-address data.
 */
function oprfResponsesFake(respondedPairs: Set<string>) {
  return (queryId: number, slot: number) => {
    const has = respondedPairs.has(`${queryId}:${slot}`);
    return Promise.resolve({ isNone: !has, isEmpty: !has });
  };
}

function fakeApi(options: {
  rosterBySlot?: Record<number, string[]>;
  queries?: FakeQueryRecord[];
  respondedPairs?: Set<string>;
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
          oprfResponses: oprfResponsesFake(options.respondedPairs ?? new Set()),
        },
      },
      tx: {
        identity: {
          submitOprfResponse: makeCall('submitOprfResponse'),
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

  it('returns one duty per outstanding (query, slot) pair for the member\'s own slot(s)', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 2: [ADDRESS] },
      queries: [
        { queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100 },
        { queryId: 2, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x22), postedAt: 200 },
      ],
    });
    mockedGetApi.mockResolvedValue(api as any);

    const duties = await fetchPendingDuties(ADDRESS);
    expect(duties).toEqual([
      { queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100, committeeSlot: 2 },
      { queryId: 2, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x22), postedAt: 200, committeeSlot: 2 },
    ]);
  });

  it('omits a (query, slot) pair that already has a response recorded', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 2: [ADDRESS] },
      queries: [
        { queryId: 1, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x11), postedAt: 100 },
        { queryId: 2, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x22), postedAt: 200 },
      ],
      respondedPairs: new Set(['1:2']),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const duties = await fetchPendingDuties(ADDRESS);
    expect(duties.map((d) => d.queryId)).toEqual([2]);
  });

  it('produces a separate duty per committee slot the member serves on', async () => {
    const { api } = fakeApi({
      rosterBySlot: { 0: [ADDRESS], 3: [ADDRESS] },
      queries: [{ queryId: 5, submitter: OTHER_ADDRESS, blindedQuery: bytes(64, 0x33), postedAt: 300 }],
    });
    mockedGetApi.mockResolvedValue(api as any);

    const duties = await fetchPendingDuties(ADDRESS);
    expect(duties.map((d) => d.committeeSlot)).toEqual([0, 3]);
  });
});

describe('fulfillDuty', () => {
  function fixtureCrypto(result: OprfEvaluationResult): CommitteeCrypto {
    return { evaluateQuery: jest.fn().mockResolvedValue(result) };
  }

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

  const evalResult: OprfEvaluationResult = {
    pk: bytes(64, 0xff),
    evaluation: (() => {
      const b = new Uint8Array(64);
      b.set(bytes(32, 0xcc), 0);
      b.set(bytes(32, 0xdd), 32);
      return b;
    })(),
    dlogProof: new Uint8Array([1, 2, 3]),
  };

  it('calls the injected crypto with the whole 64-byte blinded query and the secret key', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto(evalResult);
    const secretKeyBytes = bytes(32, 0x99);

    await fulfillDuty({ duty, secretKeyBytes, pair: {} as any, crypto });

    expect(crypto.evaluateQuery).toHaveBeenCalledWith(secretKeyBytes, duty.blindedQuery);
  });

  it('submits submitOprfResponse with query id, slot, evaluation, and dlog proof unchanged', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto(evalResult);

    await fulfillDuty({ duty, secretKeyBytes: bytes(32, 0x99), pair: {} as any, crypto });

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('submitOprfResponse');
    const [queryId, committeeSlot, evaluation, dlogProof] = calls[0].args as [number, number, Uint8Array, Uint8Array];
    expect(queryId).toBe(7);
    expect(committeeSlot).toBe(2);
    expect(evaluation).toEqual(evalResult.evaluation);
    expect(dlogProof).toEqual(evalResult.dlogProof);
  });

  it('rejects an out-of-range committee slot before calling the crypto stub or the chain', async () => {
    const crypto = fixtureCrypto(evalResult);
    await expect(
      fulfillDuty({ duty: { ...duty, committeeSlot: 5 }, secretKeyBytes: bytes(32, 0x99), pair: {} as any, crypto }),
    ).rejects.toThrow(RangeError);
    expect(crypto.evaluateQuery).not.toHaveBeenCalled();
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects a wrong-length blinded query before calling the crypto stub or the chain', async () => {
    const crypto = fixtureCrypto(evalResult);
    await expect(
      fulfillDuty({
        duty: { ...duty, blindedQuery: new Uint8Array(10) },
        secretKeyBytes: bytes(32, 0x99),
        pair: {} as any,
        crypto,
      }),
    ).rejects.toThrow(/blindedQuery is 10 bytes/);
    expect(crypto.evaluateQuery).not.toHaveBeenCalled();
    expect(mockedGetApi).not.toHaveBeenCalled();
  });

  it('rejects when the chain reports a dispatch error', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'identity.NotCommitteeMember' } });
    mockedGetApi.mockResolvedValue(api as any);
    const crypto = fixtureCrypto(evalResult);

    await expect(
      fulfillDuty({ duty, secretKeyBytes: bytes(32, 0x99), pair: {} as any, crypto }),
    ).rejects.toThrow('identity.NotCommitteeMember');
  });

  it('propagates the crypto stub\'s "not implemented" error when no crypto override is given', async () => {
    const { api } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    await expect(
      fulfillDuty({ duty, secretKeyBytes: bytes(32, 0x99), pair: {} as any }),
    ).rejects.toThrow(/not implemented/);
  });
});
