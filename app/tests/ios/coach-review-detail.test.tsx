// SPEC-31 coach-review-detail — render + edge-input regression test
// (Vitest + React Testing Library). Guards against a deep-link date change
// showing stale data, and against a partial/null DailyReviewView crashing.
import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const loadDailyReview = vi.fn();
vi.mock("@/lib/dailyReview", () => ({
  loadDailyReview: (...args: unknown[]) => loadDailyReview(...args),
  // Real-shaped pure helpers (no backend) so the component renders honestly.
  parseReview: () => [],
  extractTomorrowAction: () => null,
  todayIso: () => "2026-05-29",
  KIND_EMOJI: { food: "🍽", focus: "🎯", habit: "✅", text: "📝" },
}));

import CoachReviewDetail from "@/pages/coach-review-detail";

describe("coach-review-detail screen", () => {
  beforeEach(() => loadDailyReview.mockReset());

  it("renders the screen", () => {
    loadDailyReview.mockResolvedValue(null);
    render(<CoachReviewDetail date="2026-05-29" />);
    expect(screen.getByTestId("coach-review-detail")).toBeInTheDocument();
  });

  it("does NOT crash on a partial DailyReviewView, then null, and reacts to a date prop change without stale data", async () => {
    // Day 1: a partial-but-valid view (markdown empty, no events, unlocked).
    loadDailyReview.mockResolvedValueOnce({ markdown: "", eventCount: 0, locked: false });
    const { rerender } = render(<CoachReviewDetail date="2026-05-29" />);

    // First load settles → screen survives, shows the empty-day state for that date.
    await waitFor(() => expect(loadDailyReview).toHaveBeenCalledWith("2026-05-29"));
    await waitFor(() =>
      expect(screen.getByText(/No Life Node events for this date\./)).toBeInTheDocument(),
    );
    expect(screen.getByTestId("coach-review-detail")).toBeInTheDocument();

    // Day 2: deep-link to a new date that legitimately has no review (null).
    loadDailyReview.mockResolvedValueOnce(null);
    rerender(<CoachReviewDetail date="2026-05-28" />);

    // The new date must drive a reload; old (empty-events) state must NOT linger.
    await waitFor(() => expect(loadDailyReview).toHaveBeenCalledWith("2026-05-28"));
    await waitFor(() =>
      expect(screen.getByText(/No review for this date yet\./)).toBeInTheDocument(),
    );
    // Stale day-1 state is gone, and the screen never crashed.
    expect(screen.queryByText(/No Life Node events for this date\./)).not.toBeInTheDocument();
    expect(screen.getByTestId("coach-review-detail")).toBeInTheDocument();
  });
});
