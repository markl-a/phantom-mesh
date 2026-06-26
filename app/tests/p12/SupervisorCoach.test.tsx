import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import SupervisorCoach from "../../src/components/mobile/SupervisorCoach";

vi.mock("../../src/lib/clusterDispatch", () => ({
  clusterPost: vi.fn(async (_b: string, _s: string, path: string) =>
    path === "/rpc/review"
      ? {
          ok: true,
          status: 200,
          text: "",
          json: {
            date: "2026-06-17",
            markdown:
              "# Daily review — 2026-06-17\n**Events captured:** 1\n## fat_loss (1)\n- **food_log** (01:02): ate a salad",
          },
        }
      : { ok: false, status: 404, text: "", json: undefined },
  ),
}));
vi.mock("../../src/components/mobile/AppTemplate", async () => {
  const actual = await vi.importActual<Record<string, unknown>>(
    "../../src/components/mobile/AppTemplate",
  );
  return { ...actual, useApp: () => ({ baseUrl: "http://x", secret: "s", addLog: () => {} }) };
});

describe("SupervisorCoach (P1-2 M3)", () => {
  beforeEach(() => vi.clearAllMocks());
  it("renders the parsed daily-review rows", async () => {
    render(<SupervisorCoach />);
    await waitFor(() => expect(screen.getByText(/Events captured/i)).toBeTruthy());
    expect(screen.getByText(/ate a salad/)).toBeTruthy();
  });
});
