//! Observational Memory — Mastra-style conversation compression into compact observations.
//!
//! Compresses multi-message conversations into structured observations for 3-40x token savings.
//! Uses regex-based extraction (no LLM calls). Persists to SQLite (~/.clawtex/observations.db).

use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use tracing::debug;

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single message in a conversation (input to observation).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub timestamp: Option<String>,
}

/// A compressed observation derived from one or more conversation messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    pub content: String,
    pub source_session_id: String,
    pub message_count: usize,
    pub original_token_estimate: u64,
    pub compressed_token_estimate: u64,
    pub compression_ratio: f64,
    pub tags: Vec<String>,
    pub relevance_score: f64,
    pub created_at: DateTime<Utc>,
}

/// Category of an extracted observation item (used for relevance scoring).
#[derive(Debug, Clone, Copy, PartialEq)]
enum ObservationCategory {
    Decision,
    Solution,
    Problem,
    Action,
    Fact,
}

impl ObservationCategory {
    fn relevance_score(self) -> f64 {
        match self {
            ObservationCategory::Decision => 1.0,
            ObservationCategory::Solution => 0.9,
            ObservationCategory::Problem => 0.8,
            ObservationCategory::Action => 0.7,
            ObservationCategory::Fact => 0.5,
        }
    }

    fn label(self) -> &'static str {
        match self {
            ObservationCategory::Decision => "decision",
            ObservationCategory::Solution => "solution",
            ObservationCategory::Problem => "problem",
            ObservationCategory::Action => "action",
            ObservationCategory::Fact => "fact",
        }
    }
}

/// A single extracted item from conversation text.
#[derive(Debug, Clone)]
struct ExtractedItem {
    category: ObservationCategory,
    text: String,
}

// ── ObservationalMemory ───────────────────────────────────────────────────────

/// Mastra-style observational memory backed by SQLite.
pub struct ObservationalMemory {
    db_path: String,
}

