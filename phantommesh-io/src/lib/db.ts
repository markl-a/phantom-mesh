// D1 wrappers — narrow surface, zero ORM.

import type { Env, UserRow, DeviceRow } from "../types";
import { encryptForUser, decryptForUser, isEncryptedBlob } from "./crypto";

export async function upsertUser(
  env: Env,
  u: { email: string; provider: string; sub?: string; display_name?: string; avatar_url?: string }
): Promise<UserRow> {
  const now = Date.now();
  await env.DB
    .prepare(
      `INSERT INTO users (email, provider, sub, display_name, avatar_url, created_at, last_login_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(email) DO UPDATE SET
         provider = excluded.provider,
         sub = COALESCE(excluded.sub, users.sub),
         display_name = COALESCE(excluded.display_name, users.display_name),
         avatar_url = COALESCE(excluded.avatar_url, users.avatar_url),
         last_login_at = excluded.last_login_at`
    )
    .bind(u.email, u.provider, u.sub ?? null, u.display_name ?? null, u.avatar_url ?? null, now, now)
    .run();

  const row = await env.DB
    .prepare(`SELECT * FROM users WHERE email = ?`)
    .bind(u.email)
    .first<UserRow>();
  if (!row) throw new Error("upsertUser: row missing after insert");

  // Record this (provider, sub) pair so the user_identities table grows
  // alongside repeat sign-ins. The users.provider column still gets the
  // most-recent-login value, but user_identities preserves history.
  await recordIdentity(env, row.id, u.provider, u.sub ?? null);

  return row;
}

/// Insert-or-touch a (user_id, provider) row in user_identities.
/// Repeat sign-ins update last_used_ms; first sign-in sets first_linked_ms.
/// Best-effort — if the table doesn't exist (migration not applied),
/// swallow the error so login still succeeds. This is the only path
/// that depends on migration 0006; everything else still works without it.
export async function recordIdentity(
  env: Env,
  user_id: number,
  provider: string,
  sub: string | null,
): Promise<void> {
  const now = Date.now();
  try {
    await env.DB
      .prepare(
        `INSERT INTO user_identities (user_id, provider, sub, first_linked_ms, last_used_ms)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(user_id, provider) DO UPDATE SET
           sub = COALESCE(excluded.sub, user_identities.sub),
           last_used_ms = excluded.last_used_ms`
      )
      .bind(user_id, provider, sub, now, now)
      .run();
  } catch (e) {
    // Migration 0006 not applied yet — keep going, login still works.
    console.warn("user_identities unavailable; run migration 0006:", e);
  }
}

export type UserIdentityRow = {
  user_id: number;
  provider: string;
  sub: string | null;
  first_linked_ms: number;
  last_used_ms: number;
};

export async function getUserIdentities(env: Env, user_id: number): Promise<UserIdentityRow[]> {
  try {
    const r = await env.DB
      .prepare(
        `SELECT user_id, provider, sub, first_linked_ms, last_used_ms
         FROM user_identities
         WHERE user_id = ?
         ORDER BY first_linked_ms ASC`
      )
      .bind(user_id)
      .all<UserIdentityRow>();
    return r.results ?? [];
  } catch {
    return [];
  }
}

export async function getUserByEmail(env: Env, email: string): Promise<UserRow | null> {
  return await env.DB
    .prepare(`SELECT * FROM users WHERE email = ?`)
    .bind(email)
    .first<UserRow>();
}

export async function getUserById(env: Env, id: number): Promise<UserRow | null> {
  return await env.DB
    .prepare(`SELECT * FROM users WHERE id = ?`)
    .bind(id)
    .first<UserRow>();
}

export async function setEmailPassword(env: Env, email: string, hash: string): Promise<void> {
  await env.DB
    .prepare(`UPDATE users SET password_hash = ?, last_login_at = ? WHERE email = ?`)
    .bind(hash, Date.now(), email)
    .run();
}

