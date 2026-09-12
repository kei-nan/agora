// Sanitizes caught errors before they reach UI-facing state — the JS-side counterpart to the
// Rust `chain_rpc_err`/`ipfs_gateway_err`/`invalid_hex_err` helpers in
// `desktop/src-tauri/src/commands/chain.rs`. Same philosophy, same reason: raw JSON-RPC error
// text, smoldot-internal strings, `@polkadot/api` internals (including stack-carrying
// `RpcTimeoutError`s), and Tauri command failure text have no business reaching the frontend —
// full detail goes to `console.error` for local debugging, only a fixed, generic message is
// ever set into component state or rendered.
//
// Every catch block on the JS-side smoldot light-client path (`desktop/src/chain/client.ts`,
// `desktop/src/context/ChainContext.tsx`) and the QR-auth / AI-agent Tauri-command paths
// (`desktop/src/context/AuthContext.tsx`, `desktop/src/context/AgentContext.tsx`) should route
// through this instead of stringifying the caught value directly.

/** Generic fallback used when the caller doesn't supply a more specific one. */
export const DEFAULT_SAFE_ERROR_MESSAGE = "Unable to connect to the chain. Please try again.";

/**
 * Maps a caught error to a short, generic, user-facing message.
 *
 * @param err The value caught in a try/catch or `.catch()` — never assumed to be an `Error`.
 * @param logLabel A short tag prefixed to the full detail sent to `console.error` (e.g.
 *   `"[chain] connect() failed"`). Never included in the returned string.
 * @param fallback The message returned for anything that isn't specifically recognized.
 *
 * Duck-types on `name === "RpcTimeoutError"` rather than importing that class from
 * `chain/client.ts`, so this module has no dependency on the light-client module — `client.ts`
 * itself calls this helper, and importing the class back from there would create a circular
 * import between the two.
 */
export function toSafeErrorMessage(
  err: unknown,
  logLabel: string,
  fallback: string = DEFAULT_SAFE_ERROR_MESSAGE,
): string {
  console.error(logLabel, err);
  if (err instanceof Error && err.name === "RpcTimeoutError") {
    return "Request timed out. The chain connection may be stalled.";
  }
  return fallback;
}
