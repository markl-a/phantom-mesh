// PKCE + Google OAuth helpers.
//
// Apple Sign In was scoped out — see git history if/when it comes back.
// Cloudflare Workers ship Web Crypto + fetch + atob/btoa, so we only
// need `jose` for our own broker JWT signing/verification.

import { SignJWT, jwtVerify, decodeJwt } from "jose";

export type GoogleClaims = {
  email: string;
  email_verified?: boolean;
  sub: string;
  name?: string;
  picture?: string;
};

/* ── PKCE ─────────────────────────────────────────────────────────────── */

export function pkcePair(): { verifier: string; challenge: string } {
  const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
  return { verifier, challenge: "" /* set asynchronously */ };
}

export async function pkceChallenge(verifier: string): Promise<string> {
  const data = new TextEncoder().encode(verifier);
  const hash = await crypto.subtle.digest("SHA-256", data);
  return b64url(new Uint8Array(hash));
}

function b64url(b: Uint8Array): string {
  return btoa(String.fromCharCode(...b))
    .replace(/=+$/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
}

/* ── CSRF (double-submit / HMAC-bound to session) ───────────────────────── */

// Token = b64url(HMAC-SHA256(BROKER_JWT_SECRET, sessionToken)).
// No KV round-trip — it's deterministic given the session cookie + secret.
// Validity is bounded by the session itself: when the session expires or
// rotates, the HMAC stops matching. This is the standard "stateless CSRF
// signed by a server-only key" pattern. Empty session → empty token,
// which the verifier rejects so an attacker can't forge "" against "".
export async function csrfToken(secret: string, sessionToken: string): Promise<string> {
  if (!sessionToken) return "";
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(sessionToken));
  return b64url(new Uint8Array(sig));
}

// Constant-time compare. Returns false on length mismatch or any byte
// diff. Empty candidate or empty session → false.
export async function verifyCsrf(
  secret: string,
  sessionToken: string,
  candidate: string,
): Promise<boolean> {
  if (!candidate || !sessionToken) return false;
  const expected = await csrfToken(secret, sessionToken);
  if (expected.length !== candidate.length) return false;
  let diff = 0;
  for (let i = 0; i < expected.length; i++) {
    diff |= expected.charCodeAt(i) ^ candidate.charCodeAt(i);
  }
  return diff === 0;
}

/* ── OAuth state ↔ browser binding (B2 audit fix) ─────────────────────── */
//
// Pre-fix, the OAuth callback found its KV session purely by the `state`
// query parameter; anyone who learned `state` (Referer leak,
// shoulder-surf, Google's request logs) could complete the dance from
// their own browser and have the loopback redirect deliver the
// broker_token to their machine.
//
// Fix: at authStart / webStart we issue a fresh random `nonce`, store
// HMAC(secret, nonce) in the KV record, and set the nonce in an
// HttpOnly cookie. Every subsequent hop (googleStart, googleCallback,
// emailLogin, emailRegister) recomputes the HMAC from the cookie and
// constant-time-compares it against the stored hash. An attacker who
// sniffs `state` from a URL bar / Referer doesn't have the cookie and
// can't complete the dance.
//
// See docs/superpowers/audits/2026-05-15-broker-audit.md §2.2 B2.

export const NONCE_COOKIE = "phantom_oauth_nonce";

export function generateOAuthNonce(): string {
  return b64url(crypto.getRandomValues(new Uint8Array(32)));
}

export async function hashOAuthNonce(secret: string, nonce: string): Promise<string> {
  // Same HMAC primitive as csrfToken — keyed by BROKER_JWT_SECRET so the
  // hash is meaningless without the server-side secret. Output is
  // b64url so it round-trips through KV / JSON cleanly.
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const sig = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(nonce));
  return b64url(new Uint8Array(sig));
}

