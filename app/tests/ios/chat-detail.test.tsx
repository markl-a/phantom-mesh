// SPEC-31 chat-detail — render-smoke test (Vitest + React Testing Library).
// chat-detail is a thin mobile wrapper around the heavy self-managing
// ConversationView (history load, Tauri calls, streaming). We mock ConversationView
// so this test exercises the wrapper FRAME in isolation: it must render its
// `role="main"` chat container AND mount ConversationView inside it. Catches an
// accidental break of the wrapper structure.
import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";

vi.mock("@/components/conversation/ConversationView", () => ({
  default: () => <div data-testid="conversation-view-stub" />,
}));

import ChatDetail from "@/pages/chat-detail";

describe("chat-detail screen", () => {
  it("renders the mobile chat frame", () => {
    render(<ChatDetail />);
    // wrapper frame present
    expect(screen.getByTestId("chat-detail")).toBeInTheDocument();
    // wrapper mounts ConversationView (mocked stub)
    expect(screen.getByTestId("conversation-view-stub")).toBeInTheDocument();
  });
});
