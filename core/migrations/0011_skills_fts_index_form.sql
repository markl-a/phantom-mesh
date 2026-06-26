-- 0011_skills_fts_index_form.sql — P0-2: skills FTS index fed from Rust, not raw text.
--
-- This is the `skills`-table analogue of 0010_hermes_fts_index_form.sql (which
-- did the same for hermes_memory). When PHANTOM_ENCRYPT_MEMORY is ON, the
-- searchable `skills.name`/`skills.trigger_pattern` columns hold a sealed age
-- blob (see core/src/hermes/memory_seal.rs). The skills_fts external-content
-- FTS5 index must NOT index that ciphertext (BM25 over base64 noise is useless)
-- and must NOT index the raw PII text (that would re-leak the plaintext into the
-- index pages). It must instead hold a de-PII'd token form computed in Rust
-- (memory_seal::fts_index_form).
--
-- SQLite triggers cannot call Rust, so the AFTER INSERT / AFTER UPDATE triggers
-- (skills_ai / skills_au) that auto-copied new.name/new.trigger_pattern into the
-- FTS index are retired here, and store_skill_with_embedding now writes the FTS
-- row explicitly (with the index form when sealing is ON, or the verbatim text
-- when OFF — byte-identical to the old trigger behaviour on the default-OFF ship
-- path).
--
-- The AFTER DELETE trigger (skills_ad) must ALSO be retired: for an
-- external-content FTS5 table, the 'delete' command must be given the SAME
-- column values that were originally indexed, or FTS5 reports "database disk
-- image is malformed". When sealing is ON, skills.name/trigger_pattern hold a
-- sealed blob while the FTS index holds the de-PII'd token form, so
-- old.name/old.trigger_pattern no longer match the indexed content. The Rust
-- write path now purges + re-inserts the FTS row explicitly (recomputing the
-- index form), so the update/delete triggers are dropped to avoid the
-- double-purge / mismatch.
--
-- Safe to run on an existing DB: DROP TRIGGER IF EXISTS is idempotent, and a DB
-- created before this migration simply loses the auto-sync triggers (the Rust
-- insert path takes over). No data is rewritten.

DROP TRIGGER IF EXISTS skills_ai;
DROP TRIGGER IF EXISTS skills_au;
DROP TRIGGER IF EXISTS skills_ad;