impl ObservationalMemory {
    /// Create a new ObservationalMemory, initializing the SQLite schema.
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS observations (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                source_session_id TEXT NOT NULL,
                message_count INTEGER NOT NULL,
                original_tokens INTEGER NOT NULL,
                compressed_tokens INTEGER NOT NULL,
                compression_ratio REAL NOT NULL,
                tags_json TEXT NOT NULL DEFAULT '[]',
                relevance_score REAL NOT NULL DEFAULT 0.5,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_obs_session ON observations(source_session_id);
            CREATE INDEX IF NOT EXISTS idx_obs_relevance ON observations(relevance_score DESC);
            CREATE INDEX IF NOT EXISTS idx_obs_created ON observations(created_at DESC);"
        )?;
        Ok(Self { db_path: db_path.to_string() })
    }

    /// Observe a conversation: compress messages into a compact observation.
    ///
    /// Extracts key decisions, problems, solutions, action items, and facts
    /// using regex-based pattern matching (no LLM call needed).
    pub fn observe(
        &self,
        session_id: &str,
        messages: &[ConversationMessage],
    ) -> Result<Observation> {
        if messages.is_empty() {
            return Err(anyhow::anyhow!("No messages to observe"));
        }

        // Concatenate all message content
        let full_text: String = messages
            .iter()
            .map(|m| format!("[{}] {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        // Estimate original token count (simple: chars/4)
        let original_tokens = (full_text.len() as u64) / 4;

        // Extract structured items
        let items = extract_items(&full_text);

        // Build compressed summary
        let compressed = if items.is_empty() {
            // Fallback: take first 200 chars as summary
            truncate_str(full_text.trim(), 200)
        } else {
            items
                .iter()
                .map(|item| format!("{}: {}", capitalize(item.category.label()), item.text))
                .collect::<Vec<_>>()
                .join("\n")
        };

        let compressed_tokens = (compressed.len() as u64) / 4;
        let compression_ratio = if compressed_tokens > 0 {
            original_tokens as f64 / compressed_tokens as f64
        } else {
            1.0
        };

        // Determine relevance as max category relevance among extracted items
        let relevance_score = items
            .iter()
            .map(|i| i.category.relevance_score())
            .fold(0.0_f64, f64::max);
        // If no items extracted, use a low default
        let relevance_score = if relevance_score == 0.0 { 0.3 } else { relevance_score };

        // Build tags from categories + session
        let mut tags: Vec<String> = items
            .iter()
            .map(|i| i.category.label().to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        tags.sort();

        let observation = Observation {
            id: uuid::Uuid::new_v4().to_string(),
            content: compressed,
            source_session_id: session_id.to_string(),
            message_count: messages.len(),
            original_token_estimate: original_tokens,
            compressed_token_estimate: compressed_tokens,
            compression_ratio,
            tags,
            relevance_score,
            created_at: Utc::now(),
        };

        self.store(&observation)?;
        debug!(
            "Observation created: session={}, messages={}, ratio={:.1}x, relevance={:.2}",
            session_id, messages.len(), compression_ratio, relevance_score
        );

        Ok(observation)
    }

    /// Search observations by keyword matching in content + tags.
    /// Returns most relevant, ordered by relevance_score DESC.
    pub fn recall(&self, query: &str, limit: usize) -> Result<Vec<Observation>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let query_pattern = format!("%{}%", query.to_lowercase());
        let mut stmt = conn.prepare(
            "SELECT id, content, source_session_id, message_count, original_tokens,
                    compressed_tokens, compression_ratio, tags_json, relevance_score, created_at
             FROM observations
             WHERE LOWER(content) LIKE ?1 OR LOWER(tags_json) LIKE ?1
             ORDER BY relevance_score DESC
             LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![query_pattern, limit as i64], row_to_observation)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Retrieve the most recent observations.
    pub fn recall_recent(&self, limit: usize) -> Result<Vec<Observation>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, content, source_session_id, message_count, original_tokens,
                    compressed_tokens, compression_ratio, tags_json, relevance_score, created_at
             FROM observations
             ORDER BY created_at DESC
             LIMIT ?1"
        )?;
        let rows = stmt.query_map(params![limit as i64], row_to_observation)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Format top observations as a context block for injection into prompts.
    ///
    /// Returns a string like:
    /// ```text
    /// ## Previous Observations
    /// - [2026-03-17] Decision: Use SQLite for all persistence (compression: 15x)
    /// - [2026-03-17] Problem solved: TOML parse warnings fixed (compression: 8x)
    /// ```
    pub fn inject_context(&self, query: &str, max_tokens: usize) -> Result<String> {
        let observations = self.recall(query, 20)?;
        if observations.is_empty() {
            return Ok(String::new());
        }

        let mut lines = Vec::new();
        let mut token_count: usize = 0;
        let header = "## Previous Observations\n";
        token_count += header.len() / 4;

        for obs in &observations {
            let date = obs.created_at.format("%Y-%m-%d").to_string();
            // Take first line of content as summary
            let summary = obs.content.lines().next().unwrap_or(&obs.content);
            let line = format!(
                "- [{}] {} (compression: {:.0}x)",
                date,
                truncate_str(summary, 120),
                obs.compression_ratio
            );
            let line_tokens = line.len() / 4;
            if token_count + line_tokens > max_tokens {
                break;
            }
            token_count += line_tokens;
            lines.push(line);
        }

        if lines.is_empty() {
            return Ok(String::new());
        }

        Ok(format!("{}{}", header, lines.join("\n")))
    }

    /// Count total observations.
    pub fn count(&self) -> Result<u64> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM observations", [], |row| row.get(0)
        )?;
        Ok(count as u64)
    }

    /// Total tokens saved across all observations (sum of original - compressed).
    pub fn total_tokens_saved(&self) -> Result<u64> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let saved: i64 = conn.query_row(
            "SELECT COALESCE(SUM(original_tokens - compressed_tokens), 0) FROM observations",
            [],
            |row| row.get(0),
        )?;
        Ok(if saved > 0 { saved as u64 } else { 0 })
    }

    /// Average compression ratio across all observations.
    pub fn avg_compression_ratio(&self) -> Result<f64> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let avg: f64 = conn.query_row(
            "SELECT COALESCE(AVG(compression_ratio), 0.0) FROM observations",
            [],
            |row| row.get(0),
        )?;
        Ok(avg)
    }

    /// Remove old low-relevance observations. Returns count of pruned rows.
    pub fn prune(&self, older_than_days: u64) -> Result<u64> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let cutoff = Utc::now() - chrono::Duration::days(older_than_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        let deleted = conn.execute(
            "DELETE FROM observations WHERE created_at < ?1 AND relevance_score < 0.6",
            params![cutoff_str],
        )?;
        debug!("Pruned {} old low-relevance observations (older than {} days)", deleted, older_than_days);
        Ok(deleted as u64)
    }

    /// Store an observation to SQLite.
    fn store(&self, obs: &Observation) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let tags_json = serde_json::to_string(&obs.tags)?;
        conn.execute(
            "INSERT INTO observations (id, content, source_session_id, message_count,
                original_tokens, compressed_tokens, compression_ratio, tags_json,
                relevance_score, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                obs.id,
                obs.content,
                obs.source_session_id,
                obs.message_count as i64,
                obs.original_token_estimate as i64,
                obs.compressed_token_estimate as i64,
                obs.compression_ratio,
                tags_json,
                obs.relevance_score,
                obs.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }
}

