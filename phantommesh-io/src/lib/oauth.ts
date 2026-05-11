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
