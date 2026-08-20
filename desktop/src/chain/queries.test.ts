import { describe, it, expect, vi, beforeEach } from "vitest";

// queries.ts's only real dependency is the shared light-client `getApi()` from ./client — mock
// that boundary so these tests exercise the byte-decoding logic this codebase actually wrote
// (storage key/value parsing) without touching the network or a real smoldot connection.
const { getApiMock } = vi.hoisted(() => ({ getApiMock: vi.fn() }));
vi.mock("./client", () => ({ getApi: getApiMock }));

import { chainStatus, fetchLaws, fetchProposals } from "./queries";

function bytesToHexStr(bytes: number[]): string {
  return "0x" + bytes.map((b) => b.toString(16).padStart(2, "0")).join("");
}

function u32LEBytes(n: number): number[] {
  return [n & 0xff, (n >>> 8) & 0xff, (n >>> 16) & 0xff, (n >>> 24) & 0xff];
}

/** Builds a fake Blake2_128Concat-style storage key: 32-byte prefix + 16-byte hash + u32 suffix.
 * `marker` distinguishes keys from different storage items (real prefixes differ by pallet/item
 * hash; tests just need any distinct byte so two different items' keys don't collide). */
function fakeStorageKey(suffix: number, marker = 0): string {
  return bytesToHexStr([marker, ...new Array(47).fill(0), ...u32LEBytes(suffix)]);
}

function optionSome(hex: string) {
  return { isSome: true, unwrap: () => ({ toHex: () => hex }) };
}
function optionNone() {
  return { isSome: false, unwrap: () => { throw new Error("no value"); } };
}

describe("chainStatus", () => {
  beforeEach(() => getApiMock.mockReset());

  it("reads best and finalized block numbers off the chain header RPCs", async () => {
    const api = {
      rpc: {
        chain: {
          getHeader: vi.fn(async (hash?: string) =>
            hash
              ? { number: { toNumber: () => 99 } }
              : { number: { toNumber: () => 105 } }
          ),
          getFinalizedHead: vi.fn(async () => "0xfinalizedhash"),
        },
      },
    };
    getApiMock.mockResolvedValue(api);

    const status = await chainStatus();

    expect(status).toEqual({ best: 105, finalized: 99 });
    expect(api.rpc.chain.getHeader).toHaveBeenCalledWith("0xfinalizedhash");
  });
});

describe("fetchLaws", () => {
  beforeEach(() => getApiMock.mockReset());

  it("returns an empty array when there are no Laws storage keys", async () => {
    const api = {
      rpc: { state: { getKeysPaged: vi.fn().mockResolvedValue([]) } },
    };
    getApiMock.mockResolvedValue(api);

    expect(await fetchLaws()).toEqual([]);
  });

  it("decodes tier / status / version / ipfsHash from the raw storage bytes", async () => {
    const lawId = 7;
    const key = fakeStorageKey(lawId);
    const ipfsBytes = new Array(32).fill(0).map((_, i) => i + 1); // 0x01..0x20
    // bytes[0]=tier(1=constitutional), bytes[1]=status(1=paused), bytes[2..6]=version u32LE(3),
    // bytes[6..38]=ipfsHash
    const valueBytes = [1, 1, ...u32LEBytes(3), ...ipfsBytes];
    const valueHex = bytesToHexStr(valueBytes);

    const api = {
      rpc: {
        state: {
          getKeysPaged: vi.fn().mockResolvedValueOnce([{ toHex: () => key }]).mockResolvedValue([]),
          queryStorageAt: vi.fn().mockResolvedValue([optionSome(valueHex)]),
        },
      },
    };
    getApiMock.mockResolvedValue(api);

    const laws = await fetchLaws();

    expect(laws).toHaveLength(1);
    expect(laws[0]).toMatchObject({
      id: `law-${lawId}`,
      tier: "constitutional",
      version: 3,
      summary: "Status: paused · v3. Fetch full text from IPFS.",
      ipfsHash: bytesToHexStr(ipfsBytes),
    });
  });

  it("skips keys whose queried value is None (deleted/absent storage)", async () => {
    const key = fakeStorageKey(1);
    const api = {
      rpc: {
        state: {
          getKeysPaged: vi.fn().mockResolvedValueOnce([{ toHex: () => key }]).mockResolvedValue([]),
          queryStorageAt: vi.fn().mockResolvedValue([optionNone()]),
        },
      },
    };
    getApiMock.mockResolvedValue(api);

    expect(await fetchLaws()).toEqual([]);
  });
});

describe("fetchProposals", () => {
  beforeEach(() => getApiMock.mockReset());

  it("joins referendum records with their tally by referendum id", async () => {
    const refId = 4;
    const refKey = fakeStorageKey(refId, 1);
    const tallyKey = fakeStorageKey(refId, 2);

    const topicHash = new Array(32).fill(9);
    const endBlock = 5000;
    // referendum bytes: [0..4)=unused, [4..36)=topicHash, [36..40)=endBlock u32LE,
    // [40]=state(0=active), [41]=tier(0=ordinary)
    const refValueBytes = [0, 0, 0, 0, ...topicHash, ...u32LEBytes(endBlock), 0, 0];
    // tally bytes: [0..4)=yes u32LE, [4..8)=no u32LE
    const tallyValueBytes = [...u32LEBytes(200), ...u32LEBytes(50)];

    const api = {
      rpc: {
        state: {
          getKeysPaged: vi
            .fn()
            .mockResolvedValueOnce([{ toHex: () => refKey }]) // Voting/Referenda (< 1000 -> loop stops)
            .mockResolvedValueOnce([{ toHex: () => tallyKey }]), // Voting/ReferendumTally
          queryStorageAt: vi.fn((keys: string[]) => {
            if (keys[0] === refKey) return Promise.resolve([optionSome(bytesToHexStr(refValueBytes))]);
            if (keys[0] === tallyKey) return Promise.resolve([optionSome(bytesToHexStr(tallyValueBytes))]);
            return Promise.resolve([]);
          }),
        },
      },
    };
    getApiMock.mockResolvedValue(api);

    const proposals = await fetchProposals();

    expect(proposals).toHaveLength(1);
    expect(proposals[0]).toMatchObject({
      id: `ref-${refId}`,
      status: "active",
      tier: "ordinary",
      votesFor: 200,
      votesAgainst: 50,
      endsAt: endBlock,
    });
  });
});
