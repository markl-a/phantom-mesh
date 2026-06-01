// SPEC-31 onboarding-hello — render + focus-scenario test (Vitest + React Testing Library).
// Mirrors capture-food.test.tsx: mock the FSM lib + useHaptics, import @/pages/onboarding-hello,
// assert the screen never crashes (testid present after settle) and that step-indicator,
// Back/Continue gating, and a soft-failed advance all behave.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

// FORWARD_ORDER is the single source of truth (SPEC-28 §7.1) the page derives its
// step indicator + total from; keep it identical to the real export so assertions
// about positions/labels stay faithful. Defined via vi.hoisted so the (hoisted)
// vi.mock factory can close over the same value the test body asserts against.
const h = vi.hoisted(() => {
  const FORWARD_ORDER = [
    "fresh_install",
    "picked_language",
    "created_identity",
    "joined_cluster",
    "set_provider",
    "first_reply_received",
  ] as const;
  return {
    FORWARD_ORDER,
    loadSnapshot: vi.fn(),
    advanceOnboarding: vi.fn(),
    rollbackOnboarding: vi.fn(),
    patchContext: vi.fn(),
  };
});

const FORWARD_ORDER = h.FORWARD_ORDER;
const loadSnapshot = h.loadSnapshot;
const advanceOnboarding = h.advanceOnboarding;
const rollbackOnboarding = h.rollbackOnboarding;
const patchContext = h.patchContext;

vi.mock("@/lib/onboardingFsm", () => ({
  FORWARD_ORDER: h.FORWARD_ORDER,
  loadSnapshot: (...args: unknown[]) => h.loadSnapshot(...args),
  advanceOnboarding: (...args: unknown[]) => h.advanceOnboarding(...args),
  rollbackOnboarding: (...args: unknown[]) => h.rollbackOnboarding(...args),
  patchContext: (...args: unknown[]) => h.patchContext(...args),
}));

import OnboardingHello from "@/pages/onboarding-hello";

function snapAt(state: (typeof FORWARD_ORDER)[number]) {
  return { currentState: state, enteredAtMs: 0, retryCount: 0, lastError: null };
}

describe("onboarding-hello screen", () => {
  beforeEach(() => {
    loadSnapshot.mockReset();
    advanceOnboarding.mockReset();
    rollbackOnboarding.mockReset();
    patchContext.mockReset();
    // Default: brand-new install.
    loadSnapshot.mockReturnValue(snapAt("fresh_install"));
  });

  it("renders the screen", () => {
    render(<OnboardingHello />);
    expect(screen.getByTestId("onboarding-hello")).toBeInTheDocument();
  });

  it("shows a step indicator with one dot per FORWARD_ORDER entry and Step 1 of N copy", () => {
    render(<OnboardingHello />);
    const nav = screen.getByRole("navigation", { name: /設定進度/ });
    // One indicator span per forward state (all aria-hidden decorative spans).
    expect(nav.querySelectorAll("span").length).toBe(FORWARD_ORDER.length);
    // "步驟 1 / 6 · Step 1 of 6"
    expect(
      screen.getByText(
        new RegExp(`Step 1 of ${FORWARD_ORDER.length}`),
      ),
    ).toBeInTheDocument();
  });

  it("hides Back on the first step (only joined_cluster may go back)", () => {
    render(<OnboardingHello />);
    expect(screen.queryByRole("button", { name: /返回 \/ Back/ })).toBeNull();
    // Continue (forward) CTA is present and enabled.
    const cont = screen.getByRole("button", { name: /繼續 \/ Continue/ });
    expect(cont).toBeInTheDocument();
    expect(cont).not.toBeDisabled();
  });

  it("shows Back only on the joined_cluster step", () => {
    loadSnapshot.mockReturnValue(snapAt("joined_cluster"));
    render(<OnboardingHello />);
    expect(
      screen.getByRole("button", { name: /返回 \/ Back/ }),
    ).toBeInTheDocument();
    // Indicator now reports the 4th step (index 3 → "Step 4 of 6").
    expect(
      screen.getByText(new RegExp(`Step 4 of ${FORWARD_ORDER.length}`)),
    ).toBeInTheDocument();
  });

  it("on the last step the CTA is Finish (no advance call) and does not crash", () => {
    loadSnapshot.mockReturnValue(snapAt("first_reply_received"));
    render(<OnboardingHello />);
    const finish = screen.getByRole("button", { name: /完成 \/ Finish/ });
    fireEvent.click(finish);
    // Final step finishes locally via patchContext, never advanceOnboarding.
    expect(patchContext).toHaveBeenCalled();
    expect(advanceOnboarding).not.toHaveBeenCalled();
    expect(screen.getByTestId("onboarding-hello")).toBeInTheDocument();
  });

  it("advances on Continue and surfaces a soft-failed (deferred backend) note without crashing", async () => {
    // advanceOnboarding soft-fails: backend not wired, FSM bumped client-side.
    advanceOnboarding.mockResolvedValue({
      state: "picked_language",
      softFailed: true,
      errorMessage: "onboarding.not_yet_wired:created_identity",
    });
    render(<OnboardingHello />);
    fireEvent.click(screen.getByRole("button", { name: /繼續 \/ Continue/ }));
    await waitFor(() => expect(advanceOnboarding).toHaveBeenCalled());
    // Screen advanced to step 2 and survives the soft-note render.
    await waitFor(() =>
      expect(
        screen.getByText(new RegExp(`Step 2 of ${FORWARD_ORDER.length}`)),
      ).toBeInTheDocument(),
    );
    expect(screen.getByText(/已先在本機推進/)).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-hello")).toBeInTheDocument();
  });

  it("does NOT crash when advanceOnboarding rejects with a real backend error", async () => {
    advanceOnboarding.mockRejectedValue(new Error("backend exploded"));
    render(<OnboardingHello />);
    fireEvent.click(screen.getByRole("button", { name: /繼續 \/ Continue/ }));
    await waitFor(() => expect(advanceOnboarding).toHaveBeenCalled());
    // Error surfaces in an alert; FSM stays put (still step 1); screen alive.
    await waitFor(() =>
      expect(screen.getByRole("alert")).toBeInTheDocument(),
    );
    expect(
      screen.getByText(new RegExp(`Step 1 of ${FORWARD_ORDER.length}`)),
    ).toBeInTheDocument();
    expect(screen.getByTestId("onboarding-hello")).toBeInTheDocument();
  });
});
