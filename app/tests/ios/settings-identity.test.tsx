// SPEC-31 settings-identity — render + regression test (Vitest + React Testing
// Library). Guards the white-screen class: a partial IdentityStatus (hasIdentity
// true but `fingerprint` undefined) must NOT crash the screen via
// `identity.fingerprint.length` in the `hasKey` calc.
import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const loadIdentityStatus = vi.fn();
vi.mock("@/lib/identity", () => ({
  loadIdentityStatus: (...args: unknown[]) => loadIdentityStatus(...args),
}));

import SettingsIdentity from "@/pages/settings-identity";

describe("settings-identity screen", () => {
  beforeEach(() => loadIdentityStatus.mockReset());

  it("renders the screen", async () => {
    loadIdentityStatus.mockResolvedValue({
      hasIdentity: true,
      fingerprint: "AA:BB:CC",
      createdAt: "2026-01-01T00:00:00Z",
      keystore: "/keys/user42",
      identityLine: "user42",
    });
    render(<SettingsIdentity />);
    await waitFor(() =>
      expect(screen.getByTestId("settings-identity")).toBeInTheDocument(),
    );
  });

  it("does NOT crash on a partial status missing `fingerprint`", async () => {
    // The bug: `hasKey = identity.hasIdentity && identity.fingerprint.length > 0`
    // throws "Cannot read properties of undefined (reading 'length')" when an
    // unwired/partial backend reports hasIdentity:true but omits fingerprint.
    loadIdentityStatus.mockResolvedValue({
      hasIdentity: true,
      // no fingerprint / createdAt / keystore / identityLine
    } as unknown);
    render(<SettingsIdentity />);
    // screen survives the async load + render (would have thrown pre-fix)
    await waitFor(() =>
      expect(screen.getByTestId("settings-identity")).toBeInTheDocument(),
    );
  });
});
