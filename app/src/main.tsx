import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import "./index.css";
import { maybeRedirectToRemoteFrontend } from "./mobileThinShell";
import { installBrokerLoginBridge } from "./lib/brokerLogin";

// On-screen boot/error visualiser. A physical iOS device (or the sim) has no
// remote console, so a white-screen crash is otherwise undebuggable. Error
// banners (`[ERR]`/`[REJ]`, passed force:true) ALWAYS paint so production
// crashes stay visible on-device. The informational `[diag]` boot trace is
// noise over the real UI, so it only paints when explicitly enabled — a dev
// build, or localStorage `spectyn_mesh_diag=1` for field debugging.
const DIAG_VERBOSE = (() => {
  try {
    // import.meta.env isn't typed in this tsconfig (no vite/client ref), so
    // read it through a cast rather than pulling in global vite types.
    const env = (import.meta as unknown as { env?: { DEV?: boolean } }).env;
    return Boolean(env?.DEV) || localStorage.getItem("spectyn_mesh_diag") === "1";
  } catch { return false; }
})();

function _diagAppend(text: string, bg = "#fc0", opts: { force?: boolean } = {}) {
  if (!opts.force && !DIAG_VERBOSE) return;
  try {
    const id = "__spectyn_diag_log";
    let log = document.getElementById(id);
    if (!log) {
      log = document.createElement("div");
      log.id = id;
      // padding-top clears the status bar / Dynamic Island via the safe-area inset.
      log.style.cssText = "position:fixed;top:0;left:0;right:0;z-index:99999;background:" + bg + ";color:#000;padding:calc(env(safe-area-inset-top, 0px) + 4px) 4px 4px;font:10px monospace;text-align:left;max-height:40%;overflow:auto;white-space:pre-wrap";
      document.body.appendChild(log);
    } else if (opts.force) {
      log.style.background = bg; // surface the error colour even if the banner already exists
    }
    log.appendChild(document.createTextNode(text + "\n"));
  } catch (_e) { /* swallow */ }
}
_diagAppend("[diag] main.tsx executed " + new Date().toLocaleTimeString());

// Expose to all code so any module can append a debug line to the yellow
// on-screen log without importing this file. Useful for debugging the
// cluster-dispatch flow on devices where we don't have a remote console.
(window as { spectynDiag?: (msg: string, bg?: string) => void }).spectynDiag = _diagAppend;

window.addEventListener("error", (e) => {
  const stack = e.error && e.error.stack ? String(e.error.stack).slice(0, 900) : "";
  const head = "[ERR] " + (e.message || "?") + " @ " + (e.filename || "?") + ":" + (e.lineno || "?");
  _diagAppend(stack ? head + "\n" + stack : head, "#f88", { force: true });
});
window.addEventListener("unhandledrejection", (e) => {
  _diagAppend("[REJ] " + String((e as PromiseRejectionEvent).reason).slice(0, 200), "#f88", { force: true });
});

// Wire deep-link → broker_login_finish handoff before any Safari →
// app callback can fire. Installs idempotently; safe to call early.
installBrokerLoginBridge().catch((e) =>
  console.warn("[boot] installBrokerLoginBridge failed:", e),
);

// Demo-mode deeplink: `spectyn://demo-mode` toggles a localStorage flag
// that App.tsx reads to skip MobileOnboardingV2 + land on /settings/cluster.
// Rust forwards this URL via app_handle.emit("deep-link://demo-mode", ...)
// (allowlisted in app/src-tauri/src/lib.rs alongside the OAuth callback).
// Production / release IPAs include this listener but are inert until the
// URL is fired — no credentials, no server state, just a navigation flag.
import { listen } from "@tauri-apps/api/event";
listen<string>("deep-link://demo-mode", () => {
  try {
    localStorage.setItem("spectyn_mesh_demo_mode_enabled", "true");
    console.log("[boot] demo-mode enabled via deep-link; reloading to /settings/cluster");
    window.location.reload();
  } catch (e) {
    console.warn("[boot] demo-mode setItem failed:", e);
  }
}).catch((e) => console.warn("[boot] demo-mode listener install failed:", e));

// iOS default: render React UI with the embedded agent — LLM calls go
// directly from this device to api.openai.com / api.groq.com / etc.
// using the keys broker_sync_from_vault pulled into ~/.spectyn-mesh/env.
//
// Thin-shell (forwarding chat to a remote coordinator) is opt-in via
// the peer-picker in MobileBrokerLogin — that path sets SPECTYN_THIN_
// SHELL=1 and the user explicitly chose "let my Mac handle this".
const useThinShell = (() => {
  try { return localStorage.getItem("SPECTYN_THIN_SHELL") === "1"; }
  catch { return false; }
})();

if (!(useThinShell && maybeRedirectToRemoteFrontend())) {
  _diagAppend("[diag] React mount starting");
  const rootEl = document.getElementById("root");
  if (!rootEl) {
    _diagAppend("[ERR] #root element not found in DOM", "#f88", { force: true });
  } else {
    try {
      ReactDOM.createRoot(rootEl).render(
        <React.StrictMode>
          <BrowserRouter>
            <App />
          </BrowserRouter>
        </React.StrictMode>
      );
      _diagAppend("[diag] React.render() returned");
    } catch (e) {
      _diagAppend("[ERR] React.render threw: " + String(e).slice(0, 200), "#f88", { force: true });
    }
  }
}
