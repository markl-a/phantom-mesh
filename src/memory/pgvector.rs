// pgvector backend for semantic memory — PostgreSQL + pgvector extension
// Requires: cargo build --features pg
// Requires: PostgreSQL with pgvector extension installed

use anyhow::Result;
use async_trait::async_trait;
use pgvector::Vector;
use std::sync::Arc;
use tokio_postgres::Client;
use tracing::{debug, info};

use super::{MemoryBackend, MemoryEntry};

pub struct PgVectorMemory {
    client: Arc<Client>,
    #[allow(dead_code)]
    dimensions: usize,
}

impl PgVectorMemory {
    pub async fn new(conn_str: &str, dimensions: usize) -> Result<Self> {
        let (client, connection) =
            tokio_postgres::connect(conn_str, tokio_postgres::NoTls).await?;

        // Drive the connection in the background
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                tracing::error!("PostgreSQL connection error: {}", e);
            }
        });

        // Enable pgvector extension
        client
            .batch_execute("CREATE EXTENSION IF NOT EXISTS vector")
            .await?;

        // Create memories table with vector column
        client
            .batch_execute(&format!(
                "CREATE TABLE IF NOT EXISTS memories (
                    id TEXT PRIMARY KEY,
                    key TEXT NOT NULL,
                    content TEXT NOT NULL,
                    category TEXT NOT NULL DEFAULT 'conversation',
                    session_id TEXT,
                    embedding vector({dim}),
                    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
                );
                CREATE INDEX IF NOT EXISTS idx_memories_key ON memories(key);
                CREATE INDEX IF NOT EXISTS idx_memories_category ON memories(category);
                CREATE INDEX IF NOT EXISTS idx_memories_session ON memories(session_id);",
                dim = dimensions,
            ))
            .await?;

        // Try to create HNSW vector index (pgvector 0.5+, works with any row count)
        if let Err(e) = client
            .batch_execute(
                "CREATE INDEX IF NOT EXISTS idx_memories_embedding ON memories
                 USING hnsw (embedding vector_cosine_ops)",
            )
            .await
        {
            debug!(
                "HNSW index creation skipped: {} (exact search will be used)",
                e
            );
        }

        info!("pgvector memory backend connected (dim={})", dimensions);

        Ok(Self {
            client: Arc::new(client),
            dimensions,
        })
    }
}

#[async_trait]
impl MemoryBackend for PgVectorMemory {
    async fn store(
        &self,
        id: &str,
        key: &str,
        content: &str,
        category: &str,
        session_id: Option<&str>,
        embedding: Option<Vec<f32>>,
    ) -> Result<()> {
        let session_id_owned: Option<String> = session_id.map(|s| s.to_string());
        let embedding_pg: Option<Vector> = embedding.map(Vector::from);

        self.client
            .execute(
                "INSERT INTO memories (id, key, content, category, session_id, embedding)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (id) DO UPDATE SET
                     key = EXCLUDED.key,
                     content = EXCLUDED.content,
                     category = EXCLUDED.category,
                     session_id = EXCLUDED.session_id,
                     embedding = EXCLUDED.embedding",
                &[&id, &key, &content, &category, &session_id_owned, &embedding_pg],
            )
            .await?;

        Ok(())
    }

