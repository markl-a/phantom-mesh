// Authenticated API surface. Used by spectyn CLI's `spectyn devices`
// (planned) and the web dashboard.

import type { Context } from "hono";
import { getCookie } from "hono/cookie";
import type { Env } from "../types";
import { verifyBrokerJwt } from "../lib/oauth";
import {
  getUserById, getUserDevices, claimDevice as claimDeviceDb, revokeDevice as revokeDeviceDb,
  tokenIsRevoked,
  getUserSettings, setUserSettings, ALLOWED_ENV_KEYS,
  getUserClusterPeers, setUserClusterPeers, upsertUserClusterPeer,
  listActiveSessions, upsertSessionHeartbeat, endSession,
} from "../lib/db";
import { SESSION_COOKIE } from "./oauth";

/// Multi-tenant model (lifted from the v1 single-email allowlist 2026-05-04
/// for open-source release): any authenticated user can read/write THEIR
/// OWN vault. All DB queries are scoped by user_id from the JWT, and
/// vault payloads are encrypted with a per-user key derived via
/// HKDF(master_key, salt=user_id), so even a query that accidentally
/// crossed users couldn't decrypt the wrong row.
///
/// Trust boundary: a leaked broker_token gives the holder full vault
/// access for THAT user. The token TTL (BROKER_TOKEN_TTL_SECS, default
/// 7 days) bounds the blast radius. Rotate via /auth/logout.

export async function authn(c: Context<{ Bindings: Env }>): Promise<{ userId: number; deviceId: string } | null> {
  // Bearer token (CLI calls) takes precedence over the web cookie.
  const auth = c.req.header("Authorization") ?? "";
  let token = auth.startsWith("Bearer ") ? auth.slice(7) : "";
  if (!token) token = getCookie(c, SESSION_COOKIE) ?? "";
  if (!token) return null;
  if (await tokenIsRevoked(c.env, token)) return null;
  try {
    return await verifyBrokerJwt({ secret: c.env.BROKER_JWT_SECRET, token });
  } catch {
    return null;
  }
}

export async function me(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const user = await getUserById(c.env, id.userId);
  if (!user) return c.json({ error: "user not found" }, 404);
  return c.json({
    user: {
      id: user.id,
      email: user.email,
      provider: user.provider,
      display_name: user.display_name,
      avatar_url: user.avatar_url,
    },
    device_id: id.deviceId,
  });
}

export async function devices(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const list = await getUserDevices(c.env, id.userId);
  return c.json({ devices: list });
}

export async function claimDevice(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const target = c.req.param("device_id");
  if (!target) return c.json({ error: "missing device_id" }, 400);
  const body = await c.req.json().catch(() => ({} as { label?: string }));
  const row = await claimDeviceDb(c.env, target, id.userId, body.label);
  return c.json({ device: row });
}

export async function revokeDevice(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const target = c.req.param("device_id");
  if (!target) return c.json({ error: "missing device_id" }, 400);
  const ok = await revokeDeviceDb(c.env, id.userId, target);
  return c.json({ revoked: ok });
}

/// Common preamble for the settings endpoints: must be authenticated.
/// Each user only ever touches their own data — the user_id comes from
/// the verified JWT (not request body), and every downstream DB query
/// scopes WHERE user_id = ?. So there's no cross-user read/write path
/// even with this gate at its most permissive.
async function authnSettings(c: Context<{ Bindings: Env }>): Promise<{ userId: number; email: string } | null> {
  const id = await authn(c);
  if (!id) { c.status(401); return null; }
  const user = await getUserById(c.env, id.userId);
  if (!user) { c.status(404); return null; }
  return { userId: id.userId, email: user.email };
}

/// GET /api/me/settings — return the user's stored LLM env keys, MASKED
/// (last 4 chars only). Spectyn CLI uses a separate /api/me/settings/raw
/// endpoint when it needs the actual values to write to ~/.spectyn-mesh/env;
/// the dashboard uses this masked variant so reading the page in a screen
/// share doesn't leak secrets.
export async function getSettings(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const settings = await getUserSettings(c.env, ok.userId);
  const masked: Record<string, string> = {};
  for (const [k, v] of Object.entries(settings.env)) {
    masked[k] = v.length <= 8 ? "•".repeat(v.length) : `${"•".repeat(Math.max(0, v.length - 4))}${v.slice(-4)}`;
  }
  return c.json({
    env: masked,
    allowed_keys: [...ALLOWED_ENV_KEYS],
    updated_at: settings.updated_at,
  });
}

