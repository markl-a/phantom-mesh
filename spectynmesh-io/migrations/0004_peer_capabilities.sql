-- 0004_peer_capabilities.sql — tag each peer with capability strings.
--
-- Why: Phase 2 capability-based dispatch needs to filter peers by tag
-- (e.g. "rust+build" → only nodes that have BOTH rust + build in their
-- caps array). Storing as JSON array in TEXT keeps the schema simple +
-- works without D1 array support; CLI side parses JSON.
--
-- Default '[]' means "no caps declared yet" — caller can still --to
-- that peer explicitly, just no auto-routing match.
--
-- Apply: wrangler d1 execute phantommesh-prod --remote --file=./migrations/0004_peer_capabilities.sql

ALTER TABLE user_cluster_peers ADD COLUMN capabilities TEXT NOT NULL DEFAULT '[]';
