// SPEC-31 onboarding-broker — render + regression test (Vitest + React Testing Library).
// CRITICAL regression: a LATE broker login result (deep-link round-trip lands after
// the user pressed Skip / a startBrokerLogin rejection arrives after Skip) must NOT
// clobber the skipped flow into "done"/"error". The screen guards this with
// awaitingResultRef (cleared on Skip) + a seq counter. This test drives those edges
// by capturing the onBrokerLoginResult listener and invoking it manually.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";
import type { BrokerLoginListener } from "@/lib/brokerLogin";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const startBrokerLogin = vi.fn();
// Capture the listener the screen registers so we can fire results manually,
// including LATE ones (after Skip / unmount) to simulate the deep-link round-trip.
let capturedListener: BrokerLoginListener | null = null;
const unsubscribe = vi.fn();
vi.mock("@/lib/brokerLogin", () => ({
  startBrokerLogin: (...args: unknown[]) => startBrokerLogin(...args),
  onBrokerLoginResult: (cb: BrokerLoginListener) => {
    capturedListener = cb;
    return unsubscribe;
  },
}));

const advanceOnboarding = vi.fn();
vi.mock("@/lib/onboardingFsm", () => ({
  // Real FORWARD_ORDER mirror — the screen reads STEP_INDEX / TOTAL_STEPS from it.
  FORWARD_ORDER: [
    "fresh_install",
    "picked_language",
    "created_identity",
    "joined_cluster",
    "set_provider",
    "first_reply_received",
  ],
  advanceOnboarding: (...args: unknown[]) => advanceOnboarding(...args),
}));

import OnboardingBroker from "@/pages/onboarding-broker";

// Helper: fire a successful broker login result through the captured listener.
function fireResultOk() {
  capturedListener?.({
    ok: true,
    identity: {
      email: "user42@example.com",
      provider: "broker",
      display_name: "User 42",
      broker_token_expires_at_ms: 0,
      auth_path: "/tmp/auth",
    },
    sync: {
      keys_written: ["k1"],
      env_path: "/tmp/env",
      peers_count: 2,
      peers_path: null,
      peers: [],
    },
  });
}

function clickByLabel(label: string) {
  fireEvent.click(screen.getByRole("button", { name: label }));
}

describe("onboarding-broker screen", () => {
  beforeEach(() => {
    startBrokerLogin.mockReset();
    advanceOnboarding.mockReset();
    capturedListener = null;
    unsubscribe.mockReset();
    // Default FSM advance: a normal (non-soft) success.
    advanceOnboarding.mockResolvedValue({
      state: "set_provider",
      softFailed: false,
      errorMessage: null,
    });
  });

  it("renders the screen", () => {
    render(<OnboardingBroker />);
    expect(screen.getByTestId("onboarding-broker")).toBeInTheDocument();
    // Idle: Connect CTA present, no done/error.
    expect(
      screen.getByRole("button", { name: "登入雲端中介 / Sign in to cloud broker" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("Connect then a deep-link result commits done", async () => {
    startBrokerLogin.mockResolvedValue({
      auth_url: "https://example.com/auth",
      device_id: "dev42",
      redirect: "phantom://oauth/callback",
    });
    render(<OnboardingBroker />);

    clickByLabel("登入雲端中介 / Sign in to cloud broker");
    await waitFor(() => expect(startBrokerLogin).toHaveBeenCalled());
    // Now in connecting state — listener captured.
    expect(capturedListener).toBeTruthy();

    // Deep-link round-trip lands: fire the success result.
    fireResultOk();
    await waitFor(() =>
      expect(advanceOnboarding).toHaveBeenCalledWith({
        demoRelayUsed: false,
        providerSlug: "broker",
      }),
    );
    // Commits to done: Continue CTA appears, broker-linked status shown.
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "繼續 / Continue" }),
      ).toBeInTheDocument(),
    );
    expect(screen.getByText(/Cloud broker linked/)).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-broker")).toBeInTheDocument();
  });

  it("Skip mid-connect then a LATE result must NOT clobber into done", async () => {
    // Never resolves: handoff "succeeds" but the deep-link result will arrive late.
    startBrokerLogin.mockReturnValue(new Promise(() => {}));
    const onContinue = vi.fn();
    render(<OnboardingBroker onContinue={onContinue} />);

    clickByLabel("登入雲端中介 / Sign in to cloud broker");
    await waitFor(() => expect(startBrokerLogin).toHaveBeenCalled());

    // User escapes the hung handoff via Skip mid-connect.
    clickByLabel("略過，單機模式 / Skip, single-machine");
    await waitFor(() => expect(advanceOnboarding).toHaveBeenCalledWith({}));
    await waitFor(() => expect(onContinue).toHaveBeenCalled());

    // LATE deep-link result lands AFTER Skip cleared the awaiting gate.
    fireResultOk();

    // Give any (incorrect) commit a chance to flush, then assert it did NOT happen.
    await new Promise((r) => setTimeout(r, 20));
    // advanceOnboarding must NOT have been called with the broker provider args
    // (that would mean the late result committed the supersededConnect).
    expect(advanceOnboarding).not.toHaveBeenCalledWith({
      demoRelayUsed: false,
      providerSlug: "broker",
    });
    // No "done" Continue CTA, no broker-linked status, no crash.
    expect(
      screen.queryByRole("button", { name: "繼續 / Continue" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(/Cloud broker linked/)).not.toBeInTheDocument();
    expect(screen.getByTestId("onboarding-broker")).toBeInTheDocument();
  });

  it("startBrokerLogin rejection AFTER Skip must NOT set error", async () => {
    // Deferred rejection: the handoff fails, but only after the user pressed Skip.
    let rejectHandoff: (e: unknown) => void = () => {};
    startBrokerLogin.mockReturnValue(
      new Promise((_resolve, reject) => {
        rejectHandoff = reject;
      }),
    );
    const onContinue = vi.fn();
    render(<OnboardingBroker onContinue={onContinue} />);

    clickByLabel("登入雲端中介 / Sign in to cloud broker");
    await waitFor(() => expect(startBrokerLogin).toHaveBeenCalled());

    // Skip mid-connect (clears awaiting gate + bumps seq).
    clickByLabel("略過，單機模式 / Skip, single-machine");
    await waitFor(() => expect(onContinue).toHaveBeenCalled());

    // NOW the superseded handoff rejects — must be dropped, not surfaced as error.
    rejectHandoff(new Error("handoff blew up late"));
    await new Promise((r) => setTimeout(r, 20));

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      screen.queryByText(/Could not link broker/),
    ).not.toBeInTheDocument();
    expect(screen.getByTestId("onboarding-broker")).toBeInTheDocument();
  });
});
