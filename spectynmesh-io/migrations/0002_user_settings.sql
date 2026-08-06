-- 0002_user_settings.sql — store per-user LLM provider API keys so a
-- fresh spectyn install on any of the user's machines can `spectyn config
-- pull` and have keys appear, instead of [Environment]::SetEnvironmentVariable
-- on each box one by one.
--
-- env_json is a JSON object: {"OPENCODE_API_KEY": "...", "GROQ_API_KEY": "..."}.
-- Stored verbatim; the API layer enforces a key allowlist on write so users
-- can't stuff arbitrary process env into here. Values are NOT encrypted at
-- rest in v1 — D1 is on Cloudflare's storage, the broker JWT secret rotates
-- access. Add column-level encryption later if/when threat model warrants.
--
-- Apply via: wrangler d1 execute spectynmesh-prod --remote --file=./migrations/0002_user_settings.sql

CREATE TABLE IF NOT EXISTS user_settings (
    user_id    INTEGER PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    env_json   TEXT NOT NULL DEFAULT '{}',
    updated_at INTEGER NOT NULL
);
