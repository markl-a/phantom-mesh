// SPEC-31 capture-focus — render + edge-case regression test (Vitest + React Testing Library).
// Guards the honest-error path: when startSession resolves a NON-string (e.g. {}),
// meaning the backend is not yet wired, the screen must surface an honest
// "後端未就緒" error and NOT crash.
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const startSession = vi.fn();
vi.mock("@/lib/captureFocus", () => ({
  DEFAULT_DURATION_MS: {
    pomodoro25: 25 * 60 * 1000,
    deep_work50: 50 * 60 * 1000,
    sprint10: 10 * 60 * 1000,
    custom: 25 * 60 * 1000,
  },
  buildSessionRequest: (mode: string, opts: Record<string, unknown> = {}) => ({
    mode,
    plannedDurationMs: opts.plannedDurationMs ?? 25 * 60 * 1000,
    label: opts.label ?? null,
    tag: ["focus"],
  }),
  startSession: (...args: unknown[]) => startSession(...args),
  describeFocusError: (e: unknown) => String(e),
}));

import CaptureFocus from "@/pages/capture-focus";

describe("capture-focus screen", () => {
  beforeEach(() => startSession.mockReset());

  it("renders the screen", () => {
    render(<CaptureFocus />);
    expect(screen.getByTestId("capture-focus")).toBeInTheDocument();
  });

  it("does NOT crash and shows honest '後端未就緒' when startSession resolves a non-string", async () => {
    // The edge: startSession is typed Promise<string> but an unwired backend may
    // resolve a non-string ({}). The screen must guard `typeof id === "string"`
    // and surface an honest error instead of crashing / falsely claiming success.
    startSession.mockResolvedValue({});
    render(<CaptureFocus />);

    // select the 25-minute (pomodoro25) duration radio
    const radio = screen.getByLabelText("25 分鐘 / Pomodoro 25");
    fireEvent.click(radio);

    // click the sticky-footer Start CTA
    const buttons = screen.getAllByRole("button");
    fireEvent.click(buttons[buttons.length - 1]);

    await waitFor(() => expect(startSession).toHaveBeenCalled());

    // honest error shown, success NOT shown, and screen survives (no crash)
    await waitFor(() =>
      expect(screen.getByText("無法開始 session（後端未就緒）")).toBeInTheDocument(),
    );
    expect(screen.queryByText("已開始焦點 session")).not.toBeInTheDocument();
    expect(screen.getByTestId("capture-focus")).toBeInTheDocument();
  });
});
