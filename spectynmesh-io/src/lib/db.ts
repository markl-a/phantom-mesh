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
  // mesh. Stored in the vault so `spectyn cluster join <name>` on a
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

  // Decode path.
  //
  // SPEC-15 (broker vault E2EE): the broker MUST NOT decrypt user values on
  // the live request path. The server-side decrypt is therefore GATED behind
  // the LEGACY_VAULT_DECRYPT flag, which is enabled ONLY during the one-time
  // data-migration window that re-seals legacy "v1." rows under the new
  // /vault/* E2EE path. With the flag off (the default), an encrypted row is
  // unreadable server-side and surfaces as empty env — exactly the desired
  // E2EE behavior (no plaintext ever leaves this function).
  //
  // TODO(spec15-migration): once every legacy "v1." row has been re-sealed
  // via /vault/set, delete this decrypt branch, drop the crypto import, and
  // remove ENV_VAULT_KEY (see lib/crypto.ts banner).
  //
  // Legacy unencrypted rows (raw JSON {"K":"v",...}, written before at-rest
  // encryption landed) are still readable: they are not secrets-at-rest and
  // predate any sealing. They parse directly below.
  const legacyDecryptEnabled =
    ((env as { LEGACY_VAULT_DECRYPT?: string }).LEGACY_VAULT_DECRYPT ?? "") === "1";

  let plaintextJson: string;
  if (isEncryptedBlob(row.env_json)) {
    if (!legacyDecryptEnabled) {
      // E2EE default: broker has no business decrypting. Treat as empty so
      // the dashboard/CLI fall through to the /vault/* path. The ciphertext
      // stays untouched in the column for the migration tool to consume.
      return { user_id, env: {}, updated_at: row.updated_at };
    }
    // LEGACY-MIGRATION-ONLY branch (flag explicitly enabled).
    const decrypted = await decryptForUser(env.ENV_VAULT_KEY, user_id, row.env_json);
    if (decrypted === null) {
      // Decrypt failed — wrong key or tampered. Surface as empty so the
      // user can re-save to recover, instead of throwing 500.
      return { user_id, env: {}, updated_at: row.updated_at };
    }
    plaintextJson = decrypted;
  } else {
    // Legacy plaintext row (pre-encryption). Readable as-is.
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
/// `spectyn login`'s self-register flow so a fresh machine adds itself
/// to the user's mesh without nuking peers other machines registered.
/// Trims inputs; bails out (no-op) if name or url is empty.
///
/// capabilities semantics: explicit empty array MEANS "clear all caps"
/// (overwrite). undefined/missing MEANS "keep existing caps" (don't
/// touch). Lets `spectyn login` re-register without nuking caps the
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
  // SPEC-15 E2EE gate: by default the broker REFUSES server-side plaintext
  // writes. Secret writes must go through POST /vault/set with client-sealed
  // age-v1 ciphertext (the broker never holds a decryption key). The legacy
  // server-encrypt path below is only reachable during a one-time migration
  // with LEGACY_VAULT_WRITE=1 explicitly set on the Worker. This closes the
  // "broker still receives plaintext on PUT /api/me/settings" leak (review #2).
  if (((env as { LEGACY_VAULT_WRITE?: string }).LEGACY_VAULT_WRITE ?? "") !== "1") {
    throw new Error(
      "server-side vault write disabled (SPEC-15 E2EE): clients must use POST /vault/set with client-sealed ciphertext; set LEGACY_VAULT_WRITE=1 only for one-time migration",
    );
  }
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
  // DEPRECATED write path. SPEC-15 (broker vault E2EE) moves secret writes
  // to the dumb-storage /vault/set route, which stores client-sealed age-v1
  // ciphertext the broker can never read. This server-side encryptForUser
  // path still derives a key from ENV_VAULT_KEY, which violates the E2EE
  // invariant ("broker never holds a decryption key").
  //
  // TODO(spec15-migration): retire this write path. Once clients write via
  // /vault/set, remove encryptForUser + the ENV_VAULT_KEY dependency here so
  // no request handler touches the master key. Kept for now so the legacy
  // dashboard PUT /api/me/settings keeps working until the client migrates;
  // do NOT add new callers.
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

/* ── F205: dispatch history ──────────────────────────────────────────── */

export interface DispatchRow {
  job_id: string;
  user_id: number;
  peer: string;
  provider: string;
  model: string;
  prompt: string;
  required_caps: string[];
  status: "pending" | "running" | "done" | "cancelled" | "error";
  result: string;
  error_message: string | null;
  started_at: number;
  completed_at: number | null;
}

interface DispatchRowRaw {
  job_id: string;
  user_id: number;
  peer: string;
  provider: string;
  model: string;
  prompt: string;
  required_caps: string;
  status: string;
  result: string;
  error_message: string | null;
  started_at: number;
  completed_at: number | null;
}

function rowToDispatch(r: DispatchRowRaw): DispatchRow {
  return {
    job_id: r.job_id,
    user_id: r.user_id,
    peer: r.peer,
    provider: r.provider,
    model: r.model,
    prompt: r.prompt,
    required_caps: parseCapabilities(r.required_caps),
    status: (["pending","running","done","cancelled","error"].includes(r.status)
      ? r.status as DispatchRow["status"]
      : "pending"),
    result: r.result,
    error_message: r.error_message,
    started_at: r.started_at,
    completed_at: r.completed_at,
  };
}

/// Insert a fresh dispatch row in `pending` state. Returns the persisted row.
export async function createDispatch(
  env: Env,
  user_id: number,
  d: { job_id: string; peer: string; provider: string; model: string; prompt: string; required_caps: string[] },
): Promise<DispatchRow> {
  const now = Date.now();
  const caps = JSON.stringify(normalizeCapabilities(d.required_caps));
  await env.DB
    .prepare(
      `INSERT INTO dispatches (job_id, user_id, peer, provider, model, prompt, required_caps, status, result, started_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, 'pending', '', ?)`
    )
    .bind(d.job_id, user_id, d.peer, d.provider, d.model, d.prompt, caps, now)
    .run();
  const row = await getDispatch(env, user_id, d.job_id);
  if (!row) throw new Error("createDispatch: row missing after insert");
  return row;
}

export async function getDispatch(env: Env, user_id: number, job_id: string): Promise<DispatchRow | null> {
  const r = await env.DB
    .prepare(`SELECT * FROM dispatches WHERE user_id = ? AND job_id = ?`)
    .bind(user_id, job_id)
    .first<DispatchRowRaw>();
  return r ? rowToDispatch(r) : null;
}

/// Update status only — used by /cancel and by the SPA when it pushes a
/// terminal event. Idempotent. Returns false if job doesn't belong to user.
export async function updateDispatchStatus(
  env: Env,
  user_id: number,
  job_id: string,
  status: DispatchRow["status"],
  opts: { error_message?: string; result_append?: string } = {},
): Promise<boolean> {
  const now = Date.now();
  const completed = (status === "done" || status === "cancelled" || status === "error") ? now : null;
  // We bind result update as a CASE in SQL so we can append safely without
  // round-tripping (atomic).
  const stmt = opts.result_append !== undefined
    ? env.DB
        .prepare(
          `UPDATE dispatches
           SET status = ?, error_message = ?, completed_at = COALESCE(?, completed_at),
               result = substr(result || ?, 1, 524288)
           WHERE user_id = ? AND job_id = ?`
        )
        .bind(status, opts.error_message ?? null, completed, opts.result_append, user_id, job_id)
    : env.DB
        .prepare(
          `UPDATE dispatches
           SET status = ?, error_message = ?, completed_at = COALESCE(?, completed_at)
           WHERE user_id = ? AND job_id = ?`
        )
        .bind(status, opts.error_message ?? null, completed, user_id, job_id);
  const r = await stmt.run();
  return (r.meta?.changes ?? 0) > 0;
}

export interface DispatchListFilter {
  peer?: string;
  status?: string;
  cap?: string;
  from?: number;
  to?: number;
  q?: string;
  page?: number;
  page_size?: number;
}

export interface DispatchPage {
  rows: DispatchRow[];
  page: number;
  page_size: number;
  total: number;
}

/// Paginated history list. Uses the FTS5 index when `q` is set.
export async function listDispatches(
  env: Env,
  user_id: number,
  f: DispatchListFilter,
): Promise<DispatchPage> {
  const page = Math.max(1, Math.floor(f.page ?? 1));
  const pageSize = Math.min(200, Math.max(1, Math.floor(f.page_size ?? 50)));
  const offset = (page - 1) * pageSize;

  const where: string[] = ["user_id = ?"];
  const args: unknown[] = [user_id];
  if (f.peer) { where.push("peer = ?"); args.push(f.peer); }
  if (f.status) { where.push("status = ?"); args.push(f.status); }
  if (typeof f.from === "number") { where.push("started_at >= ?"); args.push(f.from); }
  if (typeof f.to   === "number") { where.push("started_at <= ?"); args.push(f.to); }
  // cap: substring on required_caps JSON. Cheaper than parsing in SQL.
  if (f.cap) { where.push("required_caps LIKE ?"); args.push(`%${JSON.stringify(f.cap).slice(1, -1)}%`); }

  let sql: string;
  if (f.q && f.q.trim().length > 0) {
    // Hit the FTS5 index. We join on rowid (FTS contentless table) and
    // re-apply the WHERE filters on the outer dispatches row.
    sql = `
      SELECT d.* FROM dispatches d
      JOIN dispatches_fts f ON f.rowid = d.rowid
      WHERE ${where.join(" AND ")} AND dispatches_fts MATCH ?
      ORDER BY d.started_at DESC
      LIMIT ? OFFSET ?`;
    args.push(f.q.trim(), pageSize, offset);
  } else {
    sql = `SELECT * FROM dispatches WHERE ${where.join(" AND ")}
           ORDER BY started_at DESC LIMIT ? OFFSET ?`;
    args.push(pageSize, offset);
  }

  const r = await env.DB.prepare(sql).bind(...args).all<DispatchRowRaw>();
  const rows = (r.results ?? []).map(rowToDispatch);

  // total = a second simpler COUNT(*). For very large datasets this is
  // an unnecessary cost; cap by using EXPLAIN-friendly LIMIT 1000 on the
  // count so we don't bog the worker down on a 50k history.
  const countSql = `SELECT COUNT(*) AS n FROM (
    SELECT 1 FROM dispatches WHERE ${where.slice(0, -0).join(" AND ")}
    LIMIT 1000
  )`;
  const countArgs = args.slice(0, args.length - (f.q ? 3 : 2));
  const countRow = await env.DB.prepare(countSql).bind(...countArgs).first<{ n: number }>();
  const total = countRow?.n ?? rows.length;

  return { rows, page, page_size: pageSize, total };
}

/// Retention sweep — purge dispatches older than `older_than_ms`.
/// Returns the number of rows deleted. The triggers on `dispatches` keep
/// the FTS5 index in sync, so no separate FTS purge needed.
export async function purgeDispatchesOlderThan(
  env: Env,
  older_than_ms: number,
): Promise<number> {
  const r = await env.DB
    .prepare(`DELETE FROM dispatches WHERE started_at < ?`)
    .bind(older_than_ms)
    .run();
  return r.meta?.changes ?? 0;
}

/* ── F205: dispatch recipes (per-user saved templates) ───────────────── */

export interface RecipeRow {
  id: string;
  user_id: number;
  name: string;
  peer: string;
  provider: string;
  model: string;
  prompt: string;
  required_caps: string[];
  created_at: number;
  updated_at: number;
}

interface RecipeRowRaw {
  id: string;
  user_id: number;
  name: string;
  peer: string;
  provider: string;
  model: string;
  prompt: string;
  required_caps: string;
  created_at: number;
  updated_at: number;
}

function rowToRecipe(r: RecipeRowRaw): RecipeRow {
  return {
    id: r.id,
    user_id: r.user_id,
    name: r.name,
    peer: r.peer,
    provider: r.provider,
    model: r.model,
    prompt: r.prompt,
    required_caps: parseCapabilities(r.required_caps),
    created_at: r.created_at,
    updated_at: r.updated_at,
  };
}

export async function listRecipes(env: Env, user_id: number): Promise<RecipeRow[]> {
  const r = await env.DB
    .prepare(`SELECT * FROM dispatch_recipes WHERE user_id = ? ORDER BY updated_at DESC`)
    .bind(user_id)
    .all<RecipeRowRaw>();
  return (r.results ?? []).map(rowToRecipe);
}

export async function getRecipe(env: Env, user_id: number, id: string): Promise<RecipeRow | null> {
  const r = await env.DB
    .prepare(`SELECT * FROM dispatch_recipes WHERE user_id = ? AND id = ?`)
    .bind(user_id, id)
    .first<RecipeRowRaw>();
  return r ? rowToRecipe(r) : null;
}

export async function upsertRecipe(
  env: Env,
  user_id: number,
  r: Partial<RecipeRow> & { name: string },
): Promise<RecipeRow> {
  const now = Date.now();
  const id = r.id && r.id.length > 0 ? r.id : crypto.randomUUID();
  const caps = JSON.stringify(normalizeCapabilities(r.required_caps ?? []));
  await env.DB
    .prepare(
      `INSERT INTO dispatch_recipes (id, user_id, name, peer, provider, model, prompt, required_caps, created_at, updated_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT(user_id, id) DO UPDATE SET
         name          = excluded.name,
         peer          = excluded.peer,
         provider      = excluded.provider,
         model         = excluded.model,
         prompt        = excluded.prompt,
         required_caps = excluded.required_caps,
         updated_at    = excluded.updated_at`
    )
    .bind(
      id, user_id, r.name,
      r.peer ?? "", r.provider ?? "", r.model ?? "", r.prompt ?? "",
      caps, now, now,
    )
    .run();
  const saved = await getRecipe(env, user_id, id);
  if (!saved) throw new Error("upsertRecipe: row missing after insert");
  return saved;
}

export async function deleteRecipe(env: Env, user_id: number, id: string): Promise<boolean> {
  const r = await env.DB
    .prepare(`DELETE FROM dispatch_recipes WHERE user_id = ? AND id = ?`)
    .bind(user_id, id)
    .run();
  return (r.meta?.changes ?? 0) > 0;
}

/* ── F205: user preferences (heartbeat cadence + retention) ──────────── */

export interface UserPreferences {
  user_id: number;
  heartbeat_secs: number;
  retention_days: number;
  updated_at: number;
}

const DEFAULT_HEARTBEAT_SECS = 30;
const DEFAULT_RETENTION_DAYS = 90;
/// Server-enforced clamp for heartbeat — TUI hammering the worker every
/// second is wasteful, and >10 min is effectively "off". Match in the
/// dashboard slider.
const HEARTBEAT_MIN = 10;
const HEARTBEAT_MAX = 600;
const RETENTION_MIN = 7;
const RETENTION_MAX = 365;

export async function getUserPreferences(env: Env, user_id: number): Promise<UserPreferences> {
  const r = await env.DB
    .prepare(`SELECT heartbeat_secs, retention_days, updated_at FROM user_preferences WHERE user_id = ?`)
    .bind(user_id)
    .first<{ heartbeat_secs: number; retention_days: number; updated_at: number }>();
  if (!r) {
    return {
      user_id,
      heartbeat_secs: DEFAULT_HEARTBEAT_SECS,
      retention_days: DEFAULT_RETENTION_DAYS,
      updated_at: 0,
    };
  }
  return {
    user_id,
    heartbeat_secs: r.heartbeat_secs,
    retention_days: r.retention_days,
    updated_at: r.updated_at,
  };
}

export async function setUserPreferences(
  env: Env,
  user_id: number,
  p: { heartbeat_secs?: number; retention_days?: number },
): Promise<UserPreferences> {
  const current = await getUserPreferences(env, user_id);
  const next = {
    heartbeat_secs: clamp(
      Math.floor(p.heartbeat_secs ?? current.heartbeat_secs),
      HEARTBEAT_MIN, HEARTBEAT_MAX,
    ),
    retention_days: clamp(
      Math.floor(p.retention_days ?? current.retention_days),
      RETENTION_MIN, RETENTION_MAX,
    ),
  };
  const now = Date.now();
  await env.DB
    .prepare(
      `INSERT INTO user_preferences (user_id, heartbeat_secs, retention_days, updated_at)
       VALUES (?, ?, ?, ?)
       ON CONFLICT(user_id) DO UPDATE SET
         heartbeat_secs = excluded.heartbeat_secs,
         retention_days = excluded.retention_days,
         updated_at     = excluded.updated_at`
    )
    .bind(user_id, next.heartbeat_secs, next.retention_days, now)
    .run();
  return { user_id, ...next, updated_at: now };
}

function clamp(n: number, lo: number, hi: number): number {
  if (!Number.isFinite(n)) return lo;
  return Math.max(lo, Math.min(hi, n));
}

/* ── F205: peer-capabilities editor (lifts edits out of the bulk PUT) ─ */

/// PUT /api/me/peer-capabilities body shape: { peer: string, capabilities: string[] }.
/// Updates JUST the capabilities column on the existing peer row; refuses if
/// the peer doesn't exist for this user (404 — don't auto-create from this
/// surface to keep the cluster peers list as the source of truth).
export async function setPeerCapabilities(
  env: Env,
  user_id: number,
  peer: string,
  caps: string[],
): Promise<ClusterPeerRow | null> {
  const trimmed = peer.trim();
  if (trimmed.length === 0) return null;
  const capsJson = JSON.stringify(normalizeCapabilities(caps));
  const now = Date.now();
  const r = await env.DB
    .prepare(`UPDATE user_cluster_peers SET capabilities = ?, updated_at = ? WHERE user_id = ? AND name = ?`)
    .bind(capsJson, now, user_id, trimmed)
    .run();
  if ((r.meta?.changes ?? 0) === 0) return null;
  const peers = await getUserClusterPeers(env, user_id);
  return peers.find(p => p.name === trimmed) ?? null;
}

/* ── F205: aggregate dispatches per peer/cap (consumed by F201) ─────── */

export interface PeerCapAggregate {
  cap: string;
  running_count: number;
  last_run_at: number | null;
}

export async function aggregateCapsForPeer(
  env: Env,
  user_id: number,
  peer: string,
): Promise<PeerCapAggregate[]> {
  // First find the peer's declared caps so we have the universe of rows
  // to return — even caps that have never been dispatched against still
  // show up with running_count=0.
  const peers = await getUserClusterPeers(env, user_id);
  const target = peers.find(p => p.name === peer);
  if (!target) return [];

  // Now pull recent dispatches for this peer + cap-bucket them.
  const r = await env.DB
    .prepare(
      `SELECT required_caps, status, started_at FROM dispatches
        WHERE user_id = ? AND peer = ?
        ORDER BY started_at DESC LIMIT 1000`
    )
    .bind(user_id, peer)
    .all<{ required_caps: string; status: string; started_at: number }>();
  const rows = r.results ?? [];

  const acc = new Map<string, { running: number; last: number | null }>();
  for (const cap of target.capabilities) {
    acc.set(cap, { running: 0, last: null });
  }
  for (const row of rows) {
    const caps = parseCapabilities(row.required_caps);
    for (const c of caps) {
      const entry = acc.get(c) ?? { running: 0, last: null };
      if (row.status === "running" || row.status === "pending") entry.running += 1;
      if (entry.last === null || row.started_at > entry.last) entry.last = row.started_at;
      acc.set(c, entry);
    }
  }
  return [...acc.entries()].map(([cap, v]) => ({
    cap, running_count: v.running, last_run_at: v.last,
  })).sort((a, b) => a.cap.localeCompare(b.cap));
}

/* ── SPEC-15: E2EE vault dumb-storage (DRAFT) ─────────────────────────── */
//
// IMPORTANT — these helpers store CLIENT-SEALED ciphertext VERBATIM. They
// deliberately do NOT import or call anything from lib/crypto.ts. The broker
// holds no key and can never decrypt `value_sealed` or `wrapped_vault_seal_key`,
// nor verify `client_hmac_hex` (it has no key). This is the true-E2EE
// replacement for the legacy user_settings + ENV_VAULT_KEY path.
//
// MIGRATION TODO (do NOT do it here — out of this scope/file-set):
//   * Once the client write path (SPEC-15 §8.B) is live, retire
//     getSettingsRaw / decryptForUser / encryptForUser usage in
//     getUserSettings + setUserSettings, then drop ENV_VAULT_KEY.
//   * Re-seal legacy `user_settings.env_json` rows client-side on first
//     write after upgrade, then purge.

export interface VaultItemRow {
  service: string;
  key: string;
  value_sealed: string;
  client_hmac_hex: string;
  ts_ms: number;
  wrote_at_ms: number;
}

export interface VaultItemMeta {
  service: string;
  key: string;
  ts_ms: number;
  byte_len: number;
}

/// 64 lower-hex chars. Structural-only check — the broker has no key and
/// CANNOT cryptographically verify the MAC. This rejects obviously-malformed
/// input (wrong length / non-hex), nothing more.
const HMAC_HEX_RE = /^[0-9a-f]{64}$/;

/// base64url decoded byte length WITHOUT decoding age — we only need the
/// length for list-mode `byte_len`. base64url(no-pad): every 4 chars → 3
/// bytes; remainder of 2 → 1 byte, 3 → 2 bytes.
export function base64urlByteLen(s: string): number {
  const n = s.length;
  const full = Math.floor(n / 4) * 3;
  const rem = n % 4;
  return full + (rem === 2 ? 1 : rem === 3 ? 2 : 0);
}

/// Validate a single sealed item BEFORE persist. Returns a rejection reason
/// string, or null if structurally valid. The broker does no crypto here.
export function validateSealedItem(
  it: { service?: unknown; key?: unknown; value_sealed?: unknown; ts_ms?: unknown; client_hmac_hex?: unknown },
): string | null {
  if (typeof it.service !== "string" || it.service.length === 0 || it.service.includes("\n")) return "bad_service";
  if (typeof it.key !== "string" || it.key.length === 0 || it.key.includes("\n")) return "bad_key";
  if (typeof it.value_sealed !== "string" || it.value_sealed.length === 0) return "bad_value_sealed";
  if (typeof it.ts_ms !== "number" || !Number.isFinite(it.ts_ms) || it.ts_ms < 0) return "bad_ts_ms";
  if (typeof it.client_hmac_hex !== "string" || !HMAC_HEX_RE.test(it.client_hmac_hex)) return "hmac_mismatch";
  return null;
}

/// UPSERT a batch of sealed items. Idempotent by (user_id, service, key);
/// last write wins (we overwrite verbatim). Returns counts + per-item rejects.
export async function setVaultItems(
  env: Env,
  user_id: number,
  items: Array<{ service: string; key: string; value_sealed: string; ts_ms: number; client_hmac_hex: string }>,
): Promise<{ stored: number; rejected: Array<{ service: string; key: string; reason: string }>; wrote_at_ms: number }> {
  const now = Date.now();
  const rejected: Array<{ service: string; key: string; reason: string }> = [];
  const stmts: D1PreparedStatement[] = [];
  for (const it of items) {
    const reason = validateSealedItem(it);
    if (reason) {
      rejected.push({ service: String((it as { service?: unknown }).service ?? ""), key: String((it as { key?: unknown }).key ?? ""), reason });
      continue;
    }
    stmts.push(env.DB
      .prepare(
        `INSERT INTO vault_items (user_id, service, key, value_sealed, client_hmac_hex, ts_ms, wrote_at_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(user_id, service, key) DO UPDATE SET
           value_sealed    = excluded.value_sealed,
           client_hmac_hex = excluded.client_hmac_hex,
           ts_ms           = excluded.ts_ms,
           wrote_at_ms     = excluded.wrote_at_ms`
      )
      .bind(user_id, it.service, it.key, it.value_sealed, it.client_hmac_hex, it.ts_ms, now));
  }
  if (stmts.length > 0) await env.DB.batch(stmts);
  return { stored: stmts.length, rejected, wrote_at_ms: now };
}

export async function getVaultItem(
  env: Env,
  user_id: number,
  service: string,
  key: string,
): Promise<VaultItemRow | null> {
  const r = await env.DB
    .prepare(`SELECT service, key, value_sealed, client_hmac_hex, ts_ms, wrote_at_ms FROM vault_items WHERE user_id = ? AND service = ? AND key = ?`)
    .bind(user_id, service, key)
    .first<VaultItemRow>();
  return r ?? null;
}

export async function listVaultItems(env: Env, user_id: number): Promise<VaultItemMeta[]> {
  const r = await env.DB
    .prepare(`SELECT service, key, value_sealed, ts_ms FROM vault_items WHERE user_id = ? ORDER BY service, key`)
    .bind(user_id)
    .all<{ service: string; key: string; value_sealed: string; ts_ms: number }>();
  const rows = r.results ?? [];
  // byte_len is the decoded length of the sealed blob. We compute it from
  // the base64url length WITHOUT decrypting — no key, no decode of plaintext.
  return rows.map(p => ({ service: p.service, key: p.key, ts_ms: p.ts_ms, byte_len: base64urlByteLen(p.value_sealed) }));
}

/* ── SPEC-15: vault wipe jobs ─────────────────────────────────────────── */

export interface VaultWipeJobRow {
  wipe_id: string;
  scope: string;
  status: "pending" | "in_progress" | "completed" | "failed";
  scheduled_at_ms: number;
  complete_by_ms: number;
  completed_at_ms: number | null;
}

const WIPE_SLA_MS = 24 * 60 * 60 * 1000;

/// Find an existing pending/in_progress wipe for this user (for 409 guard).
export async function getActiveWipe(env: Env, user_id: number): Promise<VaultWipeJobRow | null> {
  const r = await env.DB
    .prepare(`SELECT wipe_id, scope, status, scheduled_at_ms, complete_by_ms, completed_at_ms FROM vault_wipe_jobs WHERE user_id = ? AND status IN ('pending','in_progress') ORDER BY scheduled_at_ms DESC LIMIT 1`)
    .bind(user_id)
    .first<VaultWipeJobRow>();
  return r ?? null;
}

/// Schedule a wipe. Returns the existing active job (with `existed: true`)
/// when one is already in flight, so the route can map to 409.
export async function scheduleWipe(
  env: Env,
  user_id: number,
  scope: "vault" | "all",
  reason: string | null,
): Promise<{ job: VaultWipeJobRow; existed: boolean }> {
  const existing = await getActiveWipe(env, user_id);
  if (existing) return { job: existing, existed: true };
  const now = Date.now();
  const wipe_id = `wipe_${crypto.randomUUID().replace(/-/g, "").slice(0, 16)}`;
  const complete_by = now + WIPE_SLA_MS;
  await env.DB
    .prepare(
      `INSERT INTO vault_wipe_jobs (wipe_id, user_id, scope, reason, status, scheduled_at_ms, complete_by_ms, completed_at_ms)
       VALUES (?, ?, ?, ?, 'pending', ?, ?, NULL)`
    )
    .bind(wipe_id, user_id, scope, reason, now, complete_by)
    .run();
  return {
    job: { wipe_id, scope, status: "pending", scheduled_at_ms: now, complete_by_ms: complete_by, completed_at_ms: null },
    existed: false,
  };
}

/// Look up a wipe by its id (globally). Returns the row + owning user_id so
/// the route can enforce ownership (403 wipe_not_owned) vs 404 wipe_not_found.
export async function getWipeJob(env: Env, wipe_id: string): Promise<{ job: VaultWipeJobRow; user_id: number } | null> {
  const r = await env.DB
    .prepare(`SELECT wipe_id, user_id, scope, status, scheduled_at_ms, complete_by_ms, completed_at_ms FROM vault_wipe_jobs WHERE wipe_id = ?`)
    .bind(wipe_id)
    .first<VaultWipeJobRow & { user_id: number }>();
  if (!r) return null;
  const { user_id: owner, ...job } = r;
  return { job, user_id: owner };
}

/* ── SPEC-15: wrapped seal-key courier ────────────────────────────────── */

const WRAP_TTL_MS = 7 * 24 * 60 * 60 * 1000;

export interface VaultKeyWrapRow {
  wrap_id: string;
  target_device_pubkey_hex: string;
  wrapped_vault_seal_key: string;
  key_version: number;
  wrapped_by_device_hint: string | null;
  stored_at_ms: number;
  expires_at_ms: number;
}

/// Store a wrapped seal key for a target device. Returns the existing row
/// (with `existed: true`) when a wrap for the same target already exists,
/// so the route can map to 409 wrap_exists.
export async function storeKeyWrap(
  env: Env,
  user_id: number,
  w: { target_device_pubkey_hex: string; wrapped_vault_seal_key: string; key_version: number; wrapped_by_device_hint?: string | null },
): Promise<{ row: VaultKeyWrapRow; existed: boolean }> {
  const existing = await env.DB
    .prepare(`SELECT wrap_id, target_device_pubkey_hex, wrapped_vault_seal_key, key_version, wrapped_by_device_hint, stored_at_ms, expires_at_ms FROM vault_key_wraps WHERE user_id = ? AND target_device_pubkey_hex = ? AND expires_at_ms > ? ORDER BY stored_at_ms DESC LIMIT 1`)
    .bind(user_id, w.target_device_pubkey_hex, Date.now())
    .first<VaultKeyWrapRow>();
  if (existing) return { row: existing, existed: true };
  const now = Date.now();
  const wrap_id = `wrap_${crypto.randomUUID().replace(/-/g, "").slice(0, 16)}`;
  const expires = now + WRAP_TTL_MS;
  await env.DB
    .prepare(
      `INSERT INTO vault_key_wraps (wrap_id, user_id, target_device_pubkey_hex, wrapped_vault_seal_key, key_version, wrapped_by_device_hint, stored_at_ms, expires_at_ms)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?)`
    )
    .bind(wrap_id, user_id, w.target_device_pubkey_hex, w.wrapped_vault_seal_key, w.key_version, w.wrapped_by_device_hint ?? null, now, expires)
    .run();
  return {
    row: {
      wrap_id,
      target_device_pubkey_hex: w.target_device_pubkey_hex,
      wrapped_vault_seal_key: w.wrapped_vault_seal_key,
      key_version: w.key_version,
      wrapped_by_device_hint: w.wrapped_by_device_hint ?? null,
      stored_at_ms: now,
      expires_at_ms: expires,
    },
    existed: false,
  };
}

/// New device pulls the most recent non-expired wrap targeted at its pubkey.
export async function getKeyWrapForDevice(
  env: Env,
  user_id: number,
  target_device_pubkey_hex: string,
): Promise<VaultKeyWrapRow | null> {
  const r = await env.DB
    .prepare(`SELECT wrap_id, target_device_pubkey_hex, wrapped_vault_seal_key, key_version, wrapped_by_device_hint, stored_at_ms, expires_at_ms FROM vault_key_wraps WHERE user_id = ? AND target_device_pubkey_hex = ? AND expires_at_ms > ? ORDER BY stored_at_ms DESC LIMIT 1`)
    .bind(user_id, target_device_pubkey_hex, Date.now())
    .first<VaultKeyWrapRow>();
  return r ?? null;
}

/* ── F205: bulk revoke broker tokens for a user (DELETE /sessions/all-others) ─── */

/// Marks every broker_token issued to this user as revoked EXCEPT the
/// hash provided (which is the token of the caller making the request).
/// Returns the count of rows revoked.
export async function revokeAllOtherBrokerTokens(
  env: Env,
  user_id: number,
  keep_token: string,
): Promise<number> {
  const keep_hash = await sha256Hex(keep_token);
  const now = Date.now();
  const r = await env.DB
    .prepare(
      `UPDATE broker_tokens SET revoked_at = ?
        WHERE user_id = ? AND token_hash != ? AND revoked_at IS NULL`
    )
    .bind(now, user_id, keep_hash)
    .run();
  return r.meta?.changes ?? 0;
}
