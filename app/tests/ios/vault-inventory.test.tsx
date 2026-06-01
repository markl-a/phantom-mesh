// SPEC-31 vault-inventory — render + honest-state regression test (Vitest + RTL).
// The screen has NO read-only list command wired, so it must render honest
// loading → locked / empty / error states driven only by the truthiness of the
// `broker_login_status` probe. This guards that a {} resolve, a null resolve,
// and a rejection each land in a non-crashing state (testid survives async).
import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const safeInvoke = vi.fn();
vi.mock("@/lib/tauri-compat", () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

import VaultInventory from "@/pages/vault-inventory";

describe("vault-inventory screen", () => {
  // NB: reset the mock inline in each test rather than in a beforeEach hook.
  // A beforeEach reset trips a vitest unhandled-rejection false-positive on the
  // rejecting-probe test below, even though the component fully catches it
  // (verified in isolation — the screen renders the honest error state fine).
  it("renders the screen", async () => {
    safeInvoke.mockReset();
    safeInvoke.mockResolvedValue(null);
    render(<VaultInventory />);
    expect(screen.getByTestId("vault-inventory")).toBeInTheDocument();
    // let the in-flight probe settle so no act() warning / unmounted-setState
    await waitFor(() =>
      expect(screen.getByLabelText(/Vault locked/i)).toBeInTheDocument(),
    );
  });

  it("does NOT crash on a logged-in probe that resolves a bare object → honest empty", async () => {
    // A truthy {} (logged in / reachable) must NOT be treated as entries: with no
    // list command wired, the screen shows the honest empty state, not a crash.
    safeInvoke.mockReset();
    safeInvoke.mockResolvedValue({});
    render(<VaultInventory />);
    await waitFor(() =>
      expect(screen.getByLabelText(/No entries to show/i)).toBeInTheDocument(),
    );
    // screen survives the async settle
    expect(screen.getByTestId("vault-inventory")).toBeInTheDocument();
  });

  it("does NOT crash when the probe rejects → honest error state", async () => {
    // The component awaits safeInvoke inside try/catch → a rejection must land
    // in the honest "error" state, never an uncaught crash.
    safeInvoke.mockReset();
    safeInvoke.mockImplementation(() => Promise.reject(new Error("probe boom")));
    render(<VaultInventory />);
    await waitFor(() =>
      expect(screen.getByLabelText(/Failed to read vault/i)).toBeInTheDocument(),
    );
    // screen survives the rejection
    expect(screen.getByTestId("vault-inventory")).toBeInTheDocument();
    expect(screen.getByText(/probe boom/)).toBeInTheDocument();
  });
});
