// SPEC-31 mesh-peer-list — render + malformed-peer regression test
// (Vitest + React Testing Library). Guards the white-screen class of bug:
// a partial/unwired PeerSummary missing optional fields (`caps`, `status`,
// `display_name`) must NOT crash the peer list via unguarded `.map`/`.slice`/
// property access.
import { render, screen } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

// tauri-compat is pulled in transitively by the useClusterPeers hook chain;
// stub it so nothing tries to reach a Tauri backend under jsdom.
vi.mock("@/lib/tauri-compat", () => ({
  isTauri: () => false,
  safeInvoke: vi.fn(async () => []),
}));

// Mock the data hook (peer loader) so we control the peer list shape.
const useClusterPeers = vi.fn();
vi.mock("@/hooks/useClusterPeers", () => ({
  useClusterPeers: () => useClusterPeers(),
}));

import MeshPeerList from "@/pages/mesh-peer-list";

function hookResult(overrides: Record<string, unknown> = {}) {
  return {
    peers: [],
    status: "idle",
    error: undefined,
    lastSyncMs: 0,
    thisDeviceId: null,
    selectedPeerId: null,
    selectPeer: vi.fn(),
    refresh: vi.fn(async () => {}),
    ...overrides,
  };
}

describe("mesh-peer-list screen", () => {
  beforeEach(() => useClusterPeers.mockReset());

  it("renders the screen", () => {
    useClusterPeers.mockReturnValue(hookResult({ peers: [] }));
    render(<MeshPeerList onAddPeer={vi.fn()} />);
    expect(screen.getByTestId("mesh-peer-list")).toBeInTheDocument();
  });

  it("does NOT crash when a peer is missing optional fields (caps/status/display_name)", () => {
    // The bug class: a partial/unwired PeerSummary omits optional fields →
    // e.g. `p.caps.slice(...)` or `p.caps.map(...)` throws on undefined and
    // white-screens the list. Only `peer_id` is guaranteed present here.
    const malformedPeer = { peer_id: "peer-a", last_seen_unix: 0 };
    useClusterPeers.mockReturnValue(
      hookResult({
        peers: [malformedPeer as unknown as Record<string, unknown>],
        status: "idle",
        thisDeviceId: "user42",
      }),
    );
    render(<MeshPeerList onAddPeer={vi.fn()} />);
    // would have thrown during render pre-guard; survives and stays mounted
    expect(screen.getByTestId("mesh-peer-list")).toBeInTheDocument();
    // the peer still renders (falls back to peer_id when display_name absent)
    expect(screen.getByText("peer-a")).toBeInTheDocument();
  });
});
