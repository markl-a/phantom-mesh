//! Cross-device consistency test framework.
//! Sends the same prompt to multiple workers and measures output consistency
//! across factual content, output format, and tool usage.
//!
//! Results are persisted to SQLite (`~/.clawtex/consistency.db`) for tracking over time.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tracing::{debug, info, warn};

use crate::cluster_hub::ClusterHub;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default similarity threshold for a "pass"
const DEFAULT_THRESHOLD: f64 = 0.90;

/// Predefined test suite — 10 standard prompts covering different task types
pub const PREDEFINED_TEST_SUITE: &[(&str, &str)] = &[
    ("factual_capital", "What is the capital of France? Answer in one sentence."),
    ("factual_math", "What is 17 * 23? Show the calculation and answer."),
    ("format_json", "List 3 programming languages with their year of creation in JSON format."),
    ("format_markdown", "Create a markdown table with 3 columns: Name, Type, Description for 3 common data structures."),
    ("tool_web_search", "Search the web for the current weather in Tokyo and summarize."),
    ("tool_http_request", "Make an HTTP GET request to https://httpbin.org/get and report the response status."),
    ("reasoning_compare", "Compare Python and Rust for building web servers. List 3 pros and cons for each."),
    ("creative_short", "Write a one-paragraph summary of what a distributed computing cluster does."),
    ("instruction_follow", "List exactly 5 benefits of unit testing, numbered 1-5, one per line."),
    ("code_generate", "Write a function in Python that checks if a string is a palindrome. Include a docstring."),
];

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Result from a single worker for a given prompt
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerResult {
    pub worker: String,
    pub output: String,
    pub tool_calls: Vec<String>,
    pub latency_ms: u64,
    pub success: bool,
    pub error: Option<String>,
}

/// Pairwise similarity detail between two workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairSimilarity {
    pub worker_a: String,
    pub worker_b: String,
    pub factual_similarity: f64,
    pub format_similarity: f64,
    pub tool_similarity: f64,
    pub overall: f64,
}

/// Full consistency report for a single prompt across workers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsistencyReport {
    pub id: String,
    pub prompt: String,
    pub category: String,
    pub results: Vec<WorkerResult>,
    pub similarity_matrix: Vec<PairSimilarity>,
    pub avg_similarity: f64,
    pub factual_avg: f64,
    pub format_avg: f64,
    pub tool_avg: f64,
    pub pass: bool,
    pub threshold: f64,
    pub created_at: DateTime<Utc>,
}

