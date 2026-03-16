// SQLite backend for semantic memory — zero-config, bundled, single machine

use anyhow::Result;
use async_trait::async_trait;
use rusqlite::params;

use super::{bytes_to_vec, cosine_similarity, vec_to_bytes, MemoryBackend, MemoryEntry};

pub struct SqliteMemory {
    db_path: String,
}

impl SqliteMemory {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                key TEXT NOT NULL,
                content TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT 'conversation',
                session_id TEXT,
                embedding BLOB,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_memories_key ON memories(key);
            CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
            CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id);",
        )?;
        Ok(Self {
            db_path: db_path.to_string(),
        })
    }
}

#[async_trait]
impl MemoryBackend for SqliteMemory {
    async fn store(
        &self,
        id: &str,
        key: &str,
        content: &str,
        category: &str,
        session_id: Option<&str>,
        embedding: Option<Vec<f32>>,
    ) -> Result<()> {
        let db_path = self.db_path.clone();
        let id = id.to_string();
        let key = key.to_string();
        let content = content.to_string();
        let category = category.to_string();
        let session_id = session_id.map(String::from);
        let embedding_bytes = embedding.map(|v| vec_to_bytes(&v));

        tokio::task::spawn_blocking(move || -> Result<()> {
            let conn = rusqlite::Connection::open(&db_path)?;
            conn.execute(
                "INSERT OR REPLACE INTO memories (id, key, content, category, session_id, embedding)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, key, content, category, session_id, embedding_bytes],
            )?;
            Ok(())
        })
        .await??;
        Ok(())
    }

