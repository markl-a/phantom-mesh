-- 0006_user_identities.sql — track every (provider, sub) pair a user has
-- signed in with, instead of just the latest one in users.provider /
-- users.sub. The single-column approach overwrote the row on every
-- login: a user who started with Email + password and later linked
-- Google would show provider='google' on /account and lose the trace
-- that Email auth still works. Now /account can render "Sign-in
-- methods: Google, Email" and the audit trail survives each login.
--
-- email is still the unique key on users (so all sign-in paths still
-- map to the same row); user_identities just augments it.
--
-- PK is (user_id, provider) because a user can have at most one row
-- per provider — re-logins UPDATE last_used_ms but don't insert.

CREATE TABLE IF NOT EXISTS user_identities (
    user_id          INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT    NOT NULL,    -- 'google' | 'email' | future
    sub              TEXT,                -- IdP subject (Google sub); NULL for email
    first_linked_ms  INTEGER NOT NULL,
    last_used_ms     INTEGER NOT NULL,
    PRIMARY KEY (user_id, provider)
);

CREATE INDEX IF NOT EXISTS idx_user_identities_user ON user_identities(user_id);

-- Backfill: every existing row in users implies one identity, with
-- timestamps from the user row itself. Subsequent logins on those
-- identities will update last_used_ms.
INSERT OR IGNORE INTO user_identities (user_id, provider, sub, first_linked_ms, last_used_ms)
SELECT id, provider, sub, created_at, last_login_at
FROM users;
