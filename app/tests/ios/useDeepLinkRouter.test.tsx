// SPEC-31 §6 / G1 — useDeepLinkRouter allowlist + V8-HIGH-5 defense-in-depth test.
//
// The hook subscribes to the `deep-link://route` Tauri event (lib.rs forwards ONLY
// allowlist-gated, credential-free navigation URLs there — Rust is the primary gate).
// This test exercises the FRONTEND second gate (defense-in-depth): an allowlisted
// phantom:// route → React Router navigate(); a disallowed/unknown route → NO navigate
// (logged ios.deeplink.disallowed). It also confirms query params are preserved and the
// non-Tauri (web) path is a harmless no-op.
import { render, act, waitFor } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { vi, describe, it, expect, beforeEach } from "vitest";

// Capture the listener callback registered for "deep-link://route" so the test can
// fire synthetic OS deep-link events at it. listen() resolves to an unlisten fn.
const listeners: Record<string, (evt: { payload: string }) => void> = {};
const listenMock = vi.fn((event: string, cb: (evt: { payload: string }) => void) => {
  listeners[event] = cb;
  return Promise.resolve(() => { delete listeners[event]; });
});
vi.mock("@tauri-apps/api/event", () => ({
  listen: (event: string, cb: (evt: { payload: string }) => void) => listenMock(event, cb),
}));

// Mock useNavigate; keep the rest of react-router-dom real (MemoryRouter etc.).
const navigate = vi.fn();
vi.mock("react-router-dom", async (orig) => ({
  ...(await orig<typeof import("react-router-dom")>()),
  useNavigate: () => navigate,
}));

import { useDeepLinkRouter } from "../../src/lib/useDeepLinkRouter";

function Harness() {
  useDeepLinkRouter();
  return null;
}

async function mountAndGetEmit() {
  render(<MemoryRouter><Harness /></MemoryRouter>);
  // listen() resolves async — wait for the hook to register its callback.
  await waitFor(() => expect(listeners["deep-link://route"]).toBeTypeOf("function"));
  return (url: string) =>
    act(() => { listeners["deep-link://route"]({ payload: url }); });
}

describe("useDeepLinkRouter — allowlist routing", () => {
  beforeEach(() => { navigate.mockReset(); for (const k of Object.keys(listeners)) delete listeners[k]; });

  it("navigates for an allowlisted phantom:// route", async () => {
    const emit = await mountAndGetEmit();
    emit("phantom://coach/review");
    expect(navigate).toHaveBeenCalledWith("/coach/review");
  });

  it("preserves query params on an allowlisted route", async () => {
    const emit = await mountAndGetEmit();
    emit("phantom://coach/review?date=2026-05-30");
    expect(navigate).toHaveBeenCalledWith("/coach/review?date=2026-05-30");
  });

  it("does NOT navigate for a disallowed route (V8-HIGH-5 defense-in-depth)", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const emit = await mountAndGetEmit();
    emit("phantom://evil/payload");
    expect(navigate).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledWith("ios.deeplink.disallowed", "/evil/payload");
    warn.mockRestore();
  });

  it("does NOT navigate for a non-phantom URL", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const emit = await mountAndGetEmit();
    emit("https://example.com/hack");
    expect(navigate).not.toHaveBeenCalled();
    warn.mockRestore();
  });
});