/// GET /api/me/settings/raw — RETIRED (SPEC-15 broker vault E2EE).
///
/// This endpoint used to return ACTUAL plaintext secret values to the CLI
/// (`spectyn config pull`). Under true end-to-end encryption the broker
/// MUST NEVER emit plaintext of any sealed value, so this plaintext-returning
/// route is permanently disabled. Clients fetch sealed ciphertext via the new
/// dumb-storage GET /vault/get route and unseal locally with the device-held
/// VaultSealKey. See SPEC-15 §4 + docs/integration/2026-05-29-spec15-vault-
/// verification.md.
///
/// The handler is kept (returns 410 Gone) so the route registration in
/// index.ts still resolves during the migration window. It deliberately does
/// NOT call getUserSettings and can never leak plaintext.
///
/// TODO(spec15-deploy): delete the `app.get("/api/me/settings/raw", ...)`
/// registration in src/index.ts and remove this handler once all clients
/// have migrated to GET /vault/get.
export async function getSettingsRaw(c: Context<{ Bindings: Env }>) {
  // Authenticate so we don't reveal route behavior to anonymous callers,
  // but NEVER read or return any stored value.
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" }, c.res.status === 403 ? 403 : 401);
  }
  return c.json(
    {
      error: "gone",
      message:
        "GET /api/me/settings/raw is retired under SPEC-15 E2EE. Fetch sealed ciphertext via GET /vault/get and unseal locally.",
    },
    410,
  );
}

/// PUT /api/me/settings — full replace. Body: {env: {KEY: "value", ...}}.
/// Keys outside the allowlist are dropped silently; empty-string values
/// are treated as "unset". Returns the masked stored state on success.
export async function putSettings(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const body = await c.req.json().catch(() => ({} as { env?: Record<string, string> }));
  const incoming = (body && typeof body === "object" && body.env && typeof body.env === "object")
    ? body.env as Record<string, string>
    : {};
  const saved = await setUserSettings(c.env, ok.userId, incoming);
  const masked: Record<string, string> = {};
  for (const [k, v] of Object.entries(saved.env)) {
    masked[k] = v.length <= 8 ? "•".repeat(v.length) : `${"•".repeat(Math.max(0, v.length - 4))}${v.slice(-4)}`;
  }
  return c.json({ env: masked, updated_at: saved.updated_at });
}

/// GET /api/me/cluster-peers — return the user's mesh peer registry.
/// Not masked: peer URLs are operational config, not secrets. The
/// CLUSTER_SECRET that authenticates RPC between them IS in the env
/// vault and IS encrypted at rest.
export async function getClusterPeers(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const peers = await getUserClusterPeers(c.env, ok.userId);
  return c.json({ peers });
}

/// POST /api/me/cluster-peers/upsert — single-peer add-or-update.
/// Body: {name, url, label?}. Used by `spectyn login`'s self-register
/// flow so each machine can add itself without overwriting peers other
/// machines registered. Empty name/url is a no-op (returns current list).
export async function upsertClusterPeer(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const body = await c.req.json().catch(() => ({} as { name?: string; url?: string; label?: string }));
  const peer = (body && typeof body === "object")
    ? body as { name: string; url: string; label?: string }
    : { name: "", url: "" };
  const peers = await upsertUserClusterPeer(c.env, ok.userId, peer);
  return c.json({ peers, upserted: { name: peer.name, url: peer.url } });
}

/// PUT /api/me/cluster-peers — full replace. Body: {peers: [{name,url,label?},...]}.
/// Empty rows dropped silently in setUserClusterPeers; same name
/// twice → one wins (PRIMARY KEY conflict raises, batch fails the txn,
/// caller gets 500 — front-end is expected to dedupe before sending).
export async function putClusterPeers(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const body = await c.req.json().catch(() => ({} as { peers?: unknown }));
  const incoming = Array.isArray(body && (body as { peers?: unknown }).peers)
    ? ((body as { peers: unknown[] }).peers as { name: string; url: string; label?: string }[])
    : [];
  const saved = await setUserClusterPeers(c.env, ok.userId, incoming);
  return c.json({ peers: saved });
}

/// GET /api/me/sessions — return live TUI sessions across all the user's
/// machines (any session that heartbeated within the last 60s). Used by
/// `spectyn sessions` CLI and the dashboard's "active now" panel.
export async function getSessions(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const sessions = await listActiveSessions(c.env, ok.userId);
  return c.json({ sessions });
}

/// POST /api/me/sessions/heartbeat — TUI calls this on launch + every 30s.
/// Body: {id, machine, agent?, cwd?}. id is a uuid the client generates
/// once per TUI process. Stale rows (>60s no heartbeat) are filtered out
/// by the GET endpoint, not deleted server-side — keeps writes idempotent.
export async function postSessionHeartbeat(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const body = await c.req.json().catch(() => ({} as { id?: string; machine?: string; agent?: string; cwd?: string }));
  const s = (body && typeof body === "object")
    ? body as { id: string; machine: string; agent?: string; cwd?: string }
    : { id: "", machine: "" };
  const sessions = await upsertSessionHeartbeat(c.env, ok.userId, s);
  return c.json({ sessions, beat: { id: s.id, machine: s.machine } });
}

/// DELETE /api/me/sessions/:id — TUI calls this on graceful shutdown.
/// Best-effort: if the TUI crashes we rely on the 60s stale window
/// to drop it from the list naturally.
export async function deleteSession(c: Context<{ Bindings: Env }>) {
  const ok = await authnSettings(c);
  if (!ok) {
    return c.json({ error: c.res.status === 403 ? "forbidden" : "unauthenticated" });
  }
  const id = c.req.param("id") ?? "";
  if (id.length === 0) return c.json({ error: "missing id" }, 400);
  await endSession(c.env, ok.userId, id);
  return c.json({ ended: id });
}
