-- 0003_cluster_peers.sql — per-user mesh peer registry.
--
-- Replaces the hardcoded CLUSTER_TOPOLOGY constant in core/src/cli_config.rs
-- with a vault-backed list. Adding a new machine = a row here (via
-- /account UI) instead of a code change + redeploy. Other machines pull
-- the list via `phantom config pull` and rebuild [cluster].peers from it.
--
-- Not encrypted at rest — URLs + names aren't secret. The cross-node auth
-- secret (CLUSTER_SECRET) IS encrypted via the env_json AES-GCM path.
--
-- Apply: wrangler d1 execute phantommesh-prod --remote --file=./migrations/0003_cluster_peers.sql

CREATE TABLE IF NOT EXISTS user_cluster_peers (
    user_id    INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name       TEXT    NOT NULL,             -- node identity, e.g. "ayaneo"
    url        TEXT    NOT NULL,             -- Tailscale URL, e.g. "http://100.107.205.98:7878"
    label      TEXT,                         -- optional human description
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (user_id, name)
);

CREATE INDEX IF NOT EXISTS idx_cluster_peers_user ON user_cluster_peers(user_id);