export async function claimDevice(
  env: Env,
  device_id: string,
  user_id: number,
  label?: string
): Promise<DeviceRow> {
  const now = Date.now();
  await env.DB
    .prepare(
      `INSERT INTO devices (device_id, user_id, label, claimed_at, last_seen_at)
       VALUES (?, ?, ?, ?, ?)
       ON CONFLICT(device_id) DO UPDATE SET
         user_id = excluded.user_id,
         label = COALESCE(excluded.label, devices.label),
         last_seen_at = excluded.last_seen_at`
    )
    .bind(device_id, user_id, label ?? null, now, now)
    .run();

  const row = await env.DB
    .prepare(`SELECT * FROM devices WHERE device_id = ?`)
    .bind(device_id)
    .first<DeviceRow>();
  if (!row) throw new Error("claimDevice: row missing after insert");
  return row;
}

export async function getUserDevices(env: Env, user_id: number): Promise<DeviceRow[]> {
  const r = await env.DB
    .prepare(`SELECT * FROM devices WHERE user_id = ? ORDER BY last_seen_at DESC`)
    .bind(user_id)
    .all<DeviceRow>();
  return r.results ?? [];
}

export async function revokeDevice(env: Env, user_id: number, device_id: string): Promise<boolean> {
  const r = await env.DB
    .prepare(`DELETE FROM devices WHERE device_id = ? AND user_id = ?`)
    .bind(device_id, user_id)
    .run();
  return (r.meta?.changes ?? 0) > 0;
}

/* ── Token storage (hashed) ───────────────────────────────────────────── */

export async function recordTokenIssue(
  env: Env,
  opts: { token: string; user_id: number; device_id: string | null; ttlSecs: number }
): Promise<void> {
  const hash = await sha256Hex(opts.token);
  const now = Date.now();
  const exp = now + opts.ttlSecs * 1000;
  await env.DB
    .prepare(
      `INSERT INTO broker_tokens (token_hash, user_id, device_id, issued_at, expires_at) VALUES (?, ?, ?, ?, ?)`
    )
    .bind(hash, opts.user_id, opts.device_id, now, exp)
    .run();
}

export async function revokeBrokerToken(env: Env, token: string): Promise<void> {
  const hash = await sha256Hex(token);
  await env.DB
    .prepare(`UPDATE broker_tokens SET revoked_at = ? WHERE token_hash = ?`)
    .bind(Date.now(), hash)
    .run();
}

export async function tokenIsRevoked(env: Env, token: string): Promise<boolean> {
  const hash = await sha256Hex(token);
  const r = await env.DB
    .prepare(`SELECT revoked_at, expires_at FROM broker_tokens WHERE token_hash = ?`)
    .bind(hash)
    .first<{ revoked_at: number | null; expires_at: number }>();
  if (!r) return true; // unknown → reject
  if (r.revoked_at !== null) return true;
  if (r.expires_at < Date.now()) return true;
  return false;
}

async function sha256Hex(s: string): Promise<string> {
  const data = new TextEncoder().encode(s);
  const hash = await crypto.subtle.digest("SHA-256", data);
  return [...new Uint8Array(hash)].map(b => b.toString(16).padStart(2, "0")).join("");
}

/* ── User settings (per-user LLM API key vault) ──────────────────────── */

/// Provider env-var names the user is allowed to store. Anything outside
/// this list is silently dropped on write so an attacker can't stuff
/// arbitrary process env into the user's machines through this surface.
export const ALLOWED_ENV_KEYS = new Set<string>([
  "OPENCODE_API_KEY",
  "GROQ_API_KEY",
  "ANTHROPIC_API_KEY",
  "OPENAI_API_KEY",
  "GEMINI_API_KEY",
  "OPENROUTER_API_KEY",
  "CEREBRAS_API_KEY",
  "DEEPSEEK_API_KEY",
  "MISTRAL_API_KEY",
  "TOGETHER_API_KEY",
  "NVIDIA_NIM_API_KEY",
  // Cluster shared secret. HMAC-SHA256 key over cross-node RPC bodies
  // (X-Cluster-Auth header). Same value across every node in the user's
  // mesh. Stored in the vault so `phantom cluster join <name>` on a
  // fresh box can auto-configure agents.toml's [cluster] block from
  // the pulled env, no manual paste.
  "CLUSTER_SECRET",
]);

