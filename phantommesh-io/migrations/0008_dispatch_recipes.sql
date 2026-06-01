-- 0008_dispatch_recipes.sql — per-user saved dispatch templates.
--
-- Why: F202 (dispatch screen) lets users save a {peer, provider, model,
-- prompt, required_caps} bundle under a name and reload it later. Recipes
-- live here so they survive across browsers + machines for the same user.
--
-- Not encrypted: prompts can be sensitive but the user is already trusting
-- the worker with their broker_token + LLM API keys (in user_settings,
-- which IS encrypted). Recipe prompts are operational config of the same
-- trust class as cluster_peers (also plaintext).
--
-- Apply: wrangler d1 execute phantommesh-prod --remote --file=./migrations/0008_dispatch_recipes.sql

CREATE TABLE IF NOT EXISTS dispatch_recipes (
    id            TEXT    NOT NULL,                  -- uuid generated client-side OR by worker on POST
    user_id       INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name          TEXT    NOT NULL,                  -- user-chosen label, unique per user
    peer          TEXT    NOT NULL DEFAULT '',       -- optional peer name (empty = "ask at dispatch time")
    provider      TEXT    NOT NULL DEFAULT '',
    model         TEXT    NOT NULL DEFAULT '',
    prompt        TEXT    NOT NULL DEFAULT '',
    required_caps TEXT    NOT NULL DEFAULT '[]',     -- JSON array of cap strings
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    PRIMARY KEY (user_id, id)
);

CREATE INDEX IF NOT EXISTS idx_recipes_user
    ON dispatch_recipes(user_id, updated_at DESC);

-- Unique name per user so recipes can be looked up by name from the CLI.
CREATE UNIQUE INDEX IF NOT EXISTS uq_recipes_user_name
    ON dispatch_recipes(user_id, name);
