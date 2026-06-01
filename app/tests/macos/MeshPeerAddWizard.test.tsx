// SPEC-41 §10.12 — MeshPeerAddWizard render + state-machine regression test
// (Vitest + React Testing Library). Mirrors tests/ios/capture-food.test.tsx.
//
// Coverage:
//  (a) renders the screen (data-testid present).
//  (b) edge/primary interaction: a 5s mDNS scan that surfaces 0 peers must
//      auto-switch to the QR fallback (§10.12) WITHOUT crashing — and a scan
//      that DOES find a peer must reach the list and let "邀請加入" fire the
//      local invite state machine.
import { render, screen, fireEvent, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { vi, describe, it, expect, beforeEach, afterEach } from "vitest";

// useClusterPeers is the only non-router lib the screen pulls in. Mock it so
// we control the discovered-peer list and the refresh() promise.
const refresh = vi.fn(() => Promise.resolve());
let mockPeers: { peer_id: string; display_name: string }[] = [];
vi.mock("../../src/hooks/useClusterPeers", () => ({
  useClusterPeers: () => ({ peers: mockPeers, refresh }),
}));

import MeshPeerAddWizard from "../../src/screens/macos/MeshPeerAddWizard";

function renderWizard() {
  return render(
    <MemoryRouter>
      <MeshPeerAddWizard />
    </MemoryRouter>,
  );
}

describe("MeshPeerAddWizard screen", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    refresh.mockClear();
    mockPeers = [];
  });
  afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
  });

  it("renders the screen", () => {
    renderWizard();
    expect(screen.getByTestId("mesh-peer-add-wizard")).toBeInTheDocument();
    // Starts in the scanning state.
    expect(screen.getByTestId("mesh-peer-add-wizard")).toHaveAttribute("data-state", "scanning");
  });

  it("auto-switches to the QR fallback when a 5s scan finds 0 peers (edge, §10.12)", () => {
    mockPeers = []; // nothing nearby
    renderWizard();
    expect(refresh).toHaveBeenCalled();
    // Drive the 5s mDNS scan window to completion.
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    // Screen survives the empty-scan branch and falls back to QR (§10.12).
    const root = screen.getByTestId("mesh-peer-add-wizard");
    expect(root).toBeInTheDocument();
    expect(root).toHaveAttribute("data-state", "qr");
    expect(screen.getByTestId("wizard-qr")).toBeInTheDocument();
  });

  it("reaches the list and fires the invite state machine when a peer is found", async () => {
    mockPeers = [{ peer_id: "p-42", display_name: "Studio Mac" }];
    renderWizard();
    act(() => {
      vi.advanceTimersByTime(5000);
    });
    const root = screen.getByTestId("mesh-peer-add-wizard");
    expect(root).toHaveAttribute("data-state", "list");
    // Primary interaction: invite the discovered peer.
    fireEvent.click(screen.getByText("邀請加入"));
    expect(root).toHaveAttribute("data-state", "invited_waiting");
    expect(screen.getByTestId("wizard-waiting")).toBeInTheDocument();
    // The local state machine advances to "joined" after 1.8s (no crash).
    act(() => {
      vi.advanceTimersByTime(1800);
    });
    expect(root).toHaveAttribute("data-state", "joined");
    expect(screen.getByTestId("mesh-peer-add-wizard")).toBeInTheDocument();
  });
});
