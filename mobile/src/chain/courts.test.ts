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
  CaseFilingProof,
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
  it('defaults to a null proof for every CaseSubject variant, including CitizenConduct', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const subjects: CaseSubject[] = [
      { General: null },
      { LawChallenge: { law_id: 3 } },
      { TreasuryDispute: { department_id: 2 } },
      { CitizenConduct: { nullifier: new Uint8Array(32).fill(1), suspension_blocks: 100 } },
      { TierConflict: { law_id: 3 } },
    ];

    for (const subject of subjects) {
      await fileCase(fakePair, subject);
    }

    expect(calls.map((c) => c.args)).toEqual(subjects.map((subject) => [subject, null, null]));
  });

  it('passes zkProof/publicInputs through as the 2nd/3rd args when a proof is supplied', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const proof: CaseFilingProof = {
      zkProof: new Uint8Array([1, 2, 3]),
      publicInputs: [new Uint8Array(32).fill(9), new Uint8Array(32).fill(8)],
    };

    await fileCase(fakePair, { LawChallenge: { law_id: 3 } }, proof);

    expect(calls).toEqual([
      { name: 'fileCase', args: [{ LawChallenge: { law_id: 3 } }, proof.zkProof, proof.publicInputs] },
    ]);
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
function tierConflictSubject(lawId: number) {
  return { type: 'TierConflict', value: { law_id: num(lawId) } };
}

/** Fake `CaseFiler::Account(AccountId)` codec value, as stored in `Cases`' filer field. */
function accountFiler(address: string) {
  return { type: 'Account', value: accountId(address) };
}
/** Fake `CaseFiler::Nullifier([u8; 32])` codec value, as stored in `Cases`' filer field. */
function nullifierFiler(nullifier: Uint8Array) {
  return { type: 'Nullifier', value: { toU8a: () => nullifier } };
}

function caseTuple(
  filer: ReturnType<typeof accountFiler> | ReturnType<typeof nullifierFiler>,
  status: string,
  rulingHash: ReturnType<typeof none> | ReturnType<typeof some>,
  subject: unknown,
) {
  return [filer, statusCodec(status), rulingHash, subject];
}

function entryKey(id: number) {
  return { args: [num(id)] };
}

describe('fetchAllCases', () => {
  it('decodes multiple entries across different CaseStatus/CaseSubject variants, sorted by caseId', async () => {
    const entries = [
      [entryKey(2), some(caseTuple(accountFiler('Bob'), 'InJuryAppeal', none(), treasuryDisputeSubject(7)))],
      [entryKey(0), some(caseTuple(accountFiler('Alice'), 'Filed', none(), generalSubject()))],
      [
        entryKey(1),
        some(
          caseTuple(
            accountFiler('Carol'),
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
      {
        caseId: 0,
        filer: { kind: 'account', address: 'Alice' },
        status: 'Filed',
        rulingIpfsHash: null,
        subject: { General: null },
      },
      {
        caseId: 1,
        filer: { kind: 'account', address: 'Carol' },
        status: 'AIRulingIssued',
        rulingIpfsHash: '0xaa',
        subject: { LawChallenge: { law_id: 3 } },
      },
      {
        caseId: 2,
        filer: { kind: 'account', address: 'Bob' },
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
        some(caseTuple(accountFiler('Dave'), 'Filed', none(), citizenConductSubject(nullifier, null))),
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
        filer: { kind: 'account', address: 'Dave' },
        status: 'Filed',
        rulingIpfsHash: null,
        subject: { CitizenConduct: { nullifier, suspension_blocks: null } },
      },
    ]);
  });

  it('decodes an anonymized TierConflict case filed under a nullifier, not an account', async () => {
    const filerNullifier = new Uint8Array(32).fill(7);
    const entries = [
      [
        entryKey(6),
        some(caseTuple(nullifierFiler(filerNullifier), 'Filed', none(), tierConflictSubject(3))),
      ],
    ];
    const api = {
      query: { courts: { cases: { entries: jest.fn(async () => entries) } } },
    };
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchAllCases();

    expect(result).toEqual([
      {
        caseId: 6,
        filer: { kind: 'nullifier', value: filerNullifier },
        status: 'Filed',
        rulingIpfsHash: null,
        subject: { TierConflict: { law_id: 3 } },
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
      caseOpt: some(caseTuple(accountFiler('Alice'), 'JurySeated', none(), generalSubject())),
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
      filer: { kind: 'account', address: 'Alice' },
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
      caseOpt: some(caseTuple(accountFiler('Alice'), 'Filed', none(), generalSubject())),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchCaseDetail(3);

    expect(result?.aiRulingBlock).toBeNull();
    expect(result?.appealDeadlineBlock).toBeNull();
  });

  it('decodes a final Upheld ruling', async () => {
    const api = fakeDetailApi({
      caseOpt: some(caseTuple(accountFiler('Alice'), 'FinalRuling', some(hexHash('0xbb')), generalSubject())),
      rulingOpt: some(verdictCodec('Upheld')),
      aiRulingBlockOpt: some(num(10)),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchCaseDetail(3);

    expect(result?.ruling).toBe('Upheld');
    expect(result?.rulingIpfsHash).toBe('0xbb');
  });

  it('decodes a case filed anonymously under a nullifier (e.g. LawChallenge)', async () => {
    const filerNullifier = new Uint8Array(32).fill(5);
    const api = fakeDetailApi({
      caseOpt: some(
        caseTuple(nullifierFiler(filerNullifier), 'Filed', none(), lawChallengeSubject(9)),
      ),
    });
    mockedGetApi.mockResolvedValue(api as any);

    const result = await fetchCaseDetail(3);

    expect(result?.filer).toEqual({ kind: 'nullifier', value: filerNullifier });
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
    filer: { kind: 'account', address: 'Alice' },
    status: 'Filed',
    rulingIpfsHash: null,
    subject: { General: null },
    ruling: null,
    juryPool: [],
    juryTally: { upheld: 0, overturned: 0 },
    aiRulingBlock: null,
    appealDeadlineBlock: null,
  };

  it('is true when the caller is the account filer', () => {
    expect(isFilerOrOracle(baseDetail, 'Alice', null, ['Oracle1'])).toBe(true);
  });

  it('is true when the caller is a member of the oracle council', () => {
    expect(isFilerOrOracle(baseDetail, 'Oracle1', null, ['Oracle1', 'Oracle2'])).toBe(true);
  });

  it('is false when the caller is neither the filer nor an oracle member', () => {
    expect(isFilerOrOracle(baseDetail, 'Random', null, ['Oracle1'])).toBe(false);
  });

  it('is false when there are no oracle members and the caller is not the filer', () => {
    expect(isFilerOrOracle(baseDetail, 'Random', null, [])).toBe(false);
  });

  describe('with a nullifier-kind (anonymized) filer', () => {
    const nullifier = new Uint8Array(32).fill(6);
    const otherNullifier = new Uint8Array(32).fill(7);
    const anonDetail: CaseDetail = {
      ...baseDetail,
      filer: { kind: 'nullifier', value: nullifier },
      subject: { LawChallenge: { law_id: 3 } },
    };

    it('is true when callerCitizenNullifier matches the stored filer nullifier', () => {
      expect(isFilerOrOracle(anonDetail, 'SomeAddress', nullifier, [])).toBe(true);
    });

    it('is false when callerCitizenNullifier does not match', () => {
      expect(isFilerOrOracle(anonDetail, 'SomeAddress', otherNullifier, [])).toBe(false);
    });

    it('is false when callerCitizenNullifier is null', () => {
      expect(isFilerOrOracle(anonDetail, 'SomeAddress', null, [])).toBe(false);
    });

    it('is still true for a nullifier filer when the caller is an oracle member', () => {
      expect(isFilerOrOracle(anonDetail, 'Oracle1', null, ['Oracle1'])).toBe(true);
    });
  });
});

describe('isRuledAgainstParty', () => {
  const nullifier = new Uint8Array(32).fill(3);
  const otherNullifier = new Uint8Array(32).fill(4);

  const conductDetail: CaseDetail = {
    caseId: 1,
    filer: { kind: 'account', address: 'Alice' },
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
    ['TierConflict', { TierConflict: { law_id: 1 } }],
  ] as const)('is false for a %s subject even with a matching-looking nullifier', (_label, subject) => {
    const detail: CaseDetail = { ...conductDetail, subject: subject as CaseSubject };
    expect(isRuledAgainstParty(detail, nullifier)).toBe(false);
  });
});