    async fn keyword_search(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let keywords: Vec<String> = query
            .split_whitespace()
            .filter(|w| w.len() >= 2)
            .take(5)
            .map(|w| format!("%{}%", w))
            .collect();

        if keywords.is_empty() {
            return Ok(vec![]);
        }

        // Build dynamic ILIKE query with positional params
        let mut conditions = Vec::new();
        for i in 0..keywords.len() {
            conditions.push(format!("content ILIKE ${}", i + 1));
        }

        let next_param = keywords.len() + 1;
        let mut sql = format!(
            "SELECT id, key, content, category, session_id, created_at::text \
             FROM memories WHERE ({})",
            conditions.join(" OR ")
        );

        let mut params: Vec<String> = keywords;
        if let Some(sid) = session_id {
            sql.push_str(&format!(
                " AND (session_id = ${} OR session_id IS NULL)",
                next_param
            ));
            params.push(sid.to_string());
        }

        sql.push_str(&format!(" ORDER BY created_at DESC LIMIT {}", limit));

        let param_refs: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> =
            params.iter().map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync)).collect();

        let rows = self.client.query(&sql, &param_refs).await?;

        Ok(rows
            .iter()
            .map(|row| MemoryEntry {
                id: row.get(0),
                key: row.get(1),
                content: row.get(2),
                category: row.get(3),
                session_id: row.get(4),
                created_at: row.get(5),
                relevance_score: 1.0,
            })
            .collect())
    }

    async fn vector_search(
        &self,
        query_vec: &[f32],
        limit: usize,
        session_id: Option<&str>,
        min_relevance: f32,
    ) -> Result<Vec<MemoryEntry>> {
        let query_embedding = Vector::from(query_vec.to_vec());
        let limit_i64 = limit as i64;

        // pgvector <=> returns cosine distance (1 - similarity)
        // ORDER BY <=> ASC gives most similar first
        let rows = if let Some(sid) = session_id {
            let sid_owned = sid.to_string();
            self.client
                .query(
                    "SELECT id, key, content, category, session_id, created_at::text,
                            1 - (embedding <=> $1) as similarity
                     FROM memories
                     WHERE embedding IS NOT NULL
                       AND (session_id = $2 OR session_id IS NULL)
                     ORDER BY embedding <=> $1
                     LIMIT $3",
                    &[&query_embedding, &sid_owned, &limit_i64],
                )
                .await?
        } else {
            self.client
                .query(
                    "SELECT id, key, content, category, session_id, created_at::text,
                            1 - (embedding <=> $1) as similarity
                     FROM memories
                     WHERE embedding IS NOT NULL
                     ORDER BY embedding <=> $1
                     LIMIT $2",
                    &[&query_embedding, &limit_i64],
                )
                .await?
        };

        Ok(rows
            .iter()
            .map(|row| {
                let similarity: f64 = row.get(6);
                MemoryEntry {
                    id: row.get(0),
                    key: row.get(1),
                    content: row.get(2),
                    category: row.get(3),
                    session_id: row.get(4),
                    created_at: row.get(5),
                    relevance_score: (similarity as f32).max(0.0),
                }
            })
            .filter(|e| e.relevance_score >= min_relevance)
            .collect())
    }

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        let rows = self
            .client
            .query(
                "SELECT id, key, content, category, session_id, created_at::text \
                 FROM memories WHERE key = $1",
                &[&key],
            )
            .await?;

        Ok(rows.first().map(|row| MemoryEntry {
            id: row.get(0),
            key: row.get(1),
            content: row.get(2),
            category: row.get(3),
            session_id: row.get(4),
            created_at: row.get(5),
            relevance_score: 1.0,
        }))
    }

    async fn forget(&self, key: &str) -> Result<bool> {
        let rows = self
            .client
            .execute("DELETE FROM memories WHERE key = $1", &[&key])
            .await?;
        Ok(rows > 0)
    }

    async fn count(&self) -> Result<usize> {
        let row = self
            .client
            .query_one("SELECT COUNT(*) FROM memories", &[])
            .await?;
        let count: i64 = row.get(0);
        Ok(count as usize)
    }
}

// ── Tests (require running PostgreSQL with pgvector) ──────────────────────────

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore] // Requires: PostgreSQL with pgvector extension
    async fn test_pgvector_basic() {
        use super::*;

        let conn_str = std::env::var("CLAWTEX_PG_URL")
            .unwrap_or_else(|_| "host=localhost user=postgres dbname=clawtex_test".to_string());

        let backend = PgVectorMemory::new(&conn_str, 768).await.unwrap();

        // Clean up from previous runs
        let _ = backend.client.execute("DELETE FROM memories", &[]).await;

        assert_eq!(backend.count().await.unwrap(), 0);

        backend
            .store("id1", "test_key", "Hello world", "core", None, None)
            .await
            .unwrap();

        assert_eq!(backend.count().await.unwrap(), 1);

        let entry = backend.get("test_key").await.unwrap().unwrap();
        assert_eq!(entry.content, "Hello world");

        assert!(backend.forget("test_key").await.unwrap());
        assert_eq!(backend.count().await.unwrap(), 0);
    }

    #[tokio::test]
    #[ignore]
    async fn test_pgvector_keyword_search() {
        use super::*;

        let conn_str = std::env::var("CLAWTEX_PG_URL")
            .unwrap_or_else(|_| "host=localhost user=postgres dbname=clawtex_test".to_string());

        let backend = PgVectorMemory::new(&conn_str, 768).await.unwrap();
        let _ = backend.client.execute("DELETE FROM memories", &[]).await;

        backend
            .store(
                "id1",
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
                "id2",
                "python_fact",
                "Python is great for data science",
                "core",
                None,
                None,
            )
            .await
            .unwrap();

        let results = backend.keyword_search("Rust programming", 5, None).await.unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));

        let _ = backend.client.execute("DELETE FROM memories", &[]).await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_pgvector_vector_search() {
        use super::*;

        let conn_str = std::env::var("CLAWTEX_PG_URL")
            .unwrap_or_else(|_| "host=localhost user=postgres dbname=clawtex_test".to_string());

        let backend = PgVectorMemory::new(&conn_str, 3).await.unwrap();
        let _ = backend.client.execute("DELETE FROM memories", &[]).await;

        // Store with fake 3-dim embeddings
        backend
            .store(
                "id1",
                "north",
                "Points north",
                "core",
                None,
                Some(vec![0.0, 1.0, 0.0]),
            )
            .await
            .unwrap();
        backend
            .store(
                "id2",
                "east",
                "Points east",
                "core",
                None,
                Some(vec![1.0, 0.0, 0.0]),
            )
            .await
            .unwrap();

        // Query with vector pointing north — should get "north" first
        let results = backend
            .vector_search(&[0.0, 0.99, 0.01], 5, None, 0.1)
            .await
            .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].key, "north");

        let _ = backend.client.execute("DELETE FROM memories", &[]).await;
    }
}
