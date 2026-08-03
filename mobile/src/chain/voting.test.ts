/**
 * Tests `voting.ts`'s extrinsic wrappers against a fake `@polkadot/api` instance,
 * mirroring `identity.test.ts`'s approach.
 *
 * The motivating case: `submitProposal` previously called
 * `api.tx.voting.submitProposal(durationBlocks)` with only one argument, while
 * `pallet_voting::submit_proposal(origin, topic_hash, tier, duration_blocks)`
 * (`pallets/pallet-voting/src/lib.rs`) takes three. Nothing caught that until the
 * chain call itself failed at runtime, since `api.tx.voting.submitProposal` isn't
 * checked by TypeScript. This file exists so a regression there fails a test
 * instead of only a live chain call.
 */
jest.mock('./api', () => ({
  getApi: jest.fn(),
}));

import { getApi } from './api';
import { submitProposal, commitVote, delegateVote, voteReferendum, revokeDelegation } from './voting';

interface RecordedCall {
  name: string;
  args: unknown[];
}

/** Fake ApiPromise exposing only `tx.voting.*` and the `ProposalCreated` event `submitProposal` reads back. */
function fakeApi(options: { dispatchError?: unknown; proposalId?: number } = {}) {
  const calls: RecordedCall[] = [];
  const proposalId = options.proposalId ?? 7;
  // Mirrors the shape `submitProposal`'s onEvents callback destructures:
  // `for (const { event } of events)`, then `event.data.id.toNumber()` once
  // `api.events.voting.ProposalCreated.is(event)` matches it by reference.
  const proposalCreatedEvent = { data: { id: { toNumber: () => proposalId } } };

  function makeCall(name: string, events: unknown[] = []) {
    return (...args: unknown[]) => {
      calls.push({ name, args });
      return {
        signAndSend: (_pair: unknown, callback: (result: any) => void) => {
          queueMicrotask(() => {
            if (options.dispatchError) {
              callback({ status: { isFinalized: false }, events: [], dispatchError: options.dispatchError });
            } else {
              callback({ status: { isFinalized: true }, events, dispatchError: undefined });
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
        voting: {
          submitProposal: makeCall('submitProposal', [{ event: proposalCreatedEvent }]),
          commitVote: makeCall('commitVote'),
          delegateVote: makeCall('delegateVote'),
          voteReferendum: makeCall('voteReferendum'),
          revokeDelegation: makeCall('revokeDelegation'),
        },
      },
      events: {
        voting: {
          ProposalCreated: {
            is: (event: unknown) => event === proposalCreatedEvent,
          },
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

describe('submitProposal', () => {
  it('submits with topicHash, tier, and durationBlocks in that order — not durationBlocks alone', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    const topicHash = new Uint8Array(32).fill(0xab);
    await submitProposal(fakePair, topicHash, 'Constitutional', 14_400);

    expect(calls).toHaveLength(1);
    expect(calls[0].name).toBe('submitProposal');
    expect(calls[0].args).toEqual([topicHash, 'Constitutional', 14_400]);
  });

  it('reads the assigned proposal id back off the ProposalCreated event', async () => {
    const { api } = fakeApi({ proposalId: 42 });
    mockedGetApi.mockResolvedValue(api as any);

    const id = await submitProposal(fakePair, new Uint8Array(32), 'Ordinary', 1000);
    expect(id).toBe(42);
  });
});

describe('commitVote / delegateVote / voteReferendum / revokeDelegation', () => {
  it('each pass their arguments straight through', async () => {
    const { api, calls } = fakeApi();
    mockedGetApi.mockResolvedValue(api as any);

    await commitVote(fakePair, 3, new Uint8Array([1, 2, 3]));
    await delegateVote(fakePair, 'delegate-addr', 2, 500);
    await voteReferendum(fakePair, 9, true);
    await revokeDelegation(fakePair, 2);

    expect(calls.map((c) => c.name)).toEqual([
      'commitVote',
      'delegateVote',
      'voteReferendum',
      'revokeDelegation',
    ]);
    expect(calls[0].args).toEqual([3, new Uint8Array([1, 2, 3])]);
    expect(calls[1].args).toEqual(['delegate-addr', 2, 500]);
    expect(calls[2].args).toEqual([9, true]);
    expect(calls[3].args).toEqual([2]);
  });

  it('rejects on dispatchError', async () => {
    const { api } = fakeApi({ dispatchError: { toString: () => 'voting.NotEligible' } });
    mockedGetApi.mockResolvedValue(api as any);

    await expect(voteReferendum(fakePair, 1, true)).rejects.toThrow('voting.NotEligible');
  });
});
