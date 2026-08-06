-- phantommesh.io initial D1 schema.
-- Apply via: wrangler d1 execute spectynmesh-prod --file=./migrations/0001_init.sql

CREATE TABLE IF NOT EXISTS users (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    email         TEXT NOT NULL UNIQUE,
    provider      TEXT NOT NULL,          -- 'google' | 'apple' | 'email'
    sub           TEXT,                   -- IdP subject (Google sub / Apple sub)
    display_name  TEXT,
    avatar_url    TEXT,
    password_hash TEXT,                   -- bcrypt-style; only for provider='email'
    created_at    INTEGER NOT NULL,
    last_login_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS devices (
    device_id     TEXT PRIMARY KEY,        -- the spectyn CLI's uuid v4
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label         TEXT,                    -- hostname / mac model / etc.
    public_addr   TEXT,                    -- last-seen tailscale IP, optional
    claimed_at    INTEGER NOT NULL,
    last_seen_at  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_devices_user ON devices(user_id);

CREATE TABLE IF NOT EXISTS broker_tokens (
    token_hash    TEXT PRIMARY KEY,        -- SHA-256 of the actual JWT
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_id     TEXT,
    issued_at     INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    revoked_at    INTEGER
);

CREATE INDEX IF NOT EXISTS idx_tokens_user ON broker_tokens(user_id);
CREATE INDEX IF NOT EXISTS idx_tokens_expiry ON broker_tokens(expires_at);

-- oauth_sessions live in KV (TTL'd at 5 min); not in D1.
