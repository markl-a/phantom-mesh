import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import "./index.css";
import { maybeRedirectToRemoteFrontend } from "./mobileThinShell";
import { installBrokerLoginBridge } from "./lib/brokerLogin";

// Wire deep-link → broker_login_finish handoff before any Safari →
// app callback can fire. Installs idempotently; safe to call early.
installBrokerLoginBridge().catch((e) =>
  console.warn("[boot] installBrokerLoginBridge failed:", e),
);

// iOS default: render React UI with the embedded agent — LLM calls go
// directly from this device to api.openai.com / api.groq.com / etc.
// using the keys broker_sync_from_vault pulled into ~/.phantom-mesh/env.
//
// Thin-shell (forwarding chat to a remote coordinator) is opt-in via
// the peer-picker in MobileBrokerLogin — that path sets PHANTOM_THIN_
// SHELL=1 and the user explicitly chose "let my Mac handle this".
const useThinShell = (() => {
  try { return localStorage.getItem("PHANTOM_THIN_SHELL") === "1"; }
  catch { return false; }
})();

if (!(useThinShell && maybeRedirectToRemoteFrontend())) {
  ReactDOM.createRoot(document.getElementById("root")!).render(
    <React.StrictMode>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </React.StrictMode>
  );
}
