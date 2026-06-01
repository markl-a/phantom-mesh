//! Hermes long-term memory store backed by SQLite FTS5.
//!
//! See `core/migrations/0007_hermes_fts5.sql` for the canonical schema.
//! See spec §5 H3 for context.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use rusqlite::Connection;
use tokio::sync::Mutex;

/// Schema bootstrap text — included verbatim from the on-disk migration so
/// there is exactly one source of truth.
const SCHEMA_SQL: &str = include_str!("../../migrations/0007_hermes_fts5.sql");

/// FTS5-backed long-term memory.
#[derive(Clone)]
pub struct HermesMemory {
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

impl HermesMemory {
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
        let conn = self.conn.lock().await;
        conn.execute(
            "INSERT INTO hermes_memory (created_at, kind, source, text, tags)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![now, m.kind, m.source, m.text, m.tags],
        )
        .context("insert hermes_memory row")?;
        Ok(conn.last_insert_rowid())
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
            Ok(Some(MemoryRow {
                id: row.get(0)?,
                created_at: row.get(1)?,
                kind: row.get(2)?,
                source: row.get(3)?,
                text: row.get(4)?,
                tags: row.get(5)?,
            }))
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
        let rows: Vec<MemoryRow> = stmt
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
        let rows: Vec<MemoryRow> = stmt
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
        let rows: Vec<MemoryRow> = stmt
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
        let rows: Vec<MemoryRow> = stmt
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
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_at_creates_schema_on_fresh_db() {
        let td = tempfile::tempdir().unwrap();
        let db_path = td.path().join("hermes.db");

        // Should succeed: schema applies cleanly to an empty file.
        let mem = HermesMemory::open_at(db_path.clone()).expect("open_at fresh db");

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
        let mem = HermesMemory::open_at(td.path().join("hermes.db")).unwrap();

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
        let mem = HermesMemory::open_at(td.path().join("hermes.db")).unwrap();

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
        let mem = HermesMemory::open_at(td.path().join("hermes.db")).unwrap();
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

    /// V7 ship-blocker: skills stored in the Hermes FTS5 memory (with
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
        let td = tempfile::tempdir().unwrap();
        let db_path = td.path().join("hermes.db");

        // ── Phase 1: insert 3 skills ────────────────────────────────────────
        {
            let mem = HermesMemory::open_at(db_path.clone()).expect("open fresh");
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
            let mem = HermesMemory::open_at(db_path.clone()).expect("reopen after restart");

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
            let mem = HermesMemory::open_at(db_path.clone()).expect("reopen for search");

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
            let mem = HermesMemory::open_at(db_path.clone()).expect("reopen for isolation");
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
}
