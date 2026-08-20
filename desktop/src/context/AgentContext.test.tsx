import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, fireEvent } from "@testing-library/react";
import { AgentProvider, useAgent } from "./AgentContext";
import { invoke } from "../lib/invoke";

vi.mock("../lib/invoke", () => ({ invoke: vi.fn() }));

const mockedInvoke = vi.mocked(invoke);

// The command names CLAUDE.md and invoke.ts's own comments say are chain-write / trust-sensitive.
// AgentContext must never call any of these — the agent is documented as read-only on-chain.
const CHAIN_WRITE_COMMANDS = [
  "chain_submit_extrinsic",
  "auth_verify_nullifier",
  "auth_generate_challenge",
  "auth_poll_session",
  "auth_start_callback_server",
];

function TestHarness() {
  const { messages, isThinking, isAvailable, ask, clear } = useAgent();
  return (
    <div>
      <button onClick={() => ask("What does Article 7 change?")}>ask</button>
      <button onClick={clear}>clear</button>
      <div data-testid="thinking">{String(isThinking)}</div>
      <div data-testid="available">{String(isAvailable)}</div>
      <ul>
        {messages.map((m, i) => (
          <li key={i} data-testid={`msg-${m.role}`}>{m.content}</li>
        ))}
      </ul>
    </div>
  );
}

describe("AgentContext", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("ask() only ever calls invoke('agent_ask', ...) — never a chain-write command", async () => {
    mockedInvoke.mockResolvedValue("Article 7 lowers the quorum threshold.");
    render(
      <AgentProvider>
        <TestHarness />
      </AgentProvider>
    );

    fireEvent.click(screen.getByText("ask"));

    expect(await screen.findByTestId("msg-assistant")).toHaveTextContent(
      "Article 7 lowers the quorum threshold."
    );

    // Regression guard for the "AI agent is read-only on-chain" property from CLAUDE.md.
    expect(mockedInvoke).toHaveBeenCalledTimes(1);
    expect(mockedInvoke).toHaveBeenCalledWith(
      "agent_ask",
      expect.objectContaining({ question: "What does Article 7 change?" })
    );
    for (const writeCmd of CHAIN_WRITE_COMMANDS) {
      expect(mockedInvoke).not.toHaveBeenCalledWith(writeCmd, expect.anything());
    }
  });

  it("records the user question immediately and sets isThinking while awaiting a reply", async () => {
    let resolveInvoke!: (v: string) => void;
    mockedInvoke.mockReturnValue(
      new Promise<string>((resolve) => {
        resolveInvoke = resolve;
      }) as unknown as ReturnType<typeof invoke>
    );
    render(
      <AgentProvider>
        <TestHarness />
      </AgentProvider>
    );

    fireEvent.click(screen.getByText("ask"));

    expect(screen.getByTestId("msg-user")).toHaveTextContent("What does Article 7 change?");
    expect(screen.getByTestId("thinking")).toHaveTextContent("true");

    await act(async () => {
      resolveInvoke("done");
    });

    expect(screen.getByTestId("thinking")).toHaveTextContent("false");
  });

  it("shows an offline-specific message and flips isAvailable=false on a network error", async () => {
    mockedInvoke.mockRejectedValue(new Error("network request failed: connect ECONNREFUSED"));
    render(
      <AgentProvider>
        <TestHarness />
      </AgentProvider>
    );

    fireEvent.click(screen.getByText("ask"));

    expect(await screen.findByTestId("msg-assistant")).toHaveTextContent(/unavailable offline/i);
    expect(screen.getByTestId("available")).toHaveTextContent("false");
  });

  it("shows the raw error (and stays available) for a non-network error", async () => {
    mockedInvoke.mockRejectedValue(new Error("Claude API rate limited"));
    render(
      <AgentProvider>
        <TestHarness />
      </AgentProvider>
    );

    fireEvent.click(screen.getByText("ask"));

    expect(await screen.findByTestId("msg-assistant")).toHaveTextContent(/rate limited/);
    expect(screen.getByTestId("available")).toHaveTextContent("true");
  });

  it("clear() empties the message history", async () => {
    mockedInvoke.mockResolvedValue("some reply");
    render(
      <AgentProvider>
        <TestHarness />
      </AgentProvider>
    );

    fireEvent.click(screen.getByText("ask"));
    await screen.findByTestId("msg-assistant");

    fireEvent.click(screen.getByText("clear"));

    expect(screen.queryByTestId("msg-user")).not.toBeInTheDocument();
    expect(screen.queryByTestId("msg-assistant")).not.toBeInTheDocument();
  });
});