// ── Extraction Engine ─────────────────────────────────────────────────────────

/// Extract structured items from conversation text using regex patterns.
fn extract_items(text: &str) -> Vec<ExtractedItem> {
    let mut items = Vec::new();

    // Decision patterns
    let decision_patterns = [
        r"(?i)(?:decided\s+to|will\s+use|chose\s+to|going\s+with|decision\s*:\s*)(.{10,200})",
        r"(?i)(?:we(?:'ll|\s+will)\s+go\s+with|selected|picked|opting\s+for)\s+(.{10,200})",
    ];
    for pat in &decision_patterns {
        extract_by_pattern(text, pat, ObservationCategory::Decision, &mut items);
    }

    // Problem patterns
    let problem_patterns = [
        r"(?i)(?:issue\s*:\s*|bug\s*:\s*|error\s*:\s*|problem\s*:\s*|failed\s+to)\s*(.{10,200})",
        r"(?i)(?:broken|crash(?:ing|ed)|doesn't\s+work|not\s+working|failing)\s*[:\-]?\s*(.{10,200})",
    ];
    for pat in &problem_patterns {
        extract_by_pattern(text, pat, ObservationCategory::Problem, &mut items);
    }

    // Solution patterns
    let solution_patterns = [
        r"(?i)(?:fixed\s+by|resolved\s+by|solution\s*:\s*|workaround\s*:\s*)(.{10,200})",
        r"(?i)(?:the\s+fix\s+(?:is|was)|solved\s+by|resolved\s+(?:it\s+)?by)\s+(.{10,200})",
    ];
    for pat in &solution_patterns {
        extract_by_pattern(text, pat, ObservationCategory::Solution, &mut items);
    }

    // Action patterns
    let action_patterns = [
        r"(?i)(?:TODO\s*:\s*|need\s+to|must\s+|should\s+|next\s+step\s*:\s*)(.{10,200})",
        r"(?i)(?:action\s+item\s*:\s*|plan\s+to|going\s+to)\s+(.{10,200})",
    ];
    for pat in &action_patterns {
        extract_by_pattern(text, pat, ObservationCategory::Action, &mut items);
    }

    // Fact patterns: numbers, URLs, file paths
    let fact_patterns = [
        r"(?i)(?:measured|benchmark|result|achieved|scored|rate)\s*[:\-]?\s*(\d[\d,\.]*\s*(?:%|ms|s|MB|GB|tokens|requests|items|x))",
        r#"(https?://[^\s\)"']{10,150})"#,
        r"(?:src/|\.rs|\.toml|\.json|\.py|\.ts)[^\s]{3,100}",
    ];
    for pat in &fact_patterns {
        extract_by_pattern(text, pat, ObservationCategory::Fact, &mut items);
    }

    // Deduplicate by text similarity (simple: exact match)
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| {
        let key = item.text.to_lowercase();
        seen.insert(key)
    });

    items
}

