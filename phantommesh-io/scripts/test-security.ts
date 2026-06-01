// Security tests for B1 (CORS) + B2 (OAuth state binding) findings from
// the 2026-05-15 broker audit (PR #74).
//
// Run: npm run test:security
// Wired into ci as a deploy gate.
//
// Uses Node's built-in `node:test` runner — no extra deps. The Hono app
// is exercised in-process via `app.request(url, init)`; D1 / KV / R2
// bindings are stubbed minimally with in-memory maps.

import { test } from "node:test";
import assert from "node:assert/strict";
import app from "../src/index";
import type { Env } from "../src/types";

/* ── Minimal in-memory stubs for the Worker bindings ──────────────────── */

function makeKv(): KVNamespace {
  const store = new Map<string, { value: string; expires: number }>();
  return {
    async get(key: string): Promise<string | null> {
      const entry = store.get(key);
      if (!entry) return null;
      if (entry.expires && entry.expires < Date.now()) {
        store.delete(key);
        return null;
      }
      return entry.value;
    },
    async put(key: string, value: string, opts?: { expirationTtl?: number }): Promise<void> {
      const expires = opts?.expirationTtl ? Date.now() + opts.expirationTtl * 1000 : 0;
      store.set(key, { value, expires });
    },
    async delete(key: string): Promise<void> {
      store.delete(key);
    },
    // Unused in these tests
    async list() { return { keys: [], list_complete: true, cursor: "" }; },
    async getWithMetadata() { return { value: null, metadata: null }; },
  } as unknown as KVNamespace;
}

function makeEnv(overrides: Partial<Env> = {}): Env {
  return {
    DB: {} as D1Database,
    SESSIONS: makeKv(),
    BINARIES: {} as R2Bucket,
    APP_URL: "https://phantommesh.io",
    GOOGLE_CLIENT_ID: "test-client-id",
    BROKER_TOKEN_TTL_SECS: "604800",
    BROKER_VERSION: "test",
    CF_ANALYTICS_TOKEN: "",
    GOOGLE_CLIENT_SECRET: "test-secret",
    BROKER_JWT_SECRET: "test-jwt-secret-must-be-at-least-32-bytes-long-for-hs256",
    ENV_VAULT_KEY: Buffer.alloc(32, 1).toString("base64"),
    ...overrides,
  };
}

/* ─────────────────────────────────────────────────────────────────────── */
/* B1 — CORS allowlist tightened to phantommesh.io only                    */
/* ─────────────────────────────────────────────────────────────────────── */
//
// Audit context (`docs/superpowers/audits/2026-05-15-broker-audit.md` §2.2 B1):
// The pre-fix allowlist included `http://127.0.0.1:48181` and
// `http://localhost:48181` so any local web server running on `:48181`
// (random tutorial, Vite dev server, copilot demo) inherited the same
// trust level as `phantommesh.io`. The CLI itself does NOT need CORS —
// it makes server-side `fetch` calls with `Authorization: Bearer …`,
// which never trigger browser CORS preflight. So we drop loopback origins
// entirely.

test("[B1] CORS: phantommesh.io origin is accepted", async () => {
  const env = makeEnv();
  const res = await app.request("https://phantommesh.io/api/health", {
    method: "OPTIONS",
    headers: {
      "Origin": "https://phantommesh.io",
      "Access-Control-Request-Method": "GET",
      "Access-Control-Request-Headers": "Authorization",
    },
  }, env);
  assert.equal(res.headers.get("Access-Control-Allow-Origin"), "https://phantommesh.io",
    "phantommesh.io must remain an accepted CORS origin (legitimate browser surface)");
});

test("[B1] CORS: http://localhost:48181 origin is REJECTED", async () => {
  const env = makeEnv();
  const res = await app.request("https://phantommesh.io/api/health", {
    method: "OPTIONS",
    headers: {
      "Origin": "http://localhost:48181",
      "Access-Control-Request-Method": "GET",
      "Access-Control-Request-Headers": "Authorization",
    },
  }, env);
  // Hono's cors() returns no ACAO header (or empty) when the origin is
  // not in the allowlist; browsers then refuse the request.
  const acao = res.headers.get("Access-Control-Allow-Origin");
  assert.notEqual(acao, "http://localhost:48181",
    "loopback origin must NOT be reflected back as Access-Control-Allow-Origin (B1 fix)");
});

