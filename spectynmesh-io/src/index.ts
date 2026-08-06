// phantommesh.io — entry point.
//
// Wires every route the spectyn CLI's `login_broker` flow needs, plus a
// minimal landing + login + dashboard surface for the browser.

import { Hono } from "hono";
import { cors } from "hono/cors";
import { secureHeaders } from "hono/secure-headers";
import type { Env } from "./types";
import { health } from "./routes/health";
import { authStart, webStart, logout, googleStart, googleCallback, appleStart, appleCallback } from "./routes/oauth";
import { emailLogin, emailRegister } from "./routes/email";
import { loginPage, accountPage, landingPage } from "./routes/pages";
import { me, devices, claimDevice, revokeDevice, getSettings, getSettingsRaw, putSettings, getClusterPeers, putClusterPeers, upsertClusterPeer, getSessions, postSessionHeartbeat, deleteSession } from "./routes/api";
import { distHandler, installScript, installShellScript } from "./routes/dist";
// F205: new endpoints layered on top of the existing OAuth broker.
import { getPeerCapAggregate } from "./routes/cluster";
import { startDispatch, cancelDispatch, publishChunk, subscribeStream } from "./routes/dispatch";
import { listHistory, getHistoryItem, exportHistory } from "./routes/history";
import { getPreferences, putPreferences, getPeerCapabilities, putPeerCapabilities, revokeAllOtherSessions } from "./routes/settings_ext";
import { listRecipesRoute, getRecipeRoute, postRecipeRoute, putRecipeRoute, deleteRecipeRoute } from "./routes/recipes";
// SPEC-15 E2EE vault (DRAFT) — dumb-storage routes that store/return
// client-sealed ciphertext VERBATIM. No ENV_VAULT_KEY, no deriveUserKey,
// no crypto.subtle.decrypt. Replaces the legacy plaintext /settings/raw path.
import { vaultSet, vaultGet, vaultWipe, vaultWipeStatus, vaultKeysWrap, vaultKeysWrapped } from "./routes/vault";

// F200: dashboard SPA shell handler (serves /app/* from R2)
import { appHandler } from "./routes/app";

// F205: Durable Object class must be exported from the worker module so
// the Workers runtime can instantiate it. The class itself lives in
// src/durable/dispatch_stream.ts; we re-export here to satisfy the
// `class_name = "DispatchStream"` declaration in wrangler.toml.
export { DispatchStream } from "./durable/dispatch_stream";

const app = new Hono<{ Bindings: Env }>();

app.use("*", secureHeaders());
// API endpoints. Only the production browser surface needs CORS — the
// CLI uses Rust reqwest (server-side fetch with `Authorization: Bearer
// <broker_token>`) which never triggers a browser CORS preflight, so
// dropping loopback origins doesn't break spectyn CLI.
//
// Audit B1 (HIGH) — 2026-05-15: previously the allowlist also included
// `http://127.0.0.1:48181` and `http://localhost:48181`, intended for
// the CLI loopback redirect. That gave any local web server on :48181
// (random tutorial / Vite dev / copilot demo) the same trust level as
// `phantommesh.io`; combined with `Authorization` in `allowHeaders`,
// a captured broker JWT could be replayed cross-origin from a malicious
// page. Loopback origins removed here; CLI flow verified unaffected.
// See docs/superpowers/audits/2026-05-15-broker-audit.md §2.2 B1.
app.use("/api/*", cors({
  origin: ["https://phantommesh.io"],
  allowMethods: ["GET", "POST", "PUT", "DELETE", "OPTIONS"],
  allowHeaders: ["Authorization", "Content-Type"],
}));

// SPEC-15 E2EE vault — same browser-origin allowlist as /api/*. The CLI
// uses Rust reqwest (no CORS preflight); the dashboard would call from
// https://phantommesh.io. X-Confirm-Wipe is allowed for DELETE /vault/wipe.
app.use("/vault/*", cors({
  origin: ["https://phantommesh.io"],
  allowMethods: ["GET", "POST", "DELETE", "OPTIONS"],
  allowHeaders: ["Authorization", "Content-Type", "X-Confirm-Wipe"],
}));

// Health probe — spectyn CLI hits this with a 3-second timeout the
// instant `spectyn login` is run with no args.
app.get("/api/health", health);

