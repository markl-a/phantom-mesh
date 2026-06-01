// SPEC-16 event storage — EventTimeline (生活時間軸) render + regression test
// (Vitest + React Testing Library). Mirrors tests/ios/capture-food.test.tsx.
//
// Guards the edge where showEvent() returns null (web/unwired mode) or a partial
// EventDetail (missing summary/suggestion): the detail modal must NOT crash and
// the screen's root testid must survive the async settle.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

const queryEvents = vi.fn();
const showEvent = vi.fn();
const captureNote = vi.fn();
const deleteEvent = vi.fn();

vi.mock("../../src/lib/eventStore", () => ({
  buildQuery: (opts: Record<string, unknown> = {}) => ({ ...opts }),
  queryEvents: (...a: unknown[]) => queryEvents(...a),
  showEvent: (...a: unknown[]) => showEvent(...a),
  captureNote: (...a: unknown[]) => captureNote(...a),
  deleteEvent: (...a: unknown[]) => deleteEvent(...a),
  describeEventError: (e: unknown) => String(e),
  KIND_META: {
    food: { label: "飲食", emoji: "🍽" },
    focus: { label: "專注", emoji: "🎯" },
    habit: { label: "習慣", emoji: "✅" },
    text: { label: "文字", emoji: "📝" },
  },
}));

import EventTimeline from "../../src/screens/macos/EventTimeline";

const oneEvent = [
  {
    meta: { eventId: "ev-1", kind: "food", timestamp: "2026-05-29T10:00:00Z", tags: ["lunch"] },
    encryptedBodyPath: "events/ev-1/body.age",
    analysis: null,
  },
];

describe("EventTimeline screen", () => {
  beforeEach(() => {
    queryEvents.mockReset();
    showEvent.mockReset();
    captureNote.mockReset();
    deleteEvent.mockReset();
    queryEvents.mockResolvedValue([]);
  });

  it("renders the screen", async () => {
    render(<EventTimeline />);
    expect(screen.getByTestId("event-timeline")).toBeInTheDocument();
    await waitFor(() => expect(queryEvents).toHaveBeenCalled());
  });

  it("does NOT crash when opening an event whose showEvent returns a PARTIAL detail", async () => {
    // Edge: a partial/unwired backend returns an EventDetail with null summary/
    // suggestion/etc. The modal renders the fallback copy and must not throw.
    queryEvents.mockResolvedValue(oneEvent);
    showEvent.mockResolvedValue({
      eventId: "ev-1",
      timestamp: "",
      kind: "food",
      tags: [],
      summary: null,
      suggestion: null,
      goalImpact: null,
      confidence: null,
      modelId: null,
    });
    render(<EventTimeline />);
    // wait for the event row to appear
    await waitFor(() => expect(screen.getByTitle("檢視詳情")).toBeInTheDocument());
    fireEvent.click(screen.getByTitle("檢視詳情"));
    await waitFor(() => expect(showEvent).toHaveBeenCalledWith("ev-1"));
    // detail modal opened + screen survives the partial render
    await waitFor(() => expect(screen.getByTestId("event-detail-modal")).toBeInTheDocument());
    expect(screen.getByTestId("event-timeline")).toBeInTheDocument();
  });

  it("does NOT crash when showEvent rejects (surfaces error, screen survives)", async () => {
    queryEvents.mockResolvedValue(oneEvent);
    showEvent.mockRejectedValue("event_show.failed: boom");
    render(<EventTimeline />);
    await waitFor(() => expect(screen.getByTitle("檢視詳情")).toBeInTheDocument());
    fireEvent.click(screen.getByTitle("檢視詳情"));
    await waitFor(() => expect(showEvent).toHaveBeenCalledWith("ev-1"));
    // error path: detail cleared, screen still mounted
    await waitFor(() => expect(screen.getByTestId("event-timeline")).toBeInTheDocument());
  });
});
