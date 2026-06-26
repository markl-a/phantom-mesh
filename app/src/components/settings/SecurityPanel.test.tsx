// NO-FAKING regression — apex-④ audit surface.
//
// Guards that SecurityPanel never fabricates audit rows. When the
// `get_audit_log` invoke REJECTS, the panel must render an HONEST
// offline/empty banner and ZERO audit rows (no MOCK 'AUD-' entries).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";

// Mock the tauri-compat invoke wrapper so the component's `get_audit_log`
// call rejects, simulating an offline / unavailable backend.
const invokeMock = vi.fn();
vi.mock("../../lib/tauri-compat", () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => invokeMock(...args),
}));

import SecurityPanel from "./SecurityPanel";

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockRejectedValue(new Error("backend unavailable"));
});

afterEach(() => {
  cleanup();
});

describe("SecurityPanel — NO-FAKING audit surface", () => {
  it("renders an honest offline banner and ZERO audit rows when get_audit_log rejects", async () => {
    render(<SecurityPanel />);

    // The honest offline banner must appear.
    const banner = await screen.findByTestId("audit-offline-banner");
    expect(banner).toBeInTheDocument();
    expect(banner.textContent ?? "").toContain("無法取得審計日誌");

    // Wait for loading to settle (refresh button only shows when !loading).
    await waitFor(() => {
      expect(screen.getByText("重新整理")).toBeInTheDocument();
    });

    // ZERO fabricated 'AUD-' rows anywhere in the rendered output.
    expect(document.body.textContent ?? "").not.toMatch(/AUD-\d+/);

    // And the honest empty-state row is shown instead of invented data.
    expect(screen.getByText(/離線中 — 無審計資料可顯示/)).toBeInTheDocument();
  });
});
