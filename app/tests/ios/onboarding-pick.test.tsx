// SPEC-31 onboarding-pick — render + interaction test (Vitest + React Testing Library).
// Mirrors capture-food.test.tsx: mock libs with vi.mock, mock useHaptics ()=>({impact:vi.fn()}),
// import @/pages/onboarding-pick. Covers: render, per-card selection, Continue gating,
// advance happy-path (resolve) and soft-fail path — screen must not crash either way.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const advanceOnboarding = vi.fn();
const loadSnapshot = vi.fn(() => ({
  currentState: "joined_cluster",
  enteredAtMs: BigInt(0),
  retryCount: 0,
  lastError: null,
}));

// FORWARD_ORDER inlined inside the factory: vi.mock is hoisted above any
// top-level const, so referencing an outer array there would throw a TDZ error.
vi.mock("@/lib/onboardingFsm", () => ({
  FORWARD_ORDER: [
    "fresh_install",
    "picked_language",
    "created_identity",
    "joined_cluster",
    "set_provider",
    "first_reply_received",
  ],
  advanceOnboarding: (...args: unknown[]) => advanceOnboarding(...args),
  loadSnapshot: () => loadSnapshot(),
}));

import OnboardingPick from "@/pages/onboarding-pick";

const okResult = { state: "set_provider", softFailed: false, errorMessage: null };
const softResult = {
  state: "set_provider",
  softFailed: true,
  errorMessage: "onboarding.not_yet_wired: deferred",
};

// The Continue CTA is the last footer button (Back is first).
function continueButton(): HTMLButtonElement {
  const buttons = screen.getAllByRole("button");
  return buttons[buttons.length - 1] as HTMLButtonElement;
}

describe("onboarding-pick screen", () => {
  beforeEach(() => {
    advanceOnboarding.mockReset();
    loadSnapshot.mockReset();
    loadSnapshot.mockReturnValue({
      currentState: "joined_cluster",
      enteredAtMs: BigInt(0),
      retryCount: 0,
      lastError: null,
    });
  });

  it("renders the screen", () => {
    render(<OnboardingPick />);
    expect(screen.getByTestId("onboarding-pick")).toBeInTheDocument();
  });

  it("renders all three cluster choice cards as radios", () => {
    render(<OnboardingPick />);
    expect(screen.getAllByRole("radio")).toHaveLength(3);
  });

  it("gates Continue until a choice is selected", () => {
    render(<OnboardingPick />);
    // No selection yet → Continue disabled.
    expect(continueButton()).toBeDisabled();
    // Select first card → Continue enabled.
    fireEvent.click(screen.getAllByRole("radio")[0]);
    expect(continueButton()).not.toBeDisabled();
  });

  it("marks each cluster card selected when clicked (single choice)", () => {
    render(<OnboardingPick />);
    const radios = screen.getAllByRole("radio");
    for (const radio of radios) {
      fireEvent.click(radio);
      expect(radio).toHaveAttribute("aria-checked", "true");
      // Only the just-clicked card is checked.
      const checked = radios.filter(
        (r) => r.getAttribute("aria-checked") === "true",
      );
      expect(checked).toHaveLength(1);
    }
  });

  it("calls advanceOnboarding on Continue and reaches done state without crashing (resolve)", async () => {
    advanceOnboarding.mockResolvedValue(okResult);
    render(<OnboardingPick />);
    fireEvent.click(screen.getAllByRole("radio")[1]); // create_new
    fireEvent.click(continueButton());
    await waitFor(() => expect(advanceOnboarding).toHaveBeenCalledTimes(1));
    // Screen survives the post-advance render.
    await waitFor(() =>
      expect(screen.getByTestId("onboarding-pick")).toBeInTheDocument(),
    );
    // Done CTA shows after a successful advance.
    expect(screen.getByText(/已完成 \/ Done/)).toBeInTheDocument();
  });

  it("does NOT crash when advanceOnboarding soft-fails (backend not yet wired)", async () => {
    advanceOnboarding.mockResolvedValue(softResult);
    render(<OnboardingPick />);
    fireEvent.click(screen.getAllByRole("radio")[2]); // single_machine
    fireEvent.click(continueButton());
    await waitFor(() => expect(advanceOnboarding).toHaveBeenCalledTimes(1));
    // Soft-fail still settles to a non-crashed screen + surfaces the soft note.
    await waitFor(() =>
      expect(screen.getByTestId("onboarding-pick")).toBeInTheDocument(),
    );
    expect(
      screen.getByText(/backend not yet wired/i),
    ).toBeInTheDocument();
  });
});
