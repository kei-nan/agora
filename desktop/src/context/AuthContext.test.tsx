import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import { AuthProvider, useAuth } from "./AuthContext";
import { invoke } from "../lib/invoke";

vi.mock("../lib/invoke", () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);

function TestHarness() {
  const { session, qrChallenge, qrError, isGeneratingQr, requestQr, logout } = useAuth();
  return (
    <div>
      <button onClick={() => requestQr()}>requestQr</button>
      <button onClick={() => logout()}>logout</button>
      <div data-testid="session">{session ? session.nullifierHash : "none"}</div>
      <div data-testid="challenge">{qrChallenge ?? "none"}</div>
      <div data-testid="error">{qrError ?? "none"}</div>
      <div data-testid="generating">{String(isGeneratingQr)}</div>
    </div>
  );
}

describe("AuthContext", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  it("starts unauthenticated, with no session", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    render(
      <AuthProvider>
        <TestHarness />
      </AuthProvider>
    );
    expect(screen.getByTestId("session")).toHaveTextContent("none");
  });

  it("starts the local callback server once on mount", async () => {
    mockedInvoke.mockResolvedValue(undefined);
    render(
      <AuthProvider>
        <TestHarness />
      </AuthProvider>
    );
    expect(mockedInvoke).toHaveBeenCalledWith(
      "auth_start_callback_server",
      expect.objectContaining({ port: expect.any(Number) })
    );
  });

  it("requestQr -> auth_poll_session returning a session -> verifies nullifier -> sets session", async () => {
    const session = { nullifierHash: "0xabc", expiresAt: Math.floor(Date.now() / 1000) + 3600, token: "tok" };
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "auth_start_callback_server") return undefined;
      if (cmd === "auth_generate_challenge") return "challenge-123";
      if (cmd === "auth_poll_session") return session;
      if (cmd === "auth_verify_nullifier") return true;
      throw new Error(`unexpected command ${cmd}`);
    });

    render(
      <AuthProvider>
        <TestHarness />
      </AuthProvider>
    );

    await act(async () => {
      fireEvent.click(screen.getByText("requestQr"));
    });
    expect(screen.getByTestId("challenge")).toHaveTextContent("challenge-123");

    // Advance past the 2s poll interval so the setInterval callback fires.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });

    expect(screen.getByTestId("session")).toHaveTextContent("0xabc");
    expect(screen.getByTestId("challenge")).toHaveTextContent("none");
    expect(mockedInvoke).toHaveBeenCalledWith("auth_verify_nullifier", { nullifierHex: "0xabc" });
  });

  it("rejects a session whose nullifier is not registered on-chain", async () => {
    const session = { nullifierHash: "0xdead", expiresAt: Math.floor(Date.now() / 1000) + 3600, token: "tok" };
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "auth_start_callback_server") return undefined;
      if (cmd === "auth_generate_challenge") return "challenge-xyz";
      if (cmd === "auth_poll_session") return session;
      if (cmd === "auth_verify_nullifier") return false;
      throw new Error(`unexpected command ${cmd}`);
    });

    render(
      <AuthProvider>
        <TestHarness />
      </AuthProvider>
    );

    await act(async () => {
      fireEvent.click(screen.getByText("requestQr"));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000);
    });

    expect(screen.getByTestId("session")).toHaveTextContent("none");
    expect(screen.getByTestId("error")).toHaveTextContent(/not registered on-chain/);
  });

  it("expires the QR challenge client-side after the 5-minute timeout", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "auth_start_callback_server") return undefined;
      if (cmd === "auth_generate_challenge") return "challenge-timeout";
      if (cmd === "auth_poll_session") throw new Error("pending");
      throw new Error(`unexpected command ${cmd}`);
    });

    render(
      <AuthProvider>
        <TestHarness />
      </AuthProvider>
    );

    await act(async () => {
      fireEvent.click(screen.getByText("requestQr"));
    });
    expect(screen.getByTestId("challenge")).toHaveTextContent("challenge-timeout");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(5 * 60 * 1000);
    });

    expect(screen.getByTestId("challenge")).toHaveTextContent("none");
    expect(screen.getByTestId("error")).toHaveTextContent(/expired after 5 minutes/);
  });

  it("clears an already-authenticated session once its expiresAt passes (client-side awareness of expiry)", async () => {
    mockedInvoke.mockResolvedValue(undefined);

    function Harness() {
      const { session, requestQr } = useAuth();
      return (
        <div>
          <div data-testid="session">{session ? "authed" : "none"}</div>
          <button onClick={() => requestQr()}>go</button>
        </div>
      );
    }

    // Directly exercise the expiry-effect: set up a session that expires in 3 seconds
    // by driving through the requestQr -> poll -> verify path with a short-lived session.
    const shortSession = { nullifierHash: "0x1", expiresAt: Math.floor(Date.now() / 1000) + 3, token: "t" };
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "auth_start_callback_server") return undefined;
      if (cmd === "auth_generate_challenge") return "c1";
      if (cmd === "auth_poll_session") return shortSession;
      if (cmd === "auth_verify_nullifier") return true;
      throw new Error(`unexpected command ${cmd}`);
    });

    render(
      <AuthProvider>
        <Harness />
      </AuthProvider>
    );

    await act(async () => {
      fireEvent.click(screen.getByText("go"));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2000); // trigger the poll
    });
    expect(screen.getByTestId("session")).toHaveTextContent("authed");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(3100); // past expiresAt
    });
    expect(screen.getByTestId("session")).toHaveTextContent("none");
  });

  it("logout() clears session and any in-flight QR state", async () => {
    mockedInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "auth_start_callback_server") return undefined;
      if (cmd === "auth_generate_challenge") return "c2";
      if (cmd === "auth_poll_session") throw new Error("pending");
      throw new Error(`unexpected command ${cmd}`);
    });

    render(
      <AuthProvider>
        <TestHarness />
      </AuthProvider>
    );

    await act(async () => {
      fireEvent.click(screen.getByText("requestQr"));
    });
    expect(screen.getByTestId("challenge")).toHaveTextContent("c2");

    fireEvent.click(screen.getByText("logout"));

    expect(screen.getByTestId("session")).toHaveTextContent("none");
    expect(screen.getByTestId("challenge")).toHaveTextContent("none");
    expect(screen.getByTestId("error")).toHaveTextContent("none");
  });
});
