// SPEC-31 coach-review-list — render + edge-state regression test
// (Vitest + React Testing Library). Mirrors the proven capture-food.test.tsx
// pattern. The important guard: the screen must distinguish two async outcomes
// that a naive null-check conflates —
//   (a) every probe REJECTS  → honest error state (backend unavailable)
//   (b) every probe RESOLVES null → honest EMPTY state (legitimately no reviews),
//       which must NOT be shown as an error.
// In both cases the screen must survive the async settle without crashing
// (its root testid stays present).
import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const loadDailyReview = vi.fn();
vi.mock("@/lib/dailyReview", () => ({
  loadDailyReview: (...args: unknown[]) => loadDailyReview(...args),
  todayIso: () => "2026-05-29",
}));

import CoachReviewList from "@/pages/coach-review-list";

describe("coach-review-list screen", () => {
  // NOTE: we deliberately reset the mock at the TOP of each test body rather
  // than in a beforeEach hook. A beforeEach reset, interleaved with React
  // Testing Library's afterEach auto-cleanup and the rejected-promise impl,
  // opens a brief microtask window where vitest mis-attributes the (already
  // allSettled-handled) rejection to the test. Resetting inline avoids it.

  it("renders the screen", () => {
    loadDailyReview.mockReset();
    loadDailyReview.mockResolvedValue(null);
    render(<CoachReviewList />);
    expect(screen.getByTestId("coach-review-list")).toBeInTheDocument();
  });

  it("shows an honest ERROR state when every probe rejects (and does not crash)", async () => {
    // Every one of the SCAN_DAYS probes rejects → real backend failure.
    loadDailyReview.mockReset();
    loadDailyReview.mockImplementation(() => Promise.reject(new Error("backend down")));
    render(<CoachReviewList />);
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    // Screen survives the async settle.
    expect(screen.getByTestId("coach-review-list")).toBeInTheDocument();
    // Error state, not the empty state.
    expect(screen.queryByTestId("review-list-empty")).not.toBeInTheDocument();
  });

  it("shows an honest EMPTY state (NOT error) when every probe resolves null", async () => {
    // All probes fulfil with null → legitimately empty 14-day window, not failure.
    loadDailyReview.mockReset();
    loadDailyReview.mockResolvedValue(null);
    render(<CoachReviewList />);
    await waitFor(() => expect(screen.getByTestId("review-list-empty")).toBeInTheDocument());
    // Screen survives the async settle.
    expect(screen.getByTestId("coach-review-list")).toBeInTheDocument();
    // Empty state must NOT be presented as an error.
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });
});
