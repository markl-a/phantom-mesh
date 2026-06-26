-- 0010_hermes_fts_index_form.sql — P0-8: FTS index fed from Rust, not raw text.
--
-- When PHANTOM_ENCRYPT_MEMORY is ON, hermes_memory.text holds a sealed age blob
-- (see core/src/hermes/memory_seal.rs). The FTS5 index must NOT index that
-- ciphertext (BM25 over base64 noise is useless) and must NOT index the raw PII
-- sentence (that would re-leak the plaintext into the index pages). It must
-- instead hold a de-PII'd token form computed in Rust
-- (memory_seal::fts_index_form).
--
-- SQLite triggers cannot call Rust, so the AFTER INSERT / AFTER UPDATE triggers
-- that auto-copied `new.text` into the FTS index are retired here, and
-- HermesMemory::insert now writes the FTS row explicitly (with the index form
-- when sealing is ON, or the verbatim text when OFF — byte-identical to the old
-- trigger behaviour on the default-OFF ship path).
--
-- The AFTER DELETE trigger (hermes_memory_ad) must ALSO be retired: for an
-- external-content FTS5 table, the 'delete' command must be given the SAME
-- column values that were originally indexed, or FTS5 reports "database disk
-- image is malformed". When sealing is ON, hermes_memory.text holds a sealed
-- blob while the FTS index holds the de-PII'd token form, so old.text no longer
-- matches the indexed content. HermesMemory::delete_by_id now purges the FTS row
-- explicitly in Rust (recomputing the index form), so the trigger is dropped to
-- avoid the double-purge / mismatch.
--
-- Safe to run on an existing DB: DROP TRIGGER IF EXISTS is idempotent, and a DB
-- created before this migration simply loses the auto-sync triggers (the Rust
-- insert/delete paths take over). No data is rewritten.

DROP TRIGGER IF EXISTS hermes_memory_ai;
DROP TRIGGER IF EXISTS hermes_memory_au;
DROP TRIGGER IF EXISTS hermes_memory_ad;