/// Batch consistency report summary
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSummary {
    pub total_prompts: usize,
    pub passed: usize,
    pub failed: usize,
    pub avg_similarity: f64,
    pub reports: Vec<ConsistencyReport>,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// ConsistencyTester
// ---------------------------------------------------------------------------

/// Cross-device consistency tester.
/// Dispatches the same prompt to multiple workers and compares outputs.
pub struct ConsistencyTester {
    db_path: String,
    threshold: f64,
}

impl ConsistencyTester {
    /// Create a new ConsistencyTester with SQLite persistence.
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS consistency_reports (
                id TEXT PRIMARY KEY,
                prompt TEXT NOT NULL,
                category TEXT NOT NULL DEFAULT '',
                results_json TEXT NOT NULL,
                similarity_matrix_json TEXT NOT NULL,
                avg_similarity REAL NOT NULL,
                factual_avg REAL NOT NULL DEFAULT 0.0,
                format_avg REAL NOT NULL DEFAULT 0.0,
                tool_avg REAL NOT NULL DEFAULT 0.0,
                pass INTEGER NOT NULL,
                threshold REAL NOT NULL,
                workers_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_consistency_date ON consistency_reports(created_at);
            CREATE INDEX IF NOT EXISTS idx_consistency_pass ON consistency_reports(pass);
            CREATE INDEX IF NOT EXISTS idx_consistency_category ON consistency_reports(category);"
        )?;
        Ok(Self {
            db_path: db_path.to_string(),
            threshold: DEFAULT_THRESHOLD,
        })
    }

    /// Set a custom pass/fail threshold (0.0 - 1.0).
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Run a single consistency test: send the same prompt to all specified workers
    /// and compare their outputs.
    pub async fn run_test(
        &self,
        prompt: &str,
        workers: Vec<String>,
        hub: &ClusterHub,
    ) -> Result<ConsistencyReport> {
        self.run_test_with_category(prompt, "", workers, hub).await
    }

    /// Run a single consistency test with a category label.
    pub async fn run_test_with_category(
        &self,
        prompt: &str,
        category: &str,
        workers: Vec<String>,
        hub: &ClusterHub,
    ) -> Result<ConsistencyReport> {
        if workers.len() < 2 {
            return Err(anyhow!("Need at least 2 workers for consistency test, got {}", workers.len()));
        }

        info!(
            "Starting consistency test: prompt='{}' across {} workers",
            truncate_str(prompt, 60),
            workers.len()
        );

        // Dispatch to all workers in parallel
        let mut handles = Vec::with_capacity(workers.len());
        for worker_name in &workers {
            let hub_registry = hub.registry.clone();
            let hub_client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .unwrap_or_default();
            let worker_name = worker_name.clone();
            let prompt_owned = prompt.to_string();

            handles.push(tokio::spawn(async move {
                let start = std::time::Instant::now();

                // Get the worker node info
                let worker_node = hub_registry.get_node(&worker_name).await;
                let node = match worker_node {
                    Some(n) if n.status == "online" => n,
                    Some(_) => {
                        return WorkerResult {
                            worker: worker_name,
                            output: String::new(),
                            tool_calls: vec![],
                            latency_ms: 0,
                            success: false,
                            error: Some("Worker is offline".to_string()),
                        };
                    }
                    None => {
                        return WorkerResult {
                            worker: worker_name,
                            output: String::new(),
                            tool_calls: vec![],
                            latency_ms: 0,
                            success: false,
                            error: Some("Worker not found".to_string()),
                        };
                    }
                };

                // For push workers, dispatch via HTTP
                // For mobile workers, we use the agent task path
                let url = format!("http://{}:{}/worker/execute", node.host, node.port);
                let payload = json!({
                    "tool": "delegate_to_provider",
                    "input": {
                        "prompt": prompt_owned,
                        "provider": "auto"
                    }
                });

                let result = hub_client
                    .post(&url)
                    .json(&payload)
                    .send()
                    .await;

                let latency_ms = start.elapsed().as_millis() as u64;

                match result {
                    Ok(response) if response.status().is_success() => {
                        match response.json::<Value>().await {
                            Ok(body) => {
                                let output = body.get("output")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let tool_calls = body.get("tool_calls")
                                    .and_then(|v| v.as_array())
                                    .map(|arr| arr.iter()
                                        .filter_map(|v| v.as_str().map(String::from))
                                        .collect())
                                    .unwrap_or_default();
                                let success = body.get("success")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(true);
                                WorkerResult {
                                    worker: worker_name,
                                    output,
                                    tool_calls,
                                    latency_ms,
                                    success,
                                    error: None,
                                }
                            }
                            Err(e) => WorkerResult {
                                worker: worker_name,
                                output: String::new(),
                                tool_calls: vec![],
                                latency_ms,
                                success: false,
                                error: Some(format!("Failed to parse response: {}", e)),
                            },
                        }
                    }
                    Ok(response) => {
                        let status = response.status();
                        let body = response.text().await.unwrap_or_default();
                        WorkerResult {
                            worker: worker_name,
                            output: String::new(),
                            tool_calls: vec![],
                            latency_ms,
                            success: false,
                            error: Some(format!("HTTP {}: {}", status, body)),
                        }
                    }
                    Err(e) => WorkerResult {
                        worker: worker_name,
                        output: String::new(),
                        tool_calls: vec![],
                        latency_ms,
                        success: false,
                        error: Some(format!("Request failed: {}", e)),
                    },
                }
            }));
        }

        // Collect results
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            match handle.await {
                Ok(r) => results.push(r),
                Err(e) => {
                    warn!("Worker task join error: {}", e);
                    results.push(WorkerResult {
                        worker: "unknown".to_string(),
                        output: String::new(),
                        tool_calls: vec![],
                        latency_ms: 0,
                        success: false,
                        error: Some(format!("Join error: {}", e)),
                    });
                }
            }
        }

        // Only compare successful results
        let successful_count = results.iter().filter(|r| r.success).count();
        let total_results = results.len();

        let (similarity_matrix, avg_similarity, factual_avg, format_avg, tool_avg) = {
            let successful_refs: Vec<&WorkerResult> = results.iter().filter(|r| r.success).collect();
            if successful_refs.len() >= 2 {
                compute_similarity_matrix(&successful_refs)
            } else {
                (vec![], 0.0, 0.0, 0.0, 0.0)
            }
        };

        let pass = avg_similarity >= self.threshold;

        let report = ConsistencyReport {
            id: uuid::Uuid::new_v4().to_string(),
            prompt: prompt.to_string(),
            category: category.to_string(),
            results,
            similarity_matrix,
            avg_similarity,
            factual_avg,
            format_avg,
            tool_avg,
            pass,
            threshold: self.threshold,
            created_at: Utc::now(),
        };

        // Persist to SQLite
        if let Err(e) = self.store_report(&report) {
            warn!("Failed to store consistency report: {}", e);
        }

        info!(
            "Consistency test complete: avg_similarity={:.3}, pass={}, workers={}/{}",
            report.avg_similarity,
            report.pass,
            successful_count,
            total_results
        );

        Ok(report)
    }

    /// Run a batch of consistency tests.
    pub async fn run_batch(
        &self,
        prompts: Vec<String>,
        workers: Vec<String>,
        hub: &ClusterHub,
    ) -> Vec<ConsistencyReport> {
        info!("Starting batch consistency test: {} prompts x {} workers", prompts.len(), workers.len());
        let mut reports = Vec::with_capacity(prompts.len());
        for prompt in &prompts {
            match self.run_test(prompt, workers.clone(), hub).await {
                Ok(report) => reports.push(report),
                Err(e) => {
                    warn!("Consistency test failed for prompt '{}': {}", truncate_str(prompt, 40), e);
                }
            }
        }
        reports
    }

    /// Run a batch with category labels per prompt.
    pub async fn run_batch_with_categories(
        &self,
        prompts: Vec<(String, String)>,  // (category, prompt)
        workers: Vec<String>,
        hub: &ClusterHub,
    ) -> Vec<ConsistencyReport> {
        info!("Starting categorized batch: {} prompts x {} workers", prompts.len(), workers.len());
        let mut reports = Vec::with_capacity(prompts.len());
        for (category, prompt) in &prompts {
            match self.run_test_with_category(prompt, category, workers.clone(), hub).await {
                Ok(report) => reports.push(report),
                Err(e) => {
                    warn!("Consistency test failed for '{}': {}", category, e);
                }
            }
        }
        reports
    }

    /// Run the predefined test suite.
    pub async fn run_predefined_suite(
        &self,
        workers: Vec<String>,
        hub: &ClusterHub,
    ) -> BatchSummary {
        let prompts: Vec<(String, String)> = PREDEFINED_TEST_SUITE
            .iter()
            .map(|(cat, prompt)| (cat.to_string(), prompt.to_string()))
            .collect();

        let reports = self.run_batch_with_categories(prompts, workers, hub).await;
        build_batch_summary(reports)
    }

    /// Store a consistency report to SQLite.
    fn store_report(&self, report: &ConsistencyReport) -> Result<()> {
        let conn = Connection::open(&self.db_path)?;
        let results_json = serde_json::to_string(&report.results)?;
        let matrix_json = serde_json::to_string(&report.similarity_matrix)?;
        conn.execute(
            "INSERT INTO consistency_reports
             (id, prompt, category, results_json, similarity_matrix_json,
              avg_similarity, factual_avg, format_avg, tool_avg,
              pass, threshold, workers_count, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                report.id,
                report.prompt,
                report.category,
                results_json,
                matrix_json,
                report.avg_similarity,
                report.factual_avg,
                report.format_avg,
                report.tool_avg,
                report.pass as i32,
                report.threshold,
                report.results.len() as i32,
                report.created_at.to_rfc3339(),
            ],
        )?;
        debug!("Stored consistency report {}", report.id);
        Ok(())
    }

    /// Retrieve recent consistency reports from SQLite.
    pub fn recent_reports(&self, limit: usize) -> Result<Vec<ConsistencyReport>> {
        let conn = Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, prompt, category, results_json, similarity_matrix_json,
                    avg_similarity, factual_avg, format_avg, tool_avg,
                    pass, threshold, created_at
             FROM consistency_reports
             ORDER BY created_at DESC
             LIMIT ?1"
        )?;

        let reports = stmt.query_map(params![limit as i64], |row| {
            let results_str: String = row.get(3)?;
            let matrix_str: String = row.get(4)?;
            let pass_int: i32 = row.get(9)?;
            let ts_str: String = row.get(11)?;

            Ok(ConsistencyReport {
                id: row.get(0)?,
                prompt: row.get(1)?,
                category: row.get(2)?,
                results: serde_json::from_str(&results_str).unwrap_or_default(),
                similarity_matrix: serde_json::from_str(&matrix_str).unwrap_or_default(),
                avg_similarity: row.get(5)?,
                factual_avg: row.get(6)?,
                format_avg: row.get(7)?,
                tool_avg: row.get(8)?,
                pass: pass_int != 0,
                threshold: row.get(10)?,
                created_at: DateTime::parse_from_rfc3339(&ts_str)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?.filter_map(|r| r.ok()).collect();

        Ok(reports)
    }

    /// Get pass rate over the last N reports.
    pub fn pass_rate(&self, last_n: usize) -> Result<f64> {
        let reports = self.recent_reports(last_n)?;
        if reports.is_empty() {
            return Ok(0.0);
        }
        let passed = reports.iter().filter(|r| r.pass).count();
        Ok(passed as f64 / reports.len() as f64)
    }

    /// Get total report count.
    pub fn count(&self) -> Result<u64> {
        let conn = Connection::open(&self.db_path)?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM consistency_reports", [], |row| row.get(0)
        )?;
        Ok(count as u64)
    }

    /// Get a summary of historical consistency data.
    pub fn history_summary(&self) -> Result<Value> {
        let conn = Connection::open(&self.db_path)?;
        let total: i64 = conn.query_row(
            "SELECT COUNT(*) FROM consistency_reports", [], |row| row.get(0)
        )?;
        let passed: i64 = conn.query_row(
            "SELECT COUNT(*) FROM consistency_reports WHERE pass = 1", [], |row| row.get(0)
        )?;
        let avg_sim: f64 = conn.query_row(
            "SELECT COALESCE(AVG(avg_similarity), 0.0) FROM consistency_reports", [], |row| row.get(0)
        )?;
        let avg_factual: f64 = conn.query_row(
            "SELECT COALESCE(AVG(factual_avg), 0.0) FROM consistency_reports", [], |row| row.get(0)
        )?;
        let avg_format: f64 = conn.query_row(
            "SELECT COALESCE(AVG(format_avg), 0.0) FROM consistency_reports", [], |row| row.get(0)
        )?;
        let avg_tool: f64 = conn.query_row(
            "SELECT COALESCE(AVG(tool_avg), 0.0) FROM consistency_reports", [], |row| row.get(0)
        )?;

        Ok(json!({
            "total_tests": total,
            "passed": passed,
            "failed": total - passed,
            "pass_rate": if total > 0 { passed as f64 / total as f64 } else { 0.0 },
            "avg_similarity": avg_sim,
            "avg_factual": avg_factual,
            "avg_format": avg_format,
            "avg_tool": avg_tool,
        }))
    }
}

// ---------------------------------------------------------------------------
// Similarity computation
// ---------------------------------------------------------------------------

/// Compute pairwise similarity between all successful worker results.
/// Returns (matrix, overall_avg, factual_avg, format_avg, tool_avg).
fn compute_similarity_matrix(
    results: &[&WorkerResult],
) -> (Vec<PairSimilarity>, f64, f64, f64, f64) {
    let mut pairs = Vec::new();
    let mut overall_sum = 0.0;
    let mut factual_sum = 0.0;
    let mut format_sum = 0.0;
    let mut tool_sum = 0.0;
    let mut count = 0usize;

    for i in 0..results.len() {
        for j in (i + 1)..results.len() {
            let a = results[i];
            let b = results[j];

            let factual = factual_similarity(&a.output, &b.output);
            let format = format_similarity(&a.output, &b.output);
            let tool = tool_usage_similarity(&a.tool_calls, &b.tool_calls);

            // Weighted overall: factual 50%, format 30%, tool 20%
            let overall = factual * 0.5 + format * 0.3 + tool * 0.2;

            pairs.push(PairSimilarity {
                worker_a: a.worker.clone(),
                worker_b: b.worker.clone(),
                factual_similarity: factual,
                format_similarity: format,
                tool_similarity: tool,
                overall,
            });

            overall_sum += overall;
            factual_sum += factual;
            format_sum += format;
            tool_sum += tool;
            count += 1;
        }
    }

    let avg_overall = if count > 0 { overall_sum / count as f64 } else { 0.0 };
    let avg_factual = if count > 0 { factual_sum / count as f64 } else { 0.0 };
    let avg_format = if count > 0 { format_sum / count as f64 } else { 0.0 };
    let avg_tool = if count > 0 { tool_sum / count as f64 } else { 0.0 };

    (pairs, avg_overall, avg_factual, avg_format, avg_tool)
}

/// Factual consistency: Jaccard similarity on key phrases (word n-grams).
/// Extracts words, numbers, and short phrases. Higher overlap = more consistent facts.
fn factual_similarity(a: &str, b: &str) -> f64 {
    let tokens_a = extract_key_tokens(a);
    let tokens_b = extract_key_tokens(b);

    if tokens_a.is_empty() && tokens_b.is_empty() {
        return 1.0; // Both empty = identical
    }
    if tokens_a.is_empty() || tokens_b.is_empty() {
        return 0.0;
    }

    let set_a: HashSet<&str> = tokens_a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = tokens_b.iter().map(|s| s.as_str()).collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 { 1.0 } else { intersection as f64 / union as f64 }
}

/// Format consistency: compare structural patterns (line count, bullet points,
/// code blocks, JSON presence, heading counts, etc.).
fn format_similarity(a: &str, b: &str) -> f64 {
    let features_a = extract_format_features(a);
    let features_b = extract_format_features(b);

    let mut matches = 0.0;
    let mut total = 0.0;
    let feature_count = features_a.len().max(features_b.len());
    if feature_count == 0 {
        return 1.0;
    }

    for key in features_a.keys().chain(features_b.keys()).collect::<HashSet<_>>() {
        let va = features_a.get(key).copied().unwrap_or(0.0);
        let vb = features_b.get(key).copied().unwrap_or(0.0);
        let max_val = va.max(vb);
        if max_val > 0.0 {
            matches += va.min(vb) / max_val;
        } else {
            matches += 1.0; // Both zero = match
        }
        total += 1.0;
    }

    if total == 0.0 { 1.0 } else { matches / total }
}

/// Tool usage consistency: Jaccard similarity on tool names called.
fn tool_usage_similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0; // Neither used tools = consistent
    }
    if a.is_empty() || b.is_empty() {
        return 0.0; // One used tools, other didn't
    }

    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();

    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();

    if union == 0 { 1.0 } else { intersection as f64 / union as f64 }
}