export interface UserSettingsRow {
  user_id: number;
  env: Record<string, string>;
  updated_at: number;
}

export async function getUserSettings(env: Env, user_id: number): Promise<UserSettingsRow> {
  const row = await env.DB
    .prepare(`SELECT env_json, updated_at FROM user_settings WHERE user_id = ?`)
    .bind(user_id)
    .first<{ env_json: string; updated_at: number }>();
  if (!row) {
    return { user_id, env: {}, updated_at: 0 };
  }

  // Decode path: rows written before the at-rest encryption landed are
  // raw JSON ({"K": "v", ...}); rows written after start with the
  // version marker ("v1.<base64>"). The marker check picks the right
  // path; both end up parsed into the same shape so callers don't care.
  let plaintextJson: string;
  if (isEncryptedBlob(row.env_json)) {
    const decrypted = await decryptForUser(env.ENV_VAULT_KEY, user_id, row.env_json);
    if (decrypted === null) {
      // Decrypt failed — wrong key or tampered. Surface as empty so the
      // user can re-save to recover, instead of throwing 500.
      return { user_id, env: {}, updated_at: row.updated_at };
    }
    plaintextJson = decrypted;
  } else {
    // Legacy plaintext row. setUserSettings will re-encrypt on next write.
    plaintextJson = row.env_json;
  }

  let parsed: Record<string, string> = {};
  try {
    const obj = JSON.parse(plaintextJson);
    if (obj && typeof obj === "object") {
      for (const [k, v] of Object.entries(obj)) {
        if (typeof v === "string") parsed[k] = v;
      }
    }
  } catch {
    // Malformed row → return empty; caller can re-save to fix it.
  }
  return { user_id, env: parsed, updated_at: row.updated_at };
}

/* ── Cluster peers vault ────────────────────────────────────────────── */

export interface ClusterPeerRow {
  name: string;
  url:  string;
  label: string | null;
  capabilities: string[];   // parsed from capabilities_json TEXT column
  updated_at: number;
}

interface ClusterPeerRowRaw {
  name: string;
  url:  string;
  label: string | null;
  capabilities: string;     // TEXT JSON array as stored
  updated_at: number;
}

export async function getUserClusterPeers(env: Env, user_id: number): Promise<ClusterPeerRow[]> {
  const r = await env.DB
    .prepare(`SELECT name, url, label, capabilities, updated_at FROM user_cluster_peers WHERE user_id = ? ORDER BY name`)
    .bind(user_id)
    .all<ClusterPeerRowRaw>();
  const rows = r.results ?? [];
  return rows.map(p => ({
    name: p.name,
    url: p.url,
    label: p.label,
    updated_at: p.updated_at,
    capabilities: parseCapabilities(p.capabilities),
  }));
}

function parseCapabilities(json: string): string[] {
  try {
    const v = JSON.parse(json ?? "[]");
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === "string") : [];
  } catch {
    return [];
  }
}

function normalizeCapabilities(input: unknown): string[] {
  // Accept Array<string> directly OR comma-separated string from form input.
  // Trim, lowercase, dedupe, drop empties.
  let raw: string[] = [];
  if (Array.isArray(input)) {
    raw = input.filter((x): x is string => typeof x === "string");
  } else if (typeof input === "string") {
    raw = input.split(",");
  }
  const seen = new Set<string>();
  const out: string[] = [];
  for (const r of raw) {
    const v = r.trim().toLowerCase();
    if (v && !seen.has(v)) { seen.add(v); out.push(v); }
  }
  return out;
}

