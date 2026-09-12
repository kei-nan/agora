import { describe, it, expect, vi, afterEach } from "vitest";
import { toSafeErrorMessage, DEFAULT_SAFE_ERROR_MESSAGE } from "./errors";

describe("toSafeErrorMessage", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("returns the default fallback and never leaks the raw error text for a generic Error", () => {
    const consoleSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    const raw = new Error("connect ECONNREFUSED 127.0.0.1:9944 — internal RPC detail");

    const message = toSafeErrorMessage(raw, "[test]");

    expect(message).toBe(DEFAULT_SAFE_ERROR_MESSAGE);
    expect(message).not.toContain("ECONNREFUSED");
    // Full detail still reaches the console for local debugging.
    expect(consoleSpy).toHaveBeenCalledWith("[test]", raw);
  });

  it("never leaks raw text for a plain string or non-Error thrown value either", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});

    expect(toSafeErrorMessage("smoldot: peer disconnected mid-request", "[test]")).toBe(
      DEFAULT_SAFE_ERROR_MESSAGE,
    );
    expect(toSafeErrorMessage({ code: -32000, message: "internal RPC error" }, "[test]")).toBe(
      DEFAULT_SAFE_ERROR_MESSAGE,
    );
  });

  it("uses a caller-supplied fallback instead of the default", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});

    expect(toSafeErrorMessage(new Error("boom"), "[test]", "Custom fallback.")).toBe(
      "Custom fallback.",
    );
  });

  it("special-cases a named RpcTimeoutError with a distinct, still-generic message", () => {
    vi.spyOn(console, "error").mockImplementation(() => {});
    const timeoutErr = new Error("state_getStorage timed out after 30000ms");
    timeoutErr.name = "RpcTimeoutError";

    const message = toSafeErrorMessage(timeoutErr, "[test]");

    expect(message).toMatch(/timed out/i);
    expect(message).not.toContain("state_getStorage");
    expect(message).not.toContain("30000ms");
  });
});
