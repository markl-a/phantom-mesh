-- 0009_user_preferences.sql — per-user dashboard preferences.
--
-- Why: F204 (settings screen) exposes a few dashboard-scoped knobs that
-- aren't LLM keys: how often the TUI heartbeats and how many days of
-- dispatch history we retain. The retention sweeper (F205 cron, future)
-- reads `retention_days` from here to decide what to purge.
--
-- One row per user; absence = use defaults baked into the worker
-- (heartbeat_secs=30, retention_days=90).
--
-- Apply: wrangler d1 execute phantommesh-prod --remote --file=./migrations/0009_user_preferences.sql

CREATE TABLE IF NOT EXISTS user_preferences (
    user_id        INTEGER NOT NULL PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    heartbeat_secs INTEGER NOT NULL DEFAULT 30,     -- TUI presence cadence (10..600 enforced server-side)
    retention_days INTEGER NOT NULL DEFAULT 90,     -- dispatch history retention (7..365 enforced)
    updated_at     INTEGER NOT NULL
);
