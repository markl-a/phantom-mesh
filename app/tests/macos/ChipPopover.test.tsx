// SPEC-41 §10.3 ChipPopover — render + regression test (Vitest + RTL).
// Mirrors tests/ios/capture-food.test.tsx. Guards the partial-shape path:
// ensureCheckin returning {} (web fallback for an unwired command) must NOT
// crash via `s.currentStreak` — the screen guards with `s?.currentStreak ?? 0`.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

const ensureCheckin = vi.fn();
vi.mock("../../src/lib/captureHabit", () => ({
  STARTER_PALETTE: [
    { slug: "water", label: "水", emoji: "💧" },
    { slug: "coffee", label: "咖啡", emoji: "☕" },
  ],
  ensureCheckin: (...args: unknown[]) => ensureCheckin(...args),
  describeHabitError: (e: unknown) => String(e),
}));

import ChipPopover from "../../src/screens/macos/ChipPopover";

describe("ChipPopover screen", () => {
  beforeEach(() => ensureCheckin.mockReset());

  it("renders the screen", () => {
    render(<ChipPopover />);
    expect(screen.getByTestId("chip-popover")).toBeInTheDocument();
  });

  it("does NOT crash when ensureCheckin returns a partial shape (no currentStreak)", async () => {
    // Partial/edge: web fallback returns {} for an unwired command. The done
    // message reads s.currentStreak — would throw if not guarded.
    ensureCheckin.mockResolvedValue({} /* missing currentStreak */);
    const onLogged = vi.fn();
    render(<ChipPopover onLogged={onLogged} />);

    // Primary interaction: pick a chip, then submit.
    fireEvent.click(screen.getByLabelText("水"));
    fireEvent.click(screen.getByText("送出"));

    await waitFor(() => expect(ensureCheckin).toHaveBeenCalledWith("water", "水", { note: null }));
    // Screen survives rendering the success message with the missing field.
    expect(screen.getByTestId("chip-popover")).toBeInTheDocument();
    await waitFor(() => expect(onLogged).toHaveBeenCalledWith("water"));
    expect(screen.getByRole("status")).toHaveTextContent("連續 0 天");
  });

  it("does NOT submit (no backend call) when no chip is selected", async () => {
    // Edge: submit guard — `if (!slug) return` must short-circuit before any
    // backend call, and the screen must stay mounted.
    render(<ChipPopover />);
    // The CTA is disabled with no selection; clicking it must be a no-op.
    fireEvent.click(screen.getByText("送出"));
    expect(ensureCheckin).not.toHaveBeenCalled();
    expect(screen.getByTestId("chip-popover")).toBeInTheDocument();
  });
});
