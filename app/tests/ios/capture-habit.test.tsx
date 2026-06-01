// SPEC-31 capture-habit — render + regression test (Vitest + React Testing Library).
// Mirrors capture-food.test.tsx. Guards the edge case where ensureCheckin resolves
// a PARTIAL HabitStreak (no `currentStreak`): the screen reads `s?.currentStreak ?? 0`
// and must NOT crash on the missing field.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const ensureCheckin = vi.fn();
vi.mock("@/lib/captureHabit", () => ({
  STARTER_PALETTE: [
    { slug: "water", label: "水", emoji: "💧" },
    { slug: "coffee", label: "咖啡", emoji: "☕" },
  ],
  ensureCheckin: (...args: unknown[]) => ensureCheckin(...args),
  describeHabitError: (e: unknown) => String(e),
}));

import CaptureHabit from "@/pages/capture-habit";

describe("capture-habit screen", () => {
  beforeEach(() => ensureCheckin.mockReset());

  it("renders the screen", () => {
    render(<CaptureHabit />);
    expect(screen.getByTestId("capture-habit")).toBeInTheDocument();
  });

  it("does NOT crash when ensureCheckin returns a partial streak missing `currentStreak`", async () => {
    // The edge: HabitStreak.currentStreak is typed but a partial/unwired backend
    // may omit it → `s.currentStreak` would be undefined. The screen uses
    // `s?.currentStreak ?? 0`, so it must survive and show "連續 0 天".
    ensureCheckin.mockResolvedValue({ habitSlug: "water" /* no currentStreak */ });
    render(<CaptureHabit />);

    // pick a chip (water)
    fireEvent.click(screen.getByRole("button", { name: /水 habit/ }));

    // click the sticky-footer submit CTA (last button)
    const buttons = screen.getAllByRole("button");
    fireEvent.click(buttons[buttons.length - 1]);

    await waitFor(() => expect(ensureCheckin).toHaveBeenCalled());

    // screen survives the result render (would throw if it read .currentStreak unguarded)
    expect(screen.getByTestId("capture-habit")).toBeInTheDocument();
    // and the success state falls back to 0
    await waitFor(() =>
      expect(screen.getByRole("status")).toHaveTextContent("連續 0 天"),
    );
  });
});
