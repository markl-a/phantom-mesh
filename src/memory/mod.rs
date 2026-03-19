// Semantic Memory — pluggable backend for long-term agent memory
//
// Two backends available:
//   1. SQLite (default) — zero-config, bundled, good for single machine
//   2. pgvector (optional, feature = "pg") — PostgreSQL + pgvector extension,
//      better for production/cluster deployments
//
// Both share the same MemoryBackend trait and MemoryStore wrapper.

pub mod sqlite;
#[cfg(feature = "pg")]
pub mod pgvector;

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::debug;

// ── Types ─────────────────────────────────────────────────────────────────────

/// Memory entry category
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    Core,
    Conversation,
    TaskResult,
    Custom(String),
}

impl std::fmt::Display for MemoryCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryCategory::Core => write!(f, "core"),
            MemoryCategory::Conversation => write!(f, "conversation"),
            MemoryCategory::TaskResult => write!(f, "task_result"),
            MemoryCategory::Custom(s) => write!(f, "custom:{}", s),
        }
    }
}

/// A stored memory entry
#[derive(Debug, Clone, Serialize)]
pub struct MemoryEntry {
    pub id: String,
    pub key: String,
    pub content: String,
    pub category: String,
    pub session_id: Option<String>,
    pub created_at: String,
    pub relevance_score: f32,
}

/// Configuration for the memory system
#[derive(Debug, Clone, Deserialize)]
pub struct MemoryConfig {
    /// Backend: "sqlite" (default) or "pgvector"
    #[serde(default = "default_backend")]
    pub backend: String,
    /// PostgreSQL connection string (for pgvector backend)
    #[serde(default)]
    pub pg_url: Option<String>,
    /// Ollama URL for embeddings
    #[serde(default = "default_embed_url")]
    pub embed_url: String,
    /// Embedding model name
    #[serde(default = "default_embed_model")]
    pub embed_model: String,
    /// Embedding dimensions (must match model)
    #[serde(default = "default_dimensions")]
    pub dimensions: usize,
    /// Minimum relevance score to include in recall (0.0 - 1.0)
    #[serde(default = "default_min_relevance")]
    pub min_relevance: f32,
    /// Whether embeddings are enabled (falls back to keyword-only if false)
    #[serde(default = "default_true")]
    pub embeddings_enabled: bool,
}

fn default_backend() -> String { "sqlite".to_string() }
fn default_embed_url() -> String { "http://localhost:11434".to_string() }
fn default_embed_model() -> String { "nomic-embed-text".to_string() }
fn default_dimensions() -> usize { 768 }
fn default_min_relevance() -> f32 { 0.3 }
fn default_true() -> bool { true }

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            pg_url: None,
            embed_url: default_embed_url(),
            embed_model: default_embed_model(),
            dimensions: default_dimensions(),
            min_relevance: default_min_relevance(),
            embeddings_enabled: true,
        }
    }
}

// ── Memory Tier Hierarchy ──────────────────────────────────────────────────────

/// Memory tier — categorizes memories by temporal and functional role.
/// Inspired by cognitive science memory models (working, episodic, semantic, procedural).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// Short-term, task-specific context (cleared after session)
    Working,
    /// Event-based memories with timestamps (conversation snapshots)
    Episodic,
    /// Long-term factual knowledge (persistent)
    Semantic,
    /// How-to knowledge and procedures (tool usage patterns)
    Procedural,
    /// Relationship-based memories (entity connections)
    Relational,
}

impl MemoryTier {
    /// Classify content into an appropriate tier based on heuristics.
    pub fn classify(key: &str, content: &str) -> Self {
        let lower_key = key.to_lowercase();
        let lower_content = content.to_lowercase();

        // Procedural: how-to, steps, instructions, tool usage
        if lower_key.contains("how_to")
            || lower_key.contains("procedure")
            || lower_key.contains("tool_usage")
            || lower_content.contains("step 1")
            || lower_content.contains("steps:")
            || lower_content.contains("run the command")
        {
            return MemoryTier::Procedural;
        }

        // Relational: entity relationships, connections
        if lower_key.contains("relationship")
            || lower_key.contains("connection")
            || lower_key.contains("related_to")
            || lower_content.contains("is related to")
            || lower_content.contains("depends on")
            || lower_content.contains("connected to")
        {
            return MemoryTier::Relational;
        }

        // Episodic: events, conversations, timestamped data
        if lower_key.contains("conversation")
            || lower_key.contains("session")
            || lower_key.contains("event")
            || lower_content.contains("said:")
            || lower_content.contains("happened at")
            || lower_content.contains("during the session")
        {
            return MemoryTier::Episodic;
        }

        // Working: temporary, task-specific
        if lower_key.contains("current_task")
            || lower_key.contains("temp")
            || lower_key.contains("scratch")
            || lower_key.starts_with("wip_")
        {
            return MemoryTier::Working;
        }

        // Default: Semantic (long-term factual)
        MemoryTier::Semantic
    }

