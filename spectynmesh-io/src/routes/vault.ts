// SPEC-15 E2EE vault routes — DUMB STORAGE (DRAFT).
//
// =====================================================================
//  THE WHOLE POINT (E2EE invariants — see WIRE CONTRACT §0):
//  * The broker NEVER receives, stores, derives, or returns plaintext.
//  * The broker NEVER holds the VaultSealKey. It moves only
//    `value_sealed` (age v1 ciphertext, base64url) + `client_hmac_hex`
//    + non-secret metadata, plus the opaque age-wrapped seal-key envelope
//    (courier only).
//  * There is NO ENV_VAULT_KEY usage, NO deriveUserKey, NO
//    crypto.subtle.decrypt anywhere in this module or its db helpers.
//    These routes import nothing from lib/crypto.ts on purpose.
//  * `client_hmac_hex` is stored OPAQUELY and returned VERBATIM. The
//    broker has no key, so it CANNOT cryptographically verify the MAC.
//    `400 hmac_mismatch` is reserved for STRUCTURAL validation only
//    (wrong length / non-hex).
//
//  This module replaces the legacy server-decrypt path
//  (GET /api/me/settings/raw + deriveUserKey + decryptForUser). See:
//    docs/integration/2026-05-29-spec15-vault-verification.md
//
//  MIGRATION TODO (out of this file's scope — server-routes piece only):
//    Retire getSettingsRaw / decryptForUser / encryptForUser / ENV_VAULT_KEY
//    once the client write path (SPEC-15 §8.B) ships and rows are re-sealed.
// =====================================================================

import type { Context } from "hono";
import type { Env } from "../types";
import { authn } from "./api";
import {
  setVaultItems, getVaultItem, listVaultItems,
  scheduleWipe, getWipeJob,
  storeKeyWrap, getKeyWrapForDevice,
} from "../lib/db";

const HEX_PUBKEY_RE = /^[0-9a-f]{64}$/i;

/* ── POST /vault/set — store sealed items VERBATIM ────────────────────── */
//
// Body (SPEC-15 §7, snake_case, batch wrapper):
//   { "items": [ { service, key, value_sealed, ts_ms, client_hmac_hex } ] }
//
// The broker stores ciphertext + opaque HMAC; it never decodes the age blob.
export async function vaultSet(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "jwt_invalid" }, 401);

  const body = await c.req.json().catch(() => null) as { items?: unknown } | null;
  const rawItems = body && Array.isArray(body.items) ? body.items : null;
  if (!rawItems) return c.json({ error: "missing_items" }, 400);

  // Map to the helper's input shape; validation (incl. structural hmac
  // check) happens inside setVaultItems. We do NOT touch value_sealed bytes.
  const items = rawItems.map((it) => {
    const o = (it && typeof it === "object") ? it as Record<string, unknown> : {};
    return {
      service: o.service as string,
      key: o.key as string,
      value_sealed: o.value_sealed as string,
      ts_ms: o.ts_ms as number,
      client_hmac_hex: o.client_hmac_hex as string,
    };
  });

  const { stored, rejected, wrote_at_ms } = await setVaultItems(c.env, id.userId, items);
  return c.json({ stored, rejected, wrote_at_ms });
}

/* ── GET /vault/get — return sealed ciphertext VERBATIM ───────────────── */
//
// Query: ?service=<s>&key=<k>  → single item.
//        (both omitted)        → metadata list (no value_sealed).
//        (exactly one given)   → 400 missing_query_pair.
export async function vaultGet(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "jwt_invalid" }, 401);

  const service = c.req.query("service");
  const key = c.req.query("key");

  if (service === undefined && key === undefined) {
    const items = await listVaultItems(c.env, id.userId);
    return c.json({ items });
  }
  if (service === undefined || key === undefined) {
    return c.json({ error: "missing_query_pair" }, 400);
  }

  const row = await getVaultItem(c.env, id.userId, service, key);
  if (!row) return c.json({ error: "vault_item_not_found" }, 404);

  // `server_hmac_hex` is the server-stored ECHO of the client's uploaded
  // HMAC — NOT a server-computed MAC (the broker has no key). The client
  // re-derives its own HMAC from the unsealed payload to detect tampering.
  return c.json({
    service: row.service,
    key: row.key,
    value_sealed: row.value_sealed,
    ts_ms: row.ts_ms,
    server_hmac_hex: row.client_hmac_hex,
  });
}

