-- 0008_hermes_skills.sql — Hermes ②memory skill store (SPEC-25)
--
-- The canonical `skills` row store + the `skills_fts` external-content FTS5 index
-- that `skill_wire::fts5_search` queries (it JOINs skills_fts → skills on rowid
-- and MATCHes name/trigger_pattern). Until this landed, fts5_search hit a
-- missing table and returned an empty set; with it, store→recall works.
--
-- Mirrors the 0007_hermes_fts5.sql external-content layout (FTS5 §4.4.3):
-- triggers keep skills_fts in sync so callers just INSERT into `skills`.
-- Idempotent (IF NOT EXISTS) — safe to re-apply on every open.

CREATE TABLE IF NOT EXISTS skills (
    id                 TEXT    PRIMARY KEY,
    name               TEXT    NOT NULL,
    trigger_pattern    TEXT    NOT NULL,
    steps_json         TEXT    NOT NULL DEFAULT '[]',
    examples_json      TEXT    NOT NULL DEFAULT '[]',
    version            INTEGER NOT NULL DEFAULT 1,
    quality_score      REAL    NOT NULL DEFAULT 0.5,
    last_applied_at    INTEGER NOT NULL DEFAULT 0,
    source_event_count INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS skills_quality ON skills(quality_score DESC);

-- External-content FTS5 mirror over the searchable columns (name + trigger_pattern);
-- `content='skills'` points the index at the row table so the original text is
-- recoverable, while only name/trigger_pattern are tokenized for MATCH.
CREATE VIRTUAL TABLE IF NOT EXISTS skills_fts
USING fts5(
    name,
    trigger_pattern,
    content='skills',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

-- Keep the FTS index in sync. The 'delete' rows pass the exact previously
-- indexed content so FTS5 external-content deletes resolve correctly.
CREATE TRIGGER IF NOT EXISTS skills_ai
AFTER INSERT ON skills BEGIN
    INSERT INTO skills_fts(rowid, name, trigger_pattern)
    VALUES (new.rowid, new.name, new.trigger_pattern);
END;

CREATE TRIGGER IF NOT EXISTS skills_ad
AFTER DELETE ON skills BEGIN
    INSERT INTO skills_fts(skills_fts, rowid, name, trigger_pattern)
    VALUES ('delete', old.rowid, old.name, old.trigger_pattern);
END;

CREATE TRIGGER IF NOT EXISTS skills_au
AFTER UPDATE ON skills BEGIN
    INSERT INTO skills_fts(skills_fts, rowid, name, trigger_pattern)
    VALUES ('delete', old.rowid, old.name, old.trigger_pattern);
    INSERT INTO skills_fts(rowid, name, trigger_pattern)
    VALUES (new.rowid, new.name, new.trigger_pattern);
END;