// CLI bootstrap — CLI redirects the browser here with device_id + port + redirect.
app.get("/auth/cli/start", authStart);

// Browser-only login entry — no CLI in the loop, finishes by setting
// a spectyn_session cookie and landing on /account.
app.get("/auth/web/start", webStart);
app.post("/auth/logout", logout);

// OAuth provider start + callback. The "start" routes redirect the browser
// to the IdP's login page; the IdP redirects back to "callback", which
// finishes the dance and 302s the browser to the loopback redirect with
// the identity payload.
app.get("/auth/google/start", googleStart);
app.get("/auth/google/callback", googleCallback);

// Apple replies via response_mode=form_post → the callback is a POST.
app.get("/auth/apple/start", appleStart);
app.post("/auth/apple/callback", appleCallback);

// Email tier — local-only, doesn't touch any IdP.
app.post("/auth/email/login", emailLogin);
app.post("/auth/email/register", emailRegister);

// Authenticated API surface (used by spectyn CLI's `spectyn devices` and
// the broker's web dashboard). Bearer token = broker_token issued during
// the OAuth callback.
app.get("/api/me", me);
app.get("/api/devices", devices);
app.post("/api/devices/:device_id/claim", claimDevice);
app.delete("/api/devices/:device_id", revokeDevice);

// User-scoped LLM key vault. /settings = masked (dashboard); /settings/raw =
// full values (CLI `spectyn config pull`); PUT replaces the whole map.
// Multi-tenant: every authenticated user has their own vault, scoped
// by user_id from the JWT in routes/api.ts. (Lifted from the v1
// single-email allowlist on 2026-05-04; UI gate removed 2026-05-05.)
app.get("/api/me/settings",      getSettings);
// DEPRECATED (SPEC-15 E2EE migration): /api/me/settings/raw returns
// SERVER-DECRYPTED plaintext, which breaks the E2EE guarantee (the broker
// holds ENV_VAULT_KEY and decrypts). It is being replaced by the
// client-sealed /vault/* routes below, where the broker only ever moves
// ciphertext. MIGRATION TODO: once clients fully cut over to /vault/get
// (SPEC-15 §8.C), delete this route + getSettingsRaw + decryptForUser +
// ENV_VAULT_KEY. Kept live for now so existing CLI/Tauri callers keep
// working during the migration. See
// docs/integration/2026-05-29-spec15-vault-verification.md.
app.get("/api/me/settings/raw",  getSettingsRaw);
app.put("/api/me/settings",      putSettings);

// ── SPEC-15 E2EE vault (DRAFT) — DUMB STORAGE ─────────────────────────
// These routes store and return CLIENT-SEALED ciphertext VERBATIM. The
// broker never decrypts: no ENV_VAULT_KEY, no deriveUserKey, no
// crypto.subtle.decrypt. `client_hmac_hex` is stored opaquely and echoed
// back as `server_hmac_hex` (the broker has no key to verify it). Auth via
// the existing broker JWT (authn in routes/api.ts).
app.post(  "/vault/set",            vaultSet);
app.get(   "/vault/get",            vaultGet);
app.delete("/vault/wipe",           vaultWipe);
app.get(   "/vault/wipe/:wipe_id",  vaultWipeStatus);
// New-device seal-key courier: broker stores/forwards the age-wrapped
// VaultSealKey but can never unwrap it.
app.post(  "/vault/keys/wrap",      vaultKeysWrap);
app.get(   "/vault/keys/wrapped",   vaultKeysWrapped);

// Cluster peer registry — vault-backed list that replaces the hardcoded
// CLUSTER_TOPOLOGY constant in core/src/cli_config.rs. Same multi-tenant
// scoping as /settings — each user manages their own peer list.
app.get("/api/me/cluster-peers",         getClusterPeers);
app.put("/api/me/cluster-peers",         putClusterPeers);
app.post("/api/me/cluster-peers/upsert", upsertClusterPeer);