test("[B1] CORS: http://127.0.0.1:48181 origin is REJECTED", async () => {
  const env = makeEnv();
  const res = await app.request("https://phantommesh.io/api/health", {
    method: "OPTIONS",
    headers: {
      "Origin": "http://127.0.0.1:48181",
      "Access-Control-Request-Method": "GET",
      "Access-Control-Request-Headers": "Authorization",
    },
  }, env);
  const acao = res.headers.get("Access-Control-Allow-Origin");
  assert.notEqual(acao, "http://127.0.0.1:48181",
    "loopback IP origin must NOT be reflected back as Access-Control-Allow-Origin (B1 fix)");
});

test("[B1] CORS: arbitrary attacker origin is REJECTED", async () => {
  const env = makeEnv();
  const res = await app.request("https://phantommesh.io/api/health", {
    method: "OPTIONS",
    headers: {
      "Origin": "https://evil.example",
      "Access-Control-Request-Method": "GET",
      "Access-Control-Request-Headers": "Authorization",
    },
  }, env);
  const acao = res.headers.get("Access-Control-Allow-Origin");
  assert.notEqual(acao, "https://evil.example",
    "arbitrary origin must NOT be reflected back");
});

test("[B1] CORS: Bearer-auth fetch from CLI is unaffected (no Origin header)", async () => {
  // The CLI uses Rust reqwest — no browser, no Origin header sent. Hono's
  // cors() leaves no-Origin requests alone and the route runs normally.
  const env = makeEnv();
  const res = await app.request("https://phantommesh.io/api/health", {
    method: "GET",
  }, env);
  assert.equal(res.status, 200, "CLI Bearer flow must still work after CORS tightening");
});

/* ─────────────────────────────────────────────────────────────────────── */
/* B2 — OAuth state bound to initiator browser via HttpOnly cookie         */
/* ─────────────────────────────────────────────────────────────────────── */
//
// Audit context (`docs/superpowers/audits/2026-05-15-broker-audit.md` §2.2 B2):
// Pre-fix, the OAuth callback fetched the KV session purely by the
// `state` query parameter. An attacker who learned `state` (Referer leak,
// shoulder-surf, Google's logs) could complete the OAuth dance from
// their own browser and have the loopback redirect deliver `broker_token`
// to an attacker-controlled loopback. Fix: bind state to a fresh
// HttpOnly cookie (`phantom_oauth_nonce`) set in `authStart` / `webStart`,
// HMAC the nonce with `BROKER_JWT_SECRET`, store the hash alongside the
// KV record, verify on every transition (`googleStart`, `googleCallback`,
// `emailLogin`, `emailRegister`).

const NONCE_COOKIE = "phantom_oauth_nonce";

function getCookieValue(setCookie: string | null, name: string): string | null {
  if (!setCookie) return null;
  // Hono uses Set-Cookie like: `phantom_oauth_nonce=abc; HttpOnly; Path=/; ...`
  // node-fetch concatenates multiple Set-Cookie headers with comma; we
  // split on commas that aren't inside an Expires date (Expires uses
  // `, DD Mon`).
  const parts = setCookie.split(/,(?=\s*[A-Za-z_][A-Za-z0-9_]*=)/);
  for (const part of parts) {
    const m = part.trim().match(new RegExp(`^${name}=([^;]+)`));
    if (m) return m[1];
  }
  return null;
}

test("[B2] /auth/cli/start sets an HttpOnly nonce cookie", async () => {
  const env = makeEnv();
  const deviceId = "11111111-2222-3333-4444-555555555555";
  const redirect = "http://127.0.0.1:48181/oauth/callback";
  const res = await app.request(
    `https://phantommesh.io/auth/cli/start?device_id=${deviceId}&port=48181&redirect=${encodeURIComponent(redirect)}`,
    { method: "GET", redirect: "manual" },
    env,
  );
  assert.equal(res.status, 302, "authStart should redirect to /login");
  const setCookie = res.headers.get("Set-Cookie");
  const nonce = getCookieValue(setCookie, NONCE_COOKIE);
  assert.ok(nonce, `authStart must set the ${NONCE_COOKIE} cookie (B2 binding)`);
  assert.ok(setCookie?.includes("HttpOnly"),
    "nonce cookie must be HttpOnly so JS can't exfiltrate it");
});

