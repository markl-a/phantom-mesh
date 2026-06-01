// SPEC P2 Life Track — RecallSearch (回想) render + regression test
// (Vitest + React Testing Library). Mirrors tests/ios/capture-food.test.tsx +
// tests/macos/EventTimeline.test.tsx.
//
// Guards the edges of recallSearch(): a PARTIAL hit (missing summary/timestamp,
// as a partial/unwired backend may return) must render without crashing, and a
// rejected search must surface the error while the screen's root testid survives
// the async settle. RecallSearch calls recallSearch() on mount and on submit.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

const recallSearch = vi.fn();

vi.mock("../../src/lib/recall", () => ({
  recallSearch: (...a: unknown[]) => recallSearch(...a),
  RECALL_KIND_META: {
    food: { label: "飲食", emoji: "🍽" },
    focus: { label: "專注", emoji: "🎯" },
    habit: { label: "習慣", emoji: "✅" },
    text: { label: "文字", emoji: "📝" },
  },
}));

import RecallSearch from "../../src/screens/macos/RecallSearch";

describe("RecallSearch screen", () => {
  beforeEach(() => {
    recallSearch.mockReset();
    recallSearch.mockResolvedValue([]);
  });

  it("renders the screen", async () => {
    render(<RecallSearch />);
    expect(screen.getByTestId("recall-search")).toBeInTheDocument();
    // initial mount fires an empty-query recall
    await waitFor(() => expect(recallSearch).toHaveBeenCalled());
  });

  it("does NOT crash when search returns a PARTIAL hit missing summary/timestamp", async () => {
    // Edge: a partial/unwired backend returns a hit with no summary and an
    // unparseable timestamp + an unknown kind (not in RECALL_KIND_META). The
    // result list must render the fallback and not throw.
    recallSearch.mockResolvedValue([
      { eventId: "ev-1", kind: "mystery", summary: undefined, timestamp: "" },
    ]);
    render(<RecallSearch />);
    // submit the search form (primary interaction)
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "沙拉" } });
    fireEvent.submit(input.closest("form")!);
    await waitFor(() =>
      expect(recallSearch).toHaveBeenCalledWith(
        expect.objectContaining({ query: "沙拉" }),
      ),
    );
    // screen survives rendering the partial hit
    expect(screen.getByTestId("recall-search")).toBeInTheDocument();
  });

  it("does NOT crash when recallSearch rejects (surfaces error, screen survives)", async () => {
    recallSearch.mockRejectedValue("recall_search.failed: boom");
    render(<RecallSearch />);
    const input = screen.getByRole("textbox");
    fireEvent.change(input, { target: { value: "deep work" } });
    fireEvent.submit(input.closest("form")!);
    await waitFor(() =>
      expect(recallSearch).toHaveBeenCalledWith(
        expect.objectContaining({ query: "deep work" }),
      ),
    );
    // error path: message shown, screen still mounted
    await waitFor(() =>
      expect(screen.getByText(/recall_search\.failed/)).toBeInTheDocument(),
    );
    expect(screen.getByTestId("recall-search")).toBeInTheDocument();
  });
});
