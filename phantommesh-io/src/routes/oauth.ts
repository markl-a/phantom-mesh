// OAuth routes — Google. Email tier lives in routes/email.ts.
// Apple Sign In was scoped out — see git history if/when it comes back.

import type { Context } from "hono";
import { setCookie, deleteCookie, getCookie } from "hono/cookie";
import type { Env, OAuthSession, CliPayload, UserRow } from "../types";
import {
  googleAuthUrl, exchangeGoogleCode,
  pkcePair, pkceChallenge, mintBrokerJwt,
  verifyCsrf,
  generateOAuthNonce, hashOAuthNonce, verifyOAuthNonce,
  NONCE_COOKIE,
} from "../lib/oauth";
import {
  upsertUser, claimDevice, recordTokenIssue, revokeBrokerToken,
} from "../lib/db";

// Allowed loopback redirects for the CLI / desktop OAuth flow:
//   http://127.0.0.1:48181/oauth/callback   (Mac/Linux phantom CLI)
//   http://localhost:48181/oauth/callback   (same, hostname form)
// And the iOS / mobile-app callback that lands via tauri-plugin-deep-link
// onto the registered URL scheme — Apple sandbox blocks loopback, so the
// app registers `phantom://oauth/callback` in its Info.plist
// (CFBundleURLSchemes) and Safari's redirect → tauri's onOpenUrl handler
// fires when the broker meta-refreshes to that URL.
const REDIRECT_RE =
  /^(http:\/\/(127\.0\.0\.1|localhost):\d+\/oauth\/callback|phantom:\/\/oauth\/callback)$/;
const UUID_RE     = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export const SESSION_COOKIE = "phantom_session";

/* ── Nonce cookie helpers (B2 audit fix) ────────────────────────────────── */
//
// authStart / webStart issue a fresh random nonce, store HMAC(nonce) in
// the KV session, and set the nonce in an HttpOnly cookie. Every hop
// that consumes the KV session re-verifies the cookie HMAC matches
// before accepting `state`. An attacker who sniffs `state` from a URL
// bar / Referer doesn't have the cookie. See docs/superpowers/audits/
// 2026-05-15-broker-audit.md §2.2 B2.

function setNonceCookie(c: Context<{ Bindings: Env }>, nonce: string): void {
  setCookie(c, NONCE_COOKIE, nonce, {
    httpOnly: true,
    secure:   c.env.APP_URL.startsWith("https://"),
    sameSite: "Lax",  // Lax: needed because the OAuth callback redirect
                      // is a cross-site GET back from accounts.google.com.
                      // Strict would strip the cookie on that hop.
    path:     "/",
    maxAge:   600,    // 10 min — must outlive the KV record's 5-min TTL
                      // by a comfortable margin (clock skew + user delay).
  });
}

async function checkNonceBinding(
  c: Context<{ Bindings: Env }>,
  session: OAuthSession,
): Promise<boolean> {
  // Session records written before this commit have no `nonce_hash`
  // field — those are KV entries from older versions of this worker.
  // We refuse them outright (rather than silently passing) because the
  // KV TTL is 5 min: by the time this code ships, every legacy record
  // is gone, and accepting unbound records would defeat the audit fix.
  if (!session.nonce_hash) return false;
  const cookieNonce = getCookie(c, NONCE_COOKIE) ?? "";
  return verifyOAuthNonce(c.env.BROKER_JWT_SECRET, cookieNonce, session.nonce_hash);
}

/* ── /auth/cli/start — entry point from phantom CLI's `login_broker` ────── */

export async function authStart(c: Context<{ Bindings: Env }>) {
  const device_id = c.req.query("device_id") ?? "";
  const port      = parseInt(c.req.query("port") ?? "0", 10);
  const redirect  = c.req.query("redirect") ?? "";

  if (!UUID_RE.test(device_id))     return c.text("bad device_id", 400);
  if (!REDIRECT_RE.test(redirect))  return c.text("bad redirect",  400);
  // Mobile apps (iOS / Tauri-Android) don't have a loopback port — they
  // catch the callback through a `phantom://oauth/callback` deep link
  // routed via tauri-plugin-deep-link. For those flows port is just
  // metadata, not enforced. Desktop CLIs MUST still pass a real port.
  const isMobileFlow = redirect.startsWith("phantom://");
  if (!isMobileFlow && (port < 1024 || port > 65535)) {
    return c.text("bad port", 400);
  }

  // Stash the loopback redirect in KV under a server-issued state.
  const state = crypto.randomUUID();
  // B2: bind state to a fresh per-browser nonce so URL-bar / Referer
  // leaks of `state` alone aren't enough to hijack the dance.
  const nonce = generateOAuthNonce();
  const nonce_hash = await hashOAuthNonce(c.env.BROKER_JWT_SECRET, nonce);
  const session: OAuthSession = {
    mode: "cli",
    device_id,
    redirect,
    code_verifier: "", // filled by provider start route
    provider: "",
    created_at: Date.now(),
    nonce_hash,
  };
  await c.env.SESSIONS.put(state, JSON.stringify(session), { expirationTtl: 300 });
  setNonceCookie(c, nonce);

  return c.redirect(`/login?state=${state}`);
}