/// Extract matches for a single regex pattern and append to items.
fn extract_by_pattern(
    text: &str,
    pattern: &str,
    category: ObservationCategory,
    items: &mut Vec<ExtractedItem>,
) {
    if let Ok(re) = Regex::new(pattern) {
        for caps in re.captures_iter(text) {
            if let Some(m) = caps.get(1) {
                let val = m.as_str().trim();
                if val.len() >= 5 {
                    items.push(ExtractedItem {
                        category,
                        text: truncate_str(val, 200),
                    });
                }
            }
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Truncate string to max chars (safe for UTF-8).
fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Capitalize first letter of a string.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => format!("{}{}", c.to_uppercase(), chars.collect::<String>()),
    }
}

/// Map a SQLite row to an Observation struct.
fn row_to_observation(row: &rusqlite::Row) -> rusqlite::Result<Observation> {
    let tags_str: String = row.get(7)?;
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    let ts_str: String = row.get(9)?;
    Ok(Observation {
        id: row.get(0)?,
        content: row.get(1)?,
        source_session_id: row.get(2)?,
        message_count: row.get::<_, i64>(3)? as usize,
        original_token_estimate: row.get::<_, i64>(4)? as u64,
        compressed_token_estimate: row.get::<_, i64>(5)? as u64,
        compression_ratio: row.get(6)?,
        tags,
        relevance_score: row.get(8)?,
        created_at: DateTime::parse_from_rfc3339(&ts_str)
            .map(|d| d.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now()),
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("clawtex_test_obs_memory");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    fn make_msgs(pairs: &[(&str, &str)]) -> Vec<ConversationMessage> {
        pairs
            .iter()
            .map(|(role, content)| ConversationMessage {
                role: role.to_string(),
                content: content.to_string(),
                timestamp: None,
            })
            .collect()
    }

    #[test]
    fn test_observe_decision() {
        let (db_str, db_path) = temp_db("obs_decision");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let msgs = make_msgs(&[
            ("user", "What database should we use?"),
            ("assistant", "After comparing options, I decided to use SQLite for all persistence layers."),
        ]);
        let obs = mem.observe("sess-1", &msgs).unwrap();
        assert!(obs.content.to_lowercase().contains("decision"));
        assert!(obs.content.to_lowercase().contains("sqlite"));
        assert!(obs.relevance_score >= 0.9);
        assert!(obs.compression_ratio > 1.0);
        assert_eq!(obs.message_count, 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_observe_problem() {
        let (db_str, db_path) = temp_db("obs_problem");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let msgs = make_msgs(&[
            ("user", "The build is broken"),
            ("assistant", "Error: the TOML parser failed to handle triple-quoted strings correctly"),
        ]);
        let obs = mem.observe("sess-2", &msgs).unwrap();
        assert!(obs.content.to_lowercase().contains("problem") || obs.content.to_lowercase().contains("toml"));
        assert!(obs.relevance_score >= 0.7);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_observe_solution() {
        let (db_str, db_path) = temp_db("obs_solution");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let msgs = make_msgs(&[
            ("user", "How did you fix the crash?"),
            ("assistant", "Fixed by dropping the Arc clone before the tokio spawn to avoid a double-free."),
        ]);
        let obs = mem.observe("sess-3", &msgs).unwrap();
        assert!(obs.content.to_lowercase().contains("solution"));
        assert!(obs.relevance_score >= 0.8);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_observe_action_items() {
        let (db_str, db_path) = temp_db("obs_action");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let msgs = make_msgs(&[
            ("assistant", "TODO: implement the retry logic for rate-limited providers"),
            ("user", "Also need to add integration tests for the new endpoints"),
        ]);
        let obs = mem.observe("sess-4", &msgs).unwrap();
        assert!(obs.content.to_lowercase().contains("action"));
        assert!(obs.tags.contains(&"action".to_string()));
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_observe_facts_with_numbers() {
        let (db_str, db_path) = temp_db("obs_facts");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let msgs = make_msgs(&[
            ("assistant", "Benchmark results: achieved 99% success rate with the new approach. Measured 42ms latency."),
        ]);
        let obs = mem.observe("sess-5", &msgs).unwrap();
        assert!(obs.content.contains("99%") || obs.content.contains("42ms"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_observe_empty_messages_error() {
        let (db_str, db_path) = temp_db("obs_empty");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let result = mem.observe("sess-x", &[]);
        assert!(result.is_err());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_observe_fallback_no_patterns() {
        let (db_str, db_path) = temp_db("obs_fallback");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let msgs = make_msgs(&[
            ("user", "Hello there, just chatting about nothing structured at all in particular today"),
        ]);
        let obs = mem.observe("sess-6", &msgs).unwrap();
        // Should still create an observation (fallback)
        assert!(!obs.content.is_empty());
        assert_eq!(obs.relevance_score, 0.3); // Low default
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_compression_ratio() {
        let (db_str, db_path) = temp_db("obs_ratio");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        // Long conversation with a single decision
        let msgs = make_msgs(&[
            ("user", "We have been discussing the architecture for hours. Let me summarize the options we considered: option A with PostgreSQL, option B with MongoDB, option C with Redis, and option D with SQLite."),
            ("assistant", "After careful analysis of all four options considering our requirements for simplicity, zero configuration, and single-machine deployment, I decided to use SQLite for all persistence."),
            ("user", "Great choice. What about the caching layer?"),
            ("assistant", "For caching we will use in-memory HashMap with TTL eviction. This avoids adding another dependency while meeting our latency requirements."),
        ]);
        let obs = mem.observe("sess-7", &msgs).unwrap();
        assert!(obs.compression_ratio > 1.0);
        assert!(obs.original_token_estimate > obs.compressed_token_estimate);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_recall_by_keyword() {
        let (db_str, db_path) = temp_db("obs_recall");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        mem.observe("s1", &make_msgs(&[
            ("assistant", "I decided to use SQLite for the database backend"),
        ])).unwrap();
        mem.observe("s2", &make_msgs(&[
            ("assistant", "Error: the Redis connection timed out after 30s retry"),
        ])).unwrap();

        let results = mem.recall("sqlite", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.to_lowercase().contains("sqlite"));

        let results = mem.recall("redis", 10).unwrap();
        assert_eq!(results.len(), 1);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_recall_recent() {
        let (db_str, db_path) = temp_db("obs_recent");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        for i in 0..5 {
            mem.observe(
                &format!("s{}", i),
                &make_msgs(&[("user", &format!("Message number {} with some decided to use option {}", i, i))]),
            ).unwrap();
        }
        let recent = mem.recall_recent(3).unwrap();
        assert_eq!(recent.len(), 3);
        // Most recent should be last inserted
        assert_eq!(recent[0].source_session_id, "s4");
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_count() {
        let (db_str, db_path) = temp_db("obs_count");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        assert_eq!(mem.count().unwrap(), 0);
        mem.observe("s1", &make_msgs(&[("user", "decided to use approach alpha for the new system")])).unwrap();
        assert_eq!(mem.count().unwrap(), 1);
        mem.observe("s2", &make_msgs(&[("user", "decided to use approach beta for the old system")])).unwrap();
        assert_eq!(mem.count().unwrap(), 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_total_tokens_saved() {
        let (db_str, db_path) = temp_db("obs_saved");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        assert_eq!(mem.total_tokens_saved().unwrap(), 0);
        // Create observation from longer text
        let long_msg = "x ".repeat(400); // ~800 chars = ~200 tokens original
        let msgs = make_msgs(&[("user", &format!("{} decided to use the new framework for everything", long_msg))]);
        mem.observe("s1", &msgs).unwrap();
        let saved = mem.total_tokens_saved().unwrap();
        assert!(saved > 0);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_prune_keeps_recent() {
        let (db_str, db_path) = temp_db("obs_prune");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        // Insert observations with low relevance (just created)
        mem.observe("s1", &make_msgs(&[("user", "Just a casual chat about random things without any structure at all")])).unwrap();
        mem.observe("s2", &make_msgs(&[("user", "Another casual conversation about nothing in particular today")])).unwrap();
        assert_eq!(mem.count().unwrap(), 2);
        // Prune with 30 days cutoff should NOT delete brand-new items
        let pruned = mem.prune(30).unwrap();
        assert_eq!(pruned, 0);
        assert_eq!(mem.count().unwrap(), 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_prune_deletes_old_low_relevance() {
        let (db_str, db_path) = temp_db("obs_prune_old");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        // Insert observations with low relevance
        mem.observe("s1", &make_msgs(&[("user", "Just a casual chat about random things without any structure at all")])).unwrap();
        assert_eq!(mem.count().unwrap(), 1);
        // Prune with 0 days cutoff should delete items (cutoff = now, items created slightly before)
        let pruned = mem.prune(0).unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(mem.count().unwrap(), 0);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_inject_context() {
        let (db_str, db_path) = temp_db("obs_inject");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        mem.observe("s1", &make_msgs(&[
            ("assistant", "I decided to use SQLite for all persistence layers in the project"),
        ])).unwrap();
        mem.observe("s2", &make_msgs(&[
            ("assistant", "Fixed by adding a retry wrapper around the HTTP client calls"),
        ])).unwrap();

        let ctx = mem.inject_context("sqlite", 500).unwrap();
        assert!(ctx.contains("## Previous Observations"));
        assert!(ctx.contains("SQLite") || ctx.contains("sqlite"));
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_inject_context_max_tokens_limit() {
        let (db_str, db_path) = temp_db("obs_inject_limit");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        for i in 0..20 {
            mem.observe(
                &format!("s{}", i),
                &make_msgs(&[("assistant", &format!("Decided to use approach {} for the new distributed system architecture design", i))]),
            ).unwrap();
        }
        // Request very small max_tokens
        let ctx = mem.inject_context("approach", 20).unwrap();
        // Should have at most a few lines due to token limit
        let line_count = ctx.lines().count();
        assert!(line_count <= 5); // header + a few lines max
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_observation_serialization() {
        let obs = Observation {
            id: "test-id".to_string(),
            content: "Decision: Use SQLite".to_string(),
            source_session_id: "sess-1".to_string(),
            message_count: 3,
            original_token_estimate: 1000,
            compressed_token_estimate: 50,
            compression_ratio: 20.0,
            tags: vec!["decision".to_string()],
            relevance_score: 1.0,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&obs).unwrap();
        assert!(json.contains("test-id"));
        assert!(json.contains("SQLite"));
        let back: Observation = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "test-id");
        assert_eq!(back.compression_ratio, 20.0);
    }

    #[test]
    fn test_conversation_message_serialization() {
        let msg = ConversationMessage {
            role: "user".to_string(),
            content: "Hello world".to_string(),
            timestamp: Some("2026-03-17T10:00:00Z".to_string()),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ConversationMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.role, "user");
        assert_eq!(back.timestamp, Some("2026-03-17T10:00:00Z".to_string()));
    }

    #[test]
    fn test_multiple_categories_in_one_conversation() {
        let (db_str, db_path) = temp_db("obs_multi_cat");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        let msgs = make_msgs(&[
            ("user", "What's the status?"),
            ("assistant", "Problem: the API is returning 500 errors intermittently"),
            ("assistant", "Fixed by adding connection pooling with a max of 10 connections"),
            ("assistant", "I decided to use the r2d2 crate for connection pool management"),
            ("assistant", "TODO: add monitoring alerts for pool exhaustion events"),
        ]);
        let obs = mem.observe("sess-multi", &msgs).unwrap();
        // Should have multiple category tags
        assert!(obs.tags.len() >= 2);
        // Relevance should be high (decision present)
        assert!(obs.relevance_score >= 0.9);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_avg_compression_ratio() {
        let (db_str, db_path) = temp_db("obs_avg");
        let mem = ObservationalMemory::new(&db_str).unwrap();
        assert_eq!(mem.avg_compression_ratio().unwrap(), 0.0);
        let long_text = "a ".repeat(200);
        mem.observe("s1", &make_msgs(&[
            ("user", &format!("{} decided to use option alpha for the project", long_text)),
        ])).unwrap();
        let avg = mem.avg_compression_ratio().unwrap();
        assert!(avg > 1.0);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_extract_items_decision() {
        let items = extract_items("We decided to use Rust for the rewrite because of safety.");
        assert!(!items.is_empty());
        assert!(items.iter().any(|i| i.category == ObservationCategory::Decision));
    }

    #[test]
    fn test_extract_items_url_fact() {
        let items = extract_items("Check the docs at https://docs.rs/rusqlite/latest for more info.");
        assert!(items.iter().any(|i| i.category == ObservationCategory::Fact));
    }

    #[test]
    fn test_truncate_str_safe() {
        assert_eq!(truncate_str("hello", 10), "hello");
        let long = "a".repeat(300);
        let t = truncate_str(&long, 200);
        assert!(t.len() <= 204); // 200 + "..."
        assert!(t.ends_with("..."));
    }

    #[test]
    fn test_capitalize() {
        assert_eq!(capitalize("decision"), "Decision");
        assert_eq!(capitalize(""), "");
        assert_eq!(capitalize("a"), "A");
    }
}