    async fn keyword_search(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let db_path = self.db_path.clone();
        let query = query.to_string();
        let session_id = session_id.map(String::from);

        tokio::task::spawn_blocking(move || -> Result<Vec<MemoryEntry>> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let keywords: Vec<String> = query
                .split_whitespace()
                .filter(|w| w.len() >= 2)
                .take(5)
                .map(|w| format!("%{}%", w))
                .collect();

            if keywords.is_empty() {
                return Ok(vec![]);
            }

            let keyword_clauses: Vec<String> = keywords
                .iter()
                .enumerate()
                .map(|(i, _)| format!("content LIKE ?{}", i + 1))
                .collect();
            let where_clause = keyword_clauses.join(" OR ");

            let session_filter = if session_id.is_some() {
                format!(
                    " AND (session_id = ?{} OR session_id IS NULL)",
                    keywords.len() + 1
                )
            } else {
                String::new()
            };

            let sql = format!(
                "SELECT id, key, content, category, session_id, created_at FROM memories
                 WHERE ({}){}
                 ORDER BY created_at DESC LIMIT {}",
                where_clause, session_filter, limit
            );

            let mut stmt = conn.prepare(&sql)?;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = keywords
                .iter()
                .map(|k| Box::new(k.clone()) as Box<dyn rusqlite::types::ToSql>)
                .collect();
            if let Some(sid) = &session_id {
                param_values.push(Box::new(sid.clone()));
            }
            let params_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|p| p.as_ref()).collect();

            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(MemoryEntry {
                    id: row.get(0)?,
                    key: row.get(1)?,
                    content: row.get(2)?,
                    category: row.get(3)?,
                    session_id: row.get(4)?,
                    created_at: row.get(5)?,
                    relevance_score: 1.0,
                })
            })?;

            Ok(rows.filter_map(|r| r.ok()).collect())
        })
        .await?
    }

    async fn vector_search(
        &self,
        query_vec: &[f32],
        limit: usize,
        session_id: Option<&str>,
        min_relevance: f32,
    ) -> Result<Vec<MemoryEntry>> {
        let db_path = self.db_path.clone();
        let query_vec = query_vec.to_vec();
        let session_id = session_id.map(String::from);

        tokio::task::spawn_blocking(move || -> Result<Vec<MemoryEntry>> {
            let conn = rusqlite::Connection::open(&db_path)?;

            let sql = if session_id.is_some() {
                "SELECT id, key, content, category, session_id, embedding, created_at
                 FROM memories WHERE embedding IS NOT NULL AND (session_id = ?1 OR session_id IS NULL)
                 ORDER BY created_at DESC LIMIT 500"
            } else {
                "SELECT id, key, content, category, session_id, embedding, created_at
                 FROM memories WHERE embedding IS NOT NULL
                 ORDER BY created_at DESC LIMIT 500"
            };

            let mut stmt = conn.prepare(sql)?;

            let rows: Vec<(MemoryEntry, Vec<f32>)> = if let Some(ref sid) = session_id {
                stmt.query_map(params![sid], |row| {
                    let embedding_bytes: Vec<u8> = row.get(5)?;
                    Ok((
                        MemoryEntry {
                            id: row.get(0)?,
                            key: row.get(1)?,
                            content: row.get(2)?,
                            category: row.get(3)?,
                            session_id: row.get(4)?,
                            created_at: row.get(6)?,
                            relevance_score: 0.0,
                        },
                        bytes_to_vec(&embedding_bytes),
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect()
            } else {
                stmt.query_map([], |row| {
                    let embedding_bytes: Vec<u8> = row.get(5)?;
                    Ok((
                        MemoryEntry {
                            id: row.get(0)?,
                            key: row.get(1)?,
                            content: row.get(2)?,
                            category: row.get(3)?,
                            session_id: row.get(4)?,
                            created_at: row.get(6)?,
                            relevance_score: 0.0,
                        },
                        bytes_to_vec(&embedding_bytes),
                    ))
                })?
                .filter_map(|r| r.ok())
                .collect()
            };

            let mut scored: Vec<MemoryEntry> = rows
                .into_iter()
                .map(|(mut entry, vec)| {
                    entry.relevance_score = cosine_similarity(&query_vec, &vec);
                    entry
                })
                .filter(|e| e.relevance_score >= min_relevance)
                .collect();

            scored.sort_by(|a, b| {
                b.relevance_score
                    .partial_cmp(&a.relevance_score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            scored.truncate(limit);
            Ok(scored)
        })
        .await?
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let db_path = self.db_path.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> Result<Option<MemoryEntry>> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let mut stmt = conn.prepare(
                "SELECT id, key, content, category, session_id, created_at FROM memories WHERE key = ?1",
            )?;
            let entry = stmt
                .query_row(params![key], |row| {
                    Ok(MemoryEntry {
                        id: row.get(0)?,
                        key: row.get(1)?,
                        content: row.get(2)?,
                        category: row.get(3)?,
                        session_id: row.get(4)?,
                        created_at: row.get(5)?,
                        relevance_score: 1.0,
                    })
                })
                .ok();
            Ok(entry)
        })
        .await?
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let db_path = self.db_path.clone();
        let key = key.to_string();

        tokio::task::spawn_blocking(move || -> Result<bool> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let rows = conn.execute("DELETE FROM memories WHERE key = ?1", params![key])?;
            Ok(rows > 0)
        })
        .await?
    }

    async fn count(&self) -> Result<usize> {
        let db_path = self.db_path.clone();

        tokio::task::spawn_blocking(move || -> Result<usize> {
            let conn = rusqlite::Connection::open(&db_path)?;
            let count: i64 =
                conn.query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?;
            Ok(count as usize)
        })
        .await?
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryCategory;

    #[test]
    fn test_sqlite_memory_basic() {
        let dir = std::env::temp_dir().join("clawtex_test_memory_sqlite");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_mem.db");
        let _ = std::fs::remove_file(&db_path);

        let config = crate::memory::MemoryConfig {
            embeddings_enabled: false,
            ..Default::default()
        };
        let store = crate::memory::MemoryStore::sqlite(db_path.to_str().unwrap(), config).unwrap();

        assert_eq!(
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(store.count())
                .unwrap(),
            0
        );

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            store
                .store("user_name", "User's name is Alice", MemoryCategory::Core, None)
                .await
                .unwrap();
            store
                .store(
                    "preference",
                    "User prefers Traditional Chinese",
                    MemoryCategory::Core,
                    None,
                )
                .await
                .unwrap();
        });

        let count = rt.block_on(store.count()).unwrap();
        assert_eq!(count, 2);

        let entry = rt.block_on(store.get("user_name")).unwrap().unwrap();
        assert_eq!(entry.content, "User's name is Alice");

        assert!(rt.block_on(store.forget("user_name")).unwrap());
        assert_eq!(rt.block_on(store.count()).unwrap(), 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_sqlite_keyword_search() {
        let dir = std::env::temp_dir().join("clawtex_test_memory_sqlite_kw");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("test_kw.db");
        let _ = std::fs::remove_file(&db_path);

        let backend = SqliteMemory::new(db_path.to_str().unwrap()).unwrap();

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            backend
                .store(
                    "1",
                    "rust_fact",
                    "Rust is a systems programming language",
                    "core",
                    None,
                    None,
                )
                .await
                .unwrap();
            backend
                .store(
                    "2",
                    "python_fact",
                    "Python is great for data science",
                    "core",
                    None,
                    None,
                )
                .await
                .unwrap();
        });

        let results = rt
            .block_on(backend.keyword_search("Rust programming", 5, None))
            .unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));

        let _ = std::fs::remove_file(&db_path);
    }
}