// ---------------------------------------------------------------------------
// Feature extraction helpers
// ---------------------------------------------------------------------------

/// Extract key tokens (lowercase words, numbers, short phrases) for factual comparison.
fn extract_key_tokens(text: &str) -> Vec<String> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
        .filter(|w| w.len() >= 2)
        .collect();

    let mut tokens: Vec<String> = Vec::new();

    // Single words (unigrams)
    for w in &words {
        tokens.push(w.to_string());
    }

    // Bigrams (two consecutive words) for phrase-level comparison
    for pair in words.windows(2) {
        tokens.push(format!("{} {}", pair[0], pair[1]));
    }

    tokens
}

/// Extract structural features from text for format comparison.
fn extract_format_features(text: &str) -> HashMap<String, f64> {
    let mut features = HashMap::new();
    let lines: Vec<&str> = text.lines().collect();

    features.insert("line_count".to_string(), lines.len() as f64);
    features.insert("char_count".to_string(), text.len() as f64);
    features.insert("word_count".to_string(),
        text.split_whitespace().count() as f64);

    // Count bullet points (-, *, numbered lists)
    let bullet_count = lines.iter()
        .filter(|l| {
            let trimmed = l.trim();
            trimmed.starts_with("- ") || trimmed.starts_with("* ")
                || trimmed.starts_with("+ ")
                || (trimmed.len() > 2 && trimmed.chars().next().map_or(false, |c| c.is_ascii_digit())
                    && (trimmed.contains(". ") || trimmed.contains(") ")))
        })
        .count();
    features.insert("bullet_count".to_string(), bullet_count as f64);

    // Count markdown headings (#)
    let heading_count = lines.iter()
        .filter(|l| l.trim().starts_with('#'))
        .count();
    features.insert("heading_count".to_string(), heading_count as f64);

    // Code blocks (```)
    let code_block_count = text.matches("```").count() / 2;
    features.insert("code_block_count".to_string(), code_block_count as f64);

    // JSON presence
    let has_json = text.contains('{') && text.contains('}');
    features.insert("has_json".to_string(), if has_json { 1.0 } else { 0.0 });

    // Table presence (markdown |)
    let table_rows = lines.iter()
        .filter(|l| l.contains('|') && l.trim().starts_with('|'))
        .count();
    features.insert("table_rows".to_string(), table_rows as f64);

    // Average line length
    let avg_line_len = if lines.is_empty() {
        0.0
    } else {
        lines.iter().map(|l| l.len()).sum::<usize>() as f64 / lines.len() as f64
    };
    features.insert("avg_line_len".to_string(), avg_line_len);

    features
}