/// Insert OR update a single peer by (user_id, name). Used by
/// `phantom login`'s self-register flow so a fresh machine adds itself
/// to the user's mesh without nuking peers other machines registered.
/// Trims inputs; bails out (no-op) if name or url is empty.
///
/// capabilities semantics: explicit empty array MEANS "clear all caps"
/// (overwrite). undefined/missing MEANS "keep existing caps" (don't
/// touch). Lets `phantom login` re-register without nuking caps the
/// user manually set in dashboard.
export async function upsertUserClusterPeer(
  env: Env,
  user_id: number,
  peer: { name: string; url: string; label?: string; capabilities?: unknown },
): Promise<ClusterPeerRow[]> {
  const name  = (peer.name ?? "").trim();
  const url   = (peer.url  ?? "").trim();
  const label = (peer.label ?? "").trim() || null;
  if (name.length === 0 || url.length === 0) {
    return await getUserClusterPeers(env, user_id);
  }
  const now = Date.now();
  // capabilities present in body → normalize + serialize. Absent → keep
  // existing (use COALESCE to preserve current value via excluded comparison).
  const capsProvided = peer.capabilities !== undefined;
  const capsJson = capsProvided ? JSON.stringify(normalizeCapabilities(peer.capabilities)) : "[]";
  if (capsProvided) {
    await env.DB
      .prepare(
        `INSERT INTO user_cluster_peers (user_id, name, url, label, capabilities, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, name) DO UPDATE SET
           url          = excluded.url,
           label        = COALESCE(excluded.label, user_cluster_peers.label),
           capabilities = excluded.capabilities,
           updated_at   = excluded.updated_at`
      )
      .bind(user_id, name, url, label, capsJson, now)
      .run();
  } else {
    // Don't overwrite caps on auto-register heartbeat.
    await env.DB
      .prepare(
        `INSERT INTO user_cluster_peers (user_id, name, url, label, capabilities, updated_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, name) DO UPDATE SET
           url        = excluded.url,
           label      = COALESCE(excluded.label, user_cluster_peers.label),
           updated_at = excluded.updated_at`
      )
      .bind(user_id, name, url, label, capsJson, now)
      .run();
  }
  return await getUserClusterPeers(env, user_id);
}

/// Replace the user's entire peer list. Names are normalized + trimmed;
/// rows with empty name OR empty url are silently dropped (a fully-blank
/// row in the dashboard form should be a no-op, not an error).
export async function setUserClusterPeers(
  env: Env,
  user_id: number,
  next: { name: string; url: string; label?: string; capabilities?: unknown }[],
): Promise<ClusterPeerRow[]> {
  const filtered = next
    .map(p => ({
      name: (p.name ?? "").trim(),
      url:  (p.url  ?? "").trim(),
      label: (p.label ?? "").trim() || null,
      capabilities: JSON.stringify(normalizeCapabilities(p.capabilities ?? [])),
    }))
    .filter(p => p.name.length > 0 && p.url.length > 0);
  const now = Date.now();
  const stmts: D1PreparedStatement[] = [
    env.DB.prepare(`DELETE FROM user_cluster_peers WHERE user_id = ?`).bind(user_id),
  ];
  for (const p of filtered) {
    stmts.push(env.DB
      .prepare(`INSERT INTO user_cluster_peers (user_id, name, url, label, capabilities, updated_at) VALUES (?, ?, ?, ?, ?, ?)`)
      .bind(user_id, p.name, p.url, p.label, p.capabilities, now));
  }
  await env.DB.batch(stmts);
  return await getUserClusterPeers(env, user_id);
}

