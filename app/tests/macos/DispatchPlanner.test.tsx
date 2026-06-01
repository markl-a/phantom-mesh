// SPEC-26 DispatchPlanner — render + regression test (Vitest + React Testing Library).
// Mirrors tests/ios/capture-food.test.tsx: mock the backend lib + hook, drive the
// primary interaction (select a cap → 規劃派工), and assert the screen survives an
// edge/rejected planDispatch result without crashing (testid still present).
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

// Hook supplies the peers list; the plan button is disabled when peers is empty,
// so we must return at least one peer to exercise the primary interaction.
const useClusterPeers = vi.fn();
vi.mock("@/hooks/useClusterPeers", () => ({
  useClusterPeers: (...args: unknown[]) => useClusterPeers(...args),
}));

const planDispatch = vi.fn();
vi.mock("@/lib/clusterDispatchPlan", () => ({
  CAP_OPTIONS: ["cargo", "gpu", "git"],
  buildTask: (slugs: string[]) => ({ requiredCaps: slugs }),
  peerToCaps: (p: { peer_id: string }) => ({ peerId: p.peer_id, tags: [] }),
  planDispatch: (...args: unknown[]) => planDispatch(...args),
  describeDispatchError: (e: unknown) => String(e),
}));

import DispatchPlanner from "@/screens/macos/DispatchPlanner";

const PEER = {
  peer_id: "peer-1",
  display_name: "node-a",
  caps: ["cargo"],
  status: "Online" as const,
  last_seen_unix: 0,
};

describe("DispatchPlanner screen", () => {
  beforeEach(() => {
    useClusterPeers.mockReset();
    planDispatch.mockReset();
    useClusterPeers.mockReturnValue({ peers: [PEER] });
  });

  it("renders the screen", () => {
    render(<DispatchPlanner />);
    expect(screen.getByTestId("dispatch-planner")).toBeInTheDocument();
  });

  it("does NOT crash when planDispatch returns a partial plan (null fallbackPeerIds)", async () => {
    // Edge: a plan with selectedPeerId set but fallbackPeerIds null + scoringReason
    // null — the screen guards these with `?? []` / truthy checks, must not throw.
    planDispatch.mockResolvedValue({
      selectedPeerId: "peer-1",
      fallbackPeerIds: null,
      scoringReason: null,
    });
    render(<DispatchPlanner />);
    // Primary interaction: pick a required cap, then run the planner.
    fireEvent.click(screen.getByRole("button", { name: "cargo" }));
    fireEvent.click(screen.getByRole("button", { name: /規劃派工/ }));
    await waitFor(() => expect(planDispatch).toHaveBeenCalled());
    // Screen survives rendering the partial plan + shows the selected peer name.
    expect(screen.getByTestId("dispatch-planner")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("node-a")).toBeInTheDocument());
  });

  it("does NOT crash when planDispatch rejects (shows error, testid intact)", async () => {
    planDispatch.mockRejectedValue(new Error("NoMatchingPeer"));
    render(<DispatchPlanner />);
    fireEvent.click(screen.getByRole("button", { name: "gpu" }));
    fireEvent.click(screen.getByRole("button", { name: /規劃派工/ }));
    await waitFor(() => expect(planDispatch).toHaveBeenCalled());
    expect(screen.getByTestId("dispatch-planner")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByText(/NoMatchingPeer/)).toBeInTheDocument(),
    );
  });
});
