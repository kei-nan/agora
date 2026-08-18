/**
 * Tests `reconcileRegistrationStatus` against a fake `@polkadot/api` instance
 * (mirroring `identity.test.ts`'s approach — a hand-built `api` object
 * exposing only the `query`/`rpc`/`consts` surface this module actually
 * touches, with Substrate-codec-shaped canned values: `.isNone`/`.isSome`/
 * `.unwrap()` for Options, `.toNumber()` for numeric codecs, plain JS arrays
 * for the `ValueQuery` `BoundedVec` storage items since those decode
 * directly with a native `.length`) and a mocked `./registrationState` so
 * each test controls exactly what's "persisted" without touching real RNFS.
 *
 * What's covered: every branch of the Step 1 chain-confirmed check
 * (no nullifier -> fallback; missing/expired reverification deadline;
 * not suspended -> Active + local clear; indefinite suspension; suspended
 * until a future block; a lazily-expired suspension treated as Active) and
 * every branch of the Step 2 pipeline fallback (no record -> NotStarted;
 * simple stages returned unchanged; an OPRF-pending record past its SLA
 * deadline demoted to Failed; per-committee-slot round-1/round-2 threshold
 * polling across all of `record.committeeSlots` — since every registration
 * query goes to all 5 OPRF committees symmetrically, not just slot 0 (see
 * changelog 073) — including promoting through two stages of evidence
 * within a single call, a record with fewer than 5 slots to prove the logic
 * is genuinely generic, and `ProofCombining` making no further chain
 * queries at all).
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));
jest.mock('./registrationState', () => ({
  readRegistrationStatus: jest.fn(),
  writeRegistrationStatus: jest.fn(),
  clearRegistrationStatus: jest.fn(),
}));

import { getApi } from './api';
import {
  clearRegistrationStatus,
  PersistableStatus,
  readRegistrationStatus,
  writeRegistrationStatus,
} from './registrationState';
import { reconcileRegistrationStatus } from './registrationReconciler';

const mockedGetApi = getApi as jest.MockedFunction<typeof getApi>;
const mockedReadRegistrationStatus = readRegistrationStatus as jest.MockedFunction<typeof readRegistrationStatus>;
const mockedWriteRegistrationStatus = writeRegistrationStatus as jest.MockedFunction<typeof writeRegistrationStatus>;
const mockedClearRegistrationStatus = clearRegistrationStatus as jest.MockedFunction<typeof clearRegistrationStatus>;

function none() {
  return {
    isNone: true,
    isSome: false,
    unwrap: () => {
      throw new Error('unwrap() called on a None value');
    },
  };
}
function some(value: unknown) {
  return { isNone: false, isSome: true, unwrap: () => value };
}
function blockNumber(n: number) {
  return { toNumber: () => n };
}

interface FakeApiOptions {
  nullifier: ReturnType<typeof none> | ReturnType<typeof some>;
  currentBlock: number;
  deadline?: ReturnType<typeof none> | ReturnType<typeof some>;
  suspended?: ReturnType<typeof none> | ReturnType<typeof some>;
  /** Per-committee-slot round1/round2 response arrays, keyed by `committee_slot`. Slots with no entry default to `[]`. */
  round1ForSlot?: Record<number, unknown[]>;
  round2ForSlot?: Record<number, unknown[]>;
  threshold?: number;
}

function fakeApi(opts: FakeApiOptions) {
  const citizenNullifier = jest.fn(async () => opts.nullifier);
  const reverificationDeadline = jest.fn(async () => opts.deadline ?? none());
  const suspendedNullifiers = jest.fn(async () => opts.suspended ?? none());
  const oprfRound1Commitments = jest.fn(async (_queryId: string, slot: number) => opts.round1ForSlot?.[slot] ?? []);
  const oprfRound2Responses = jest.fn(async (_queryId: string, slot: number) => opts.round2ForSlot?.[slot] ?? []);
  const getHeader = jest.fn(async () => ({ number: blockNumber(opts.currentBlock) }));

  return {
    calls: {
      citizenNullifier,
      reverificationDeadline,
      suspendedNullifiers,
      oprfRound1Commitments,
      oprfRound2Responses,
      getHeader,
    },
    api: {
      query: {
        identity: {
          citizenNullifier,
          reverificationDeadline,
          suspendedNullifiers,
          oprfRound1Commitments,
          oprfRound2Responses,
        },
      },
      rpc: { chain: { getHeader } },
      consts: {
        identity: {
          oprfThreshold: { toNumber: () => opts.threshold ?? 3 },
        },
      },
    },
  };
}

