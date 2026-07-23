import { MOCKS } from "./mocks";

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

export async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri) {
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
