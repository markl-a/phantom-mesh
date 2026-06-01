// SPEC-41 §10.2 — S1 MenuBarDropdown render + interaction test (Vitest + RTL).
// Mirrors tests/ios/capture-food.test.tsx: render guard + primary-interaction guard
// against an edge input (peers hook returning a peer with an unexpected status
// shape) — the screen must not crash and the invoke callback must fire.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

// Control the live cluster-peers hook the screen reads. Default: empty array
// (matches the hook's "array always" contract). Individual tests override.
const peersRef: { current: unknown[] } = { current: [] };
vi.mock("@/hooks/useClusterPeers", () => ({
  useClusterPeers: () => ({ peers: peersRef.current }),
}));

import MenuBarDropdown from "@/screens/macos/MenuBarDropdown";
import type { MenuBarState } from "@/screens/macos/types";

const baseState: MenuBarState = {
  peerAliveCount: 2,
  daemonRunning: true,
  todayEventCount: 5,
  coachPending: true,
  onboarded: true,
};

describe("MenuBarDropdown screen", () => {
  beforeEach(() => {
    peersRef.current = [];
  });

  it("renders the screen", () => {
    render(<MenuBarDropdown screenId="s1" state={baseState} />);
    expect(screen.getByTestId("menu-bar-dropdown")).toBeInTheDocument();
  });

  it("invokes onItemInvoke + onClose on an enabled item click and survives an edge peer status", async () => {
    // Edge input: a peer carrying an unexpected/partial status the screen does
    // not branch on. `peers.filter(p => p.status === "Online")` must tolerate it
    // (count 0) and the screen must still render every row without throwing.
    peersRef.current = [
      { peer_id: "p1", display_name: "alpha", caps: [], status: "Bogus", last_seen_unix: 0 },
      { peer_id: "p2" /* partial: missing status entirely */ },
    ];
    const onItemInvoke = vi.fn();
    const onClose = vi.fn();

    render(
      <MenuBarDropdown
        screenId="s1"
        state={baseState}
        onItemInvoke={onItemInvoke}
        onClose={onClose}
      />,
    );

    // Primary interaction: click the "Open settings…" item (group "settings",
    // enabled: always) — an actionable button (not an informational row).
    fireEvent.click(screen.getByText("⚙ 開啟設定…"));

    await waitFor(() => expect(onItemInvoke).toHaveBeenCalledTimes(1));
    expect(onItemInvoke.mock.calls[0][0]).toMatchObject({ item_id: "open_settings" });
    expect(onClose).toHaveBeenCalledTimes(1);
    // screen survived the edge peer-status render
    expect(screen.getByTestId("menu-bar-dropdown")).toBeInTheDocument();
  });
});
