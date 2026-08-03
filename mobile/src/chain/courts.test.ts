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
import { fileCase, appealRuling, castJuryVote, CaseSubject } from './courts';

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
