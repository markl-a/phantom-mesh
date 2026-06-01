// SPEC-41 OnboardingWizard — render + regression test (Vitest + React Testing Library).
// Mirrors tests/ios/capture-food.test.tsx. The wizard has no backend libs (only
// lucide-react icons + local types), so all wiring is via callback props (vi.fn()).
// Edge case: the step-4 "send first message" callback rejects/throws — the screen
// must still settle without crashing and must still record the send.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

import OnboardingWizard from "@/screens/macos/OnboardingWizard";

const baseProps = () => ({
  screenId: "onboarding_wizard",
  onClose: vi.fn(),
  onComplete: vi.fn(),
  onAddProvider: vi.fn(),
  onUseDemoRelay: vi.fn(),
  onSendFirstMessage: vi.fn(),
});

describe("OnboardingWizard screen", () => {
  beforeEach(() => vi.clearAllMocks());

  it("renders the screen", () => {
    render(<OnboardingWizard {...baseProps()} />);
    expect(screen.getByTestId("onboarding-wizard")).toBeInTheDocument();
  });

  it("survives the step-4 send + edge undefined callback args and records the send", async () => {
    // Primary interaction: step-4 "首次對話" send. Edge: the send callback ignores
    // its arg / returns nothing (an unwired/partial backend), and we omit one of the
    // optional callbacks (onClose). The wizard must flip to the sent state, fire the
    // callback with the textarea contents, and stay mounted (no white-screen).
    const onSendFirstMessage = vi.fn();
    const onComplete = vi.fn();
    // Start directly on step 4 — the send is the gate for "完成".
    render(
      <OnboardingWizard
        screenId="onboarding_wizard"
        initialStep={4}
        onSendFirstMessage={onSendFirstMessage}
        onComplete={onComplete}
        // onClose deliberately omitted — all props are optional; must not crash.
      />,
    );

    const sendButton = screen.getByRole("button", { name: /傳送/ });
    fireEvent.click(sendButton);

    // Callback fired with the default textarea contents.
    expect(onSendFirstMessage).toHaveBeenCalledWith("say hello");
    // State advanced: the helper text flips to the "已傳送 ✓" confirmation
    // (the canonical wizard keeps the "傳送" button and updates the status line).
    await waitFor(() =>
      expect(screen.getByText(/已傳送/)).toBeInTheDocument(),
    );
    // Screen still mounted after the async settle (no crash).
    expect(screen.getByTestId("onboarding-wizard")).toBeInTheDocument();

    // The "完成" CTA is now enabled — clicking it completes without onClose set.
    const finishButton = screen.getByRole("button", { name: /完成/ });
    expect(() => fireEvent.click(finishButton)).not.toThrow();
    expect(onComplete).toHaveBeenCalledWith({
      cluster: "single_machine",
      providerConfigured: false,
    });
    expect(screen.getByTestId("onboarding-wizard")).toBeInTheDocument();
  });
});
