// phantommesh.io — entry point.
//
// Wires every route the phantom CLI's `login_broker` flow needs, plus a
// minimal landing + login + dashboard surface for the browser.

import { Hono } from "hono";
import { cors } from "hono/cors";
import { secureHeaders } from "hono/secure-headers";
import type { Env } from "./types";
import { health } from "./routes/health";
import { authStart, webStart, logout, googleStart, googleCallback } from "./routes/oauth";
import { emailLogin, emailRegister } from "./routes/email";
import { loginPage, accountPage, landingPage } from "./routes/pages";
import { me, devices, claimDevice, revokeDevice, getSettings, getSettingsRaw, putSettings, getClusterPeers, putClusterPeers, upsertClusterPeer, getSessions, postSessionHeartbeat, deleteSession } from "./routes/api";
import { distHandler, installScript, installShellScript } from "./routes/dist";

const app = new Hono<{ Bindings: Env }>();

app.use("*", secureHeaders());
// API endpoints called by phantom CLI loopback or by self-hosted brokers
// pointing at us.
app.use("/api/*", cors({
  origin: ["https://phantommesh.io", "http://127.0.0.1:48181", "http://localhost:48181"],
  allowMethods: ["GET", "POST", "DELETE", "OPTIONS"],
  allowHeaders: ["Authorization", "Content-Type"],
}));

// Health probe — phantom CLI hits this with a 3-second timeout the
// instant `phantom login` is run with no args.
app.get("/api/health", health);

// CLI bootstrap — CLI redirects the browser here with device_id + port + redirect.
app.get("/auth/cli/start", authStart);

// Browser-only login entry — no CLI in the loop, finishes by setting
// a phantom_session cookie and landing on /account.
app.get("/auth/web/start", webStart);
app.post("/auth/logout", logout);

// OAuth provider start + callback. The "start" routes redirect the browser
// to the IdP's login page; the IdP redirects back to "callback", which
// finishes the dance and 302s the browser to the loopback redirect with
// the identity payload.
app.get("/auth/google/start", googleStart);
app.get("/auth/google/callback", googleCallback);

// Email tier — local-only, doesn't touch any IdP.
app.post("/auth/email/login", emailLogin);
app.post("/auth/email/register", emailRegister);

// Authenticated API surface (used by phantom CLI's `phantom devices` and
// the broker's web dashboard). Bearer token = broker_token issued during
// the OAuth callback.
app.get("/api/me", me);
app.get("/api/devices", devices);
app.post("/api/devices/:device_id/claim", claimDevice);
app.delete("/api/devices/:device_id", revokeDevice);

// User-scoped LLM key vault. /settings = masked (dashboard); /settings/raw =
// full values (CLI `phantom config pull`); PUT replaces the whole map.
// Multi-tenant: every authenticated user has their own vault, scoped
// by user_id from the JWT in routes/api.ts. (Lifted from the v1
// single-email allowlist on 2026-05-04; UI gate removed 2026-05-05.)
app.get("/api/me/settings",      getSettings);
app.get("/api/me/settings/raw",  getSettingsRaw);
app.put("/api/me/settings",      putSettings);

// Cluster peer registry — vault-backed list that replaces the hardcoded
// CLUSTER_TOPOLOGY constant in core/src/cli_config.rs. Same multi-tenant
// scoping as /settings — each user manages their own peer list.
app.get("/api/me/cluster-peers",         getClusterPeers);
app.put("/api/me/cluster-peers",         putClusterPeers);
app.post("/api/me/cluster-peers/upsert", upsertClusterPeer);

// Live TUI session presence — heartbeat from each phantom TUI on launch
// + every 30s; GET shows the union across the user's machines (rows
// older than 60s are filtered out as stale).
app.get("/api/me/sessions",              getSessions);
app.post("/api/me/sessions/heartbeat",   postSessionHeartbeat);
app.delete("/api/me/sessions/:id",       deleteSession);

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
    hint: "phantommesh.io broker. CLI surface: /api/health, /auth/cli/start. See https://github.com/markl-a/phantom-mesh/blob/main/docs/PHANTOMMESH-IO-DESIGN.md",
  }, 404);
});

app.onError((err, c) => {
  console.error("[broker]", err);
  return c.json({ error: err.message ?? "internal" }, 500);
});

export default app;
