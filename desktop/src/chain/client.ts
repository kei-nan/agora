// Embedded smoldot light-client connection to the local Agora dev node.
//
// This is the JS-side counterpart to CLAUDE.md's "Desktop App > Stack" note about the
// intended long-term architecture. `smoldot` and `@polkadot/api` were already listed as JS
// dependencies (not Rust ones) — that's not an accident: smoldot's primary distribution is a
// JS/WASM package, and `@polkadot/api`'s `ScProvider` is the standard, well-supported way to
// drive it. So the light client lives here, in the React frontend, not in the Tauri Rust
// backend. See `desktop/src/lib/invoke.ts` for how this plugs into the existing
// `invoke("fetch_proposals")`-style command surface the pages already call.
import { start, type Client as SmoldotClient, type Chain as SmoldotChain } from "smoldot";
import { ApiPromise, ScProvider } from "@polkadot/api";

/** Same hardcoded local dev node address the Rust backend has always used. */
export const NODE_URL = "http://127.0.0.1:9944";

/** Static asset: `agora-node build-spec --dev --raw` output, checked in with `bootNodes: []`
 * (patched in at connect time — see `discoverWsBootnode` below). Regenerate this file whenever
 * the runtime's dev genesis changes (spec_version bump, new pallet, etc.) — a stale chain spec
 * will fail smoldot's genesis-hash verification against the live node rather than silently
 * serving wrong data. */
const CHAINSPEC_URL = "/chainspecs/dev-chainspec-raw.json";

export type ChainConnState = "connecting" | "syncing" | "ready" | "error";
type StateListener = (state: ChainConnState, error?: string) => void;

const listeners = new Set<StateListener>();
let currentState: ChainConnState = "connecting";
let currentError: string | undefined;

function setState(state: ChainConnState, error?: string) {
  currentState = state;
  currentError = error;
  for (const fn of listeners) fn(state, error);
}

/** Subscribe to light-client connection state changes. Immediately fires with current state. */
export function onConnectionState(fn: StateListener): () => void {
  listeners.add(fn);
  fn(currentState, currentError);
  return () => listeners.delete(fn);
}

let smoldotClient: SmoldotClient | null = null;
function getSmoldotClient(): SmoldotClient {
  if (!smoldotClient) {
    smoldotClient = start({ maxLogLevel: 3 });
  }
  return smoldotClient;
}

/**
 * Minimal adapter satisfying the `@substrate/connect`-shaped interface `ScProvider` expects
 * (`{ WellKnownChain, createScClient }`) — backed directly by the raw `smoldot` package, since
 * `@substrate/connect` itself isn't a dependency here (and doesn't need to be: it's a thin
 * wrapper `ScProvider` was written against, and its actual shape is small enough to satisfy
 * directly). `smoldot`'s own `Chain` API surfaces responses via an async iterator
 * (`chain.jsonRpcResponses`) rather than a callback, so this adapter's only real job is
 * pumping that iterator into the callback shape `ScProvider` expects.
 */
export const ScShim = {
  // Never used — Agora is never a `WellKnownChain`, only ever connected via its own raw chain
  // spec, but `ScProvider`'s constructor asserts this is an object.
  WellKnownChain: {},
  createScClient() {
    return {
      async addChain(chainSpec: string, jsonRpcCallback: (response: string) => void) {
        const client = getSmoldotClient();
        const chain: SmoldotChain = await client.addChain({ chainSpec });
        (async () => {
          try {
            for await (const response of chain.jsonRpcResponses!) {
              jsonRpcCallback(response);
            }
          } catch {
            // Chain was removed / client terminated — stop pumping silently, `ScProvider`
            // already treats loss of the underlying chain as a disconnect.
          }
        })();
        return {
          sendJsonRpc: (rpc: string) => chain.sendJsonRpc(rpc),
          remove: () => chain.remove(),
        };
      },
      async addWellKnownChain(): Promise<never> {
        throw new Error("well-known chains unsupported — Agora only connects via its own raw chain spec");
      },
    };
  },
};

/**
 * Discovers the local dev node's actual `/ws` libp2p multiaddr via a single plain
 * `system_localListenAddresses` JSON-RPC call, so the checked-in chain spec doesn't need to
 * hardcode a peer ID — which changes every time the node restarts with a fresh `--node-key`
 * (a plain `--dev --tmp` run generates a random one each time).
 *
 * NOT a re-introduction of the trust boundary flagged on `lookup_registered_account` in
 * `commands/chain.rs`: this call discovers *where to dial*, not *what to believe*. Every
 * subsequent state read is independently verified by smoldot against finalized GRANDPA
 * consensus regardless of which peer told it this address — unlike a plain `state_getStorage`
 * call, a malicious answer here can at worst make the light client fail to find a peer, not
 * lie about chain state.
 */
export async function discoverWsBootnode(nodeUrl: string): Promise<string> {
  let res: Response;
  try {
    res = await fetch(nodeUrl, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ jsonrpc: "2.0", id: 1, method: "system_localListenAddresses", params: [] }),
    });
  } catch (e) {
    throw new Error(`chain unreachable: ${e instanceof Error ? e.message : String(e)}`);
  }
  if (!res.ok) throw new Error(`chain unreachable: HTTP ${res.status}`);
  const json = await res.json();
  if (json.error) throw new Error(`chain unreachable: ${json.error.message ?? "RPC error"}`);
  const addrs: string[] = json.result ?? [];
  const wsAddr = addrs.find((a) => a.includes("/ws/") && a.includes("127.0.0.1"));
  if (!wsAddr) {
    throw new Error(
      "local node has no /ws listen address (system_localListenAddresses returned none of the " +
      "right shape). The embedded light client can only dial WebSocket addresses from inside a " +
      "webview, not raw TCP — restart the node with an extra flag, e.g.: " +
      "agora-node --dev --tmp --listen-addr /ip4/0.0.0.0/tcp/30333/ws"
    );
  }
  return wsAddr;
}

let apiPromise: Promise<ApiPromise> | null = null;

/** Returns the shared light-client-backed `ApiPromise`, connecting on first call. */
export function getApi(): Promise<ApiPromise> {
  if (!apiPromise) {
    apiPromise = connect();
  }
  return apiPromise;
}

async function connect(): Promise<ApiPromise> {
  try {
    setState("connecting");
    const [rawSpec, bootnode] = await Promise.all([
      fetch(CHAINSPEC_URL).then((r) => {
        if (!r.ok) throw new Error(`failed to load bundled chain spec: HTTP ${r.status}`);
        return r.json();
      }),
      discoverWsBootnode(NODE_URL),
    ]);
    rawSpec.bootNodes = [bootnode];

    const provider = new ScProvider(ScShim as never, JSON.stringify(rawSpec));
    setState("syncing");
    await provider.connect();
    const api = await ApiPromise.create({ provider, throwOnConnect: true });
    await api.isReady;
    setState("ready");
    api.on("disconnected", () => setState("error", "light client disconnected"));
    api.on("error", (e: unknown) => setState("error", e instanceof Error ? e.message : String(e)));
    return api;
  } catch (err) {
    apiPromise = null; // allow a fresh connect() attempt on the next getApi() call
    const message = err instanceof Error ? err.message : String(err);
    setState("error", message);
    throw new Error(message);
  }
}
