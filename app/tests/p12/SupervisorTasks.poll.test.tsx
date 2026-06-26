import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "@testing-library/react";
import { clusterPost } from "../../src/lib/clusterDispatch";
import SupervisorTasks from "../../src/components/mobile/SupervisorTasks";

vi.mock("../../src/lib/clusterDispatch", () => ({
  clusterPost: vi.fn(async () => ({
    ok: true,
    status: 200,
    text: "",
    json: { tasks: [], pending: [] },
  })),
}));
vi.mock("../../src/components/mobile/AppTemplate", async () => {
  const actual = await vi.importActual<Record<string, unknown>>(
    "../../src/components/mobile/AppTemplate",
  );
  return { ...actual, useApp: () => ({ baseUrl: "http://x", secret: "s", addLog: () => {} }) };
});

const calls = () => (clusterPost as unknown as { mock: { calls: unknown[] } }).mock.calls.length;

describe("SupervisorTasks auto-refresh (P1-2 M4)", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });
  afterEach(() => vi.useRealTimers());

  it("re-fetches on the poll interval", async () => {
    render(<SupervisorTasks />);
    // Settle the mount fetch (one tick) without advancing the 15s interval.
    await vi.advanceTimersByTimeAsync(0);
    const afterMount = calls();
    expect(afterMount).toBeGreaterThanOrEqual(1);

    // Crossing the 15s boundary fires the interval → at least one more fetch.
    await vi.advanceTimersByTimeAsync(15_000);
    expect(calls()).toBeGreaterThan(afterMount);
  });
});
