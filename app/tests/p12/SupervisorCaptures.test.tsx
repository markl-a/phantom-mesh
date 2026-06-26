import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import SupervisorCaptures from "../../src/components/mobile/SupervisorCaptures";

vi.mock("../../src/lib/clusterDispatch", () => ({
  clusterPost: vi.fn(async (_b: string, _s: string, path: string) =>
    path === "/rpc/captures/recent"
      ? {
          ok: true,
          status: 200,
          text: "",
          json: {
            captures: [
              { event_id: "e1", timestamp: "2026-06-17T01:02:03Z", kind: "food", tags: ["fat_loss"] },
            ],
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

describe("SupervisorCaptures (P1-2 M2)", () => {
  beforeEach(() => vi.clearAllMocks());
  it("renders a captured event row", async () => {
    render(<SupervisorCaptures />);
    await waitFor(() => expect(screen.getByText(/food/)).toBeTruthy());
    expect(screen.getByText(/fat_loss/)).toBeTruthy();
  });
});
