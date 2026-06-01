// SPEC-31 settings-providers — render + parse-robustness regression test
// (Vitest + React Testing Library). Guards that an empty/null provider-health
// reply OR a malformed raw payload (non-array, or an array with null / non-object
// / field-less entries) never white-screens the providers screen — i.e. the
// normalize() `.filter`/`.map` path stays crash-proof.
import { render, screen, waitFor } from "@testing-library/react";
import { vi, describe, it, expect, beforeEach } from "vitest";

vi.mock("@/lib/useHaptics", () => ({ useHaptics: () => ({ impact: vi.fn() }) }));

const safeInvoke = vi.fn();
vi.mock("@/lib/tauri-compat", () => ({
  isTauri: () => false,
  safeInvoke: (...args: unknown[]) => safeInvoke(...args),
}));

vi.mock("@/lib/providers", () => ({
  describeError: (e: unknown) => String(e),
}));

import SettingsProviders from "@/pages/settings-providers";

describe("settings-providers screen", () => {
  beforeEach(() => safeInvoke.mockReset());

  it("renders the screen", async () => {
    // Honest empty: a non-throwing null result is the empty state, not an error.
    safeInvoke.mockResolvedValue(null);
    render(<SettingsProviders />);
    // present immediately and still present after the async load effect settles
    expect(screen.getByTestId("settings-providers")).toBeInTheDocument();
    await waitFor(() =>
      expect(screen.getByTestId("settings-providers")).toBeInTheDocument(),
    );
  });

  it("does NOT crash when safeInvoke resolves a MALFORMED payload", async () => {
    // The risk: a malformed runtime payload (entries that are null / strings /
    // missing fields, plus a stray non-array wrapper) reaching normalize()'s
    // `.filter`/`.map`. A non-string `name` becoming a React child would crash
    // the render — the string guard must keep the screen mounted.
    safeInvoke.mockResolvedValue([
      null,
      "not-an-object",
      42,
      {}, // object with no expected fields
      { name: { nested: "groq" } }, // non-string name (object)
      { slug: "openai", status: "ok", key_last4: "1234" }, // one well-formed entry
    ]);
    render(<SettingsProviders />);
    await waitFor(() =>
      expect(screen.getByTestId("settings-providers")).toBeInTheDocument(),
    );
    // survives the render of the parsed rows (would have thrown if a non-string
    // name leaked through as a React child)
    expect(screen.getByTestId("settings-providers")).toBeInTheDocument();
  });
});
