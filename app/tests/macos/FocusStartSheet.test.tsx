// SPEC-41 §10.4 FocusStartSheet — render + regression test (Vitest + React Testing Library).
// Guards the edge case the screen already defends against: startSession resolving
// with a non-id value ({} from the web fallback for an unwired command) must NOT
// be forwarded as a session nor crash the sheet — it should surface an inline error.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

const startSession = vi.fn();
vi.mock("@/lib/captureFocus", () => ({
  DEFAULT_DURATION_MS: { pomodoro25: 1500000, deep_work50: 3000000, sprint10: 600000, custom: 1500000 },
  buildSessionRequest: (mode: string, opts: Record<string, unknown> = {}) => ({ mode, ...opts }),
  startSession: (...args: unknown[]) => startSession(...args),
  describeFocusError: (e: unknown) => String(e),
}));

import FocusStartSheet from "@/screens/macos/FocusStartSheet";

describe("FocusStartSheet screen", () => {
  beforeEach(() => startSession.mockReset());

  it("renders the screen", () => {
    render(<FocusStartSheet />);
    expect(screen.getByTestId("focus-start-sheet")).toBeInTheDocument();
  });

  it("does NOT crash and shows an error when startSession resolves with a non-id value", async () => {
    // Edge: web fallback / unwired backend resolves with {} instead of a session id string.
    startSession.mockResolvedValue({});
    const onStart = vi.fn();
    render(<FocusStartSheet onStart={onStart} onCancel={vi.fn()} />);

    // Primary interaction: click the "開始" (start) CTA in the footer (last button).
    const buttons = screen.getAllByRole("button");
    fireEvent.click(buttons[buttons.length - 1]);

    await waitFor(() => expect(startSession).toHaveBeenCalled());
    // Screen survives the non-id result render and surfaces the inline error.
    expect(screen.getByTestId("focus-start-sheet")).toBeInTheDocument();
    expect(onStart).not.toHaveBeenCalled();
    await waitFor(() => expect(screen.getByRole("alert")).toBeInTheDocument());
  });
});