/// Merge `next` into the user's existing stored env.
///
/// Semantics:
///   - keys in `next` with a non-empty trimmed value → overwrite stored
///   - keys in `next` with an empty value → DELETE that key from stored
///   - keys NOT in `next` → kept untouched (this is the merge half)
///   - keys in `next` not in ALLOWED_ENV_KEYS → silently dropped
///
/// Why merge instead of full replace: the dashboard form submits ONLY
/// the inputs the user typed into. Empty inputs are "no change" — full-
/// replace semantics meant a stray Save with all boxes blank wiped every
/// previously-stored key. The user hit this trap on first use; merge
/// makes the form forgiving while still letting them blank a key by
/// typing a single space (which trims to "" → explicit delete).
export async function setUserSettings(
  env: Env,
  user_id: number,
  next: Record<string, string>,
): Promise<UserSettingsRow> {
  const current = await getUserSettings(env, user_id);
  const merged: Record<string, string> = { ...current.env };
  for (const [k, v] of Object.entries(next)) {
    if (!ALLOWED_ENV_KEYS.has(k)) continue;
    if (typeof v !== "string") continue;
    const trimmed = v.trim();
    if (trimmed.length === 0) {
      // Explicit delete: client sent the key with an empty value.
      delete merged[k];
    } else {
      merged[k] = trimmed;
    }
  }
  const now = Date.now();
  const plaintextJson = JSON.stringify(merged);
  // Encrypt before writing — every row stored after this commit lands
  // is "v1." prefixed. getUserSettings handles legacy plaintext rows
  // transparently for back-compat.
  const ciphertext = await encryptForUser(env.ENV_VAULT_KEY, user_id, plaintextJson);
  await env.DB
    .prepare(
      `INSERT INTO user_settings (user_id, env_json, updated_at) VALUES (?, ?, ?)
       ON CONFLICT(user_id) DO UPDATE SET env_json = excluded.env_json, updated_at = excluded.updated_at`
    )
    .bind(user_id, ciphertext, now)
    .run();
  return { user_id, env: merged, updated_at: now };
}

/* ── Active TUI sessions (presence) ─────────────────────────────────── */

export interface ActiveSessionRow {
  id: string;
  machine: string;
  agent: string;
  cwd: string;
  started_at: number;
  last_seen_at: number;
}

/// Stale threshold: 60s without a heartbeat → treat as dead.
/// CLI heartbeats every 30s, so this gives one missed beat of slack.
const SESSION_STALE_AFTER_MS = 60_000;

export async function listActiveSessions(env: Env, user_id: number): Promise<ActiveSessionRow[]> {
  const cutoff = Date.now() - SESSION_STALE_AFTER_MS;
  const r = await env.DB
    .prepare(
      `SELECT id, machine, agent, cwd, started_at, last_seen_at
         FROM user_active_sessions
        WHERE user_id = ? AND last_seen_at >= ?
        ORDER BY machine, started_at`
    )
    .bind(user_id, cutoff)
    .all<ActiveSessionRow>();
  return r.results ?? [];
}

export async function upsertSessionHeartbeat(
  env: Env,
  user_id: number,
  s: { id: string; machine: string; agent?: string; cwd?: string },
): Promise<ActiveSessionRow[]> {
  const id      = (s.id ?? "").trim();
  const machine = (s.machine ?? "").trim();
  if (id.length === 0 || machine.length === 0) {
    return await listActiveSessions(env, user_id);
  }
  const agent = (s.agent ?? "master").trim() || "master";
  const cwd   = (s.cwd ?? "").trim();
  const now   = Date.now();
  await env.DB
    .prepare(
      `INSERT INTO user_active_sessions (id, user_id, machine, agent, cwd, started_at, last_seen_at)
       VALUES (?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(user_id, id) DO UPDATE SET
         machine      = excluded.machine,
         agent        = excluded.agent,
         cwd          = excluded.cwd,
         last_seen_at = excluded.last_seen_at`
    )
    .bind(id, user_id, machine, agent, cwd, now, now)
    .run();
  return await listActiveSessions(env, user_id);
}

export async function endSession(env: Env, user_id: number, id: string): Promise<void> {
  await env.DB
    .prepare(`DELETE FROM user_active_sessions WHERE user_id = ? AND id = ?`)
    .bind(user_id, id)
    .run();
}
