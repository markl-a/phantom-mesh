// SPEC-41 §10.6 — CoachReviewList (S6) render + edge-input regression test
// (Vitest + React Testing Library). Mirrors the iOS capture-food pattern:
// mock the backend lib (lib/dailyReview) and assert the screen survives a
// partial / unavailable result without crashing (testid stays mounted).
//
// Edge focus: when the offline `daily_review_load` is unavailable for *every*
// probed day (web mode → null for all dates), the screen must surface the error
// banner instead of a silent empty list — and must NOT crash.

import { render, screen, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { vi, describe, it, expect, beforeEach } from "vitest";

const loadDailyReview = vi.fn();
vi.mock("@/lib/dailyReview", () => ({
  loadDailyReview: (...args: unknown[]) => loadDailyReview(...args),
  // Stable today so the screen's date-probing is deterministic.
  todayIso: () => "2026-05-30",
}));

import CoachReviewList from "@/screens/macos/CoachReviewList";

function renderScreen() {
  return render(
    <MemoryRouter>
      <CoachReviewList />
    </MemoryRouter>,
  );
}

describe("CoachReviewList (macOS S6)", () => {
  beforeEach(() => loadDailyReview.mockReset());

  it("renders the screen", async () => {
    // Default: a single day with events; just confirm the root mounts.
    loadDailyReview.mockResolvedValue({
      date: "2026-05-30",
      markdown: "# Daily review",
      eventCount: 3,
      locked: false,
      flagged: false,
    });
    renderScreen();
    expect(screen.getByTestId("coach-review-list")).toBeInTheDocument();
  });

  it("does NOT crash when every probed day returns null (backend unavailable)", async () => {
    // Edge: web mode → loadDailyReview resolves null for all 14 probed dates.
    // The screen should show the error banner, not crash or hang on skeleton.
    loadDailyReview.mockResolvedValue(null);
    renderScreen();
    await waitFor(() => expect(loadDailyReview).toHaveBeenCalled());
    await waitFor(() => {
      expect(screen.getByRole("alert")).toBeInTheDocument();
      expect(screen.getByText("每日回顧後端暫時無法使用")).toBeInTheDocument();
    });
    // Screen survived the all-null path.
    expect(screen.getByTestId("coach-review-list")).toBeInTheDocument();
  });

  it("does NOT crash when a day returns a locked, zero-event view", async () => {
    // Edge: a locked day (identity.key absent) with eventCount 0 — the screen
    // keeps locked days in the list and renders the encrypted banner.
    loadDailyReview.mockImplementation(async (iso?: string) =>
      iso === "2026-05-30"
        ? { date: iso, markdown: "# Daily review", eventCount: 0, locked: true, flagged: false }
        : { date: iso, markdown: "# Daily review", eventCount: 0, locked: false, flagged: false },
    );
    renderScreen();
    await waitFor(() => expect(loadDailyReview).toHaveBeenCalled());
    await waitFor(() => {
      const rows = screen.getAllByTestId(/^review-row-/);
      expect(rows.length).toBe(1);
      expect(rows[0].getAttribute("href")).toMatch(/\/review\?date=/);
    });
    expect(screen.getByTestId("coach-review-list")).toBeInTheDocument();
  });
});
