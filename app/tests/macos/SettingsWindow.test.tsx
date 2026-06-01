// SPEC-41 §10 SettingsWindow — render + edge-case regression test
// (Vitest + React Testing Library).
//
// Mirrors tests/ios/capture-food.test.tsx. The Cluster tab embeds
// ClusterStatusDashboard, which reads the F101 useClusterPeers hook. We mock
// the hook to (a) avoid the Tauri/store machinery and (b) feed an edge-case
// peer shape — a peer with a NULL `caps` field + zeroed `last_seen_unix` — to
// guard the dashboard's `(p.caps ?? []).slice(...)` / relTime(0) paths from
// crashing when the backend returns a partial PeerSummary.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

// Edge-case peer: caps is null (not []), last_seen_unix is 0 → exercises the
// nullish-coalescing guard + the relTime "—" branch.
const edgePeers = [
  {
    peer_id: "p-edge",
    display_name: "edge-node",
    status: "Unhealthy",
    caps: null,
    last_seen_unix: 0,
  },
];

const refresh = vi.fn().mockResolvedValue(undefined);
vi.mock("../../src/hooks/useClusterPeers", () => ({
  PEER_EVENT_NAME: "cluster::peer_event",
  useClusterPeers: () => ({
    peers: edgePeers,
    status: "idle",
    error: undefined,
    lastSyncMs: 0,
    thisDeviceId: null,
    selectedPeerId: null,
    selectPeer: vi.fn(),
    refresh,
  }),
}));

import SettingsWindow from "@/screens/macos/SettingsWindow";

function makeProps() {
  return {
    onTabChange: vi.fn(),
    onOpenAddPeer: vi.fn(),
    onOpenCoachReviews: vi.fn(),
    onRestartDaemon: vi.fn(),
    onOpenDaemonLog: vi.fn(),
    onWipeAllData: vi.fn(),
    onClose: vi.fn(),
  };
}

describe("SettingsWindow screen", () => {
  beforeEach(() => refresh.mockClear());

  it("renders the screen", () => {
    render(<SettingsWindow {...makeProps()} />);
    expect(screen.getByTestId("settings-window")).toBeInTheDocument();
  });

  it("switches to the Cluster tab with an edge-case peer (null caps, no last-seen) without crashing", async () => {
    const props = makeProps();
    render(<SettingsWindow {...props} />);

    // Primary interaction: select the 叢集 (cluster) tab → fires onTabChange
    // AND mounts ClusterStatusDashboard with the partial peer shape.
    fireEvent.click(screen.getByRole("tab", { name: /叢集/ }));

    await waitFor(() =>
      expect(screen.getByTestId("cluster-status-dashboard")).toBeInTheDocument(),
    );

    // Right callback fired with the right tab id.
    expect(props.onTabChange).toHaveBeenCalledWith("cluster");
    // Screen survives the edge-case peer render (would throw on caps.slice if
    // the `?? []` guard regressed).
    expect(screen.getByTestId("settings-window")).toBeInTheDocument();
    expect(screen.getByText("edge-node")).toBeInTheDocument();
  });
});