beforeEach(() => {
  mockedGetApi.mockReset();
  mockedReadRegistrationStatus.mockReset();
  mockedWriteRegistrationStatus.mockReset().mockResolvedValue(undefined);
  mockedClearRegistrationStatus.mockReset().mockResolvedValue(undefined);
});

describe('Step 1 — chain-confirmed check', () => {
  it('falls through to the pipeline fallback when no nullifier is on file', async () => {
    const { api } = fakeApi({ nullifier: none(), currentBlock: 100 });
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(null);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'NotStarted' });
  });

  it('resolves Active and clears local state when a deadline is valid and not suspended', async () => {
    const { api } = fakeApi({
      nullifier: some(new Uint8Array(32).fill(7)),
      currentBlock: 100,
      deadline: some(blockNumber(1000)),
      suspended: none(),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'Active' });
    expect(mockedClearRegistrationStatus).toHaveBeenCalledTimes(1);
    expect(mockedClearRegistrationStatus).toHaveBeenCalledWith('5Addr1');
  });

  it('resolves ReverificationDue when the deadline entry is missing', async () => {
    const { api } = fakeApi({
      nullifier: some(new Uint8Array(32).fill(7)),
      currentBlock: 100,
      deadline: none(),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'ReverificationDue' });
    expect(mockedClearRegistrationStatus).not.toHaveBeenCalled();
  });

  it('resolves ReverificationDue when the current block is past the deadline', async () => {
    const { api } = fakeApi({
      nullifier: some(new Uint8Array(32).fill(7)),
      currentBlock: 1001,
      deadline: some(blockNumber(1000)),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'ReverificationDue' });
  });

  it('resolves Suspended with until: null for an indefinite suspension', async () => {
    const { api } = fakeApi({
      nullifier: some(new Uint8Array(32).fill(7)),
      currentBlock: 100,
      deadline: some(blockNumber(1000)),
      suspended: some(none()), // Some(None) — suspended indefinitely
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'Suspended', until: null });
    expect(mockedClearRegistrationStatus).not.toHaveBeenCalled();
  });

  it('resolves Suspended with until: <block> when suspended until a future block', async () => {
    const { api } = fakeApi({
      nullifier: some(new Uint8Array(32).fill(7)),
      currentBlock: 100,
      deadline: some(blockNumber(1000)),
      suspended: some(some(blockNumber(500))), // Some(Some(500))
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'Suspended', until: 500 });
  });

  it('treats a lazily-expired suspension (current block past `until`) as Active', async () => {
    const { api } = fakeApi({
      nullifier: some(new Uint8Array(32).fill(7)),
      currentBlock: 600,
      deadline: some(blockNumber(1000)),
      suspended: some(some(blockNumber(500))), // Some(Some(500)), but currentBlock(600) > 500
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'Active' });
    expect(mockedClearRegistrationStatus).toHaveBeenCalledTimes(1);
  });
});