test("[B2] /auth/google/start REJECTS request with no nonce cookie", async () => {
  const env = makeEnv();
  // First, prime a state via authStart so the KV record exists.
  const deviceId = "11111111-2222-3333-4444-555555555555";
  const redirect = "http://127.0.0.1:48181/oauth/callback";
  const startRes = await app.request(
    `https://phantommesh.io/auth/cli/start?device_id=${deviceId}&port=48181&redirect=${encodeURIComponent(redirect)}`,
    { method: "GET", redirect: "manual" },
    env,
  );
  const location = startRes.headers.get("Location") ?? "";
  const stateMatch = location.match(/state=([^&]+)/);
  assert.ok(stateMatch, "authStart should include state in redirect");
  const state = stateMatch[1];

  // Now call googleStart from a DIFFERENT browser (no cookie). This is
  // the attack: attacker steals `state` from URL bar / Referer, calls
  // googleStart from their own browser.
  const res = await app.request(
    `https://phantommesh.io/auth/google/start?state=${state}`,
    { method: "GET", redirect: "manual" },
    env,
  );
  assert.equal(res.status, 400,
    "googleStart must reject when nonce cookie is absent (B2 attacker scenario)");
});

test("[B2] /auth/google/start REJECTS request with TAMPERED nonce cookie", async () => {
  const env = makeEnv();
  const deviceId = "11111111-2222-3333-4444-555555555555";
  const redirect = "http://127.0.0.1:48181/oauth/callback";
  const startRes = await app.request(
    `https://phantommesh.io/auth/cli/start?device_id=${deviceId}&port=48181&redirect=${encodeURIComponent(redirect)}`,
    { method: "GET", redirect: "manual" },
    env,
  );
  const location = startRes.headers.get("Location") ?? "";
  const state = location.match(/state=([^&]+)/)![1];
  // Attacker tampers: any nonce that's not the legitimate one must fail
  // the HMAC compare.
  const res = await app.request(
    `https://phantommesh.io/auth/google/start?state=${state}`,
    {
      method: "GET",
      headers: { "Cookie": `${NONCE_COOKIE}=attacker-controlled-value` },
      redirect: "manual",
    },
    env,
  );
  assert.equal(res.status, 400,
    "googleStart must reject when nonce cookie does not match the stored hash");
});

test("[B2] /auth/google/start ACCEPTS request with the legitimate nonce cookie", async () => {
  const env = makeEnv();
  const deviceId = "11111111-2222-3333-4444-555555555555";
  const redirect = "http://127.0.0.1:48181/oauth/callback";
  const startRes = await app.request(
    `https://phantommesh.io/auth/cli/start?device_id=${deviceId}&port=48181&redirect=${encodeURIComponent(redirect)}`,
    { method: "GET", redirect: "manual" },
    env,
  );
  const location = startRes.headers.get("Location") ?? "";
  const state = location.match(/state=([^&]+)/)![1];
  const setCookie = startRes.headers.get("Set-Cookie");
  const nonce = getCookieValue(setCookie, NONCE_COOKIE);
  assert.ok(nonce, "authStart must set nonce cookie");

  const res = await app.request(
    `https://phantommesh.io/auth/google/start?state=${state}`,
    {
      method: "GET",
      headers: { "Cookie": `${NONCE_COOKIE}=${nonce}` },
      redirect: "manual",
    },
    env,
  );
  assert.equal(res.status, 302,
    "googleStart with the legitimate nonce cookie must succeed (backwards compat)");
  const provLoc = res.headers.get("Location") ?? "";
  assert.ok(provLoc.includes("accounts.google.com"),
    "successful flow should redirect to Google's auth URL");
});

test("[B2] /auth/web/start also sets the nonce cookie", async () => {
  const env = makeEnv();
  const res = await app.request(
    "https://phantommesh.io/auth/web/start",
    { method: "GET", redirect: "manual" },
    env,
  );
  assert.equal(res.status, 302);
  const setCookie = res.headers.get("Set-Cookie");
  const nonce = getCookieValue(setCookie, NONCE_COOKIE);
  assert.ok(nonce, "webStart must also set the nonce cookie (B2 covers web flow too)");
  assert.ok(setCookie?.includes("HttpOnly"),
    "nonce cookie must be HttpOnly on web flow as well");
});
