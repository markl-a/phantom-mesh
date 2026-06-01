// SPEC-41 §10.7 — S7 ClusterStatusDashboard render + regression test
// (Vitest + React Testing Library).
//
// Mirrors tests/ios/capture-food.test.tsx: a render smoke test plus an
// edge-input test that asserts the screen survives a degraded backend.
//
// ClusterStatusDashboard takes NO props — it pulls everything from the
// F101 `useClusterPeers` hook — so we mock that hook directly. The primary
// interaction is the "重新整理" (refresh) button, which calls `refresh()`.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import type { PeerSummary } from "@/stores/clusterPeersStore";

const refresh = vi.fn();
const selectPeer = vi.fn();

let mockState: {
  peers: PeerSummary[];
  status: "idle" | "loading" | "error";
  error?: string;
  lastSyncMs: number;
  thisDeviceId: string | null;
};

vi.mock("@/hooks/useClusterPeers", () => ({
  useClusterPeers: () => ({
    ...mockState,
    selectedPeerId: null,
    selectPeer,
    refresh: (...args: unknown[]) => refresh(...args),
  }),
}));

import ClusterStatusDashboard from "@/screens/macos/ClusterStatusDashboard";

beforeEach(() => {
  refresh.mockReset();
  selectPeer.mockReset();
  mockState = {
    peers: [],
    status: "idle",
    error: undefined,
    lastSyncMs: 0,
    thisDeviceId: null,
  };
});

describe("ClusterStatusDashboard screen", () => {
  it("renders the screen (isolated / no peers)", () => {
    render(<ClusterStatusDashboard />);
    expect(screen.getByTestId("cluster-status-dashboard")).toBeInTheDocument();
  });

  it("does NOT crash on an edge peer row missing `caps` + a rejected refresh", async () => {
    // Edge input: a partial PeerSummary whose `caps` is null (unwired backend).
    // The screen does `(p.caps ?? []).slice(...)` so it must survive — and the
    // refresh button click must fire `refresh()` even when it rejects.
    refresh.mockRejectedValue(new Error("broker unreachable"));
    mockState.peers = [
      // `caps` intentionally null to stress the `?? []` guard.
      { peer_id: "p1", display_name: "node-a", caps: null as unknown as string[], status: "Unhealthy", last_seen_unix: 0 },
    ];
    mockState.status = "error";
    mockState.error = "broker unreachable";

    render(<ClusterStatusDashboard />);
    // The degraded peer row + error banner rendered without throwing.
    expect(screen.getByText("node-a")).toBeInTheDocument();

    // Primary interaction: click the refresh button.
    fireEvent.click(screen.getByRole("button", { name: /重新整理/ }));
    await waitFor(() => expect(refresh).toHaveBeenCalled());

    // Screen survives the rejected refresh (testid still present).
    expect(screen.getByTestId("cluster-status-dashboard")).toBeInTheDocument();
  });
});
