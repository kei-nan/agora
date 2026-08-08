/**
 * WsProvider + ApiPromise singleton for the Agora chain, for the committee-duty app.
 *
 * This is a deliberate near-duplicate of `mobile/src/chain/api.ts`, not an import from
 * it. Per `docs/project/changelog/082.md` entry 82, this app is intentionally kept out
 * of the citizen-facing app's dependency graph so the OPRF secret-share attack surface
 * stays isolated — sharing this module via a workspace/package dependency would put
 * `agora-committee`'s build back in the same graph as `agora-mobile`'s. The duplication
 * is a few dozen lines; the isolation is the point of this whole app existing as a
 * separate package (see this repo's root `committee/package.json` description and the
 * final task report for the full rationale).
 *
 * Requires the polyfills wired up in index.js (Buffer, crypto.getRandomValues,
 * TextEncoder/TextDecoder, wasm-crypto asm.js fallback) to have run first, exactly as
 * mobile/index.js documents for the same reasons.
 */
import { ApiPromise, WsProvider } from '@polkadot/api';

/**
 * Chain RPC endpoint. See `mobile/src/chain/api.ts`'s doc comment for the
 * emulator/simulator/physical-device address conventions this mirrors.
 */
export const NODE_WS_URL = 'ws://10.0.2.2:9944';

let _apiPromise: Promise<ApiPromise> | null = null;

/**
 * Returns a cached, ready ApiPromise connected to NODE_WS_URL. Concurrent callers
 * share the same in-flight connection attempt. If connecting fails, the failure is not
 * cached — the next call starts a fresh attempt.
 *
 * Throws in a release build (`!__DEV__`) if `NODE_WS_URL` is still an unencrypted
 * `ws://` endpoint — same rationale as mobile's `getApi`: this is a single hardcoded
 * dev-chain constant, not a per-environment config, and this at least fails loudly
 * instead of silently shipping plaintext RPC in a build whose committee members are
 * submitting OPRF evaluation material.
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
