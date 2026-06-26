//! Long-term skill/memory store backed by SQLite FTS5.
//!
//! See `core/migrations/0007_hermes_fts5.sql` for the canonical schema.
//! See spec §5 H3 for context.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use tokio::sync::Mutex;

/// Schema bootstrap text — included verbatim from the on-disk migrations so
/// there is exactly one source of truth.
///
/// 0007 creates the two tables + the FTS sync triggers. 0010 (P0-8) retires the
/// auto-insert/update FTS triggers so `insert` can feed the FTS index a de-PII'd
/// token form (`memory_seal::fts_index_form`) when `PHANTOM_ENCRYPT_MEMORY` is
/// ON; the delete trigger is left in place. Concatenated (not a real migration
/// runner) because `open_at` bootstraps via a single `execute_batch`.
const SCHEMA_SQL: &str = concat!(
    include_str!("../../migrations/0007_hermes_fts5.sql"),
    "\n",
    include_str!("../../migrations/0010_hermes_fts_index_form.sql"),
);

/// FTS5-backed long-term memory.
#[derive(Clone)]
pub struct SkillMemory {
    conn: Arc<Mutex<Connection>>,
}

/// Borrowed input for a new memory row. Caller never sees `id` or `created_at`
/// — those are assigned by the store.
#[derive(Debug, Clone, Copy)]
pub struct NewMemory<'a> {
    pub kind: &'a str,
    pub source: &'a str,
    pub text: &'a str,
    pub tags: &'a str,
}

/// A row read back from the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRow {
    pub id: i64,
    pub created_at: i64,
    pub kind: String,
    pub source: String,
    pub text: String,
    pub tags: String,
}

/// Wrap arbitrary user text as an FTS5 literal-phrase query.
///
/// FTS5's query syntax has its own operators (`AND`, `OR`, `NOT`, `NEAR`,
/// `*`, `:`, `(`, `)`) that are NOT neutralized by SQL parameter binding —
/// a search for the literal text `AND` would be parsed as the boolean
/// operator. Wrapping the input in double quotes turns the entire string
/// into a phrase token; per the FTS5 docs, embedded double-quotes inside a
/// quoted phrase are escaped by doubling them.
///
/// Use this for any query string that originates from outside the
/// application (HTTP body, user prompt, CLI argument). Callers that build
/// FTS5 expressions internally and want operator semantics should call
/// `search()` directly with the operator-bearing string.
pub fn escape_fts5_query(raw: &str) -> String {
    let escaped = raw.replace('"', "\"\"");
    format!("\"{escaped}\"")
}

impl SkillMemory {
    /// Open (or create) the store at the given path, applying the schema if
    /// the tables don't already exist (`CREATE TABLE IF NOT EXISTS`).
    pub fn open_at(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("open sqlite at {}", db_path.display()))?;
        conn.execute_batch(SCHEMA_SQL)
            .context("apply hermes_memory schema (0007_hermes_fts5.sql)")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Insert a new memory row. Returns the assigned rowid.
    ///
    /// Uses bound parameters — none of `kind/source/text/tags` reach SQL as
    /// concatenated strings, so this is safe against SQL injection.
    pub async fn insert(&self, m: NewMemory<'_>) -> Result<i64> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // P0-8: when PHANTOM_ENCRYPT_MEMORY is ON, seal text/source at rest and
        // feed the FTS index a de-PII'd token form (NOT the ciphertext, NOT the
        // raw sentence). Fail CLOSED — `seal()` returns Err(NoKey) if the flag is
        // on but no EventKey is loaded, so the row is never silently written in
        // plaintext. When OFF, every value is the verbatim input, so the row +
        // FTS feed are byte-identical to the pre-P0-8 trigger behaviour.
        let sealing = crate::skillbank::memory_seal::memory_e2ee_enabled();
        let (stored_text, stored_source, fts_text) = if sealing {
            (
                crate::skillbank::memory_seal::seal(m.text).context("seal hermes_memory.text")?,
                crate::skillbank::memory_seal::seal(m.source).context("seal hermes_memory.source")?,
                crate::skillbank::memory_seal::fts_index_form(m.text),
            )
        } else {
            (m.text.to_string(), m.source.to_string(), m.text.to_string())
        };

        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO hermes_memory (created_at, kind, source, text, tags)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![now, m.kind, stored_source, stored_text, m.tags],
        )
        .context("insert hermes_memory row")?;
        let rowid = conn.last_insert_rowid();

