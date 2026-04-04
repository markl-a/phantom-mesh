//! Knowledge Capture — extracts structured knowledge from hand outputs.
//! Uses regex-based extraction (no LLM calls). Persists to SQLite.

use anyhow::Result;
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tracing::debug;

/// A single unit of captured knowledge
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeNode {
    pub id: String,
    pub hand_name: String,
    pub phase_name: String,
    pub problem: Option<String>,
    pub decision: Option<String>,
    pub result: Option<String>,
    pub lesson: Option<String>,
    pub confidence: f32,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Knowledge capturer with SQLite persistence
pub struct KnowledgeCapturer {
    #[allow(dead_code)]
    db_path: String,
    conn: std::sync::Mutex<rusqlite::Connection>,
}

impl KnowledgeCapturer {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS knowledge_nodes (
                id TEXT PRIMARY KEY,
                hand_name TEXT NOT NULL,
                phase_name TEXT NOT NULL,
                problem TEXT,
                decision TEXT,
                result TEXT,
                lesson TEXT,
                confidence REAL NOT NULL DEFAULT 0.5,
                tags TEXT NOT NULL DEFAULT '[]',
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_knowledge_hand ON knowledge_nodes(hand_name);
            CREATE INDEX IF NOT EXISTS idx_knowledge_tags ON knowledge_nodes(tags);"
        )?;
        Ok(Self { db_path: db_path.to_string(), conn: std::sync::Mutex::new(conn) })
    }

    /// Extract knowledge from a hand phase output using regex patterns.
    /// Returns extracted KnowledgeNodes (may be empty if no patterns match).
    pub fn capture_from_output(
        &self,
        hand_name: &str,
        phase_name: &str,
        _prompt: &str,
        output: &str,
    ) -> Result<Vec<KnowledgeNode>> {
        let problem = extract_field(output, &[r"(?i)(?:problem|issue|challenge)\s*:\s*(.+)"]);
        let decision = extract_field(output, &[r"(?i)(?:decided|chose|selected|approach)\s*:\s*(.+)"]);
        let result_text = extract_field(output, &[r"(?i)(?:result|outcome|achieved|generated)\s*:\s*(.+)"]);
        let lesson = extract_field(output, &[r"(?i)(?:lesson|learned|takeaway|note)\s*:\s*(.+)"]);

        // Calculate confidence based on how many fields were extracted
        let field_count = [&problem, &decision, &result_text, &lesson]
            .iter()
            .filter(|f| f.is_some())
            .count();

        if field_count == 0 {
            // Fallback: store entire output as result if non-empty
            if output.trim().len() > 20 {
                let node = KnowledgeNode {
                    id: uuid::Uuid::new_v4().to_string(),
                    hand_name: hand_name.to_string(),
                    phase_name: phase_name.to_string(),
                    problem: None,
                    decision: None,
                    result: Some(truncate_str(output.trim(), 2000)),
                    lesson: None,
                    confidence: 0.2, // Low confidence for raw fallback
                    tags: vec![hand_name.to_string(), phase_name.to_string()],
                    created_at: Utc::now(),
                };
                self.store(&node)?;
                return Ok(vec![node]);
            }
            return Ok(vec![]);
        }

        let confidence = match field_count {
            1 => 0.4,
            2 => 0.6,
            3 => 0.8,
            4 => 1.0,
            _ => 0.5,
        };

        let node = KnowledgeNode {
            id: uuid::Uuid::new_v4().to_string(),
            hand_name: hand_name.to_string(),
            phase_name: phase_name.to_string(),
            problem,
            decision,
            result: result_text,
            lesson,
            confidence: confidence as f32,
            tags: vec![hand_name.to_string(), phase_name.to_string()],
            created_at: Utc::now(),
        };

        self.store(&node)?;
        debug!(
            "Knowledge captured from {}/{}: confidence={:.1}",
            hand_name, phase_name, confidence
        );
        Ok(vec![node])
    }

    /// Store a knowledge node to SQLite
    fn store(&self, node: &KnowledgeNode) -> Result<()> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let tags_json = serde_json::to_string(&node.tags)?;
        conn.execute(
            "INSERT INTO knowledge_nodes (id, hand_name, phase_name, problem, decision, result, lesson, confidence, tags, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                node.id,
                node.hand_name,
                node.phase_name,
                node.problem,
                node.decision,
                node.result,
                node.lesson,
                node.confidence,
                tags_json,
                node.created_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Recall relevant knowledge by searching tags.
    /// Returns nodes matching any of the given tags, ordered by confidence desc.
    pub fn recall_relevant(&self, tags: &[&str], limit: usize) -> Result<Vec<KnowledgeNode>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        // Build LIKE conditions for tag matching
        let conditions: Vec<String> = tags.iter()
            .map(|t| format!("tags LIKE '%\"{}%'", t.replace('\'', "''")))
            .collect();
        if conditions.is_empty() {
            return Ok(vec![]);
        }
        let where_clause = conditions.join(" OR ");
        let query = format!(
            "SELECT id, hand_name, phase_name, problem, decision, result, lesson, confidence, tags, created_at
             FROM knowledge_nodes WHERE {} ORDER BY confidence DESC LIMIT {}",
            where_clause, limit
        );
        let mut stmt = conn.prepare(&query)?;
        let nodes = stmt.query_map([], |row| {
            let tags_str: String = row.get(8)?;
            let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
            let ts_str: String = row.get(9)?;
            Ok(KnowledgeNode {
                id: row.get(0)?,
                hand_name: row.get(1)?,
                phase_name: row.get(2)?,
                problem: row.get(3)?,
                decision: row.get(4)?,
                result: row.get(5)?,
                lesson: row.get(6)?,
                confidence: row.get(7)?,
                tags,
                created_at: DateTime::parse_from_rfc3339(&ts_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?.filter_map(|r| r.ok()).collect();
        Ok(nodes)
    }

    /// Count total knowledge nodes
    pub fn count(&self) -> Result<u64> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM knowledge_nodes", [], |row| row.get(0)
        )?;
        Ok(count as u64)
    }

    /// Get all knowledge for a specific hand
    pub fn by_hand(&self, hand_name: &str, limit: usize) -> Result<Vec<KnowledgeNode>> {
        self.recall_relevant(&[hand_name], limit)
    }
}

/// Extract a field from text using multiple regex patterns.
/// Returns the first match found.
fn extract_field(text: &str, patterns: &[&str]) -> Option<String> {
    for pat in patterns {
        if let Ok(re) = Regex::new(pat) {
            if let Some(caps) = re.captures(text) {
                if let Some(m) = caps.get(1) {
                    let val = m.as_str().trim().to_string();
                    if !val.is_empty() {
                        return Some(truncate_str(&val, 500));
                    }
                }
            }
        }
    }
    None
}

/// Truncate string to max chars (safe for UTF-8)
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("phantom_mesh_test_knowledge");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    #[test]
    fn test_capture_with_all_fields() {
        let (db_str, db_path) = temp_db("all_fields");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        let output = "Problem: API rate limit hit\nDecided: Use exponential backoff\nResult: 99% success rate\nLesson: Always add retry logic";
        let nodes = cap.capture_from_output("test_hand", "phase1", "prompt", output).unwrap();
        assert_eq!(nodes.len(), 1);
        let n = &nodes[0];
        assert!(n.problem.as_ref().unwrap().contains("rate limit"));
        assert!(n.decision.as_ref().unwrap().contains("backoff"));
        assert!(n.result.as_ref().unwrap().contains("99%"));
        assert!(n.lesson.as_ref().unwrap().contains("retry"));
        assert_eq!(n.confidence, 1.0);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_capture_partial_fields() {
        let (db_str, db_path) = temp_db("partial");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        let output = "Issue: Memory leak in worker\nOutcome: Fixed by dropping Arc clone";
        let nodes = cap.capture_from_output("debug", "fix", "prompt", output).unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].problem.is_some());
        assert!(nodes[0].result.is_some());
        assert_eq!(nodes[0].confidence, 0.6); // 2 fields
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_capture_fallback_raw_output() {
        let (db_str, db_path) = temp_db("fallback");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        let output = "This is a long output without any structured fields that should be stored as raw result data for future reference.";
        let nodes = cap.capture_from_output("hand", "phase", "prompt", output).unwrap();
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].result.is_some());
        assert_eq!(nodes[0].confidence, 0.2); // Fallback
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_capture_empty_output() {
        let (db_str, db_path) = temp_db("empty");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        let nodes = cap.capture_from_output("hand", "phase", "prompt", "short").unwrap();
        assert!(nodes.is_empty()); // Too short for fallback
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_recall_by_tags() {
        let (db_str, db_path) = temp_db("recall");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        cap.capture_from_output("content", "write", "p", "Problem: SEO ranking low\nDecided: Use long-tail keywords").unwrap();
        cap.capture_from_output("seo", "analyze", "p", "Challenge: Competitor analysis\nResult: Found 5 gaps").unwrap();

        let results = cap.recall_relevant(&["content"], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].hand_name, "content");

        let all = cap.recall_relevant(&["content", "seo"], 10).unwrap();
        assert_eq!(all.len(), 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_recall_empty_tags() {
        let (db_str, db_path) = temp_db("empty_tags");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        let results = cap.recall_relevant(&[], 10).unwrap();
        assert!(results.is_empty());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_count() {
        let (db_str, db_path) = temp_db("count");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        assert_eq!(cap.count().unwrap(), 0);
        cap.capture_from_output("h", "p", "pr", "Problem: test\nResult: ok").unwrap();
        assert_eq!(cap.count().unwrap(), 1);
        cap.capture_from_output("h2", "p2", "pr", "Issue: bug\nOutcome: fixed").unwrap();
        assert_eq!(cap.count().unwrap(), 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_by_hand() {
        let (db_str, db_path) = temp_db("by_hand");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        cap.capture_from_output("content", "p1", "pr", "Problem: slow gen\nResult: cached").unwrap();
        cap.capture_from_output("content", "p2", "pr", "Issue: format\nOutcome: fixed").unwrap();
        cap.capture_from_output("seo", "p1", "pr", "Challenge: rank\nResult: improved").unwrap();

        let content_nodes = cap.by_hand("content", 10).unwrap();
        assert_eq!(content_nodes.len(), 2);
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_extract_field_patterns() {
        assert!(extract_field("Problem: memory leak", &[r"(?i)(?:problem|issue)\s*:\s*(.+)"]).is_some());
        assert!(extract_field("No match here", &[r"(?i)problem\s*:\s*(.+)"]).is_none());
    }

    #[test]
    fn test_truncate_str() {
        assert_eq!(truncate_str("hello", 10), "hello");
        let long = "a".repeat(600);
        let t = truncate_str(&long, 500);
        assert!(t.len() <= 504); // 500 + "..."
        assert!(t.ends_with("..."));
    }

    #[test]
    fn test_knowledge_node_serialize() {
        let node = KnowledgeNode {
            id: "test-id".to_string(),
            hand_name: "content".to_string(),
            phase_name: "write".to_string(),
            problem: Some("slow".to_string()),
            decision: None,
            result: Some("fast now".to_string()),
            lesson: None,
            confidence: 0.6,
            tags: vec!["content".to_string()],
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&node).unwrap();
        assert!(json.contains("content"));
        let back: KnowledgeNode = serde_json::from_str(&json).unwrap();
        assert_eq!(back.hand_name, "content");
    }

    #[test]
    fn test_recall_ordered_by_confidence() {
        let (db_str, db_path) = temp_db("ordered");
        let cap = KnowledgeCapturer::new(&db_str).unwrap();
        // Low confidence (fallback)
        cap.capture_from_output("test", "p1", "pr", "This is just some raw text without any specific structured patterns at all").unwrap();
        // High confidence (all fields)
        cap.capture_from_output("test", "p2", "pr", "Problem: X\nDecided: Y\nResult: Z\nLesson: W").unwrap();

        let results = cap.recall_relevant(&["test"], 10).unwrap();
        assert!(results.len() >= 2);
        // First result should have higher confidence
        assert!(results[0].confidence >= results[1].confidence);
        let _ = std::fs::remove_file(&db_path);
    }
}
