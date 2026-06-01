// SPEC-31 capture-food — render + regression test (Vitest + React Testing Library).
// Guards the QA-found white-screen: a partial analyzeFood result (no `suggestion`)
// must NOT crash the screen via `result.suggestion.trim()`.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const analyzeFood = vi.fn();
vi.mock("@/lib/captureFood", () => ({
  FOOD_LOG_KIND: "food_log",
  buildFoodRequest: (text: string) => ({ text, imagePath: null }),
  analyzeFood: (...args: unknown[]) => analyzeFood(...args),
  describeFoodError: (e: unknown) => String(e),
}));

import CaptureFood from "@/pages/capture-food";

describe("capture-food screen", () => {
  beforeEach(() => analyzeFood.mockReset());

  it("renders the screen", () => {
    render(<CaptureFood />);
    expect(screen.getByTestId("capture-food")).toBeInTheDocument();
  });

  it("does NOT crash when analyzeFood returns a partial result missing `suggestion`", async () => {
    // The bug: suggestion is typed string but a partial/unwired backend omits it →
    // result.suggestion.trim() threw "Cannot read properties of undefined".
    analyzeFood.mockResolvedValue({ summary: "便當", macroEstimate: null /* no suggestion */ });
    render(<CaptureFood />);
    fireEvent.change(screen.getByRole("textbox"), { target: { value: "便當 chicken rice" } });
    const buttons = screen.getAllByRole("button");
    fireEvent.click(buttons[buttons.length - 1]); // sticky-footer submit CTA
    await waitFor(() => expect(analyzeFood).toHaveBeenCalled());
    // screen survives the render of the result (would have thrown pre-fix)
    expect(screen.getByTestId("capture-food")).toBeInTheDocument();
  });
});