/* ── /auth/web/start — entry point for browser-only login ──────────────── */
//
// Used when someone clicks "Log in" on the marketing site (no CLI in
// the loop). Same KV-backed state, but mode === "web" so the callbacks
// finish by setting a session cookie and landing on /account.

export async function webStart(c: Context<{ Bindings: Env }>) {
  const state = crypto.randomUUID();
  // B2: same nonce binding as authStart — browser-only login is the
  // same threat shape (attacker captures `state`, races to /account).
  const nonce = generateOAuthNonce();
  const nonce_hash = await hashOAuthNonce(c.env.BROKER_JWT_SECRET, nonce);
  const session: OAuthSession = {
    mode: "web",
    device_id: "",
    redirect: "",
    code_verifier: "",
    provider: "",
    created_at: Date.now(),
    nonce_hash,
  };
  await c.env.SESSIONS.put(state, JSON.stringify(session), { expirationTtl: 300 });
  setNonceCookie(c, nonce);
  return c.redirect(`/login?state=${state}`);
}

/* ── /auth/logout ──────────────────────────────────────────────────────── */

export async function logout(c: Context<{ Bindings: Env }>) {
  const tok = getCookie(c, SESSION_COOKIE);

  // CSRF: form must include a `csrf` field that's HMAC-SHA256 of the
  // session cookie under BROKER_JWT_SECRET. Without this, any HTML page
  // the user happens to load could POST /auth/logout and sign them out
  // (low-stakes attack, but still annoying — and the same gate now
  // applies to future /api/billing/* state-changing POSTs).
  // Only enforce when there's actually a session to validate; an
  // unauthenticated logout is a no-op redirect either way.
  if (tok) {
    let csrfFromForm = "";
    const ct = c.req.header("Content-Type") ?? "";
    if (ct.includes("application/x-www-form-urlencoded") || ct.includes("multipart/form-data")) {
      const form = await c.req.parseBody().catch(() => ({} as Record<string, unknown>));
      const v = (form as Record<string, unknown>)["csrf"];
      if (typeof v === "string") csrfFromForm = v;
    }
    const ok = await verifyCsrf(c.env.BROKER_JWT_SECRET, tok, csrfFromForm);
    if (!ok) return c.text("CSRF check failed", 403);
    try { await revokeBrokerToken(c.env, tok); } catch { /* best-effort */ }
  }
  deleteCookie(c, SESSION_COOKIE, { path: "/" });
  return c.redirect("/");
}

/* ── /auth/google/start ─────────────────────────────────────────────────── */

export async function googleStart(c: Context<{ Bindings: Env }>) {
  const state = c.req.query("state") ?? "";
  const raw = await c.env.SESSIONS.get(state);
  if (!raw) return c.text("session expired — start over from /login", 400);
  const session = JSON.parse(raw) as OAuthSession;

  // B2: refuse to advance the dance if the caller doesn't own the
  // browser that started it. Generic 400 — don't leak whether the
  // failure is missing-cookie vs hash-mismatch (state-fixation
  // enumeration).
  if (!(await checkNonceBinding(c, session))) {
    return c.text("session expired — start over from /login", 400);
  }

  const verifier = pkcePair().verifier;
  const challenge = await pkceChallenge(verifier);
  session.code_verifier = verifier;
  session.provider = "google";
  await c.env.SESSIONS.put(state, JSON.stringify(session), { expirationTtl: 300 });

  const redirect = `${c.env.APP_URL}/auth/google/callback`;
  return c.redirect(googleAuthUrl({
    clientId: c.env.GOOGLE_CLIENT_ID,
    redirect,
    state,
    challenge,
  }));
}

/* ── /auth/google/callback ─────────────────────────────────────────────── */

export async function googleCallback(c: Context<{ Bindings: Env }>) {
  const code  = c.req.query("code")  ?? "";
  const state = c.req.query("state") ?? "";
  const raw = await c.env.SESSIONS.get(state);
  if (!raw)  return c.text("session expired — try /login again", 400);
  const session = JSON.parse(raw) as OAuthSession;

  // B2: verify the same browser that started the dance is finishing it
  // BEFORE deleting the KV record. If the cookie is missing or
  // tampered, leave the KV record in place — the legitimate user can
  // retry within the 5-min TTL.
  if (!(await checkNonceBinding(c, session))) {
    return c.text("session expired — try /login again", 400);
  }

  // Single-use: only delete once we know the caller owns the dance.
  await c.env.SESSIONS.delete(state);
  // Clear the nonce cookie now that the binding has been consumed.
  deleteCookie(c, NONCE_COOKIE, { path: "/" });

  if (!code) return c.text("missing code", 400);

  const redirect = `${c.env.APP_URL}/auth/google/callback`;
  const { id_token, access_token, claims } = await exchangeGoogleCode({
    clientId:     c.env.GOOGLE_CLIENT_ID,
    clientSecret: c.env.GOOGLE_CLIENT_SECRET,
    redirect,
    code,
    verifier:     session.code_verifier,
  });

  if (!claims.email) return c.text("Google id_token missing email", 502);

  const user = await upsertUser(c.env, {
    email:        claims.email,
    provider:     "google",
    sub:          claims.sub,
    display_name: claims.name,
    avatar_url:   claims.picture,
  });

  return finishOAuthLogin(c, session, user, {
    provider:     "google",
    email:        claims.email,
    sub:          claims.sub,
    name:         claims.name ?? null,
    picture:      claims.picture ?? null,
    id_token,
    access_token,
  });
}

