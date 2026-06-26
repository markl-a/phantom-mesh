import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import SupervisorTasks from "../../src/components/mobile/SupervisorTasks";

vi.mock("../../src/lib/clusterDispatch", () => ({
  clusterPost: vi.fn(async (_b: string, _s: string, path: string) => {
    if (path === "/rpc/tasks/list")
      return {
        ok: true,
        status: 200,
        text: "",
        json: {
          tasks: [
            {
              task_id: "t1",
              agent_name: "coder",
              prompt: "fix bug",
              status: "running",
              created_at: 1718600000000,
              cost_usd: 0.02,
              turns: 3,
              error: null,
              output: null,
            },
          ],
          pending: [
            {
              approval_id: "a1",
              task_id: "t1",
              tool: "Bash",
              risk: "execute_high",
              reason: "pre-action approval",
              created_ms: 1718600001000,
            },
          ],
        },
      };
    return { ok: false, status: 404, text: "", json: undefined };
  }),
}));

// Minimal useApp() ctx mock — SupervisorTasks reads baseUrl/secret/addLog only.
vi.mock("../../src/components/mobile/AppTemplate", async () => {
  const actual = await vi.importActual<Record<string, unknown>>(
    "../../src/components/mobile/AppTemplate",
  );
  return { ...actual, useApp: () => ({ baseUrl: "http://x", secret: "s", addLog: () => {} }) };
});

describe("SupervisorTasks (P1-2 M1)", () => {
  beforeEach(() => vi.clearAllMocks());
  it("renders running task + pending approval after load", async () => {
    render(<SupervisorTasks />);
    await waitFor(() => expect(screen.getByText(/coder/)).toBeTruthy());
    expect(screen.getByText(/fix bug/)).toBeTruthy();
    expect(screen.getByText(/execute_high/)).toBeTruthy();
  });
});
