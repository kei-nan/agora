/**
 * WsProvider + ApiPromise singleton for the Agora chain.
 *
 * Requires the polyfills wired up in index.js (Buffer, crypto.getRandomValues,
 * TextEncoder/TextDecoder, wasm-crypto asm.js fallback) to have run first —
 * they're imported at the very top of index.js, before this module (or
 * anything that transitively imports @polkadot/api) is ever loaded.
 */
import { ApiPromise, WsProvider } from '@polkadot/api';

/**
 * Chain RPC endpoint. Change this for your environment:
 *  - Android emulator (default below): 10.0.2.2 is the documented alias the
 *    AVD provides for the host machine's loopback interface, so this reaches
 *    a `--dev` node running on your dev machine at localhost:9944.
 *  - iOS Simulator: use 'ws://localhost:9944' — the simulator shares the
 *    host's network namespace directly.
 *  - Physical device: use the dev machine's LAN IP (e.g. 'ws://192.168.1.23:9944')
 *    and start the node with an RPC flag that accepts non-localhost
 *    connections (e.g. --rpc-external, plus --rpc-cors=all for dev).
 */
export const NODE_WS_URL = 'ws://10.0.2.2:9944';

let _apiPromise: Promise<ApiPromise> | null = null;

/**
 * Returns a cached, ready ApiPromise connected to NODE_WS_URL. Concurrent
 * callers share the same in-flight connection attempt. If connecting fails,
 * the failure is not cached — the next call starts a fresh attempt.
 *
 * Throws in a release build (`!__DEV__`) if `NODE_WS_URL` is still an
 * unencrypted `ws://` endpoint. There is no build-flavor config layer here —
 * `NODE_WS_URL` is a single hardcoded constant meant for a `--dev` chain on
 * an emulator/local network, and nothing else in this codebase would stop a
 * release build from shipping with it unchanged, silently sending every
 * submitted extrinsic and query response over a plaintext, on-path-
 * interceptable connection. Failing loudly here at least turns that into a
 * build-time surprise instead of a silent one; a real fix is a `wss://`
 * endpoint behind an actual per-environment config, not a workaround here.
 */
export async function getApi(): Promise<ApiPromise> {
  if (!__DEV__ && NODE_WS_URL.startsWith('ws://')) {
    throw new Error(
      `getApi: refusing to connect to unencrypted ${NODE_WS_URL} in a release build. ` +
        'NODE_WS_URL must be a wss:// endpoint outside of development.',
    );
  }
  if (!_apiPromise) {
    _apiPromise = (async () => {
      const provider = new WsProvider(NODE_WS_URL);
      const api = await ApiPromise.create({ provider });
      await api.isReady;
      return api;
    })();
  }
  try {
    return await _apiPromise;
  } catch (e) {
    _apiPromise = null;
    throw e;
  }
}

/** Disconnects the shared API instance, if one has been created. Safe to call repeatedly. */
export async function disconnect(): Promise<void> {
  if (!_apiPromise) return;
  const pending = _apiPromise;
  _apiPromise = null;
  const api = await pending.catch(() => null);
  if (api) await api.disconnect();
}