/* ── shared: finish login (web → cookie, cli → loopback) ──────────────── */

type IdentityPayload = {
  provider: string;
  email: string;
  sub: string | null;
  name: string | null;
  picture: string | null;
  id_token: string;
  access_token: string;
};

export async function finishOAuthLogin(
  c: Context<{ Bindings: Env }>,
  session: OAuthSession,
  user: UserRow,
  identity: IdentityPayload,
) {
  const ttl = parseInt(c.env.BROKER_TOKEN_TTL_SECS, 10);
  const deviceId = session.mode === "cli" ? session.device_id : "";
  const { token: broker_token, expires_at_ms } = await mintBrokerJwt({
    secret:   c.env.BROKER_JWT_SECRET,
    userId:   user.id,
    deviceId,
    ttlSecs:  ttl,
  });
  await recordTokenIssue(c.env, {
    token:     broker_token,
    user_id:   user.id,
    device_id: deviceId || null,
    ttlSecs:   ttl,
  });

  if (session.mode === "web") {
    setSessionCookie(c, broker_token, ttl);
    return c.redirect("/account");
  }

  // CLI flow — claim the device and bounce back to loopback.
  await claimDevice(c.env, session.device_id, user.id);
  return redirectToLoopback(c, session.redirect, {
    provider:     identity.provider,
    email:        identity.email,
    sub:          identity.sub,
    name:         identity.name,
    picture:      identity.picture,
    id_token:     identity.id_token,
    access_token: identity.access_token,
    broker_token,
    broker_token_expires_at_ms: expires_at_ms,
  });
}

export function setSessionCookie(c: Context<{ Bindings: Env }>, token: string, ttlSecs: number) {
  setCookie(c, SESSION_COOKIE, token, {
    httpOnly: true,
    secure:   c.env.APP_URL.startsWith("https://"),
    sameSite: "Lax",
    path:     "/",
    maxAge:   ttlSecs,
  });
}

/* ── shared: redirect to the CLI's loopback with the payload ─────────── */

function redirectToLoopback(
  c: Context<{ Bindings: Env }>,
  redirect: string,
  payload: CliPayload
) {
  const json = JSON.stringify(payload);
  // btoa() only accepts Latin-1 — payloads with CJK display names (very
  // common for Google profiles) blow up with "btoa() can only operate on
  // characters in the Latin1 (ISO/IEC 8859-1) range." Encode to UTF-8
  // bytes first, THEN re-interpret as Latin-1-safe binary string before
  // btoa. This is the standard JS workaround for "base64 a UTF-8 string".
  const utf8Bytes = new TextEncoder().encode(json);
  let binary = "";
  for (let i = 0; i < utf8Bytes.length; i++) binary += String.fromCharCode(utf8Bytes[i]);
  // base64url so it survives URL encoding cleanly.
  const b64 = btoa(binary)
    .replace(/=+$/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
  // Brief intermediate page so the user sees what just happened
  // before the browser jumps back to localhost.
  return c.html(
    `<!doctype html>
<html><head>
  <meta charset="utf-8">
  <meta http-equiv="refresh" content="2;url=${redirect}?p=${encodeURIComponent(b64)}">
  <title>Logging you in to phantom…</title>
  <style>
    body{font-family:-apple-system,system-ui,sans-serif;background:#0c0c10;color:#e8e2d4;
         display:flex;align-items:center;justify-content:center;min-height:100vh;margin:0}
    .b{text-align:center}
    h1{color:#d6b270}
    a{color:#88ccaa}
    code{background:#1d1d24;padding:2px 6px;border-radius:4px}
  </style>
</head><body>
  <div class="b">
    <h1>✓ ${payload.provider} sign-in complete</h1>
    <p>Welcome, ${escapeHtml(payload.email)}. Sending you back to phantom…</p>
    <p><small>If nothing happens in 2 seconds:
       <a href="${redirect}?p=${encodeURIComponent(b64)}">click here</a></small></p>
  </div>
</body></html>`
  );
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (ch) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;"
  }[ch] ?? ch));
}
