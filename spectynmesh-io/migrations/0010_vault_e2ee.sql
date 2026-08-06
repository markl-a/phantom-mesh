-- 0010_vault_e2ee.sql — SPEC-15 E2EE vault dumb-storage tables.
--
-- DRAFT (SPEC-15 E2EE). These tables back the new /vault/* routes that
-- store CLIENT-SEALED ciphertext VERBATIM. The broker holds NO key and
-- can NOT decrypt — this is the deliberate replacement for the legacy
-- at-rest path (user_settings.env_json + ENV_VAULT_KEY + deriveUserKey),
-- which let the Worker decrypt. See:
--   docs/integration/2026-05-29-spec15-vault-verification.md
--
-- INVARIANTS (enforced by the route layer, not the schema):
--   * value_sealed       = base64url(no-pad) of an age v1 ciphertext.
--                          The broker never decodes/decrypts it.
--   * client_hmac_hex     = 64-char lower-hex HMAC computed CLIENT-side
--                          with the VaultSealKey. The broker has no key,
--                          so it stores this opaquely and returns it
--                          verbatim. It CANNOT verify the MAC.
--   * NO plaintext column exists. There is intentionally no value_clear.
--
-- Apply via: wrangler d1 execute spectynmesh-prod --remote --file=./migrations/0010_vault_e2ee.sql

-- ── Sealed vault items (POST /vault/set, GET /vault/get) ───────────────
CREATE TABLE IF NOT EXISTS vault_items (
    user_id         INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    service         TEXT    NOT NULL,   -- slug, no newline (e.g. "cerebras")
    key             TEXT    NOT NULL,   -- slug under service (e.g. "default")
    value_sealed    TEXT    NOT NULL,   -- base64url age v1 ciphertext, stored verbatim
    client_hmac_hex TEXT    NOT NULL,   -- client-computed HMAC, opaque to broker
    ts_ms           INTEGER NOT NULL,   -- client wall-clock write intent (LWW tiebreaker)
    wrote_at_ms     INTEGER NOT NULL,   -- server write time
    PRIMARY KEY (user_id, service, key)
);

CREATE INDEX IF NOT EXISTS idx_vault_items_user
    ON vault_items(user_id, service, key);

-- ── Wipe jobs (DELETE /vault/wipe, GET /vault/wipe/{wipe_id}) ──────────
CREATE TABLE IF NOT EXISTS vault_wipe_jobs (
    wipe_id          TEXT    PRIMARY KEY,           -- e.g. "wipe_abc123"
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    scope            TEXT    NOT NULL,              -- "vault" | "all"
    reason           TEXT,                          -- optional free text
    status           TEXT    NOT NULL DEFAULT 'pending', -- pending|in_progress|completed|failed
    scheduled_at_ms  INTEGER NOT NULL,
    complete_by_ms   INTEGER NOT NULL,              -- scheduled_at_ms + 24h SLA
    completed_at_ms  INTEGER                        -- non-null only when status == completed
);

-- One in-flight wipe per user: lets DELETE /vault/wipe return 409 when a
-- pending/in_progress job already exists.
CREATE INDEX IF NOT EXISTS idx_vault_wipe_user_status
    ON vault_wipe_jobs(user_id, status);

-- ── Wrapped seal-key envelopes (POST /vault/keys/wrap, GET /vault/keys/wrapped) ─
-- The broker is a COURIER only: it stores the age-recipient-mode wrapped
-- VaultSealKey ciphertext targeted at a new device's pubkey, and hands it
-- back when that device pulls. It can NOT unwrap (no X25519 secret).
CREATE TABLE IF NOT EXISTS vault_key_wraps (
    wrap_id                   TEXT    PRIMARY KEY,    -- e.g. "wrap_xyz789"
    user_id                   INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    target_device_pubkey_hex  TEXT    NOT NULL,       -- new device's age recipient pubkey (hex)
    wrapped_vault_seal_key    TEXT    NOT NULL,       -- base64url age v1 ciphertext, verbatim
    key_version               INTEGER NOT NULL,       -- rotation pin
    wrapped_by_device_hint    TEXT,                   -- UI hint (source device label)
    stored_at_ms              INTEGER NOT NULL,
    expires_at_ms             INTEGER NOT NULL         -- stored_at_ms + 7d
);

CREATE INDEX IF NOT EXISTS idx_vault_key_wraps_target
    ON vault_key_wraps(user_id, target_device_pubkey_hex);
