import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// The 9 command names invoke.ts documents as routed to the embedded light client instead of
// Tauri IPC (see the LIGHT_CLIENT_COMMANDS comment in src/lib/invoke.ts, and CLAUDE.md's
// "Desktop App > Stack" section, which enumerates the same 9 names).
const LIGHT_CLIENT_COMMAND_NAMES = [
  "chain_status",
  "fetch_proposals",
  "fetch_laws",
  "fetch_treasury",
  "fetch_department_budgets",
  "fetch_rulings",
  "fetch_legislature_data",
  "fetch_elections_data",
  "fetch_anticorruption_data",
];

// Commands that CLAUDE.md/invoke.ts explicitly say stayed on the Tauri/reqwest path.
const TAURI_ONLY_COMMAND_NAMES = [
  "auth_generate_challenge",
  "auth_poll_session",
  "auth_start_callback_server",
  "auth_verify_nullifier",
  "chain_submit_extrinsic",
  "fetch_ipfs_content",
  "agent_ask",
];

describe("invoke() command routing", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
    vi.doUnmock("../chain/queries");
    vi.doUnmock("@tauri-apps/api/core");
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
  });

  describe("inside a Tauri window", () => {
    beforeEach(() => {
      (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    });

    it.each(LIGHT_CLIENT_COMMAND_NAMES)(
      "routes %s to the light-client query module, not Tauri IPC",
      async (cmd) => {
        vi.resetModules();
        const queryFns = {
          chainStatus: vi.fn().mockResolvedValue({ best: 1, finalized: 1 }),
          fetchProposals: vi.fn().mockResolvedValue([]),
          fetchLaws: vi.fn().mockResolvedValue([]),
          fetchTreasury: vi.fn().mockResolvedValue([]),
          fetchDepartmentBudgets: vi.fn().mockResolvedValue([]),
          fetchRulings: vi.fn().mockResolvedValue([]),
          fetchLegislatureData: vi.fn().mockResolvedValue({ members: [], motions: [] }),
          fetchElectionsData: vi.fn().mockResolvedValue({ delegates: [] }),
          fetchAnticorruptionData: vi.fn().mockResolvedValue({
            assetDisclosures: [],
            conflicts: [],
            reports: [],
            investigatorCount: 0,
          }),
        };
        vi.doMock("../chain/queries", () => queryFns);
        const tauriInvoke = vi.fn().mockResolvedValue("should not be called");
        vi.doMock("@tauri-apps/api/core", () => ({ invoke: tauriInvoke }));

        const { invoke } = await import("./invoke");
        await invoke(cmd);

        // Exactly one of the 9 query functions should have fired, and Tauri IPC never should.
        const totalCalls = Object.values(queryFns).reduce((n, fn) => n + fn.mock.calls.length, 0);
        expect(totalCalls).toBe(1);
        expect(tauriInvoke).not.toHaveBeenCalled();
      }
    );

    it.each(TAURI_ONLY_COMMAND_NAMES)(
      "falls through %s to Tauri IPC, not the light client",
      async (cmd) => {
        vi.resetModules();
        const queryFns = {
          chainStatus: vi.fn(),
          fetchProposals: vi.fn(),
          fetchLaws: vi.fn(),
          fetchTreasury: vi.fn(),
          fetchDepartmentBudgets: vi.fn(),
          fetchRulings: vi.fn(),
          fetchLegislatureData: vi.fn(),
          fetchElectionsData: vi.fn(),
          fetchAnticorruptionData: vi.fn(),
        };
        vi.doMock("../chain/queries", () => queryFns);
        const tauriInvoke = vi.fn().mockResolvedValue("ok");
        vi.doMock("@tauri-apps/api/core", () => ({ invoke: tauriInvoke }));

        const { invoke } = await import("./invoke");
        const args = { foo: "bar" };
        await invoke(cmd, args);

        expect(tauriInvoke).toHaveBeenCalledWith(cmd, args);
        for (const fn of Object.values(queryFns)) {
          expect(fn).not.toHaveBeenCalled();
        }
      }
    );

    it("passes args through to the underlying Tauri invoke for unknown commands", async () => {
      vi.doMock("../chain/queries", () => ({}));
      const tauriInvoke = vi.fn().mockResolvedValue(42);
      vi.doMock("@tauri-apps/api/core", () => ({ invoke: tauriInvoke }));

      const { invoke } = await import("./invoke");
      const result = await invoke<number>("some_future_command", { a: 1 });

      expect(result).toBe(42);
      expect(tauriInvoke).toHaveBeenCalledWith("some_future_command", { a: 1 });
    });
  });

  describe("outside Tauri (browser dev mode)", () => {
    it("uses the mock table and never touches Tauri IPC or the light client", async () => {
      // No __TAURI_INTERNALS__ on window in this block.
      const queryFns = { chainStatus: vi.fn() };
      vi.doMock("../chain/queries", () => queryFns);
      const tauriInvoke = vi.fn();
      vi.doMock("@tauri-apps/api/core", () => ({ invoke: tauriInvoke }));

      const { invoke } = await import("./invoke");
      const result = await invoke<{ best: number; finalized: number }>("chain_status");

      expect(result).toEqual(expect.objectContaining({ best: expect.any(Number) }));
      expect(tauriInvoke).not.toHaveBeenCalled();
      expect(queryFns.chainStatus).not.toHaveBeenCalled();
    });

    it("throws for a command with no registered mock", async () => {
      vi.doMock("../chain/queries", () => ({}));
      vi.doMock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

      const { invoke } = await import("./invoke");
      await expect(invoke("totally_unknown_command")).rejects.toThrow(
        /No mock for command/
      );
    });
  });
});