/* ── DELETE /vault/wipe — schedule a destructive wipe ─────────────────── */
//
// Header: X-Confirm-Wipe: <user_id> (must equal JWT sub).
// Body:   { scope: "vault" | "all", reason?: string }
export async function vaultWipe(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "jwt_invalid" }, 401);

  // X-Confirm-Wipe must equal the authenticated user_id (JWT sub). The
  // JWT sub is a numeric user id in this broker.
  const confirm = c.req.header("X-Confirm-Wipe") ?? "";
  if (confirm !== String(id.userId)) {
    return c.json({ error: "confirm_header_mismatch" }, 400);
  }

  const body = await c.req.json().catch(() => ({} as { scope?: unknown; reason?: unknown }));
  const scope = (body && (body as { scope?: unknown }).scope === "all") ? "all" : "vault";
  const reason = (body && typeof (body as { reason?: unknown }).reason === "string")
    ? (body as { reason: string }).reason
    : null;

  const { job, existed } = await scheduleWipe(c.env, id.userId, scope, reason);
  if (existed) {
    return c.json({ error: "wipe_already_in_progress", wipe_id: job.wipe_id }, 409);
  }
  return c.json({
    wipe_id: job.wipe_id,
    scheduled_at_ms: job.scheduled_at_ms,
    complete_by_ms: job.complete_by_ms,
    scope: job.scope,
  }, 202);
}

/* ── GET /vault/wipe/:wipe_id — poll wipe status ──────────────────────── */
export async function vaultWipeStatus(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "jwt_invalid" }, 401);

  const wipe_id = c.req.param("wipe_id") ?? "";
  const found = await getWipeJob(c.env, wipe_id);
  if (!found) return c.json({ error: "wipe_not_found" }, 404);
  if (found.user_id !== id.userId) return c.json({ error: "wipe_not_owned" }, 403);

  const j = found.job;
  return c.json({
    wipe_id: j.wipe_id,
    scheduled_at_ms: j.scheduled_at_ms,
    complete_by_ms: j.complete_by_ms,
    status: j.status,
    scope: j.scope,
    completed_at_ms: j.completed_at_ms,
  });
}

/* ── POST /vault/keys/wrap — store an age-wrapped seal key (courier) ───── */
//
// Body: { target_device_pubkey_hex, wrapped_vault_seal_key, key_version,
//         wrapped_by_device_hint? }
// The broker stores the opaque ciphertext; it can NOT unwrap it.
export async function vaultKeysWrap(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "jwt_invalid" }, 401);

  const body = await c.req.json().catch(() => ({} as Record<string, unknown>));
  const target = (body as { target_device_pubkey_hex?: unknown }).target_device_pubkey_hex;
  const wrapped = (body as { wrapped_vault_seal_key?: unknown }).wrapped_vault_seal_key;
  const keyVersionRaw = (body as { key_version?: unknown }).key_version;
  const hint = (body as { wrapped_by_device_hint?: unknown }).wrapped_by_device_hint;

  if (typeof target !== "string" || !HEX_PUBKEY_RE.test(target)
    || typeof wrapped !== "string" || wrapped.length === 0
    || typeof keyVersionRaw !== "number" || !Number.isFinite(keyVersionRaw) || keyVersionRaw < 0) {
    return c.json({ error: "invalid_wrap_format" }, 400);
  }

  const { row, existed } = await storeKeyWrap(c.env, id.userId, {
    target_device_pubkey_hex: target.toLowerCase(),
    wrapped_vault_seal_key: wrapped,
    key_version: Math.floor(keyVersionRaw),
    wrapped_by_device_hint: typeof hint === "string" ? hint : null,
  });
  if (existed) {
    return c.json({ error: "wrap_exists", wrap_id: row.wrap_id }, 409);
  }
  return c.json({
    wrap_id: row.wrap_id,
    stored_at_ms: row.stored_at_ms,
    expires_at_ms: row.expires_at_ms,
  }, 201);
}

/* ── GET /vault/keys/wrapped — new device pulls its wrap ──────────────── */
//
// Query: ?target_device_pubkey_hex=<hex>  (this device's own pubkey).
// Returns the opaque wrapped seal key for local unwrap.
export async function vaultKeysWrapped(c: Context<{ Bindings: Env }>) {
  const id = await authn(c);
  if (!id) return c.json({ error: "jwt_invalid" }, 401);

  const target = c.req.query("target_device_pubkey_hex") ?? "";
  if (!HEX_PUBKEY_RE.test(target)) {
    return c.json({ error: "invalid_wrap_format" }, 400);
  }

  const row = await getKeyWrapForDevice(c.env, id.userId, target.toLowerCase());
  if (!row) return c.json({ error: "wrap_not_found" }, 404);

  return c.json({
    wrap_id: row.wrap_id,
    wrapped_vault_seal_key: row.wrapped_vault_seal_key,
    key_version: row.key_version,
    wrapped_by_device_hint: row.wrapped_by_device_hint,
  });
}
