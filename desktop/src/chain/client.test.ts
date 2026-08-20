import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// smoldot's `start()` spins up a real WASM worker — stub it out so these tests only exercise
// the adapter logic this codebase wrote (ScShim), not smoldot's internals.
const addChainMock = vi.fn();
vi.mock("smoldot", () => ({
  start: vi.fn(() => ({ addChain: addChainMock })),
}));

import { ScShim, discoverWsBootnode } from "./client";

function asyncIterableFromArray(items: string[]) {
  return {
    async *[Symbol.asyncIterator]() {
      for (const item of items) {
        yield item;
      }
    },
  };
}

describe("ScShim (smoldot -> ScProvider adapter)", () => {
  beforeEach(() => {
    addChainMock.mockReset();
  });

  it("pumps chain.jsonRpcResponses into the ScProvider callback, in order", async () => {
    const responses = ['{"id":1,"result":"a"}', '{"id":2,"result":"b"}', '{"id":3,"result":"c"}'];
    const fakeChain = {
      jsonRpcResponses: asyncIterableFromArray(responses),
      sendJsonRpc: vi.fn(),
      remove: vi.fn(),
    };
    addChainMock.mockResolvedValue(fakeChain);

    const received: string[] = [];
    const client = ScShim.createScClient();
    const handle = await client.addChain("{}", (response: string) => {
      received.push(response);
    });

    // The pump loop runs in a detached async IIFE — flush microtasks so it can drain the
    // (short, finite) async iterable before we assert on it.
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(received).toEqual(responses);
    expect(handle).toBeDefined();
  });

  it("delegates sendJsonRpc and remove to the underlying smoldot chain", async () => {
    const fakeChain = {
      jsonRpcResponses: asyncIterableFromArray([]),
      sendJsonRpc: vi.fn(),
      remove: vi.fn(),
    };
    addChainMock.mockResolvedValue(fakeChain);

    const client = ScShim.createScClient();
    const handle = await client.addChain("{}", () => {});

    handle.sendJsonRpc('{"id":1,"method":"chain_getHeader"}');
    expect(fakeChain.sendJsonRpc).toHaveBeenCalledWith('{"id":1,"method":"chain_getHeader"}');

    handle.remove();
    expect(fakeChain.remove).toHaveBeenCalledTimes(1);
  });

  it("passes the chain spec through to smoldot's addChain", async () => {
    const fakeChain = { jsonRpcResponses: asyncIterableFromArray([]), sendJsonRpc: vi.fn(), remove: vi.fn() };
    addChainMock.mockResolvedValue(fakeChain);

    const spec = JSON.stringify({ name: "agora-dev" });
    await ScShim.createScClient().addChain(spec, () => {});

    expect(addChainMock).toHaveBeenCalledWith({ chainSpec: spec });
  });

  it("silently stops pumping if the underlying iterator throws (chain removed)", async () => {
    const throwingIterable = {
      async *[Symbol.asyncIterator]() {
        yield '{"id":1}';
        throw new Error("chain removed");
      },
    };
    const fakeChain = { jsonRpcResponses: throwingIterable, sendJsonRpc: vi.fn(), remove: vi.fn() };
    addChainMock.mockResolvedValue(fakeChain);

    const received: string[] = [];
    // Should not reject / throw an unhandled rejection — the pump loop catches internally.
    await expect(
      ScShim.createScClient().addChain("{}", (r: string) => received.push(r))
    ).resolves.toBeDefined();

    await new Promise((r) => setTimeout(r, 0));
    expect(received).toEqual(['{"id":1}']);
  });

  it("addWellKnownChain always rejects — Agora is never a well-known chain", async () => {
    await expect(ScShim.createScClient().addWellKnownChain()).rejects.toThrow(
      /well-known chains unsupported/
    );
  });
});

describe("discoverWsBootnode", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("picks the local loopback /ws multiaddr out of system_localListenAddresses", async () => {
    const addrs = [
      "/ip4/127.0.0.1/tcp/30333/ws/p2p/12D3KooWabc",
      "/ip4/192.168.1.5/tcp/30333/ws/p2p/12D3KooWabc",
      "/ip4/127.0.0.1/tcp/30334/p2p/12D3KooWabc",
    ];
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ jsonrpc: "2.0", id: 1, result: addrs }),
    });
    vi.stubGlobal("fetch", fetchMock);

    const bootnode = await discoverWsBootnode("http://127.0.0.1:9944");

    expect(bootnode).toBe("/ip4/127.0.0.1/tcp/30333/ws/p2p/12D3KooWabc");
    expect(fetchMock).toHaveBeenCalledWith(
      "http://127.0.0.1:9944",
      expect.objectContaining({
        method: "POST",
        body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "system_localListenAddresses", params: [] }),
      })
    );
  });

  it("throws a helpful error when no /ws loopback address is advertised", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ jsonrpc: "2.0", id: 1, result: ["/ip4/127.0.0.1/tcp/30333/p2p/12D3KooWabc"] }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(discoverWsBootnode("http://127.0.0.1:9944")).rejects.toThrow(
      /no \/ws listen address/
    );
  });

  it("throws when the node is unreachable (network error)", async () => {
    const fetchMock = vi.fn().mockRejectedValue(new TypeError("fetch failed"));
    vi.stubGlobal("fetch", fetchMock);

    await expect(discoverWsBootnode("http://127.0.0.1:9944")).rejects.toThrow(/chain unreachable/);
  });

  it("throws on a non-2xx HTTP response", async () => {
    const fetchMock = vi.fn().mockResolvedValue({ ok: false, status: 500 });
    vi.stubGlobal("fetch", fetchMock);

    await expect(discoverWsBootnode("http://127.0.0.1:9944")).rejects.toThrow(/HTTP 500/);
  });

  it("throws when the RPC response itself carries a JSON-RPC error", async () => {
    const fetchMock = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ jsonrpc: "2.0", id: 1, error: { message: "method not found" } }),
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(discoverWsBootnode("http://127.0.0.1:9944")).rejects.toThrow(/method not found/);
  });
});
