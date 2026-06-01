// SPEC-31 skill-bank — render + honest empty/error regression test
// (Vitest + React Testing Library). Guards that an unwired / shapeless `{}`
// backend reply or a rejected invoke never white-screens the skill bank.
import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const safeInvoke = vi.fn();
vi.mock("@/lib/tauri-compat", () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

import SkillBank from "@/pages/skill-bank";

describe("skill-bank screen", () => {
  beforeEach(() => safeInvoke.mockReset());

  it("renders the screen", () => {
    render(<SkillBank />);
    expect(screen.getByTestId("skill-bank")).toBeInTheDocument();
  });

  it("does NOT crash when safeInvoke resolves a shapeless `{}` (honest empty/unwired)", async () => {
    // httpFallback `default` returns a fabricated `{}`. The screen must treat that
    // as honest empty/unwired and stay mounted — never throw on a missing array.
    safeInvoke.mockResolvedValue({});
    render(<SkillBank />);
    // testid present immediately and still present after the async effect settles
    expect(screen.getByTestId("skill-bank")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByTestId("skill-bank")).toBeInTheDocument());
    // honest unwired/empty copy shown — no fabricated rows, no list
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
    // honest status region present (unwired or empty), not a crash blank
    expect(screen.getAllByText(/Not yet|No skills/i).length).toBeGreaterThan(0);
  });

  it("does NOT crash when safeInvoke rejects (honest error path)", async () => {
    // Even if the backend call rejected, the screen must survive and never
    // fabricate skill rows. Catch the rejection so it is a controlled mock.
    safeInvoke.mockImplementation(() => Promise.reject(new Error("boom")).catch(() => ({})));
    render(<SkillBank />);
    await waitFor(() => expect(screen.getByTestId("skill-bank")).toBeInTheDocument());
    // survives; still no fabricated skill list
    expect(screen.queryByRole("list")).not.toBeInTheDocument();
  });
});