// Live TUI session presence — heartbeat from each spectyn TUI on launch
// + every 30s; GET shows the union across the user's machines (rows
// older than 60s are filtered out as stale).
app.get("/api/me/sessions",              getSessions);
app.post("/api/me/sessions/heartbeat",   postSessionHeartbeat);
// F205: bulk revoke MUST come before the :id parametric route so the
// Hono trie matches the static path first. Otherwise "all-others" would
// be captured as :id and deleteSession would receive it as a session id.
app.delete("/api/me/sessions/all-others", revokeAllOtherSessions);
app.delete("/api/me/sessions/:id",       deleteSession);

// ── F205: dashboard control-plane endpoints ───────────────────────────
// Per-peer cap aggregate for the F201 cluster screen.
app.get("/api/me/cluster-peers/:peer/caps", getPeerCapAggregate);

// Dispatch start / cancel / stream-fan-out.
// Streaming model: see src/routes/dispatch.ts header. POST start writes
// a D1 row, returns job_id; SPA then talks to localhost `spectyn serve`
// directly and pushes received chunks to POST stream/:job_id for cross-
// tab fan-out via the DispatchStream Durable Object.
app.post("/api/me/dispatch/start",           startDispatch);
app.post("/api/me/dispatch/:job_id/cancel",  cancelDispatch);
app.post("/api/me/dispatch/stream/:job_id",  publishChunk);
app.get( "/api/me/dispatch/stream/:job_id",  subscribeStream);

// Dispatch history (F203).
app.get("/api/me/dispatches",               listHistory);
app.get("/api/me/dispatches/export",        exportHistory);
app.get("/api/me/dispatches/:job_id",       getHistoryItem);

// Saved dispatch recipes (F202).
app.get(   "/api/me/recipes",     listRecipesRoute);
app.post(  "/api/me/recipes",     postRecipeRoute);
app.get(   "/api/me/recipes/:id", getRecipeRoute);
app.put(   "/api/me/recipes/:id", putRecipeRoute);
app.delete("/api/me/recipes/:id", deleteRecipeRoute);

// Dashboard preferences (F204).
app.get("/api/me/preferences", getPreferences);
app.put("/api/me/preferences", putPreferences);

// Narrow peer-capabilities editor — distinct from the bulk PUT
// /api/me/cluster-peers so the dashboard can update caps without
// re-sending the whole peer list (which races with the CLI's auto-
// register on other machines).
app.get("/api/me/peer-capabilities", getPeerCapabilities);
app.put("/api/me/peer-capabilities", putPeerCapabilities);

// Browser-facing pages — server-rendered HTML, no SPA.
app.get("/", landingPage);
app.get("/login", loginPage);
app.get("/account", accountPage);

// Public binary distribution. /install.ps1 returns a curl-pipeable
// PowerShell installer; /dist/<name> streams the matching R2 object.
// Both are unauthenticated by design — the binary itself is public,
// the user's LLM keys live behind /api/me/* and are still gated.
app.get("/install.ps1",  installScript);
app.get("/install.sh",   installShellScript);
app.get("/dist/:name",   distHandler);

// Dashboard SPA (E003 / F200). `/app` is the canonical entry point;
// `/app/*` deep links all return the same `index.html` so React
// Router can take over client-side. Strict CSP applied per route.
// Built bundle ships from R2 under the `app/` prefix (see
// src/routes/app.ts for the lookup + fallback).
app.get("/app",      appHandler);
app.get("/app/*",    appHandler);

// Browsers always request /favicon.ico on first visit; without a
// handler that's a noisy 404 in the console. Serve the brand glyph
// (◆) inline as SVG so we don't need to ship a binary asset.
app.get("/favicon.ico", (c) => {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><text y="26" font-size="28" fill="#d6b270">◆</text></svg>`;
  return new Response(svg, {
    headers: { "Content-Type": "image/svg+xml", "Cache-Control": "public, max-age=86400" },
  });
});

// 404 — surface the API endpoints inline so the CLI's broker probe gets
// a useful response if it accidentally hits the wrong path.
app.notFound((c) => {
  return c.json({
    status: "not_found",
    hint: "phantommesh.io broker. CLI surface: /api/health, /auth/cli/start. See https://github.com/markl-a/spectyn-mesh/blob/main/docs/SPECTYNMESH-IO-DESIGN.md",
  }, 404);
});

app.onError((err, c) => {
  console.error("[broker]", err);
  return c.json({ error: err.message ?? "internal" }, 500);
});

export default app;
