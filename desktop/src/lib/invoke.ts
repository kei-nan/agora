import { MOCKS } from "./mocks";
import * as chainQueries from "../chain/queries";

// Detect whether we're running inside the native Tauri window.
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

// Lazy-load the real Tauri invoke only when we're actually inside Tauri.
// This avoids the "Cannot read properties of undefined" crash in browser dev.
let _tauriInvoke: (<T>(cmd: string, args?: Record<string, unknown>) => Promise<T>) | null = null;

async function getTauriInvoke() {
  if (!_tauriInvoke) {
    const mod = await import("@tauri-apps/api/core");
    _tauriInvoke = mod.invoke;
  }
  return _tauriInvoke;
}

// Commands whose real implementation now lives client-side, backed by the embedded smoldot
// light client (`../chain/client.ts`, `../chain/queries.ts`) instead of the Tauri/reqwest RPC
// path in `desktop/src-tauri/src/commands/chain.rs`. This is the desktop app's actual chain
// connectivity as of the smoldot integration (see CLAUDE.md's "Desktop App > Stack" section) —
// `@polkadot/api` + `smoldot` were already JS-only dependencies, which is why this lives in the
// frontend rather than the Rust backend.
//
// Deliberately NOT migrated here (still Tauri/reqwest, unchanged):
//   - auth_generate_challenge / auth_poll_session / auth_start_callback_server: the QR-auth
//     flow's HTTP callback server has to run in the Rust process, not a webview.
//   - auth_verify_nullifier / chain_submit_extrinsic: session-gated or trust-sensitive paths
//     that intentionally stayed out of scope for this pass — see the long comment on
//     `lookup_registered_account` in `commands/chain.rs` for why `auth_verify_nullifier`
//     specifically still has an open trust-boundary gap this change does not close.
//   - fetch_ipfs_content: a plain HTTPS gateway fetch, not a chain read.
//   - agent_ask: calls the Claude API from the Rust backend, unrelated to chain connectivity.
const LIGHT_CLIENT_COMMANDS: Record<string, (args?: Record<string, unknown>) => Promise<unknown>> = {
  chain_status: () => chainQueries.chainStatus(),
  fetch_proposals: () => chainQueries.fetchProposals(),
  fetch_laws: () => chainQueries.fetchLaws(),
  fetch_treasury: () => chainQueries.fetchTreasury(),
  fetch_department_budgets: () => chainQueries.fetchDepartmentBudgets(),
  fetch_rulings: () => chainQueries.fetchRulings(),
  fetch_legislature_data: () => chainQueries.fetchLegislatureData(),
  fetch_elections_data: () => chainQueries.fetchElectionsData(),
  fetch_anticorruption_data: () => chainQueries.fetchAnticorruptionData(),
};

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
    const lightClientFn = LIGHT_CLIENT_COMMANDS[cmd];
    if (lightClientFn) {
      return lightClientFn(args) as Promise<T>;
    }
    const fn = await getTauriInvoke();
    return fn<T>(cmd, args);
  }

  const mock = MOCKS[cmd];
  if (!mock) throw new Error(`No mock for command "${cmd}"`);

  const result = mock(...(args ? [args] : []));
  if (result instanceof Promise) return result as Promise<T>;
  if (result instanceof Error) throw result;

  // Small artificial delay so loading states are visible in browser dev
  await new Promise((r) => setTimeout(r, 300));
  return result as T;
}
