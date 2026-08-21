/**
 * Tests `courts.ts`'s extrinsic wrappers against a fake `@polkadot/api` instance,
 * mirroring `identity.test.ts`'s approach.
 *
 * The motivating case: `castJuryVote` previously took `verdict: boolean` and
 * passed it straight through, while `pallet_courts::cast_jury_vote`'s second
 * argument is a `Verdict` enum (`Upheld`/`Overturned`,
 * `pallets/pallet-courts/src/lib.rs`), not a bool. This asserts the wrapper
 * now passes one of the real variant strings through unchanged, so a
 * regression back to a boolean fails a type-check here rather than only
 * surfacing as a miscast jury vote on a live chain.
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));

import { getApi } from './api';
import {
  fileCase,
  appealRuling,
  castJuryVote,
  fetchAllCases,
  fetchCaseDetail,
  hasJurorVoted,
  getOracleMembers,
  isFilerOrOracle,
  isRuledAgainstParty,
  CaseSubject,
  CaseDetail,
} from './courts';

interface RecordedCall {
  name: string;
  args: unknown[];
}

function fakeApi(options: { dispatchError?: unknown } = {}) {
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
      tx: {
        courts: {
          fileCase: makeCall('fileCase'),
          appealRuling: makeCall('appealRuling'),
          castJuryVote: makeCall('castJuryVote'),
        },
      },
    },
  };
}

const mockedGetApi = getApi as jest.MockedFunction<typeof getApi>;
const fakePair = {} as any;

beforeEach(() => {
  mockedGetApi.mockReset();
});

describe('castJuryVote', () => {
  it.each(['Upheld', 'Overturned'] as const)(
    'passes the %s Verdict variant through as a string, not a boolean',
    async (verdict) => {
      const { api, calls } = fakeApi();
      mockedGetApi.mockResolvedValue(api as any);

      await castJuryVote(fakePair, 5, verdict);

      expect(calls).toEqual([{ name: 'castJuryVote', args: [5, verdict] }]);
    },
  );

  it('rejects on dispatchError', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'courts.NotEligibleJuror' } });
    mockedGetApi.mockResolvedValue(api as any);

    await expect(castJuryVote(fakePair, 1, 'Upheld')).rejects.toThrow('courts.NotEligibleJuror');
  });
});

describe('fileCase', () => {
  it('accepts every CaseSubject variant, including CitizenConduct', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const subjects: CaseSubject[] = [
      { General: null },
      { LawChallenge: { law_id: 3 } },
      { TreasuryDispute: { department_id: 2 } },
      { CitizenConduct: { nullifier: new Uint8Array(32).fill(1), suspension_blocks: 100 } },
    ];

    for (const subject of subjects) {
      await fileCase(fakePair, subject);
    }

    expect(calls.map((c) => c.args[0])).toEqual(subjects);
  });
});

describe('appealRuling', () => {
  it('passes caseId through', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    await appealRuling(fakePair, 11);

    expect(calls).toEqual([{ name: 'appealRuling', args: [11] }]);
  });
});

// ── Read-function tests ─────────────────────────────────────────────────────
//
// Fake Substrate-codec-shaped values, mirroring registrationReconciler.test.ts's
// approach: `.isNone`/`.isSome`/`.unwrap()` for Options, `.toNumber()` for numeric
// codecs, `.toHex()` for hashes, `.toString()` for AccountIds, `.type`/`.value` for
// enums (matching how governance.ts's `tier.type`/`status.type` already read
// fieldless-enum codecs, extended here to data-carrying variants via `.value`).

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
function num(n: number) {
  return { toNumber: () => n };
}
function accountId(address: string) {
  return { toString: () => address };
}
function hexHash(hex: string) {
  return { toHex: () => hex };
}
function statusCodec(type: string) {
  return { type };
}
function verdictCodec(type: 'Upheld' | 'Overturned') {
  return { type };
}

function generalSubject() {
  return { type: 'General' };
}
function lawChallengeSubject(lawId: number) {
  return { type: 'LawChallenge', value: { law_id: num(lawId) } };
}
function treasuryDisputeSubject(departmentId: number) {
  return { type: 'TreasuryDispute', value: { department_id: num(departmentId) } };
}
function citizenConductSubject(nullifier: Uint8Array, suspensionBlocks: number | null) {
  return {
    type: 'CitizenConduct',
    value: {
      nullifier: { toU8a: () => nullifier },
      suspension_blocks: suspensionBlocks === null ? none() : some(num(suspensionBlocks)),
    },
  };
}

function caseTuple(
  filer: string,
  status: string,
  rulingHash: ReturnType<typeof none> | ReturnType<typeof some>,
  subject: unknown,
) {
  return [accountId(filer), statusCodec(status), rulingHash, subject];
}

function entryKey(id: number) {
  return { args: [num(id)] };
}

describe('fetchAllCases', () => {
  it('decodes multiple entries across different CaseStatus/CaseSubject variants, sorted by caseId', async () => {
    const entries = [
      [entryKey(2), some(caseTuple('Bob', 'InJuryAppeal', none(), treasuryDisputeSubject(7)))],
      [entryKey(0), some(caseTuple('Alice', 'Filed', none(), generalSubject()))],
      [
        entryKey(1),
        some(
          caseTuple(
            'Carol',
            'AIRulingIssued',
            some(hexHash('0xaa')),
            lawChallengeSubject(3),
          ),
        ),
      ],
    ];
    const api = {
      query: { courts: { cases: { entries: jest.fn(async () => entries) } } },
    };
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchAllCases();

    expect(result).toEqual([
      { caseId: 0, filer: 'Alice', status: 'Filed', rulingIpfsHash: null, subject: { General: null } },
      {
        caseId: 1,
        filer: 'Carol',
        status: 'AIRulingIssued',
        rulingIpfsHash: '0xaa',
        subject: { LawChallenge: { law_id: 3 } },
      },
      {
        caseId: 2,
        filer: 'Bob',
        status: 'InJuryAppeal',
        rulingIpfsHash: null,
        subject: { TreasuryDispute: { department_id: 7 } },
      },
    ]);
  });

  it('decodes a CitizenConduct case, including a null suspension_blocks', async () => {
    const nullifier = new Uint8Array(32).fill(9);
    const entries = [
      [
        entryKey(5),
        some(caseTuple('Dave', 'Filed', none(), citizenConductSubject(nullifier, null))),
      ],
    ];
    const api = {
      query: { courts: { cases: { entries: jest.fn(async () => entries) } } },
    };
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchAllCases();

    expect(result).toEqual([
      {
        caseId: 5,
        filer: 'Dave',
        status: 'Filed',
        rulingIpfsHash: null,
        subject: { CitizenConduct: { nullifier, suspension_blocks: null } },
      },
    ]);
  });

  it('skips None entries', async () => {
    const entries = [[entryKey(0), none()]];
    const api = {
      query: { courts: { cases: { entries: jest.fn(async () => entries) } } },
    };
    mockedGetApi.mockResolvedValue(api as any);

    expect(await fetchAllCases()).toEqual([]);
  });
});

describe('fetchCaseDetail', () => {
  function fakeDetailApi(opts: {
    caseOpt: ReturnType<typeof none> | ReturnType<typeof some>;
    rulingOpt?: ReturnType<typeof none> | ReturnType<typeof some>;
    juryPoolOpt?: ReturnType<typeof none> | ReturnType<typeof some>;
    tally?: [number, number];
    aiRulingBlockOpt?: ReturnType<typeof none> | ReturnType<typeof some>;
    appealWindowBlocks?: number;
  }) {
    const [u, o] = opts.tally ?? [0, 0];
    return {
      query: {
        courts: {
          cases: jest.fn(async () => opts.caseOpt),
          rulings: jest.fn(async () => opts.rulingOpt ?? none()),
          juryPool: jest.fn(async () => opts.juryPoolOpt ?? none()),
          juryTally: jest.fn(async () => [num(u), num(o)]),
          aiRulingBlock: jest.fn(async () => opts.aiRulingBlockOpt ?? none()),
        },
      },
      consts: { courts: { appealWindowBlocks: num(opts.appealWindowBlocks ?? 100) } },
    };
  }

  it('returns null for a nonexistent case id', async () => {
    mockedGetApi.mockResolvedValue(fakeDetailApi({ caseOpt: none() }) as any);

    expect(await fetchCaseDetail(999)).toBeNull();
  });

  it('composes every sub-read, including the appealDeadlineBlock computation', async () => {
    const api = fakeDetailApi({
      caseOpt: some(caseTuple('Alice', 'JurySeated', none(), generalSubject())),
      juryPoolOpt: some([accountId('Juror1'), accountId('Juror2')]),
      tally: [4, 1],
      aiRulingBlockOpt: some(num(50)),
      rulingOpt: none(),
      appealWindowBlocks: 100,
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchCaseDetail(3);

    expect(result).toEqual<CaseDetail>({
      caseId: 3,
      filer: 'Alice',
      status: 'JurySeated',
      rulingIpfsHash: null,
      subject: { General: null },
      ruling: null,
      juryPool: ['Juror1', 'Juror2'],
      juryTally: { upheld: 4, overturned: 1 },
      aiRulingBlock: 50,
      appealDeadlineBlock: 150,
    });
  });

  it('reports a null appealDeadlineBlock when there is no aiRulingBlock yet', async () => {
    const api = fakeDetailApi({
      caseOpt: some(caseTuple('Alice', 'Filed', none(), generalSubject())),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchCaseDetail(3);

    expect(result?.aiRulingBlock).toBeNull();
    expect(result?.appealDeadlineBlock).toBeNull();
  });

  it('decodes a final Upheld ruling', async () => {
    const api = fakeDetailApi({
      caseOpt: some(caseTuple('Alice', 'FinalRuling', some(hexHash('0xbb')), generalSubject())),
      rulingOpt: some(verdictCodec('Upheld')),
      aiRulingBlockOpt: some(num(10)),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchCaseDetail(3);

    expect(result?.ruling).toBe('Upheld');
    expect(result?.rulingIpfsHash).toBe('0xbb');
  });
});

describe('hasJurorVoted', () => {
  it('returns true when the juror has voted', async () => {
    const juryVotes = jest.fn(async () => some(verdictCodec('Upheld')));
    mockedGetApi.mockResolvedValue({ query: { courts: { juryVotes } } } as any);

    expect(await hasJurorVoted(3, 'Juror1')).toBe(true);
    expect(juryVotes).toHaveBeenCalledWith([3, 'Juror1']);
  });

  it('returns false when the juror has not voted', async () => {
    const juryVotes = jest.fn(async () => none());
    mockedGetApi.mockResolvedValue({ query: { courts: { juryVotes } } } as any);

    expect(await hasJurorVoted(3, 'Juror2')).toBe(false);
  });
});

describe('getOracleMembers', () => {
  it('returns the member addresses from OracleMembers', async () => {
    const oracleMembers = jest.fn(async () => [accountId('Oracle1'), accountId('Oracle2')]);
    mockedGetApi.mockResolvedValue({ query: { courts: { oracleMembers } } } as any);

    expect(await getOracleMembers()).toEqual(['Oracle1', 'Oracle2']);
  });

  it('returns an empty array when OracleMembers is empty', async () => {
    const oracleMembers = jest.fn(async () => []);
    mockedGetApi.mockResolvedValue({ query: { courts: { oracleMembers } } } as any);

    expect(await getOracleMembers()).toEqual([]);
  });
});

describe('isFilerOrOracle', () => {
  const baseDetail: CaseDetail = {
    caseId: 1,
    filer: 'Alice',
    status: 'Filed',
    rulingIpfsHash: null,
    subject: { General: null },
    ruling: null,
    juryPool: [],
    juryTally: { upheld: 0, overturned: 0 },
    aiRulingBlock: null,
    appealDeadlineBlock: null,
  };

  it('is true when the caller is the filer', () => {
    expect(isFilerOrOracle(baseDetail, 'Alice', ['Oracle1'])).toBe(true);
  });

  it('is true when the caller is a member of the oracle council', () => {
    expect(isFilerOrOracle(baseDetail, 'Oracle1', ['Oracle1', 'Oracle2'])).toBe(true);
  });

  it('is false when the caller is neither the filer nor an oracle member', () => {
    expect(isFilerOrOracle(baseDetail, 'Random', ['Oracle1'])).toBe(false);
  });

  it('is false when there are no oracle members and the caller is not the filer', () => {
    expect(isFilerOrOracle(baseDetail, 'Random', [])).toBe(false);
  });
});

describe('isRuledAgainstParty', () => {
  const nullifier = new Uint8Array(32).fill(3);
  const otherNullifier = new Uint8Array(32).fill(4);

  const conductDetail: CaseDetail = {
    caseId: 1,
    filer: 'Alice',
    status: 'Filed',
    rulingIpfsHash: null,
    subject: { CitizenConduct: { nullifier, suspension_blocks: null } },
    ruling: null,
    juryPool: [],
    juryTally: { upheld: 0, overturned: 0 },
    aiRulingBlock: null,
    appealDeadlineBlock: null,
  };

  it('is true for a CitizenConduct case whose nullifier matches', () => {
    expect(isRuledAgainstParty(conductDetail, nullifier)).toBe(true);
  });

  it('is false for a CitizenConduct case whose nullifier does not match', () => {
    expect(isRuledAgainstParty(conductDetail, otherNullifier)).toBe(false);
  });

  it('is false when callerCitizenNullifier is null', () => {
    expect(isRuledAgainstParty(conductDetail, null)).toBe(false);
  });

  it.each([
    ['General', { General: null }],
    ['LawChallenge', { LawChallenge: { law_id: 1 } }],
    ['TreasuryDispute', { TreasuryDispute: { department_id: 1 } }],
  ] as const)('is false for a %s subject even with a matching-looking nullifier', (_label, subject) => {
    const detail: CaseDetail = { ...conductDetail, subject: subject as CaseSubject };
    expect(isRuledAgainstParty(detail, nullifier)).toBe(false);
  });
});