// Returns true iff `cookieNonce` HMACs to `storedHash` under `secret`.
// Constant-time on the comparison step. `cookieNonce` may be "" (no
// cookie sent) or "anything-arbitrary" (attacker-controlled); both
// must return false.
export async function verifyOAuthNonce(
  secret: string,
  cookieNonce: string,
  storedHash: string,
): Promise<boolean> {
  if (!cookieNonce || !storedHash) return false;
  const expected = await hashOAuthNonce(secret, cookieNonce);
  if (expected.length !== storedHash.length) return false;
  let diff = 0;
  for (let i = 0; i < expected.length; i++) {
    diff |= expected.charCodeAt(i) ^ storedHash.charCodeAt(i);
  }
  return diff === 0;
}

/* ── Google OAuth ─────────────────────────────────────────────────────── */

export function googleAuthUrl(params: {
  clientId: string;
  redirect: string;
  state: string;
  challenge: string;
}): string {
  const u = new URL("https://accounts.google.com/o/oauth2/v2/auth");
  u.searchParams.set("client_id", params.clientId);
  u.searchParams.set("response_type", "code");
  u.searchParams.set("scope", "openid email profile");
  u.searchParams.set("redirect_uri", params.redirect);
  u.searchParams.set("state", params.state);
  u.searchParams.set("code_challenge", params.challenge);
  u.searchParams.set("code_challenge_method", "S256");
  u.searchParams.set("access_type", "online");
  u.searchParams.set("prompt", "select_account");
  return u.toString();
}

export async function exchangeGoogleCode(opts: {
  clientId: string;
  clientSecret: string;
  redirect: string;
  code: string;
  verifier: string;
}): Promise<{ id_token: string; access_token: string; claims: GoogleClaims }> {
  const body = new URLSearchParams({
    client_id: opts.clientId,
    client_secret: opts.clientSecret,
    redirect_uri: opts.redirect,
    grant_type: "authorization_code",
    code: opts.code,
    code_verifier: opts.verifier,
  });
  const r = await fetch("https://oauth2.googleapis.com/token", {
    method: "POST",
    headers: { "Content-Type": "application/x-www-form-urlencoded" },
    body,
  });
  if (!r.ok) {
    const txt = await r.text();
    throw new Error(`google token exchange failed: ${r.status} ${txt}`);
  }
  const json = await r.json() as { id_token: string; access_token: string };
  // We trust Google's id_token here — it just came from accounts.google.com
  // over TLS in response to our (server-known) PKCE verifier. Skip JWKS
  // fetch for simplicity in v1.
  const claims = decodeJwt(json.id_token) as unknown as GoogleClaims;
  return { id_token: json.id_token, access_token: json.access_token, claims };
}

/* ── Broker token (our own JWT) ───────────────────────────────────────── */

export async function mintBrokerJwt(opts: {
  secret: string;
  userId: number;
  deviceId: string;
  ttlSecs: number;
}): Promise<{ token: string; expires_at_ms: number }> {
  const now = Math.floor(Date.now() / 1000);
  const exp = now + opts.ttlSecs;
  const key = new TextEncoder().encode(opts.secret);
  const token = await new SignJWT({
    sub: String(opts.userId),
    device_id: opts.deviceId,
  })
    .setProtectedHeader({ alg: "HS256" })
    .setIssuedAt(now)
    .setExpirationTime(exp)
    .setIssuer("phantommesh.io")
    .setAudience("phantom-cli")
    .sign(key);
  return { token, expires_at_ms: exp * 1000 };
}

export async function verifyBrokerJwt(opts: {
  secret: string;
  token: string;
}): Promise<{ userId: number; deviceId: string }> {
  const key = new TextEncoder().encode(opts.secret);
  const { payload } = await jwtVerify(opts.token, key, {
    issuer: "phantommesh.io",
    audience: "phantom-cli",
  });
  return {
    userId: Number(payload.sub),
    deviceId: String(payload.device_id ?? ""),
  };
}
