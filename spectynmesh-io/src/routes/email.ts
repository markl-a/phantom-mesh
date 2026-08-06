// Email tier login + register. No external IdP; bcrypt-style hash via
// PBKDF2 (Web Crypto, no extra deps).

import type { Context } from "hono";
import { getCookie, deleteCookie } from "hono/cookie";
import type { Env, OAuthSession, CliPayload } from "../types";
import { upsertUser, getUserByEmail, setEmailPassword, claimDevice, recordTokenIssue } from "../lib/db";
import { mintBrokerJwt, verifyOAuthNonce, NONCE_COOKIE } from "../lib/oauth";
import { setSessionCookie } from "./oauth";

// B2 audit fix — same nonce binding the OAuth callback does. The
// email tier consumes the same OAuthSession KV record (state was issued
// by /auth/cli/start or /auth/web/start), so the same cookie-bound
// proof-of-initiator applies. See docs/superpowers/audits/
// 2026-05-15-broker-audit.md §2.2 B2.
async function emailNonceBindingOk(
  c: Context<{ Bindings: Env }>,
  session: OAuthSession,
): Promise<boolean> {
  if (!session.nonce_hash) return false;
  const cookieNonce = getCookie(c, NONCE_COOKIE) ?? "";
  return verifyOAuthNonce(c.env.BROKER_JWT_SECRET, cookieNonce, session.nonce_hash);
}

const PBKDF2_ITERS = 100_000;
const SALT_BYTES = 16;

export async function emailRegister(c: Context<{ Bindings: Env }>) {
  const body = await c.req.parseBody();
  const state    = String(body.state ?? "");
  const email    = String(body.email ?? "").trim().toLowerCase();
  const password = String(body.password ?? "");
  if (!state)            return c.text("missing state", 400);
  if (!email.includes("@")) return c.text("invalid email", 400);
  if (password.length < 8)  return c.text("password ≥8 chars", 400);

  const raw = await c.env.SESSIONS.get(state);
  if (!raw) return c.text("session expired", 400);
  const session = JSON.parse(raw) as OAuthSession;
  // B2: bind the caller to the originator browser before mutating
  // anything (user creation, session deletion). Same generic error
  // shape as missing-state.
  if (!(await emailNonceBindingOk(c, session))) {
    return c.text("session expired", 400);
  }
  await c.env.SESSIONS.delete(state);
  deleteCookie(c, NONCE_COOKIE, { path: "/" });

  const existing = await getUserByEmail(c.env, email);
  if (existing && existing.password_hash) {
    return c.text("email already registered — use login instead", 409);
  }

  const { hash } = await hashPassword(password);
  const user = await upsertUser(c.env, { email, provider: "email" });
  await setEmailPassword(c.env, email, hash);

  return redirectWithBrokerToken(c, session, user.id, email, "email");
}

export async function emailLogin(c: Context<{ Bindings: Env }>) {
  const body = await c.req.parseBody();
  const state    = String(body.state ?? "");
  const email    = String(body.email ?? "").trim().toLowerCase();
  const password = String(body.password ?? "");
  if (!state) return c.text("missing state", 400);

  const raw = await c.env.SESSIONS.get(state);
  if (!raw) return c.text("session expired", 400);
  const session = JSON.parse(raw) as OAuthSession;
  if (!(await emailNonceBindingOk(c, session))) {
    return c.text("session expired", 400);
  }
  await c.env.SESSIONS.delete(state);
  deleteCookie(c, NONCE_COOKIE, { path: "/" });

  const user = await getUserByEmail(c.env, email);
  if (!user || !user.password_hash) return c.text("no such account", 401);

  const ok = await verifyPassword(password, user.password_hash);
  if (!ok) return c.text("wrong password", 401);

  return redirectWithBrokerToken(c, session, user.id, email, "email");
}

async function redirectWithBrokerToken(
  c: Context<{ Bindings: Env }>,
  session: OAuthSession,
  userId: number,
  email: string,
  provider: string,
) {
  const ttl = parseInt(c.env.BROKER_TOKEN_TTL_SECS, 10);
  const deviceId = session.mode === "cli" ? session.device_id : "";
  const { token, expires_at_ms } = await mintBrokerJwt({
    secret:   c.env.BROKER_JWT_SECRET,
    userId,
    deviceId,
    ttlSecs:  ttl,
  });
  await recordTokenIssue(c.env, {
    token,
    user_id:   userId,
    device_id: deviceId || null,
    ttlSecs:   ttl,
  });

  if (session.mode === "web") {
    setSessionCookie(c, token, ttl);
    return c.redirect("/account");
  }

  await claimDevice(c.env, session.device_id, userId);
  const payload: CliPayload = {
    provider, email,
    sub:          null,
    name:         null,
    picture:      null,
    id_token:     "",
    access_token: "",
    broker_token: token,
    broker_token_expires_at_ms: expires_at_ms,
  };
  // UTF-8 → base64url (btoa is Latin-1 only; CJK in name field would throw)
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  const b64 = btoa(bin)
    .replace(/=+$/g, "").replace(/\+/g, "-").replace(/\//g, "_");
  return c.redirect(`${session.redirect}?p=${encodeURIComponent(b64)}`);
}

/* ── PBKDF2 password hashing (no deps) ────────────────────────────────── */

async function hashPassword(password: string): Promise<{ hash: string; salt: string }> {
  const salt = crypto.getRandomValues(new Uint8Array(SALT_BYTES));
  const key  = await deriveKey(password, salt);
  const hash = `pbkdf2$${PBKDF2_ITERS}$${b64(salt)}$${b64(key)}`;
  return { hash, salt: b64(salt) };
}

async function verifyPassword(password: string, stored: string): Promise<boolean> {
  // stored format: pbkdf2$<iters>$<salt-b64>$<hash-b64>
  const parts = stored.split("$");
  if (parts.length !== 4 || parts[0] !== "pbkdf2") return false;
  const iters = parseInt(parts[1], 10);
  const salt  = unb64(parts[2]);
  const want  = unb64(parts[3]);
  const got = await deriveKey(password, salt, iters);
  return constantTimeEq(got, want);
}

async function deriveKey(
  password: string,
  salt: Uint8Array,
  iterations = PBKDF2_ITERS,
): Promise<Uint8Array> {
  const baseKey = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(password),
    { name: "PBKDF2" }, false, ["deriveBits"]
  );
  const bits = await crypto.subtle.deriveBits(
    { name: "PBKDF2", salt, iterations, hash: "SHA-256" },
    baseKey, 256
  );
  return new Uint8Array(bits);
}

function b64(b: Uint8Array): string {
  return btoa(String.fromCharCode(...b));
}
function unb64(s: string): Uint8Array {
  const bin = atob(s);
  const arr = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
  return arr;
}
function constantTimeEq(a: Uint8Array, b: Uint8Array): boolean {
  if (a.length !== b.length) return false;
  let diff = 0;
  for (let i = 0; i < a.length; i++) diff |= a[i] ^ b[i];
  return diff === 0;
}
