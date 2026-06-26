-- 0009_skill_embeddings.sql — Hermes ②memory embedding column (SPEC-25 §8.4)
--
-- Adds a nullable `embedding BLOB` column to the `skills` row table so the
-- recall step can persist a per-skill semantic vector (little-endian f32 LE
-- bytes) alongside the FTS5-searchable text. NULL = "no embedding stored"
-- (current behavior); the recall embedding leg already degrades to FTS5-only
-- when the vector is absent, so an unfilled column changes nothing.
--
-- IDEMPOTENCY: SQLite has no `ADD COLUMN IF NOT EXISTS`, and this schema is
-- re-applied on every `store_skill` write (via execute_batch, mirroring how
-- 0008 self-provisions). A bare `ALTER TABLE ... ADD COLUMN` would abort the
-- batch with "duplicate column name" on the second open. The Rust store path
-- therefore applies this ALTER through a tolerant runner
-- (`apply_embedding_column`) that first probes `PRAGMA table_info(skills)` and
-- skips the ALTER when the column already exists — so re-applying is a no-op.
-- The 0008 base table is created first (its own `IF NOT EXISTS` is idempotent)
-- so this file is also safe to apply on a brand-new DB.

ALTER TABLE skills ADD COLUMN embedding BLOB;
