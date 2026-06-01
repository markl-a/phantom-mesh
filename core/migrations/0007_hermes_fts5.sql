-- 0007_hermes_fts5.sql — Hermes long-term memory backend
--
-- Two-table layout per FTS5 §4.4.3 "External content tables":
--   hermes_memory: canonical row store (one row per remembered fact)
--   hermes_memory_fts: contentless FTS5 index over (kind, text, tags)
--
-- The FTS table is kept in sync via triggers so callers can either INSERT
-- into hermes_memory directly or use the helpers in core::hermes::memory.

CREATE TABLE IF NOT EXISTS hermes_memory (
    id           INTEGER PRIMARY KEY,
    created_at   INTEGER NOT NULL,        -- unix seconds (UTC)
    kind         TEXT    NOT NULL,        -- e.g. 'fact', 'lesson', 'observation'
    source       TEXT    NOT NULL,        -- agent or skill that wrote this
    text         TEXT    NOT NULL,        -- the memory body itself
    tags         TEXT    NOT NULL DEFAULT '' -- space-separated tag tokens
);

CREATE INDEX IF NOT EXISTS hermes_memory_kind_created
    ON hermes_memory(kind, created_at DESC);

CREATE VIRTUAL TABLE IF NOT EXISTS hermes_memory_fts
USING fts5(
    kind,
    text,
    tags,
    content='hermes_memory',
    content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);

-- Trigger: keep FTS index in sync after row inserts
CREATE TRIGGER IF NOT EXISTS hermes_memory_ai
AFTER INSERT ON hermes_memory BEGIN
    INSERT INTO hermes_memory_fts(rowid, kind, text, tags)
    VALUES (new.id, new.kind, new.text, new.tags);
END;

-- Trigger: keep FTS index in sync after row deletes
CREATE TRIGGER IF NOT EXISTS hermes_memory_ad
AFTER DELETE ON hermes_memory BEGIN
    INSERT INTO hermes_memory_fts(hermes_memory_fts, rowid, kind, text, tags)
    VALUES ('delete', old.id, old.kind, old.text, old.tags);
END;

-- Trigger: keep FTS index in sync after row updates
CREATE TRIGGER IF NOT EXISTS hermes_memory_au
AFTER UPDATE ON hermes_memory BEGIN
    INSERT INTO hermes_memory_fts(hermes_memory_fts, rowid, kind, text, tags)
    VALUES ('delete', old.id, old.kind, old.text, old.tags);
    INSERT INTO hermes_memory_fts(rowid, kind, text, tags)
    VALUES (new.id, new.kind, new.text, new.tags);
END;
