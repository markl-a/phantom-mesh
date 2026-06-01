// SPEC-41 macOS screen #3 — CoachReviewReader (每日回顧) render + edge-case test.
// Mirrors tests/ios/capture-food.test.tsx: render guard + a primary-interaction
// regression that drives the "產生回顧" coach button with a rejected backend and
// asserts the screen survives (testid stays mounted) and surfaces the error.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { vi, describe, it, expect, beforeEach } from "vitest";

const loadDailyReview = vi.fn();
const generateReview = vi.fn();

// Mock the data lib (the only backend dependency). Keep the pure helpers real so
// parseReview / extractTomorrowAction exercise the markdown the screen renders.
vi.mock("@/lib/dailyReview", async () => {
  const actual = await vi.importActual<typeof import("@/lib/dailyReview")>("@/lib/dailyReview");
  return {
    ...actual,
    loadDailyReview: (...a: unknown[]) => loadDailyReview(...a),
    generateReview: (...a: unknown[]) => generateReview(...a),
  };
});

import CoachReviewReader from "@/screens/macos/CoachReviewReader";

const renderScreen = () =>
  render(
    <MemoryRouter>
      <CoachReviewReader />
    </MemoryRouter>,
  );

// A minimal has-events view so `canGenerate` is true and the "產生回顧" CTA shows.
const viewWithEvents = {
  date: "2026-05-29",
  markdown: "# Daily review — 2026-05-29\n**Events captured:** 1\n## focus (1)\n- **focus** (2026-05-29T09:00): deep work\n",
  eventCount: 1,
  locked: false,
  flagged: false,
};

describe("CoachReviewReader (macOS daily review)", () => {
  beforeEach(() => {
    loadDailyReview.mockReset();
    generateReview.mockReset();
  });

  it("renders the screen", async () => {
    loadDailyReview.mockResolvedValue(viewWithEvents);
    renderScreen();
    expect(screen.getByTestId("coach-review-reader")).toBeInTheDocument();
    await waitFor(() => expect(loadDailyReview).toHaveBeenCalled());
  });

  it("does NOT crash when generateReview rejects on the coach pass", async () => {
    // Edge case: the Gemini "tomorrow's action" pass throws (no key / backend
    // error). The screen must stay mounted and show the error banner, not blank.
    loadDailyReview.mockResolvedValue(viewWithEvents);
    generateReview.mockRejectedValue(new Error("coach backend exploded"));
    renderScreen();

    // Wait for the load to settle so the generate CTA renders.
    await waitFor(() => expect(screen.getByText("產生回顧")).toBeInTheDocument());
    fireEvent.click(screen.getByText("產生回顧"));

    await waitFor(() => expect(generateReview).toHaveBeenCalled());
    // Screen survives the rejected promise and reports it via role=alert.
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
    expect(screen.getByTestId("coach-review-reader")).toBeInTheDocument();
  });
});