    /// Decay factor for memory recall scoring (0.0 - 1.0).
    /// Lower values decay faster (less relevant over time).
    pub fn decay_factor(&self) -> f32 {
        match self {
            MemoryTier::Working => 0.3,      // Decays fastest
            MemoryTier::Episodic => 0.6,     // Moderate decay
            MemoryTier::Semantic => 0.95,    // Very slow decay
            MemoryTier::Procedural => 0.9,   // Slow decay
            MemoryTier::Relational => 0.85,  // Fairly persistent
        }
    }

    /// Display name for the tier.
    pub fn label(&self) -> &str {
        match self {
            MemoryTier::Working => "working",
            MemoryTier::Episodic => "episodic",
            MemoryTier::Semantic => "semantic",
            MemoryTier::Procedural => "procedural",
            MemoryTier::Relational => "relational",
        }
    }
}

impl std::fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ── Backend Trait ──────────────────────────────────────────────────────────────

#[async_trait]
pub trait MemoryBackend: Send + Sync {
    async fn store(
        &self,
        id: &str,
        key: &str,
        content: &str,
        category: &str,
        session_id: Option<&str>,
        embedding: Option<Vec<f32>>,
    ) -> Result<()>;

    async fn keyword_search(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>>;

    async fn vector_search(
        &self,
        query_vec: &[f32],
        limit: usize,
        session_id: Option<&str>,
        min_relevance: f32,
    ) -> Result<Vec<MemoryEntry>>;

    async fn get(&self, key: &str) -> Result<Option<MemoryEntry>>;
    async fn forget(&self, key: &str) -> Result<bool>;
    async fn count(&self) -> Result<usize>;
}

// ── Vector Operations ─────────────────────────────────────────────────────────

/// Cosine similarity between two vectors (returns 0.0 - 1.0)
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| *x as f64 * *y as f64).sum();
    let norm_a: f64 = a.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| (*x as f64) * (*x as f64)).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    ((dot / (norm_a * norm_b)).clamp(0.0, 1.0)) as f32
}

/// Reciprocal Rank Fusion — merge keyword and vector results
pub fn rrf_merge(
    keyword_results: &[(String, f32)],
    vector_results: &[(String, f32)],
    k: f32,
) -> Vec<(String, f32)> {
    use std::collections::HashMap;
    let mut scores: HashMap<String, f32> = HashMap::new();
    for (rank, (id, _)) in keyword_results.iter().enumerate() {
        *scores.entry(id.clone()).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }
    for (rank, (id, _)) in vector_results.iter().enumerate() {
        *scores.entry(id.clone()).or_default() += 1.0 / (k + rank as f32 + 1.0);
    }
    let mut merged: Vec<(String, f32)> = scores.into_iter().collect();
    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    merged
}

/// Serialize f32 vector to bytes for SQLite blob storage
pub fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Deserialize bytes from SQLite blob back to f32 vector
pub fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
    bytes.chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

// ── Embedding Client ──────────────────────────────────────────────────────────

/// Get embedding vector from Ollama /api/embed
async fn get_embedding(client: &Client, base_url: &str, model: &str, text: &str) -> Result<Vec<f32>> {
    let url = format!("{}/api/embed", base_url);
    let body = serde_json::json!({ "model": model, "input": text });
    let resp = client.post(&url).json(&body).send().await?;
    let json: serde_json::Value = resp.json().await?;

    let embeddings = json.get("embeddings")
        .and_then(|e| e.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("Invalid embedding response"))?;

    let vec: Vec<f32> = embeddings.iter()
        .filter_map(|v| v.as_f64().map(|f| f as f32))
        .collect();
    if vec.is_empty() {
        return Err(anyhow!("Empty embedding vector"));
    }
    Ok(vec)
}

