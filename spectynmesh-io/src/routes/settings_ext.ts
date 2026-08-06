// F205 — settings-screen extensions.
//
// Adds new endpoints on top of the existing /api/me/settings (env vault,
// already in src/routes/api.ts):
//   GET/PUT /api/me/preferences            — heartbeat cadence + retention
//   GET/PUT /api/me/peer-capabilities      — narrow editor for caps only
//   DELETE  /api/me/sessions/all-others    — revoke every broker_token but ours

import type { Context } from "hono";
import { getCookie } from "hono/cookie";
import type { Env } from "../types";
import { authn } from "./api";
import {
  getUserPreferences, setUserPreferences,
  setPeerCapabilities,
  revokeAllOtherBrokerTokens,
} from "../lib/db";
import { SESSION_COOKIE } from "./oauth";

/// GET /api/me/preferences
export async function getPreferences(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  const prefs = await getUserPreferences(c.env, id.userId);
  return c.json({ preferences: prefs });
}

/// PUT /api/me/preferences
/// Body: { heartbeat_secs?: number, retention_days?: number }
export async function putPreferences(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  let body: { heartbeat_secs?: unknown; retention_days?: unknown };
  try { body = await c.req.json(); }
  catch { return c.json({ error: "malformed json" }, 400); }
  const next = {
    heartbeat_secs: typeof body.heartbeat_secs === "number" ? body.heartbeat_secs : undefined,
    retention_days: typeof body.retention_days === "number" ? body.retention_days : undefined,
  };
  const prefs = await setUserPreferences(c.env, id.userId, next);
  return c.json({ preferences: prefs });
}

/// GET /api/me/peer-capabilities — convenience getter that returns just
/// the {name, capabilities[]} columns from the cluster peers list.
export async function getPeerCapabilities(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  // Reuse existing getUserClusterPeers for parity.
  const { getUserClusterPeers } = await import("../lib/db");
  const peers = await getUserClusterPeers(c.env, id.userId);
  return c.json({ peers: peers.map(p => ({ name: p.name, capabilities: p.capabilities })) });
}

/// PUT /api/me/peer-capabilities
/// Body: { peer: string, capabilities: string[] }.
/// 404 if the peer doesn't exist in the user's mesh.
export async function putPeerCapabilities(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  let body: { peer?: unknown; capabilities?: unknown };
  try { body = await c.req.json(); }
  catch { return c.json({ error: "malformed json" }, 400); }
  const peer = typeof body.peer === "string" ? body.peer : "";
  if (peer.trim().length === 0) return c.json({ error: "missing peer" }, 400);
  const caps = Array.isArray(body.capabilities)
    ? body.capabilities.filter((x): x is string => typeof x === "string")
    : [];
  const updated = await setPeerCapabilities(c.env, id.userId, peer, caps);
  if (!updated) return c.json({ error: "peer not found" }, 404);
  return c.json({ peer: updated });
}

/// DELETE /api/me/sessions/all-others — revokes every broker_token for
/// this user except the one making the request. The current session
/// stays active so the SPA doesn't redirect to /login mid-action.
export async function revokeAllOtherSessions(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "unauthenticated" }, 401);
  // The token we want to keep is whichever the current request used.
  const auth = c.req.header("Authorization") ?? "";
  const bearer = auth.startsWith("Bearer ") ? auth.slice(7) : "";
  const cookie = getCookie(c, SESSION_COOKIE) ?? "";
  const keep = bearer || cookie;
  if (!keep) return c.json({ error: "no current session token" }, 400);
  const revoked = await revokeAllOtherBrokerTokens(c.env, id.userId, keep);
  return c.json({ revoked });
}