// ---------------------------------------------------------------------------
// Batch summary builder
// ---------------------------------------------------------------------------

/// Build a batch summary from individual reports.
fn build_batch_summary(reports: Vec<ConsistencyReport>) -> BatchSummary {
    let total = reports.len();
    let passed = reports.iter().filter(|r| r.pass).count();
    let avg_sim = if total > 0 {
        reports.iter().map(|r| r.avg_similarity).sum::<f64>() / total as f64
    } else {
        0.0
    };

    BatchSummary {
        total_prompts: total,
        passed,
        failed: total - passed,
        avg_similarity: avg_sim,
        reports,
        created_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("clawtex_test_consistency");
        let _ = std::fs::create_dir_all(&dir);
        // Use a unique suffix to avoid collisions with leftover DB files
        let unique = uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0000").to_string();
        let db_path = dir.join(format!("{}_{}.db", name, unique));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    // -- Factual similarity tests --

    #[test]
    fn test_factual_similarity_identical() {
        let score = factual_similarity(
            "The capital of France is Paris.",
            "The capital of France is Paris.",
        );
        assert_eq!(score, 1.0);
    }

    #[test]
    fn test_factual_similarity_high() {
        let score = factual_similarity(
            "The capital of France is Paris. It is a beautiful city.",
            "Paris is the capital of France. It is a wonderful city.",
        );
        // Should be moderate-to-high since same key facts (bigrams differ due to word order)
        assert!(score >= 0.4, "Expected moderate+ similarity, got {}", score);
    }

    #[test]
    fn test_factual_similarity_low() {
        let score = factual_similarity(
            "The capital of France is Paris.",
            "Rust is a systems programming language.",
        );
        assert!(score < 0.3, "Expected low similarity, got {}", score);
    }

    #[test]
    fn test_factual_similarity_empty() {
        assert_eq!(factual_similarity("", ""), 1.0);
        assert_eq!(factual_similarity("hello world", ""), 0.0);
    }

    // -- Format similarity tests --

    #[test]
    fn test_format_similarity_identical() {
        let text = "# Heading\n- Item 1\n- Item 2\n- Item 3\n";
        let score = format_similarity(text, text);
        assert_eq!(score, 1.0);
    }

    #[test]
    fn test_format_similarity_similar_structure() {
        let a = "# Title\n- Point A\n- Point B\n- Point C\n";
        let b = "# Header\n- Item 1\n- Item 2\n- Item 3\n";
        let score = format_similarity(a, b);
        // Same structure: 1 heading, 3 bullets, similar line count
        assert!(score > 0.7, "Expected high format similarity, got {}", score);
    }

    #[test]
    fn test_format_similarity_different_structure() {
        let a = "Just a single paragraph of text without any special formatting at all.";
        let b = "# Title\n\n- Item 1\n- Item 2\n\n```\ncode block\n```\n";
        let score = format_similarity(a, b);
        assert!(score < 0.7, "Expected low format similarity, got {}", score);
    }

    // -- Tool similarity tests --

    #[test]
    fn test_tool_similarity_identical() {
        let a = vec!["web_search".to_string(), "http_request".to_string()];
        let b = vec!["web_search".to_string(), "http_request".to_string()];
        assert_eq!(tool_usage_similarity(&a, &b), 1.0);
    }

    #[test]
    fn test_tool_similarity_partial() {
        let a = vec!["web_search".to_string(), "http_request".to_string()];
        let b = vec!["web_search".to_string(), "file_read".to_string()];
        let score = tool_usage_similarity(&a, &b);
        // 1 overlap / 3 union = 0.333
        assert!((score - 1.0 / 3.0).abs() < 0.01, "Expected ~0.33, got {}", score);
    }

    #[test]
    fn test_tool_similarity_none() {
        let a = vec!["web_search".to_string()];
        let b = vec!["file_read".to_string()];
        assert_eq!(tool_usage_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_tool_similarity_both_empty() {
        let a: Vec<String> = vec![];
        let b: Vec<String> = vec![];
        assert_eq!(tool_usage_similarity(&a, &b), 1.0);
    }

    #[test]
    fn test_tool_similarity_one_empty() {
        let a = vec!["web_search".to_string()];
        let b: Vec<String> = vec![];
        assert_eq!(tool_usage_similarity(&a, &b), 0.0);
    }

    // -- Extract key tokens tests --

    #[test]
    fn test_extract_key_tokens_basic() {
        let tokens = extract_key_tokens("Hello World 42");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"42".to_string()));
        // Should also have bigrams
        assert!(tokens.contains(&"hello world".to_string()));
    }

    #[test]
    fn test_extract_key_tokens_filters_short() {
        let tokens = extract_key_tokens("I am a cat");
        // "I", "a" should be filtered (< 2 chars)
        assert!(!tokens.contains(&"i".to_string()));
        assert!(!tokens.contains(&"a".to_string()));
        assert!(tokens.contains(&"am".to_string()));
        assert!(tokens.contains(&"cat".to_string()));
    }

    // -- Format features tests --

    #[test]
    fn test_format_features_bullets() {
        let text = "- Item 1\n- Item 2\n* Item 3\n1. Item 4\n";
        let features = extract_format_features(text);
        assert_eq!(*features.get("bullet_count").unwrap(), 4.0);
    }

    #[test]
    fn test_format_features_code_blocks() {
        let text = "Some text\n```python\nprint('hello')\n```\nMore text";
        let features = extract_format_features(text);
        assert_eq!(*features.get("code_block_count").unwrap(), 1.0);
    }

    #[test]
    fn test_format_features_json() {
        let text = "{\"key\": \"value\"}";
        let features = extract_format_features(text);
        assert_eq!(*features.get("has_json").unwrap(), 1.0);
    }

    #[test]
    fn test_format_features_table() {
        let text = "| Name | Value |\n|------|-------|\n| A | 1 |\n| B | 2 |";
        let features = extract_format_features(text);
        assert_eq!(*features.get("table_rows").unwrap(), 4.0);
    }

    // -- Similarity matrix tests --

    #[test]
    fn test_similarity_matrix_two_identical() {
        let r1 = WorkerResult {
            worker: "w1".to_string(),
            output: "The capital of France is Paris.".to_string(),
            tool_calls: vec![],
            latency_ms: 100,
            success: true,
            error: None,
        };
        let r2 = WorkerResult {
            worker: "w2".to_string(),
            output: "The capital of France is Paris.".to_string(),
            tool_calls: vec![],
            latency_ms: 120,
            success: true,
            error: None,
        };
        let refs = vec![&r1, &r2];
        let (matrix, avg, _, _, _) = compute_similarity_matrix(&refs);

        assert_eq!(matrix.len(), 1); // C(2,2) = 1 pair
        assert_eq!(avg, 1.0);
        assert_eq!(matrix[0].worker_a, "w1");
        assert_eq!(matrix[0].worker_b, "w2");
    }

    #[test]
    fn test_similarity_matrix_three_workers() {
        let r1 = WorkerResult {
            worker: "w1".to_string(),
            output: "Paris is the capital.".to_string(),
            tool_calls: vec!["web_search".to_string()],
            latency_ms: 100,
            success: true,
            error: None,
        };
        let r2 = WorkerResult {
            worker: "w2".to_string(),
            output: "The capital is Paris.".to_string(),
            tool_calls: vec!["web_search".to_string()],
            latency_ms: 120,
            success: true,
            error: None,
        };
        let r3 = WorkerResult {
            worker: "w3".to_string(),
            output: "France has its capital in Paris.".to_string(),
            tool_calls: vec!["web_search".to_string()],
            latency_ms: 130,
            success: true,
            error: None,
        };
        let refs = vec![&r1, &r2, &r3];
        let (matrix, avg, factual_avg, _, tool_avg) = compute_similarity_matrix(&refs);

        assert_eq!(matrix.len(), 3); // C(3,2) = 3 pairs
        assert!(avg > 0.0);
        assert!(factual_avg > 0.0);
        assert_eq!(tool_avg, 1.0); // All used web_search
    }

    // -- ConsistencyTester persistence tests --

    #[test]
    fn test_tester_new_creates_db() {
        let (db_str, db_path) = temp_db("new_creates");
        let _tester = ConsistencyTester::new(&db_str).unwrap();
        assert!(db_path.exists());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_store_and_retrieve_report() {
        let (db_str, db_path) = temp_db("store_retrieve");
        let tester = ConsistencyTester::new(&db_str).unwrap();

        let report = ConsistencyReport {
            id: "test-id-1".to_string(),
            prompt: "What is 2+2?".to_string(),
            category: "factual_math".to_string(),
            results: vec![
                WorkerResult {
                    worker: "w1".to_string(),
                    output: "4".to_string(),
                    tool_calls: vec![],
                    latency_ms: 50,
                    success: true,
                    error: None,
                },
                WorkerResult {
                    worker: "w2".to_string(),
                    output: "4".to_string(),
                    tool_calls: vec![],
                    latency_ms: 60,
                    success: true,
                    error: None,
                },
            ],
            similarity_matrix: vec![PairSimilarity {
                worker_a: "w1".to_string(),
                worker_b: "w2".to_string(),
                factual_similarity: 1.0,
                format_similarity: 1.0,
                tool_similarity: 1.0,
                overall: 1.0,
            }],
            avg_similarity: 1.0,
            factual_avg: 1.0,
            format_avg: 1.0,
            tool_avg: 1.0,
            pass: true,
            threshold: 0.9,
            created_at: Utc::now(),
        };

        tester.store_report(&report).unwrap();

        let reports = tester.recent_reports(10).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].id, "test-id-1");
        assert_eq!(reports[0].prompt, "What is 2+2?");
        assert_eq!(reports[0].category, "factual_math");
        assert!(reports[0].pass);
        assert_eq!(reports[0].results.len(), 2);
        assert_eq!(reports[0].similarity_matrix.len(), 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_count_reports() {
        let (db_str, db_path) = temp_db("count_reports");
        let tester = ConsistencyTester::new(&db_str).unwrap();
        assert_eq!(tester.count().unwrap(), 0);

        let report = ConsistencyReport {
            id: "r1".to_string(),
            prompt: "test".to_string(),
            category: "".to_string(),
            results: vec![],
            similarity_matrix: vec![],
            avg_similarity: 0.95,
            factual_avg: 0.95,
            format_avg: 0.95,
            tool_avg: 1.0,
            pass: true,
            threshold: 0.9,
            created_at: Utc::now(),
        };
        tester.store_report(&report).unwrap();
        assert_eq!(tester.count().unwrap(), 1);

        let report2 = ConsistencyReport {
            id: "r2".to_string(),
            ..report.clone()
        };
        tester.store_report(&report2).unwrap();
        assert_eq!(tester.count().unwrap(), 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_pass_rate() {
        let (db_str, db_path) = temp_db("pass_rate");
        let tester = ConsistencyTester::new(&db_str).unwrap();

        // Empty = 0%
        assert_eq!(tester.pass_rate(10).unwrap(), 0.0);

        // 1 pass
        let r1 = ConsistencyReport {
            id: "p1".to_string(),
            prompt: "a".to_string(),
            category: "".to_string(),
            results: vec![],
            similarity_matrix: vec![],
            avg_similarity: 0.95,
            factual_avg: 0.95,
            format_avg: 0.95,
            tool_avg: 1.0,
            pass: true,
            threshold: 0.9,
            created_at: Utc::now(),
        };
        tester.store_report(&r1).unwrap();

        // 1 fail
        let r2 = ConsistencyReport {
            id: "p2".to_string(),
            pass: false,
            avg_similarity: 0.5,
            ..r1.clone()
        };
        tester.store_report(&r2).unwrap();

        assert_eq!(tester.pass_rate(10).unwrap(), 0.5);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_history_summary() {
        let (db_str, db_path) = temp_db("history_summary");
        let tester = ConsistencyTester::new(&db_str).unwrap();

        let summary = tester.history_summary().unwrap();
        assert_eq!(summary["total_tests"], 0);

        let r1 = ConsistencyReport {
            id: "h1".to_string(),
            prompt: "test".to_string(),
            category: "factual".to_string(),
            results: vec![],
            similarity_matrix: vec![],
            avg_similarity: 0.92,
            factual_avg: 0.90,
            format_avg: 0.95,
            tool_avg: 1.0,
            pass: true,
            threshold: 0.9,
            created_at: Utc::now(),
        };
        tester.store_report(&r1).unwrap();

        let summary = tester.history_summary().unwrap();
        assert_eq!(summary["total_tests"], 1);
        assert_eq!(summary["passed"], 1);
        assert_eq!(summary["failed"], 0);
        assert_eq!(summary["pass_rate"], 1.0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_with_threshold() {
        let (db_str, db_path) = temp_db("threshold");
        let tester = ConsistencyTester::new(&db_str).unwrap().with_threshold(0.80);
        assert_eq!(tester.threshold, 0.80);

        // Clamp test
        let tester2 = ConsistencyTester::new(&db_str).unwrap().with_threshold(1.5);
        assert_eq!(tester2.threshold, 1.0);
        let tester3 = ConsistencyTester::new(&db_str).unwrap().with_threshold(-0.5);
        assert_eq!(tester3.threshold, 0.0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_predefined_suite_has_10_prompts() {
        assert_eq!(PREDEFINED_TEST_SUITE.len(), 10);
        // Each should have a category and a prompt
        for (cat, prompt) in PREDEFINED_TEST_SUITE {
            assert!(!cat.is_empty());
            assert!(!prompt.is_empty());
        }
    }

    #[test]
    fn test_batch_summary_builder() {
        let r1 = ConsistencyReport {
            id: "b1".to_string(),
            prompt: "p1".to_string(),
            category: "".to_string(),
            results: vec![],
            similarity_matrix: vec![],
            avg_similarity: 0.95,
            factual_avg: 0.95,
            format_avg: 0.95,
            tool_avg: 1.0,
            pass: true,
            threshold: 0.9,
            created_at: Utc::now(),
        };
        let r2 = ConsistencyReport {
            id: "b2".to_string(),
            prompt: "p2".to_string(),
            avg_similarity: 0.80,
            pass: false,
            ..r1.clone()
        };

        let summary = build_batch_summary(vec![r1, r2]);
        assert_eq!(summary.total_prompts, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert!((summary.avg_similarity - 0.875).abs() < 0.01);
    }

    #[test]
    fn test_truncate_str_basic() {
        assert_eq!(truncate_str("hello", 10), "hello");
        assert_eq!(truncate_str("hello world foo", 5), "hello...");
    }

    #[test]
    fn test_truncate_str_utf8_safe() {
        // Multi-byte UTF-8 character boundary safety
        let s = "Hello \u{4f60}\u{597d}"; // "Hello ??????"
        let t = truncate_str(s, 7);
        // Should not panic and should end at a valid char boundary
        assert!(t.ends_with("...") || t.len() <= 7);
    }

    #[test]
    fn test_worker_result_serialize() {
        let r = WorkerResult {
            worker: "test-worker".to_string(),
            output: "The answer is 42.".to_string(),
            tool_calls: vec!["web_search".to_string()],
            latency_ms: 150,
            success: true,
            error: None,
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("test-worker"));
        let back: WorkerResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.worker, "test-worker");
        assert_eq!(back.latency_ms, 150);
    }

    #[test]
    fn test_pair_similarity_serialize() {
        let p = PairSimilarity {
            worker_a: "w1".to_string(),
            worker_b: "w2".to_string(),
            factual_similarity: 0.85,
            format_similarity: 0.90,
            tool_similarity: 1.0,
            overall: 0.895,
        };
        let json = serde_json::to_string(&p).unwrap();
        let back: PairSimilarity = serde_json::from_str(&json).unwrap();
        assert_eq!(back.overall, 0.895);
    }

    #[test]
    fn test_consistency_report_serialize() {
        let report = ConsistencyReport {
            id: "test".to_string(),
            prompt: "What is AI?".to_string(),
            category: "factual".to_string(),
            results: vec![],
            similarity_matrix: vec![],
            avg_similarity: 0.92,
            factual_avg: 0.90,
            format_avg: 0.95,
            tool_avg: 1.0,
            pass: true,
            threshold: 0.90,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&report).unwrap();
        let back: ConsistencyReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.prompt, "What is AI?");
        assert!(back.pass);
    }

    #[test]
    fn test_multiple_reports_ordering() {
        let (db_str, db_path) = temp_db("ordering");
        let tester = ConsistencyTester::new(&db_str).unwrap();

        for i in 0..5 {
            let r = ConsistencyReport {
                id: format!("ord-{}", i),
                prompt: format!("prompt {}", i),
                category: "".to_string(),
                results: vec![],
                similarity_matrix: vec![],
                avg_similarity: 0.5 + i as f64 * 0.1,
                factual_avg: 0.5 + i as f64 * 0.1,
                format_avg: 0.5 + i as f64 * 0.1,
                tool_avg: 1.0,
                pass: i >= 3,
                threshold: 0.9,
                created_at: Utc::now(),
            };
            tester.store_report(&r).unwrap();
        }

        let reports = tester.recent_reports(3).unwrap();
        assert_eq!(reports.len(), 3);

        let _ = std::fs::remove_file(&db_path);
    }
}