describe('Step 2 — pipeline fallback (no nullifier on file)', () => {
  function noNullifierApi(overrides: Partial<FakeApiOptions> = {}) {
    return fakeApi({ nullifier: none(), currentBlock: 100, ...overrides });
  }

  it('resolves NotStarted when there is no persisted record', async () => {
    const { api } = noNullifierApi();
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(null);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({ stage: 'NotStarted' });
  });

  it('returns a simple pipeline stage (ProofMaterialAssembled) unchanged', async () => {
    const { api } = noNullifierApi();
    mockedGetApi.mockResolvedValue(api as any);
    const record: PersistableStatus = { stage: 'ProofMaterialAssembled' };
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual(record);
    expect(mockedWriteRegistrationStatus).not.toHaveBeenCalled();
  });

  it('returns a LivenessVerified record unchanged', async () => {
    const { api } = noNullifierApi();
    mockedGetApi.mockResolvedValue(api as any);
    const record: PersistableStatus = { stage: 'LivenessVerified', faceMatched: true };
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual(record);
    expect(mockedWriteRegistrationStatus).not.toHaveBeenCalled();
  });

  it('demotes an OPRF-pending record past its SLA deadline to Failed and persists it', async () => {
    const record: PersistableStatus = {
      stage: 'AwaitingCommitteeRound1',
      queryId: '42',
      committeeSlots: [0, 1, 2, 3, 4],
      submittedAtBlock: 10,
      slaExpiresAtBlock: 50,
    };
    const { api } = noNullifierApi({ currentBlock: 51 });
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({
      stage: 'Failed',
      failedStage: 'AwaitingCommitteeRound1',
      reason: 'The OPRF committee did not respond within the SLA window.',
      retryable: true,
      expiresAtBlock: 50,
    });
    expect(mockedWriteRegistrationStatus).toHaveBeenCalledTimes(1);
    expect(mockedWriteRegistrationStatus).toHaveBeenCalledWith('5Addr1', result.status);
  });

  it('stays AwaitingCommitteeRound1 when only some of the 5 slots have hit round1 threshold, and skips round2 reads for the rest', async () => {
    const record: PersistableStatus = {
      stage: 'AwaitingCommitteeRound1',
      queryId: '42',
      committeeSlots: [0, 1, 2, 3, 4],
      submittedAtBlock: 10,
      slaExpiresAtBlock: 500,
    };
    // Slots 0 and 1 have hit threshold (3); slots 2-4 haven't.
    const { api, calls } = noNullifierApi({
      round1ForSlot: { 0: [1, 2, 3], 1: [1, 2, 3], 2: [1], 3: [1, 2], 4: [] },
      round2ForSlot: { 0: [1, 2], 1: [1] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual(record);
    expect(result.oprfProgress).toEqual({
      threshold: 3,
      slots: [
        { slot: 0, round1Count: 3, round2Count: 2 },
        { slot: 1, round1Count: 3, round2Count: 1 },
        { slot: 2, round1Count: 1, round2Count: 0 },
        { slot: 3, round1Count: 2, round2Count: 0 },
        { slot: 4, round1Count: 0, round2Count: 0 },
      ],
    });
    // round1 is always read for every slot.
    expect(calls.oprfRound1Commitments).toHaveBeenCalledTimes(5);
    // round2 is only read for slots that already hit round1 threshold (0 and 1).
    expect(calls.oprfRound2Responses).toHaveBeenCalledTimes(2);
    expect(calls.oprfRound2Responses).toHaveBeenCalledWith('42', 0);
    expect(calls.oprfRound2Responses).toHaveBeenCalledWith('42', 1);
    expect(calls.oprfRound2Responses).not.toHaveBeenCalledWith('42', 2);
    expect(calls.oprfRound2Responses).not.toHaveBeenCalledWith('42', 3);
    expect(calls.oprfRound2Responses).not.toHaveBeenCalledWith('42', 4);
    // Stage didn't change, so no write should have happened.
    expect(mockedWriteRegistrationStatus).not.toHaveBeenCalled();
  });

  it('promotes AwaitingCommitteeRound1 -> AwaitingCommitteeRound2 once all 5 slots hit round1 threshold, even if round2 is incomplete', async () => {
    const record: PersistableStatus = {
      stage: 'AwaitingCommitteeRound1',
      queryId: '42',
      committeeSlots: [0, 1, 2, 3, 4],
      submittedAtBlock: 10,
      slaExpiresAtBlock: 500,
    };
    const { api, calls } = noNullifierApi({
      round1ForSlot: { 0: [1, 2, 3], 1: [1, 2, 3], 2: [1, 2, 3], 3: [1, 2, 3], 4: [1, 2, 3] },
      round2ForSlot: { 0: [1], 1: [1, 2, 3], 2: [], 3: [1], 4: [1, 2] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({
      stage: 'AwaitingCommitteeRound2',
      queryId: '42',
      committeeSlots: [0, 1, 2, 3, 4],
      submittedAtBlock: 10,
      slaExpiresAtBlock: 500,
    });
    expect(result.oprfProgress).toEqual({
      threshold: 3,
      slots: [
        { slot: 0, round1Count: 3, round2Count: 1 },
        { slot: 1, round1Count: 3, round2Count: 3 },
        { slot: 2, round1Count: 3, round2Count: 0 },
        { slot: 3, round1Count: 3, round2Count: 1 },
        { slot: 4, round1Count: 3, round2Count: 2 },
      ],
    });
    // All 5 slots are round1-locked now, so round2 is queried for every one of them.
    expect(calls.oprfRound2Responses).toHaveBeenCalledTimes(5);
    expect(mockedWriteRegistrationStatus).toHaveBeenCalledTimes(1);
    expect(mockedWriteRegistrationStatus).toHaveBeenCalledWith('5Addr1', result.status);
  });

  it('promotes to ProofCombining when all 5 slots hit both round1 and round2 threshold', async () => {
    const record: PersistableStatus = {
      stage: 'AwaitingCommitteeRound2',
      queryId: '42',
      committeeSlots: [0, 1, 2, 3, 4],
      submittedAtBlock: 10,
      slaExpiresAtBlock: 500,
    };
    const { api } = noNullifierApi({
      round1ForSlot: { 0: [1, 2, 3], 1: [1, 2, 3], 2: [1, 2, 3], 3: [1, 2, 3], 4: [1, 2, 3] },
      round2ForSlot: { 0: [1, 2, 3], 1: [1, 2, 3], 2: [1, 2, 3], 3: [1, 2, 3], 4: [1, 2, 3] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({
      stage: 'ProofCombining',
      queryId: '42',
      committeeSlots: [0, 1, 2, 3, 4],
    });
    expect(result.oprfProgress).toEqual({
      threshold: 3,
      slots: [
        { slot: 0, round1Count: 3, round2Count: 3 },
        { slot: 1, round1Count: 3, round2Count: 3 },
        { slot: 2, round1Count: 3, round2Count: 3 },
        { slot: 3, round1Count: 3, round2Count: 3 },
        { slot: 4, round1Count: 3, round2Count: 3 },
      ],
    });
    expect(mockedWriteRegistrationStatus).toHaveBeenCalledTimes(1);
  });

  it('is generic over committeeSlots length — a single-slot record (not hardcoded to 5) still polls and promotes correctly', async () => {
    const record: PersistableStatus = {
      stage: 'AwaitingCommitteeRound1',
      queryId: '99',
      committeeSlots: [2],
      submittedAtBlock: 10,
      slaExpiresAtBlock: 500,
    };
    const { api, calls } = noNullifierApi({
      round1ForSlot: { 2: [1, 2, 3] },
      round2ForSlot: { 2: [1, 2, 3] },
      threshold: 3,
    });
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual({
      stage: 'ProofCombining',
      queryId: '99',
      committeeSlots: [2],
    });
    expect(result.oprfProgress).toEqual({
      threshold: 3,
      slots: [{ slot: 2, round1Count: 3, round2Count: 3 }],
    });
    expect(calls.oprfRound1Commitments).toHaveBeenCalledTimes(1);
    expect(calls.oprfRound1Commitments).toHaveBeenCalledWith('99', 2);
    expect(calls.oprfRound2Responses).toHaveBeenCalledTimes(1);
    expect(calls.oprfRound2Responses).toHaveBeenCalledWith('99', 2);
    expect(mockedWriteRegistrationStatus).toHaveBeenCalledTimes(1);
  });

  it('returns ProofCombining unchanged and makes no round1/round2 queries', async () => {
    const record: PersistableStatus = {
      stage: 'ProofCombining',
      queryId: '42',
      committeeSlots: [0, 1, 2, 3, 4],
    };
    const { api, calls } = noNullifierApi();
    mockedGetApi.mockResolvedValue(api as any);
    mockedReadRegistrationStatus.mockResolvedValue(record);

    const result = await reconcileRegistrationStatus('5Addr1');
    expect(result.status).toEqual(record);
    expect(result.oprfProgress).toBeUndefined();
    expect(calls.oprfRound1Commitments).not.toHaveBeenCalled();
    expect(calls.oprfRound2Responses).not.toHaveBeenCalled();
    expect(mockedWriteRegistrationStatus).not.toHaveBeenCalled();
  });
});
