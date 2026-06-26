// UpdatePanel renders the bare `__APP_VERSION__` identifier that the Vite
// `define` (vite.config.ts) — and its mirror in vitest.config.ts — replace at
// build time with the package.json version. This guards both halves:
//   1. the define is actually configured in the vitest pipeline (otherwise
//      rendering UpdatePanel throws ReferenceError: __APP_VERSION__ is not
//      defined), and
//   2. the substituted value is the real package version, not the literal
//      "__APP_VERSION__" token a quoted define would leave behind.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import UpdatePanel from "@/components/settings/UpdatePanel";
import pkg from "../../package.json";

describe("UpdatePanel version define", () => {
  it("renders the build-time package version, not the literal token", () => {
    render(<UpdatePanel />);
    expect(
      screen.getByText(`目前版本：v${pkg.version}`)
    ).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("__APP_VERSION__");
  });
});