// ── MemoryStore (wraps any backend) ───────────────────────────────────────────

pub struct MemoryStore {
    backend: Box<dyn MemoryBackend>,
    config: MemoryConfig,
    client: Client,
}

impl MemoryStore {
    pub fn new(backend: Box<dyn MemoryBackend>, config: MemoryConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { backend, config, client })
    }

    /// Create a SQLite-backed MemoryStore
    pub fn sqlite(db_path: &str, config: MemoryConfig) -> Result<Self> {
        let backend = Box::new(sqlite::SqliteMemory::new(db_path)?);
        Self::new(backend, config)
    }

    /// Create a pgvector-backed MemoryStore
    #[cfg(feature = "pg")]
    pub async fn pgvector(pg_url: &str, config: MemoryConfig) -> Result<Self> {
        let dims = config.dimensions;
        let backend = Box::new(pgvector::PgVectorMemory::new(pg_url, dims).await?);
        Self::new(backend, config)
    }

    /// Create from config (auto-selects backend based on config.backend)
    pub async fn from_config(config: MemoryConfig, default_db_path: &str) -> Result<Self> {
        match config.backend.as_str() {
            #[cfg(feature = "pg")]
            "pgvector" | "pg" | "postgres" => {
                let pg_url = config.pg_url.clone()
                    .ok_or_else(|| anyhow!("pg_url required for pgvector backend"))?;
                Self::pgvector(&pg_url, config).await
            }
            #[cfg(not(feature = "pg"))]
            "pgvector" | "pg" | "postgres" => {
                Err(anyhow!(
                    "pgvector backend requested but 'pg' feature not enabled. \
                     Rebuild with: cargo build --features pg"
                ))
            }
            _ => Self::sqlite(default_db_path, config),
        }
    }

    /// Store a memory with optional embedding
    pub async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();

        let embedding = if self.config.embeddings_enabled {
            match get_embedding(&self.client, &self.config.embed_url, &self.config.embed_model, content).await {
                Ok(vec) => Some(vec),
                Err(e) => {
                    debug!("Embedding generation failed (storing without): {}", e);
                    None
                }
            }
        } else {
            None
        };

        let has_embedding = embedding.is_some();
        self.backend.store(&id, key, content, &category.to_string(), session_id, embedding).await?;
        debug!("Stored memory '{}' (has_embedding={})", key, has_embedding);
        Ok(id)
    }

    /// Recall relevant memories using hybrid search (keyword + vector)
    pub async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> Result<Vec<MemoryEntry>> {
        let keyword_results = self.backend.keyword_search(query, limit * 2, session_id).await?;

        let vector_results = if self.config.embeddings_enabled {
            match get_embedding(&self.client, &self.config.embed_url, &self.config.embed_model, query).await {
                Ok(query_vec) => {
                    self.backend.vector_search(&query_vec, limit * 2, session_id, self.config.min_relevance).await?
                }
                Err(e) => {
                    debug!("Embedding query failed, keyword-only recall: {}", e);
                    vec![]
                }
            }
        } else {
            vec![]
        };

        if vector_results.is_empty() {
            return Ok(keyword_results.into_iter().take(limit).collect());
        }

        let kw_ids: Vec<(String, f32)> = keyword_results.iter()
            .map(|e| (e.id.clone(), e.relevance_score)).collect();
        let vec_ids: Vec<(String, f32)> = vector_results.iter()
            .map(|e| (e.id.clone(), e.relevance_score)).collect();
        let merged = rrf_merge(&kw_ids, &vec_ids, 60.0);

        let all_entries: std::collections::HashMap<String, MemoryEntry> = keyword_results.into_iter()
            .chain(vector_results)
            .map(|e| (e.id.clone(), e))
            .collect();

        Ok(merged.into_iter()
            .take(limit)
            .filter_map(|(id, score)| {
                all_entries.get(&id).map(|e| MemoryEntry {
                    relevance_score: score,
                    ..e.clone()
                })
            })
            .collect())
    }

    pub async fn get(&self, key: &str) -> Result<Option<MemoryEntry>> {
        self.backend.get(key).await
    }

    pub async fn forget(&self, key: &str) -> Result<bool> {
        self.backend.forget(key).await
    }

    pub async fn count(&self) -> Result<usize> {
        self.backend.count().await
    }

    /// Format recalled memories as context string for injection into system prompt
    pub fn format_context(memories: &[MemoryEntry]) -> String {
        if memories.is_empty() {
            return String::new();
        }
        let mut ctx = String::from("\n[Recalled memories]\n");
        for entry in memories {
            ctx.push_str(&format!("- [{}] {}: {}\n", entry.category, entry.key, entry.content));
        }
        ctx
    }

    pub fn backend_name(&self) -> &str {
        &self.config.backend
    }

    /// Store a memory with automatic tier classification.
    /// The tier is determined from the key and content heuristics.
    pub async fn store_tiered(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> Result<(String, MemoryTier)> {
        let tier = MemoryTier::classify(key, content);
        let id = self.store(key, content, category, session_id).await?;
        debug!("Stored tiered memory '{}' tier={}", key, tier);
        Ok((id, tier))
    }

    /// Store a memory with an explicit tier.
    pub async fn store_with_tier(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        tier: MemoryTier,
        session_id: Option<&str>,
    ) -> Result<String> {
        // For now, tiers are metadata-only; the actual storage is identical
        // to regular store(). Tier info is encoded in the key prefix.
        let tiered_key = format!("[{}] {}", tier.label(), key);
        let id = self.store(&tiered_key, content, category, session_id).await?;
        debug!("Stored memory '{}' with explicit tier={}", key, tier);
        Ok(id)
    }

    /// Recall memories with tier-aware scoring.
    /// Applies decay_factor based on memory tier to adjust relevance scores.
    pub async fn recall_tiered(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
        preferred_tiers: Option<&[MemoryTier]>,
    ) -> Result<Vec<MemoryEntry>> {
        // Fetch more than needed, then apply tier-based re-ranking
        let mut entries = self.recall(query, limit * 3, session_id).await?;

        // Apply tier-based scoring adjustments
        for entry in &mut entries {
            let tier = MemoryTier::classify(&entry.key, &entry.content);
            let decay = tier.decay_factor();

            // Boost preferred tiers
            let tier_boost = if let Some(preferred) = preferred_tiers {
                if preferred.contains(&tier) {
                    1.2 // 20% boost for preferred tiers
                } else {
                    1.0
                }
            } else {
                1.0
            };

            entry.relevance_score *= decay * tier_boost;
        }

        // Re-sort by adjusted score
        entries.sort_by(|a, b| {
            b.relevance_score
                .partial_cmp(&a.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(entries.into_iter().take(limit).collect())
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 0.001);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!(cosine_similarity(&a, &b) < 0.001);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
        assert_eq!(cosine_similarity(&[1.0], &[]), 0.0);
    }

    #[test]
    fn test_vec_roundtrip() {
        let original = vec![1.5_f32, -2.3, 0.0, 42.0];
        let bytes = vec_to_bytes(&original);
        let restored = bytes_to_vec(&bytes);
        assert_eq!(original.len(), restored.len());
        for (a, b) in original.iter().zip(restored.iter()) {
            assert!((a - b).abs() < 0.0001);
        }
    }

    #[test]
    fn test_rrf_merge() {
        let kw = vec![
            ("a".to_string(), 1.0_f32),
            ("b".to_string(), 0.8),
            ("c".to_string(), 0.5),
        ];
        let vec_results = vec![
            ("b".to_string(), 0.95_f32),
            ("d".to_string(), 0.7),
            ("a".to_string(), 0.6),
        ];
        let merged = rrf_merge(&kw, &vec_results, 60.0);
        assert!(merged.len() >= 2);
        let top_ids: Vec<&str> = merged.iter().take(2).map(|(id, _)| id.as_str()).collect();
        assert!(top_ids.contains(&"a") || top_ids.contains(&"b"));
    }

    #[test]
    fn test_format_context() {
        let entries = vec![MemoryEntry {
            id: "1".to_string(),
            key: "name".to_string(),
            content: "Alice".to_string(),
            category: "core".to_string(),
            session_id: None,
            created_at: "2026-01-01".to_string(),
            relevance_score: 0.9,
        }];
        let ctx = MemoryStore::format_context(&entries);
        assert!(ctx.contains("[core] name: Alice"));
    }

    #[test]
    fn test_format_context_empty() {
        assert_eq!(MemoryStore::format_context(&[]), "");
    }

    // ── MemoryTier tests ────────────────────────────────────────────────────

    #[test]
    fn test_tier_classify_procedural() {
        assert_eq!(MemoryTier::classify("how_to_deploy", "Run these steps"), MemoryTier::Procedural);
        assert_eq!(MemoryTier::classify("deploy", "Step 1: install deps"), MemoryTier::Procedural);
        assert_eq!(MemoryTier::classify("tool_usage_shell", "use shell to..."), MemoryTier::Procedural);
    }

    #[test]
    fn test_tier_classify_relational() {
        assert_eq!(MemoryTier::classify("relationship_user_project", "User is related to project X"), MemoryTier::Relational);
        assert_eq!(MemoryTier::classify("deps", "Module A depends on Module B"), MemoryTier::Relational);
    }

    #[test]
    fn test_tier_classify_episodic() {
        assert_eq!(MemoryTier::classify("conversation_2024", "User said: hello"), MemoryTier::Episodic);
        assert_eq!(MemoryTier::classify("session_123", "During the session..."), MemoryTier::Episodic);
    }

    #[test]
    fn test_tier_classify_working() {
        assert_eq!(MemoryTier::classify("current_task_debug", "Debugging X"), MemoryTier::Working);
        assert_eq!(MemoryTier::classify("temp_analysis", "Temporary data"), MemoryTier::Working);
        assert_eq!(MemoryTier::classify("wip_draft", "Draft content"), MemoryTier::Working);
    }

    #[test]
    fn test_tier_classify_semantic_default() {
        assert_eq!(MemoryTier::classify("user_name", "Alice"), MemoryTier::Semantic);
        assert_eq!(MemoryTier::classify("api_key_location", "In .env file"), MemoryTier::Semantic);
    }

    #[test]
    fn test_tier_decay_factors() {
        // Working decays fastest, Semantic slowest
        assert!(MemoryTier::Working.decay_factor() < MemoryTier::Episodic.decay_factor());
        assert!(MemoryTier::Episodic.decay_factor() < MemoryTier::Relational.decay_factor());
        assert!(MemoryTier::Relational.decay_factor() < MemoryTier::Procedural.decay_factor());
        assert!(MemoryTier::Procedural.decay_factor() < MemoryTier::Semantic.decay_factor());
    }

    #[test]
    fn test_tier_labels() {
        assert_eq!(MemoryTier::Working.label(), "working");
        assert_eq!(MemoryTier::Episodic.label(), "episodic");
        assert_eq!(MemoryTier::Semantic.label(), "semantic");
        assert_eq!(MemoryTier::Procedural.label(), "procedural");
        assert_eq!(MemoryTier::Relational.label(), "relational");
    }

    #[test]
    fn test_tier_display() {
        assert_eq!(format!("{}", MemoryTier::Semantic), "semantic");
        assert_eq!(format!("{}", MemoryTier::Working), "working");
    }

    #[test]
    fn test_tier_serialization() {
        let tier = MemoryTier::Procedural;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"procedural\"");
        let back: MemoryTier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, MemoryTier::Procedural);
    }

    #[test]
    fn test_all_tiers_have_valid_decay() {
        for tier in &[
            MemoryTier::Working,
            MemoryTier::Episodic,
            MemoryTier::Semantic,
            MemoryTier::Procedural,
            MemoryTier::Relational,
        ] {
            let decay = tier.decay_factor();
            assert!(decay > 0.0 && decay <= 1.0, "Invalid decay for {:?}: {}", tier, decay);
        }
    }

    #[test]
    fn test_tier_classify_case_insensitive() {
        // Key matching is lowercase
        assert_eq!(MemoryTier::classify("HOW_TO_fix", "fix things"), MemoryTier::Procedural);
        assert_eq!(MemoryTier::classify("CONVERSATION_log", "User said: hi"), MemoryTier::Episodic);
    }
}