        // The 0007 auto-insert FTS trigger was retired by 0010 so the index can
        // receive `fts_index_form` (plaintext tokens) instead of `new.text` (now
        // possibly a sealed blob). Feed it explicitly. `source` is not an FTS
        // column, so it is unaffected.
        conn.execute(
            "INSERT INTO hermes_memory_fts(rowid, kind, text, tags)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![rowid, m.kind, fts_text, m.tags],
        )
        .context("insert hermes_memory_fts row")?;
        Ok(rowid)
    }

    /// Delete a row by rowid, purging the matching FTS index entry.
    ///
    /// The 0007 `AFTER DELETE` trigger was retired by 0010 because, for an
    /// external-content FTS5 table, the `'delete'` command must be given the
    /// EXACT values that were indexed — but when sealing is ON the canonical
    /// `text` column holds a sealed blob while the FTS index holds the de-PII'd
    /// token form, so a `old.text`-based trigger mismatches and FTS5 reports
    /// "database disk image is malformed". So we purge the FTS row in Rust,
    /// recomputing the index form to match exactly what `insert` wrote.
    pub async fn delete_by_id(&self, id: i64) -> Result<()> {
        let conn = self.conn.lock().await;

        // Recover the values needed to issue the FTS5 'delete' command. We read
        // the stored (possibly-sealed) text and recompute the SAME index form
        // `insert` used. A row with no FTS entry (e.g. id not found) is a no-op.
        let existing: Option<(String, String, String)> = conn
            .query_row(
                "SELECT kind, text, tags FROM hermes_memory WHERE id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .context("lookup hermes_memory row for delete")?;

        if let Some((kind, stored_text, tags)) = existing {
            // Recompute the indexed FTS text to match exactly what `insert`
            // wrote, so the FTS5 'delete' command does not corrupt the
            // external-content index. The decision keys off whether the STORED
            // value is sealed — NOT the current `PHANTOM_ENCRYPT_MEMORY` flag —
            // because the flag may have been toggled OFF after the row was
            // sealed (and an unsealed row may exist while the flag is ON). If the
            // stored text is a sealed blob it was indexed as `fts_index_form`, so
            // decrypt then recompute that form; otherwise it was indexed verbatim.
            let fts_text = if crate::skillbank::memory_seal::is_sealed(&stored_text) {
                let plain = crate::skillbank::memory_seal::open(&stored_text)
                    .map_err(|e| anyhow::anyhow!("decrypt for delete: {e}"))?;
                crate::skillbank::memory_seal::fts_index_form(&plain)
            } else {
                stored_text
            };
            conn.execute(
                "INSERT INTO hermes_memory_fts(hermes_memory_fts, rowid, kind, text, tags)
                 VALUES ('delete', ?1, ?2, ?3, ?4)",
                rusqlite::params![id, kind, fts_text, tags],
            )
            .context("purge hermes_memory_fts row")?;
        }

        conn.execute(
            "DELETE FROM hermes_memory WHERE id = ?1",
            rusqlite::params![id],
        )
        .context("delete hermes_memory row")?;
        Ok(())
    }

    /// Open the possibly-sealed `text`/`source` columns of a freshly-read row.
    ///
    /// Fail CLOSED: a value that probes as sealed (age-magic after base64) but
    /// won't decrypt returns `Err` — the caller surfaces a read error rather
    /// than leaking ciphertext as if it were plaintext. A plaintext value
    /// (flag-off / legacy / free-form pre-flip row) passes through unchanged, so
    /// OFF-path reads are byte-identical to before. The error text carries
    /// neither plaintext nor ciphertext.
    fn open_row(mut row: MemoryRow) -> Result<MemoryRow> {
        row.text = crate::skillbank::memory_seal::open(&row.text)
            .map_err(|e| anyhow::anyhow!("hermes_memory.text decrypt failed: {e}"))?;
        row.source = crate::skillbank::memory_seal::open(&row.source)
            .map_err(|e| anyhow::anyhow!("hermes_memory.source decrypt failed: {e}"))?;
        Ok(row)
    }

    /// Fetch a single row by rowid. Returns `Ok(None)` if not found.
    pub async fn get_by_id(&self, id: i64) -> Result<Option<MemoryRow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT id, created_at, kind, source, text, tags
             FROM hermes_memory WHERE id = ?1",
        )?;
        let mut rows = stmt.query(rusqlite::params![id])?;
        if let Some(row) = rows.next()? {
            let raw = MemoryRow {
                id: row.get(0)?,
                created_at: row.get(1)?,
                kind: row.get(2)?,
                source: row.get(3)?,
                text: row.get(4)?,
                tags: row.get(5)?,
            };
            Ok(Some(Self::open_row(raw)?))
        } else {
            Ok(None)
        }
    }

    /// F400 helper: paginated list of rows of a given `kind`, ordered by
    /// `created_at DESC` (newest first). Returns `(rows, total_count)`.
    ///
    /// `offset/limit` are clamped to `usize` and applied at the SQL layer so
    /// large skill banks don't materialize fully in memory. `total` ignores
    /// the limit so the UI can render "N of M" labels.
    pub async fn list_by_kind(
        &self,
        kind: &str,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<MemoryRow>, usize)> {
        let conn = self.conn.lock().await;

        let total: i64 = conn
            .prepare_cached("SELECT COUNT(*) FROM hermes_memory WHERE kind = ?1")?
            .query_row(rusqlite::params![kind], |r| r.get(0))?;

        let mut stmt = conn.prepare_cached(
            "SELECT id, created_at, kind, source, text, tags
             FROM hermes_memory
             WHERE kind = ?1
             ORDER BY created_at DESC, id DESC
             LIMIT ?2 OFFSET ?3",
        )?;
        let raw: Vec<MemoryRow> = stmt
            .query_map(
                rusqlite::params![kind, limit as i64, offset as i64],
                |row| {
                    Ok(MemoryRow {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        kind: row.get(2)?,
                        source: row.get(3)?,
                        text: row.get(4)?,
                        tags: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        let rows = raw
            .into_iter()
            .map(Self::open_row)
            .collect::<Result<Vec<_>>>()?;
        Ok((rows, total as usize))
    }

    /// F400 helper: FTS5 search restricted to a `kind` (post-filter), with
    /// pagination. Returns `(rows, total_matches)`. `query` is an FTS5
    /// expression — caller is responsible for any escaping (see
    /// [`escape_fts5_query`]).
    pub async fn search_by_kind_paginated(
        &self,
        kind: &str,
        query: &str,
        limit: usize,
        offset: usize,
    ) -> Result<(Vec<MemoryRow>, usize)> {
        let conn = self.conn.lock().await;

        let total: i64 = conn
            .prepare_cached(
                "SELECT COUNT(*)
                 FROM hermes_memory_fts f
                 JOIN hermes_memory m ON m.id = f.rowid
                 WHERE hermes_memory_fts MATCH ?1 AND m.kind = ?2",
            )?
            .query_row(rusqlite::params![query, kind], |r| r.get(0))?;

        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.created_at, m.kind, m.source, m.text, m.tags
             FROM hermes_memory_fts f
             JOIN hermes_memory m ON m.id = f.rowid
             WHERE hermes_memory_fts MATCH ?1 AND m.kind = ?2
             ORDER BY bm25(hermes_memory_fts)
             LIMIT ?3 OFFSET ?4",
        )?;
        let raw: Vec<MemoryRow> = stmt
            .query_map(
                rusqlite::params![query, kind, limit as i64, offset as i64],
                |row| {
                    Ok(MemoryRow {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        kind: row.get(2)?,
                        source: row.get(3)?,
                        text: row.get(4)?,
                        tags: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        let rows = raw
            .into_iter()
            .map(Self::open_row)
            .collect::<Result<Vec<_>>>()?;
        Ok((rows, total as usize))
    }

    /// F400 helper: chronological list of rows of a given `kind` with
    /// `created_at >= since_secs`. Ordered oldest-first (true timeline shape)
    /// and capped at `cap` rows so a long-lived DB can't blow up the
    /// response. `cap=0` is treated as "use a sane default" (1024).
    pub async fn list_since(
        &self,
        kind: &str,
        since_secs: i64,
        cap: usize,
    ) -> Result<Vec<MemoryRow>> {
        let effective_cap = if cap == 0 { 1024 } else { cap };
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT id, created_at, kind, source, text, tags
             FROM hermes_memory
             WHERE kind = ?1 AND created_at >= ?2
             ORDER BY created_at ASC, id ASC
             LIMIT ?3",
        )?;
        let raw: Vec<MemoryRow> = stmt
            .query_map(
                rusqlite::params![kind, since_secs, effective_cap as i64],
                |row| {
                    Ok(MemoryRow {
                        id: row.get(0)?,
                        created_at: row.get(1)?,
                        kind: row.get(2)?,
                        source: row.get(3)?,
                        text: row.get(4)?,
                        tags: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<_>>()?;
        let rows = raw
            .into_iter()
            .map(Self::open_row)
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Full-text search over the memory body via FTS5 `MATCH`.
    ///
    /// `query` is passed as a bound parameter — never string-interpolated —
    /// and is expected to use FTS5 query syntax (tokens, AND/OR/NOT,
    /// quoted phrases). Callers feeding raw user text should pre-process
    /// with [`escape_fts5_query`] to neutralize syntax characters.
    ///
    /// Returns rows ordered by FTS5's default `bm25()` rank (best first),
    /// capped at `limit`.
    pub async fn search(&self, query: &str, limit: usize) -> Result<Vec<MemoryRow>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT m.id, m.created_at, m.kind, m.source, m.text, m.tags
             FROM hermes_memory_fts f
             JOIN hermes_memory m ON m.id = f.rowid
             WHERE hermes_memory_fts MATCH ?1
             ORDER BY bm25(hermes_memory_fts)
             LIMIT ?2",
        )?;
        let raw: Vec<MemoryRow> = stmt
            .query_map(rusqlite::params![query, limit as i64], |row| {
                Ok(MemoryRow {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    kind: row.get(2)?,
                    source: row.get(3)?,
                    text: row.get(4)?,
                    tags: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let rows = raw
            .into_iter()
            .map(Self::open_row)
            .collect::<Result<Vec<_>>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    // The seal tests deliberately hold a std::sync::Mutex across `.await` to
    // serialize the process-global EventKey cache + PHANTOM_ENCRYPT_MEMORY env
    // for the WHOLE async body (an async-aware mutex would release between
    // awaits, defeating the serialization). Suppress the (correct-in-general)
    // lint for this intentional test-only pattern.
    #![allow(clippy::await_holding_lock)]

    use super::*;

    // PHANTOM_ENCRYPT_MEMORY + the EVENT_KEY_CACHE are process-global; every
    // flag-ON test serializes on this mutex so they don't clobber each other's
    // env/key. Single-thread (`--test-threads=1`) is belt-and-suspenders.
    static SEAL_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[tokio::test]
    async fn open_at_creates_schema_on_fresh_db() {
        let td = tempfile::tempdir().unwrap();
        let db_path = td.path().join("hermes.db");

        // Should succeed: schema applies cleanly to an empty file.
        let mem = SkillMemory::open_at(db_path.clone()).expect("open_at fresh db");

        // Verify both tables exist by querying sqlite_master.
        let conn = mem.conn.lock().await;
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','view') ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(
            names.contains(&"hermes_memory".to_string()),
            "missing hermes_memory: {names:?}"
        );
        assert!(
            names.contains(&"hermes_memory_fts".to_string()),
            "missing hermes_memory_fts: {names:?}"
        );
    }

    #[tokio::test]
    async fn insert_then_get_by_id_round_trips() {
        let td = tempfile::tempdir().unwrap();
        let mem = SkillMemory::open_at(td.path().join("hermes.db")).unwrap();

        let id = mem
            .insert(NewMemory {
                kind: "fact",
                source: "test",
                text: "the quick brown fox jumps over the lazy dog",
                tags: "english pangram",
            })
            .await
            .expect("insert");

        let got = mem
            .get_by_id(id)
            .await
            .expect("get_by_id ok")
            .expect("row exists");
        assert_eq!(got.id, id);
        assert_eq!(got.kind, "fact");
        assert_eq!(got.source, "test");
        assert_eq!(got.text, "the quick brown fox jumps over the lazy dog");
        assert_eq!(got.tags, "english pangram");
        assert!(got.created_at > 0, "created_at should be set");
    }

    #[tokio::test]
    async fn search_matches_inserted_token() {
        let td = tempfile::tempdir().unwrap();
        let mem = SkillMemory::open_at(td.path().join("hermes.db")).unwrap();

        for (text, tags) in [
            ("the quick brown fox", "animals"),
            ("rust is a memory-safe language", "programming"),
            ("FTS5 supports unicode tokenization", "sqlite"),
        ] {
            mem.insert(NewMemory {
                kind: "fact",
                source: "seed",
                text,
                tags,
            })
            .await
            .unwrap();
        }

        let hits = mem.search("rust", 10).await.expect("search ok");
        assert_eq!(
            hits.len(),
            1,
            "expected exactly one match for 'rust', got {hits:?}"
        );
        assert!(hits[0].text.contains("rust"));

        let many = mem.search("supports OR fox", 10).await.expect("search ok");
        assert_eq!(
            many.len(),
            2,
            "expected 2 matches for OR query, got {many:?}"
        );
    }

    #[test]
    fn escape_fts5_query_wraps_in_quoted_phrase() {
        // Plain words → safe quoted phrase
        assert_eq!(escape_fts5_query("hello world"), "\"hello world\"");
        // Embedded double-quote is doubled (FTS5 quoting rule)
        assert_eq!(escape_fts5_query(r#"say "hi""#), "\"say \"\"hi\"\"\"");
        // Operator-y input is treated as literal
        assert_eq!(
            escape_fts5_query("AND OR NOT NEAR(*)"),
            "\"AND OR NOT NEAR(*)\""
        );
        // Column-filter attempt is neutralized (no bare colon outside quotes)
        assert_eq!(escape_fts5_query("text:secret"), "\"text:secret\"");
    }

    #[tokio::test]
    async fn search_with_escaped_query_finds_literal() {
        let td = tempfile::tempdir().unwrap();
        let mem = SkillMemory::open_at(td.path().join("hermes.db")).unwrap();
        mem.insert(NewMemory {
            kind: "fact",
            source: "seed",
            text: "the literal phrase: AND OR NOT NEAR appears here",
            tags: "",
        })
        .await
        .unwrap();

        // Without escaping this would be a parse error in FTS5.
        let escaped = escape_fts5_query("AND OR NOT NEAR");
        let hits = mem.search(&escaped, 10).await.expect("escaped search ok");
        assert_eq!(hits.len(), 1, "expected literal-phrase match, got {hits:?}");
    }

    /// V7 ship-blocker: skills stored in the skillbank FTS5 memory (with
    /// `kind = "skill"`) MUST survive a phantom restart. This is the
    /// regression guard for the skill self-evolution loop's persistence
    /// layer.
    ///
    /// Scenario:
    /// 1. Open store → insert 3 skill records → drop store.
    /// 2. Re-open at same path → all 3 skills must be present.
    /// 3. FTS5 search must find skills by keyword after restart.
    /// 4. `list_by_kind("skill", ...)` must return all 3.
    #[tokio::test]
    async fn skills_persist_across_phantom_restart() {
        // Test-isolation guard (mirrors the P0-8 sibling sealing tests): this
        // test asserts the PLAINTEXT-path persistence, so it must run with
        // at-rest sealing OFF while holding SEAL_TEST_LOCK. Without the lock a
        // concurrent P0-8 sealing test can flip the process-global
        // PHANTOM_ENCRYPT_MEMORY + EventKey cache mid-run, sealing these rows
        // with a key that is then cleared → "undecryptable sealed value" on
        // reopen. Holding the lock + forcing the flag OFF serializes against
        // them so the global state can't change underneath this test.
        use crate::encryption_wire::clear_event_key_cache;
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        clear_event_key_cache();

        let td = tempfile::tempdir().unwrap();
        let db_path = td.path().join("hermes.db");

        // ── Phase 1: insert 3 skills ────────────────────────────────────────
        {
            let mem = SkillMemory::open_at(db_path.clone()).expect("open fresh");
            mem.insert(NewMemory {
                kind: "skill",
                source: "auto-evolve",
                text: "rebase feature branch onto main using git rebase",
                tags: "git version-control",
            })
            .await
            .expect("insert skill 1");

            mem.insert(NewMemory {
                kind: "skill",
                source: "auto-evolve",
                text: "run cargo test with --no-capture flag for debugging",
                tags: "rust testing",
            })
            .await
            .expect("insert skill 2");

            mem.insert(NewMemory {
                kind: "skill",
                source: "user-teach",
                text: "use ripgrep instead of grep for faster code search",
                tags: "tools search",
            })
            .await
            .expect("insert skill 3");

            // mem drops here → simulates phantom process exit
        }

        // ── Phase 2: reopen → all 3 skills must survive ─────────────────────
        {
            let mem = SkillMemory::open_at(db_path.clone()).expect("reopen after restart");

            // list_by_kind should return all 3 skills.
            let (skills, total) = mem
                .list_by_kind("skill", 100, 0)
                .await
                .expect("list_by_kind");
            assert_eq!(total, 3, "expected 3 skills total after restart");
            assert_eq!(skills.len(), 3, "expected 3 skill rows returned");

            // All rows must have kind = "skill".
            for s in &skills {
                assert_eq!(s.kind, "skill", "row kind must be 'skill'");
            }

            // Verify specific content survived.
            let texts: Vec<&str> = skills.iter().map(|s| s.text.as_str()).collect();
            assert!(
                texts.iter().any(|t| t.contains("rebase")),
                "skill 1 (rebase) must survive restart"
            );
            assert!(
                texts.iter().any(|t| t.contains("cargo test")),
                "skill 2 (cargo test) must survive restart"
            );
            assert!(
                texts.iter().any(|t| t.contains("ripgrep")),
                "skill 3 (ripgrep) must survive restart"
            );
        }

        // ── Phase 3: FTS5 search works after restart ────────────────────────
        {
            let mem = SkillMemory::open_at(db_path.clone()).expect("reopen for search");

            // Search for "rebase" should find exactly 1 skill.
            let hits = mem.search("rebase", 10).await.expect("search ok");
            assert_eq!(hits.len(), 1, "expected 1 match for 'rebase'");
            assert!(hits[0].text.contains("rebase"));
            assert_eq!(hits[0].kind, "skill");

            // search_by_kind_paginated should restrict to kind="skill".
            let (kind_hits, total) = mem
                .search_by_kind_paginated("skill", "cargo", 10, 0)
                .await
                .expect("search_by_kind_paginated ok");
            assert_eq!(total, 1, "expected 1 match for 'cargo' in skills");
            assert_eq!(kind_hits.len(), 1);
            assert!(kind_hits[0].text.contains("cargo test"));

            // Non-matching search returns empty.
            let (empty, zero) = mem
                .search_by_kind_paginated("skill", &escape_fts5_query("nonexistent_xyz"), 10, 0)
                .await
                .expect("empty search ok");
            assert_eq!(zero, 0);
            assert!(empty.is_empty());
        }

        // ── Phase 4: facts and skills don't interfere ───────────────────────
        {
            let mem = SkillMemory::open_at(db_path.clone()).expect("reopen for isolation");
            // Insert a fact (not a skill).
            mem.insert(NewMemory {
                kind: "fact",
                source: "test",
                text: "cargo is the Rust build tool",
                tags: "rust",
            })
            .await
            .expect("insert fact");

            // list_by_kind("skill") must still return 3, not 4.
            let (skills, total) = mem
                .list_by_kind("skill", 100, 0)
                .await
                .expect("list skills only");
            assert_eq!(total, 3, "fact must not appear in skill list");
            assert_eq!(skills.len(), 3);
        }
    }

    // ─── P0-8 at-rest encryption (PHANTOM_ENCRYPT_MEMORY) ────────────────────

    /// Task 2+4: with the flag ON, the on-disk `text`/`source` columns are
    /// sealed (the plaintext needle is NOT present), and a reopen + read returns
    /// the decrypted plaintext.
    #[tokio::test]
    async fn insert_seals_text_and_source_on_disk_when_flag_on() {
        use crate::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "1");
        install_event_key_from_seed(&[0x5Au8; 32]).unwrap();

        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("hermes.db");
        let id = {
            let mem = SkillMemory::open_at(db.clone()).unwrap();
            mem.insert(NewMemory {
                kind: "fact",
                source: "secret-source",
                text: "SECRET-NEEDLE classified owned memory",
                tags: "t",
            })
            .await
            .unwrap()
        }; // drop store → release the sqlite handle so we can reopen raw

        // Raw column bytes on disk must NOT contain the plaintext needle.
        let (raw_text, raw_source): (String, String) = {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.query_row(
                "SELECT text, source FROM hermes_memory WHERE id=?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
        };
        assert!(
            !raw_text.contains("SECRET-NEEDLE"),
            "plaintext text leaked: {raw_text}"
        );
        assert!(
            !raw_source.contains("secret-source"),
            "plaintext source leaked: {raw_source}"
        );
        assert!(
            crate::skillbank::memory_seal::is_sealed(&raw_text),
            "text column must be sealed"
        );
        assert!(
            crate::skillbank::memory_seal::is_sealed(&raw_source),
            "source column must be sealed"
        );

        // Round-trip read returns plaintext (decrypt-on-read).
        let mem2 = SkillMemory::open_at(db).unwrap();
        let row = mem2.get_by_id(id).await.unwrap().unwrap();
        assert_eq!(row.text, "SECRET-NEEDLE classified owned memory");
        assert_eq!(row.source, "secret-source");

        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        clear_event_key_cache();
    }

    /// Task 3: with the flag ON, FTS5 keyword recall still works over sealed
    /// rows, because the index is fed the de-PII'd token form (not the blob).
    #[tokio::test]
    async fn fts_search_works_over_sealed_rows_via_index_form() {
        use crate::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "1");
        install_event_key_from_seed(&[0x6Bu8; 32]).unwrap();

        let td = tempfile::tempdir().unwrap();
        let mem = SkillMemory::open_at(td.path().join("h.db")).unwrap();
        mem.insert(NewMemory {
            kind: "skill",
            source: "auto-evolve",
            text: "rebase onto main feature branch",
            tags: "git",
        })
        .await
        .unwrap();

        let hits = mem.search("rebase", 10).await.expect("search rebase");
        assert_eq!(hits.len(), 1, "FTS must find the sealed row via index form");
        // The returned text is the decrypted plaintext, not the index form.
        assert_eq!(hits[0].text, "rebase onto main feature branch");

        let hits2 = mem.search("onto", 10).await.expect("search onto");
        assert_eq!(hits2.len(), 1, "second keyword must also recall the row");

        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        clear_event_key_cache();
    }

    /// Task 4: reading a sealed row with the WRONG key fails CLOSED — the call
    /// returns Err and the error text carries neither plaintext nor ciphertext.
    #[tokio::test]
    async fn read_fails_closed_on_wrong_key() {
        use crate::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "1");
        install_event_key_from_seed(&[0xAAu8; 32]).unwrap();
        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("h.db");
        let id = {
            let m = SkillMemory::open_at(db.clone()).unwrap();
            m.insert(NewMemory {
                kind: "fact",
                source: "s",
                text: "top secret",
                tags: "",
            })
            .await
            .unwrap()
        };
        clear_event_key_cache();
        install_event_key_from_seed(&[0xBBu8; 32]).unwrap(); // wrong key

        let m2 = SkillMemory::open_at(db).unwrap();
        let r = m2.get_by_id(id).await;
        assert!(
            r.is_err(),
            "wrong key must fail closed, not surface ciphertext"
        );
        let msg = format!("{:#}", r.unwrap_err());
        assert!(!msg.contains("top secret"), "plaintext in error: {msg}");

        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        clear_event_key_cache();
    }

    /// Task 6.2: a DB written with the flag OFF (plaintext rows) must still read
    /// correctly when the flag is later turned ON + a key installed. The
    /// free-form plaintext `text`/`source` (which do NOT start with `{`) must
    /// pass through `open()` untouched, NOT be mis-decrypted. This is the
    /// migration-window correctness guard.
    #[tokio::test]
    async fn flag_on_reads_legacy_plaintext_rows_unchanged() {
        use crate::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // Phase 1: write rows with the flag OFF (plaintext on disk).
        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("h.db");
        let id = {
            let mem = SkillMemory::open_at(db.clone()).unwrap();
            mem.insert(NewMemory {
                kind: "skill",
                source: "user-teach",
                text: "rebase onto main using git rebase",
                tags: "git",
            })
            .await
            .unwrap()
        };

        // Phase 2: flip the flag ON + install a key, reopen, read.
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "1");
        install_event_key_from_seed(&[0xCDu8; 32]).unwrap();
        let mem2 = SkillMemory::open_at(db).unwrap();
        let row = mem2
            .get_by_id(id)
            .await
            .expect("legacy plaintext row must read, not fail-decrypt")
            .unwrap();
        assert_eq!(row.text, "rebase onto main using git rebase");
        assert_eq!(row.source, "user-teach");
        // FTS recall over the legacy plaintext row still works.
        let hits = mem2.search("rebase", 10).await.unwrap();
        assert_eq!(hits.len(), 1);

        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        clear_event_key_cache();
    }

    /// Task 5.4 (unit slice): row delete purges the FTS index even after the
    /// 0010 trigger retirement (the AFTER DELETE trigger was NOT dropped).
    #[tokio::test]
    async fn delete_purges_fts_index() {
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        let td = tempfile::tempdir().unwrap();
        let mem = SkillMemory::open_at(td.path().join("h.db")).unwrap();
        let id = mem
            .insert(NewMemory {
                kind: "fact",
                source: "s",
                text: "ephemeral note about widgets",
                tags: "",
            })
            .await
            .unwrap();
        assert_eq!(mem.search("widgets", 10).await.unwrap().len(), 1);
        mem.delete_by_id(id).await.unwrap();
        assert!(mem.get_by_id(id).await.unwrap().is_none());
        assert_eq!(
            mem.search("widgets", 10).await.unwrap().len(),
            0,
            "delete must purge the FTS index"
        );
    }

    /// Regression for the integration-test-surfaced bug: deleting a SEALED row
    /// (flag ON) must purge the FTS index without "database disk image is
    /// malformed". The FTS 'delete' command must be fed the recomputed index
    /// form (decrypt → fts_index_form), not the sealed `text` blob.
    #[tokio::test]
    async fn delete_purges_fts_index_when_sealed() {
        use crate::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "1");
        install_event_key_from_seed(&[0x7Eu8; 32]).unwrap();

        let td = tempfile::tempdir().unwrap();
        let mem = SkillMemory::open_at(td.path().join("h.db")).unwrap();
        let id = mem
            .insert(NewMemory {
                kind: "fact",
                source: "s",
                text: "sealed disposable gadget memo",
                tags: "",
            })
            .await
            .unwrap();
        assert_eq!(mem.search("gadget", 10).await.unwrap().len(), 1);
        mem.delete_by_id(id).await.expect("delete sealed row");
        assert!(mem.get_by_id(id).await.unwrap().is_none());
        assert_eq!(
            mem.search("gadget", 10).await.unwrap().len(),
            0,
            "sealed-row delete must purge the FTS index"
        );

        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        clear_event_key_cache();
    }

    /// Regression for the agy-review-surfaced flag-toggle delete bug: a row
    /// sealed with the flag ON, then deleted AFTER the flag is toggled OFF, must
    /// still purge the FTS index without corruption. The delete path must key
    /// off whether the STORED value is sealed, not the current flag state.
    #[tokio::test]
    async fn delete_sealed_row_after_flag_toggled_off() {
        use crate::encryption_wire::{clear_event_key_cache, install_event_key_from_seed};
        let _g = SEAL_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let td = tempfile::tempdir().unwrap();
        let db = td.path().join("h.db");

        // Phase 1: flag ON → insert a sealed row (FTS indexed as token form).
        std::env::set_var("PHANTOM_ENCRYPT_MEMORY", "1");
        install_event_key_from_seed(&[0x9Fu8; 32]).unwrap();
        let id = {
            let mem = SkillMemory::open_at(db.clone()).unwrap();
            let id = mem
                .insert(NewMemory {
                    kind: "fact",
                    source: "s",
                    text: "toggle gizmo memo to be deleted later",
                    tags: "",
                })
                .await
                .unwrap();
            assert_eq!(mem.search("gizmo", 10).await.unwrap().len(), 1);
            id
        };

        // Phase 2: flag OFF (key still loaded), reopen, delete. Must not corrupt.
        std::env::remove_var("PHANTOM_ENCRYPT_MEMORY");
        let mem2 = SkillMemory::open_at(db).unwrap();
        mem2.delete_by_id(id)
            .await
            .expect("delete sealed row with flag OFF must not corrupt FTS");
        assert!(mem2.get_by_id(id).await.unwrap().is_none());
        assert_eq!(
            mem2.search("gizmo", 10).await.unwrap().len(),
            0,
            "FTS index must be purged"
        );

        clear_event_key_cache();
    }
}
