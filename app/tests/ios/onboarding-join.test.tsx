// SPEC-31 onboarding-join — render + behavior test (Vitest + React Testing Library).
// Mirrors capture-food.test.tsx: mocks the page's libs with vi.mock so the screen
// renders in a plain jsdom env (no Tauri). Guards:
//   - the screen renders (testid present)
//   - peer-status dot colors map Online→success / Unhealthy→danger / Unknown→muted
//   - honest empty-state (load finished, zero peers) is shown
//   - join calls patchContext + advanceOnboarding (soft-fail) without crashing
//   - refresh wiring fires the hook's refresh()
import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

// --- onboardingFsm mock ---------------------------------------------------
// FORWARD_ORDER must include "joined_cluster" (the page derives its step index
// from it via indexOf). patchContext + advanceOnboarding are spied. advance
// defaults to a soft-success (backend deferred) so the honest soft-note path
// is the one exercised here.
const patchContext = vi.fn();
const advanceOnboarding = vi.fn();
vi.mock("@/lib/onboardingFsm", () => ({
  FORWARD_ORDER: [
    "fresh_install",
    "picked_language",
    "created_identity",
    "joined_cluster",
    "set_provider",
    "first_reply_received",
  ],
  patchContext: (...args: unknown[]) => patchContext(...args),
  advanceOnboarding: (...args: unknown[]) => advanceOnboarding(...args),
}));

// --- useClusterPeers mock -------------------------------------------------
// Swappable per-test return value so we can drive empty vs populated states.
const refresh = vi.fn(() => Promise.resolve());
let peersReturn: {
  peers: Array<{
    peer_id: string;
    display_name: string;
    status: "Online" | "Unhealthy" | "Unknown";
    last_seen_unix: number;
  }>;
  status: "idle" | "loading" | "error";
  error?: string;
  lastSyncMs: number;
  refresh: () => Promise<void>;
};
vi.mock("@/hooks/useClusterPeers", () => ({
  useClusterPeers: () => peersReturn,
}));

import OnboardingJoin from "@/pages/onboarding-join";

const THREE_PEERS = [
  { peer_id: "p-online", display_name: "Peer Online", status: "Online" as const, last_seen_unix: 0 },
  { peer_id: "p-unhealthy", display_name: "Peer Unhealthy", status: "Unhealthy" as const, last_seen_unix: 0 },
  { peer_id: "p-unknown", display_name: "Peer Unknown", status: "Unknown" as const, last_seen_unix: 0 },
];

describe("onboarding-join screen", () => {
  beforeEach(() => {
    patchContext.mockReset();
    advanceOnboarding.mockReset();
    refresh.mockClear();
    // Default: loaded, with the three peers.
    peersReturn = {
      peers: THREE_PEERS,
      status: "idle",
      error: undefined,
      lastSyncMs: 0,
      refresh,
    };
  });

  it("renders the screen", () => {
    render(<OnboardingJoin />);
    expect(screen.getByTestId("onboarding-join")).toBeInTheDocument();
  });

  it("maps peer-status dot colors: Online=success, Unhealthy=danger, Unknown=muted", () => {
    render(<OnboardingJoin />);
    const list = screen.getByRole("list", { name: /Joinable peers/i });
    const rows = within(list).getAllByRole("button");
    expect(rows).toHaveLength(3);
    // The status dot is the first child <span> of each peer button (aria-hidden).
    const dot = (btn: HTMLElement) => btn.querySelector("span[aria-hidden='true']");
    expect(dot(rows[0])?.className).toContain("bg-phantom-success");
    expect(dot(rows[1])?.className).toContain("bg-phantom-danger");
    expect(dot(rows[2])?.className).toContain("bg-phantom-muted");
  });

  it("shows the honest empty-state when the load finished with no peers", () => {
    peersReturn = { peers: [], status: "idle", error: undefined, lastSyncMs: 0, refresh };
    render(<OnboardingJoin />);
    expect(screen.getByText(/No peers discovered/i)).toBeInTheDocument();
    // No peer list rendered.
    expect(screen.queryByRole("list", { name: /Joinable peers/i })).not.toBeInTheDocument();
  });

  it("join (pick peer → primary CTA) calls patchContext + advanceOnboarding and does not crash on soft-fail", async () => {
    advanceOnboarding.mockResolvedValue({
      state: "set_provider",
      softFailed: true,
      errorMessage: "onboarding.not_yet_wired: stage 3 deferred",
    });
    render(<OnboardingJoin />);

    // Pick the Online peer.
    const list = screen.getByRole("list", { name: /Joinable peers/i });
    const onlineRow = within(list).getAllByRole("button")[0];
    fireEvent.click(onlineRow);
    expect(onlineRow).toHaveAttribute("aria-pressed", "true");

    // Tap the primary CTA (footer). It is the last button on screen.
    const allButtons = screen.getAllByRole("button");
    fireEvent.click(allButtons[allButtons.length - 1]);

    await waitFor(() => expect(advanceOnboarding).toHaveBeenCalled());
    // patchContext recorded the picked peer.
    expect(patchContext).toHaveBeenCalledWith({ clusterIdHash: "p-online" });
    // Honest soft-note surfaces; the green "Joined" banner must NOT (soft path).
    await waitFor(() =>
      expect(screen.getByText(/advanced locally/i)).toBeInTheDocument(),
    );
    expect(screen.queryByText(/Joined the cluster/i)).not.toBeInTheDocument();
    // Screen survives the whole transition.
    expect(screen.getByTestId("onboarding-join")).toBeInTheDocument();
  });

  it("skip (no peer picked → primary CTA) advances with a null cluster id", async () => {
    advanceOnboarding.mockResolvedValue({ state: "set_provider", softFailed: false, errorMessage: null });
    render(<OnboardingJoin />);

    const allButtons = screen.getAllByRole("button");
    fireEvent.click(allButtons[allButtons.length - 1]); // primary CTA, nothing picked

    await waitFor(() => expect(advanceOnboarding).toHaveBeenCalled());
    expect(patchContext).toHaveBeenCalledWith({ clusterIdHash: null });
    expect(screen.getByTestId("onboarding-join")).toBeInTheDocument();
  });

  it("refresh button calls the hook's refresh()", () => {
    render(<OnboardingJoin />);
    fireEvent.click(screen.getByRole("button", { name: /Refresh discovered peers/i }));
    expect(refresh).toHaveBeenCalled();
  });
});
