//! Quality Pipeline — Ground Truth Feedback Loop (Phase F).
//!
//! Four components:
//! 1. **FeedbackCollector** — SQLite-backed human feedback (rating 1-5) per hand/phase output
//! 2. **QualityScorer** — rule-based output quality scoring (0.0-1.0) across multiple criteria
//! 3. **ImprovementTracker** — tracks quality trends over time and suggests improvements
//! 4. **AbTest** — A/B testing with simple z-test for statistical significance
//!
//! DB: `~/.clawtex/feedback.db`

use anyhow::Result;
use regex::Regex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::debug;

// ---------------------------------------------------------------------------
// 1. Feedback Collector
// ---------------------------------------------------------------------------

/// Aggregated feedback statistics for a hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackStats {
    pub hand: String,
    pub avg_rating: f64,
    pub count: u64,
    /// Trend based on comparing first half vs second half of ratings.
    /// Positive = improving, negative = declining, near-zero = stable.
    pub trend: f64,
}

/// A single feedback record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackRecord {
    pub id: i64,
    pub hand: String,
    pub phase: String,
    pub output_hash: String,
    pub rating: u8,
    pub notes: String,
    pub created_at: String,
}

/// Collects and stores human feedback on hand outputs.
/// SQLite-backed for persistence.
pub struct FeedbackCollector {
    conn: Arc<Mutex<Connection>>,
}

impl FeedbackCollector {
    /// Open (or create) the feedback database and ensure the schema exists.
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS feedback (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                hand        TEXT NOT NULL,
                phase       TEXT NOT NULL,
                output_hash TEXT NOT NULL,
                rating      INTEGER NOT NULL CHECK(rating >= 1 AND rating <= 5),
                notes       TEXT NOT NULL DEFAULT '',
                created_at  TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_feedback_hand ON feedback(hand);
            CREATE INDEX IF NOT EXISTS idx_feedback_created ON feedback(created_at);",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Record a feedback entry for a hand/phase output.
    /// Rating must be 1-5, otherwise returns an error.
    pub fn record_feedback(
        &self,
        hand: &str,
        phase: &str,
        output_hash: &str,
        rating: u8,
        notes: &str,
    ) -> Result<()> {
        if rating < 1 || rating > 5 {
            anyhow::bail!("Rating must be 1-5, got {}", rating);
        }
        let now = chrono::Utc::now().to_rfc3339();
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO feedback (hand, phase, output_hash, rating, notes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![hand, phase, output_hash, rating as i32, notes, now],
        )?;
        debug!(
            "Feedback recorded: hand={} phase={} rating={} hash={}",
            hand, phase, rating, output_hash
        );
        Ok(())
    }

    /// Get aggregated feedback statistics for a hand.
    pub fn get_feedback_stats(&self, hand: &str) -> Result<FeedbackStats> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;

        // Fetch avg and count
        let (avg_rating, count): (f64, i64) = conn.query_row(
            "SELECT COALESCE(AVG(rating), 0.0), COUNT(*) FROM feedback WHERE hand = ?1",
            params![hand],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;

        // Calculate trend: compare first half vs second half of ratings (ordered by created_at)
        let trend = if count >= 4 {
            let mut stmt = conn.prepare(
                "SELECT rating FROM feedback WHERE hand = ?1 ORDER BY created_at ASC",
            )?;
            let ratings: Vec<f64> = stmt
                .query_map(params![hand], |row| {
                    let r: i32 = row.get(0)?;
                    Ok(r as f64)
                })?
                .filter_map(|r| r.ok())
                .collect();

            let mid = ratings.len() / 2;
            let first_half_avg: f64 = ratings[..mid].iter().sum::<f64>() / mid as f64;
            let second_half_avg: f64 = ratings[mid..].iter().sum::<f64>()
                / (ratings.len() - mid) as f64;
            second_half_avg - first_half_avg
        } else {
            0.0
        };

        Ok(FeedbackStats {
            hand: hand.to_string(),
            avg_rating,
            count: count as u64,
            trend,
        })
    }

    /// Retrieve all feedback records for a hand, most recent first.
    pub fn list_feedback(&self, hand: &str, limit: usize) -> Result<Vec<FeedbackRecord>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT id, hand, phase, output_hash, rating, notes, created_at
             FROM feedback WHERE hand = ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let records = stmt
            .query_map(params![hand, limit as i64], |row| {
                Ok(FeedbackRecord {
                    id: row.get(0)?,
                    hand: row.get(1)?,
                    phase: row.get(2)?,
                    output_hash: row.get(3)?,
                    rating: row.get::<_, i32>(4)? as u8,
                    notes: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(records)
    }

    /// Count total feedback entries across all hands.
    pub fn total_count(&self) -> Result<u64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let count: i64 =
            conn.query_row("SELECT COUNT(*) FROM feedback", [], |row| row.get(0))?;
        Ok(count as u64)
    }
}

// ---------------------------------------------------------------------------
// 2. Quality Scorer
// ---------------------------------------------------------------------------

/// Per-criterion score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriterionScore {
    pub name: String,
    pub score: f64,
    pub detail: String,
}

/// Composite quality score for an output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityScore {
    pub overall: f64,
    pub criteria: Vec<CriterionScore>,
}

/// Rule-based quality scorer. No LLM calls — pure heuristic evaluation.
pub struct QualityScorer;

impl QualityScorer {
    pub fn new() -> Self {
        Self
    }

    /// Score an output against a set of criteria.
    /// Built-in criteria: `length_ok`, `has_structure`, `no_repetition`,
    /// `factual_density`, `actionable`.
    /// If `criteria` is empty, all built-in criteria are used.
    pub fn score_output(&self, output: &str, criteria: &[&str]) -> QualityScore {
        let all_criteria: Vec<&str> = if criteria.is_empty() {
            vec![
                "length_ok",
                "has_structure",
                "no_repetition",
                "factual_density",
                "actionable",
            ]
        } else {
            criteria.to_vec()
        };

        let mut scores: Vec<CriterionScore> = Vec::new();

        for criterion in &all_criteria {
            let cs = match *criterion {
                "length_ok" => self.check_length(output),
                "has_structure" => self.check_structure(output),
                "no_repetition" => self.check_repetition(output),
                "factual_density" => self.check_factual_density(output),
                "actionable" => self.check_actionable(output),
                other => CriterionScore {
                    name: other.to_string(),
                    score: 0.5,
                    detail: format!("Unknown criterion: {}", other),
                },
            };
            scores.push(cs);
        }

        let overall = if scores.is_empty() {
            0.0
        } else {
            scores.iter().map(|s| s.score).sum::<f64>() / scores.len() as f64
        };

        QualityScore {
            overall,
            criteria: scores,
        }
    }

    /// Length check: penalizes too short (<50 chars) or excessively long (>10000 chars) outputs.
    fn check_length(&self, output: &str) -> CriterionScore {
        let len = output.trim().len();
        let (score, detail) = if len < 50 {
            (0.2, format!("Too short ({} chars)", len))
        } else if len < 100 {
            (0.5, format!("Short ({} chars)", len))
        } else if len > 10000 {
            (0.6, format!("Very long ({} chars), may lack focus", len))
        } else {
            (1.0, format!("Good length ({} chars)", len))
        };
        CriterionScore {
            name: "length_ok".to_string(),
            score,
            detail,
        }
    }

    /// Structure check: looks for headings, lists, paragraphs, code blocks.
    fn check_structure(&self, output: &str) -> CriterionScore {
        let mut structure_signals = 0u32;

        // Markdown headings
        if Regex::new(r"(?m)^#{1,6}\s+\S").unwrap().is_match(output) {
            structure_signals += 1;
        }
        // Bullet lists
        if Regex::new(r"(?m)^[\s]*[-*+]\s+\S").unwrap().is_match(output) {
            structure_signals += 1;
        }
        // Numbered lists
        if Regex::new(r"(?m)^[\s]*\d+[.)]\s+\S").unwrap().is_match(output) {
            structure_signals += 1;
        }
        // Multiple paragraphs (double newline)
        if output.matches("\n\n").count() >= 2 {
            structure_signals += 1;
        }
        // Code blocks
        if output.contains("```") {
            structure_signals += 1;
        }

        let score = match structure_signals {
            0 => 0.2,
            1 => 0.5,
            2 => 0.7,
            3 => 0.85,
            _ => 1.0,
        };

        CriterionScore {
            name: "has_structure".to_string(),
            score,
            detail: format!("{} structural elements detected", structure_signals),
        }
    }

    /// Repetition check: detects repeated sentences/phrases.
    fn check_repetition(&self, output: &str) -> CriterionScore {
        let sentences: Vec<&str> = output
            .split(|c: char| c == '.' || c == '!' || c == '?')
            .map(|s| s.trim())
            .filter(|s| s.len() > 10)
            .collect();

        if sentences.is_empty() {
            return CriterionScore {
                name: "no_repetition".to_string(),
                score: 1.0,
                detail: "No sentences to check".to_string(),
            };
        }

        // Count duplicates
        let mut seen: HashMap<String, u32> = HashMap::new();
        for s in &sentences {
            let normalized = s.to_lowercase();
            *seen.entry(normalized).or_insert(0) += 1;
        }

        let duplicates = seen.values().filter(|&&count| count > 1).count();
        let dup_ratio = duplicates as f64 / sentences.len().max(1) as f64;

        let (score, detail) = if dup_ratio > 0.3 {
            (0.2, format!("High repetition: {} duplicated phrases", duplicates))
        } else if dup_ratio > 0.1 {
            (0.6, format!("Some repetition: {} duplicated phrases", duplicates))
        } else {
            (1.0, "No significant repetition".to_string())
        };

        CriterionScore {
            name: "no_repetition".to_string(),
            score,
            detail,
        }
    }

    /// Factual density: checks for numbers, proper nouns, specific terms.
    fn check_factual_density(&self, output: &str) -> CriterionScore {
        let word_count = output.split_whitespace().count().max(1);

        let mut factual_signals = 0u32;

        // Numbers (percentages, dollar amounts, dates, quantities)
        let number_matches = Regex::new(r"\b\d+[\d.,]*%?\b")
            .unwrap()
            .find_iter(output)
            .count();
        factual_signals += number_matches as u32;

        // URLs or references
        let url_matches = Regex::new(r"https?://\S+")
            .unwrap()
            .find_iter(output)
            .count();
        factual_signals += url_matches as u32;

        // Technical terms (capitalized multi-word phrases)
        let tech_matches = Regex::new(r"\b[A-Z][a-z]+(?:\s+[A-Z][a-z]+)+\b")
            .unwrap()
            .find_iter(output)
            .count();
        factual_signals += tech_matches as u32;

        let density = factual_signals as f64 / word_count as f64;

        let (score, detail) = if density < 0.01 {
            (0.3, format!("Low factual density ({} signals in {} words)", factual_signals, word_count))
        } else if density < 0.05 {
            (0.6, format!("Moderate factual density ({} signals in {} words)", factual_signals, word_count))
        } else {
            (1.0, format!("High factual density ({} signals in {} words)", factual_signals, word_count))
        };

        CriterionScore {
            name: "factual_density".to_string(),
            score,
            detail,
        }
    }

    /// Actionable check: looks for action verbs, recommendations, next steps.
    fn check_actionable(&self, output: &str) -> CriterionScore {
        let lower = output.to_lowercase();

        let action_patterns = [
            r"(?i)\b(?:should|must|need to|recommend|suggest|consider)\b",
            r"(?i)\b(?:step \d|first|second|third|next|then|finally)\b",
            r"(?i)\b(?:action item|todo|task|deliverable|milestone)\b",
            r"(?i)\b(?:implement|deploy|configure|install|setup|create|build)\b",
        ];

        let mut action_score = 0u32;
        for pattern in &action_patterns {
            if Regex::new(pattern).unwrap().is_match(&lower) {
                action_score += 1;
            }
        }

        let score = match action_score {
            0 => 0.2,
            1 => 0.5,
            2 => 0.7,
            3 => 0.85,
            _ => 1.0,
        };

        CriterionScore {
            name: "actionable".to_string(),
            score,
            detail: format!("{}/4 actionable patterns found", action_score),
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Improvement Tracker
// ---------------------------------------------------------------------------

/// Trend direction for quality over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TrendDirection {
    Improving,
    Stable,
    Declining,
}

/// Quality trend result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendResult {
    pub direction: TrendDirection,
    /// Slope of the linear trend line (positive = improving).
    pub slope: f64,
    pub data_points: usize,
}

/// Suggested improvement for a hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    pub criterion: String,
    pub avg_score: f64,
    pub recommendation: String,
}

/// Tracks quality scores over time per hand.
/// SQLite-backed for persistence.
pub struct ImprovementTracker {
    conn: Arc<Mutex<Connection>>,
}

impl ImprovementTracker {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS quality_history (
                id        INTEGER PRIMARY KEY AUTOINCREMENT,
                hand      TEXT NOT NULL,
                score     REAL NOT NULL,
                criteria_json TEXT NOT NULL DEFAULT '{}',
                timestamp INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_qh_hand ON quality_history(hand);
            CREATE INDEX IF NOT EXISTS idx_qh_ts ON quality_history(timestamp);",
        )?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Record a quality score for a hand at a given timestamp.
    pub fn record_quality(&self, hand: &str, score: f64, timestamp: i64) -> Result<()> {
        self.record_quality_with_criteria(hand, score, timestamp, &HashMap::new())
    }

    /// Record a quality score with per-criterion breakdown.
    pub fn record_quality_with_criteria(
        &self,
        hand: &str,
        score: f64,
        timestamp: i64,
        criteria: &HashMap<String, f64>,
    ) -> Result<()> {
        let criteria_json = serde_json::to_string(criteria)?;
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        conn.execute(
            "INSERT INTO quality_history (hand, score, criteria_json, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![hand, score, criteria_json, timestamp],
        )?;
        debug!(
            "Quality recorded: hand={} score={:.3} ts={}",
            hand, score, timestamp
        );
        Ok(())
    }

    /// Calculate the quality trend for a hand over the last `last_n` data points.
    /// Uses simple linear regression to determine slope.
    pub fn quality_trend(&self, hand: &str, last_n: usize) -> Result<TrendResult> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT score FROM quality_history
             WHERE hand = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        )?;
        let scores: Vec<f64> = stmt
            .query_map(params![hand, last_n as i64], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        if scores.len() < 2 {
            return Ok(TrendResult {
                direction: TrendDirection::Stable,
                slope: 0.0,
                data_points: scores.len(),
            });
        }

        // Reverse so chronological order (oldest first)
        let mut chronological: Vec<f64> = scores;
        chronological.reverse();

        // Simple linear regression: y = a + b*x
        let n = chronological.len() as f64;
        let mut sum_x = 0.0f64;
        let mut sum_y = 0.0f64;
        let mut sum_xy = 0.0f64;
        let mut sum_xx = 0.0f64;

        for (i, &y) in chronological.iter().enumerate() {
            let x = i as f64;
            sum_x += x;
            sum_y += y;
            sum_xy += x * y;
            sum_xx += x * x;
        }

        let denom = n * sum_xx - sum_x * sum_x;
        let slope = if denom.abs() < 1e-12 {
            0.0
        } else {
            (n * sum_xy - sum_x * sum_y) / denom
        };

        let direction = if slope > 0.01 {
            TrendDirection::Improving
        } else if slope < -0.01 {
            TrendDirection::Declining
        } else {
            TrendDirection::Stable
        };

        Ok(TrendResult {
            direction,
            slope,
            data_points: chronological.len(),
        })
    }

    /// Suggest improvements for a hand based on lowest-scoring criteria.
    /// Looks at the last `last_n` quality records and identifies weak criteria.
    pub fn suggest_improvements(&self, hand: &str, last_n: usize) -> Result<Vec<Suggestion>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let mut stmt = conn.prepare(
            "SELECT criteria_json FROM quality_history
             WHERE hand = ?1
             ORDER BY timestamp DESC
             LIMIT ?2",
        )?;
        let criteria_rows: Vec<String> = stmt
            .query_map(params![hand, last_n as i64], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        // Aggregate per-criterion averages
        let mut criterion_sums: HashMap<String, (f64, u32)> = HashMap::new();
        for json_str in &criteria_rows {
            if let Ok(map) = serde_json::from_str::<HashMap<String, f64>>(json_str) {
                for (k, v) in map {
                    let entry = criterion_sums.entry(k).or_insert((0.0, 0));
                    entry.0 += v;
                    entry.1 += 1;
                }
            }
        }

        let mut suggestions: Vec<Suggestion> = criterion_sums
            .into_iter()
            .map(|(name, (sum, count))| {
                let avg = sum / count as f64;
                let recommendation = recommendation_for(&name, avg);
                Suggestion {
                    criterion: name,
                    avg_score: avg,
                    recommendation,
                }
            })
            .filter(|s| s.avg_score < 0.7) // Only suggest for weak criteria
            .collect();

        // Sort by avg_score ascending (worst first)
        suggestions.sort_by(|a, b| a.avg_score.partial_cmp(&b.avg_score).unwrap_or(std::cmp::Ordering::Equal));

        Ok(suggestions)
    }

    /// Get the average quality score for a hand.
    pub fn avg_quality(&self, hand: &str) -> Result<f64> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("lock poisoned: {}", e))?;
        let avg: f64 = conn.query_row(
            "SELECT COALESCE(AVG(score), 0.0) FROM quality_history WHERE hand = ?1",
            params![hand],
            |row| row.get(0),
        )?;
        Ok(avg)
    }
}

/// Generate a recommendation based on criterion name and its average score.
fn recommendation_for(criterion: &str, avg_score: f64) -> String {
    let severity = if avg_score < 0.3 {
        "Critical"
    } else if avg_score < 0.5 {
        "Important"
    } else {
        "Minor"
    };

    match criterion {
        "length_ok" => format!(
            "[{}] Output length is suboptimal (avg {:.2}). Consider adjusting prompt to request more detailed responses.",
            severity, avg_score
        ),
        "has_structure" => format!(
            "[{}] Outputs lack structure (avg {:.2}). Add instructions for headings, bullet points, and sections.",
            severity, avg_score
        ),
        "no_repetition" => format!(
            "[{}] Outputs contain repetition (avg {:.2}). Add 'avoid repeating points' to the system prompt.",
            severity, avg_score
        ),
        "factual_density" => format!(
            "[{}] Low factual density (avg {:.2}). Request specific data, numbers, and references.",
            severity, avg_score
        ),
        "actionable" => format!(
            "[{}] Outputs lack actionable content (avg {:.2}). Request concrete next steps and recommendations.",
            severity, avg_score
        ),
        other => format!(
            "[{}] Criterion '{}' scoring low (avg {:.2}). Review and improve related prompt instructions.",
            severity, other, avg_score
        ),
    }
}

// ---------------------------------------------------------------------------
// 4. A/B Testing
// ---------------------------------------------------------------------------

/// An A/B test comparing two prompt variants.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbTest {
    pub name: String,
    pub variant_a: String,
    pub variant_b: String,
    pub results_a: Vec<f64>,
    pub results_b: Vec<f64>,
}

impl AbTest {
    /// Create a new A/B test with two prompt variants.
    pub fn new(name: &str, variant_a: &str, variant_b: &str) -> Self {
        Self {
            name: name.to_string(),
            variant_a: variant_a.to_string(),
            variant_b: variant_b.to_string(),
            results_a: Vec::new(),
            results_b: Vec::new(),
        }
    }

    /// Record a quality score for a variant ("a" or "b").
    pub fn record_result(&mut self, variant: &str, score: f64) {
        match variant.to_lowercase().as_str() {
            "a" => self.results_a.push(score),
            "b" => self.results_b.push(score),
            _ => {} // Ignore invalid variants
        }
    }

    /// Determine the winner using a simple z-test for difference in means.
    /// Returns `Some("a")` or `Some("b")` if statistically significant (p < 0.05),
    /// or `None` if no clear winner yet (insufficient data or no significant difference).
    pub fn winner(&self) -> Option<&str> {
        if self.results_a.len() < 5 || self.results_b.len() < 5 {
            return None; // Need at least 5 samples per variant
        }

        let (mean_a, var_a) = mean_and_variance(&self.results_a);
        let (mean_b, var_b) = mean_and_variance(&self.results_b);

        let na = self.results_a.len() as f64;
        let nb = self.results_b.len() as f64;

        // Pooled standard error
        let se = (var_a / na + var_b / nb).sqrt();
        if se < 1e-12 {
            // No variance — if means differ, pick the higher one
            if (mean_a - mean_b).abs() < 1e-12 {
                return None;
            }
            return if mean_a > mean_b {
                Some("a")
            } else {
                Some("b")
            };
        }

        let z = (mean_a - mean_b) / se;

        // Two-tailed z-test at p < 0.05 => |z| > 1.96
        if z > 1.96 {
            Some("a")
        } else if z < -1.96 {
            Some("b")
        } else {
            None // Not statistically significant
        }
    }

    /// Get summary statistics for both variants.
    pub fn summary(&self) -> AbTestSummary {
        let (mean_a, var_a) = if self.results_a.is_empty() {
            (0.0, 0.0)
        } else {
            mean_and_variance(&self.results_a)
        };
        let (mean_b, var_b) = if self.results_b.is_empty() {
            (0.0, 0.0)
        } else {
            mean_and_variance(&self.results_b)
        };

        AbTestSummary {
            name: self.name.clone(),
            count_a: self.results_a.len(),
            count_b: self.results_b.len(),
            mean_a,
            mean_b,
            std_a: var_a.sqrt(),
            std_b: var_b.sqrt(),
            winner: self.winner().map(|s| s.to_string()),
        }
    }
}

/// Summary statistics for an A/B test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AbTestSummary {
    pub name: String,
    pub count_a: usize,
    pub count_b: usize,
    pub mean_a: f64,
    pub mean_b: f64,
    pub std_a: f64,
    pub std_b: f64,
    pub winner: Option<String>,
}

/// Compute mean and sample variance for a non-empty slice.
fn mean_and_variance(data: &[f64]) -> (f64, f64) {
    let n = data.len() as f64;
    if n < 1.0 {
        return (0.0, 0.0);
    }
    let mean = data.iter().sum::<f64>() / n;
    if n < 2.0 {
        return (mean, 0.0);
    }
    let variance = data.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0);
    (mean, variance)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp DB path, removing any stale file.
    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("clawtex_test_quality_pipeline");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    // -----------------------------------------------------------------------
    // FeedbackCollector tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_feedback_record_and_retrieve() {
        let (db_str, db_path) = temp_db("fb_record");
        let fc = FeedbackCollector::new(&db_str).unwrap();

        fc.record_feedback("content", "write", "abc123", 4, "Good article").unwrap();
        fc.record_feedback("content", "edit", "def456", 5, "Excellent").unwrap();

        let records = fc.list_feedback("content", 10).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].rating, 5); // Most recent first
        assert_eq!(records[1].rating, 4);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_feedback_invalid_rating() {
        let (db_str, db_path) = temp_db("fb_invalid");
        let fc = FeedbackCollector::new(&db_str).unwrap();

        assert!(fc.record_feedback("h", "p", "hash", 0, "").is_err());
        assert!(fc.record_feedback("h", "p", "hash", 6, "").is_err());
        assert!(fc.record_feedback("h", "p", "hash", 1, "").is_ok());
        assert!(fc.record_feedback("h", "p", "hash", 5, "").is_ok());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_feedback_stats_avg_and_count() {
        let (db_str, db_path) = temp_db("fb_stats");
        let fc = FeedbackCollector::new(&db_str).unwrap();

        fc.record_feedback("seo", "p1", "h1", 3, "").unwrap();
        fc.record_feedback("seo", "p1", "h2", 4, "").unwrap();
        fc.record_feedback("seo", "p2", "h3", 5, "").unwrap();

        let stats = fc.get_feedback_stats("seo").unwrap();
        assert_eq!(stats.count, 3);
        assert!((stats.avg_rating - 4.0).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_feedback_stats_trend() {
        let (db_str, db_path) = temp_db("fb_trend");
        let fc = FeedbackCollector::new(&db_str).unwrap();

        // Insert ratings that improve over time: 1,2,3,4,5,5
        for rating in [1u8, 2, 3, 4, 5, 5] {
            fc.record_feedback("trending", "p", &format!("h{}", rating), rating, "").unwrap();
        }

        let stats = fc.get_feedback_stats("trending").unwrap();
        assert!(stats.trend > 0.0, "Trend should be positive (improving)");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_feedback_stats_empty_hand() {
        let (db_str, db_path) = temp_db("fb_empty");
        let fc = FeedbackCollector::new(&db_str).unwrap();

        let stats = fc.get_feedback_stats("nonexistent").unwrap();
        assert_eq!(stats.count, 0);
        assert!((stats.avg_rating).abs() < 0.01);
        assert!((stats.trend).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_feedback_total_count() {
        let (db_str, db_path) = temp_db("fb_total");
        let fc = FeedbackCollector::new(&db_str).unwrap();

        assert_eq!(fc.total_count().unwrap(), 0);
        fc.record_feedback("a", "p", "h", 3, "").unwrap();
        fc.record_feedback("b", "p", "h", 4, "").unwrap();
        assert_eq!(fc.total_count().unwrap(), 2);

        let _ = std::fs::remove_file(&db_path);
    }

    // -----------------------------------------------------------------------
    // QualityScorer tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_quality_scorer_all_criteria() {
        let scorer = QualityScorer::new();
        let output = "# Analysis Report\n\nWe analyzed 15 competitors and found 3 key gaps.\n\n## Recommendations\n\n- **Step 1**: Implement SEO optimization (expected 25% traffic increase)\n- **Step 2**: Deploy new landing page\n- Consider using A/B testing for conversion rates\n\n## Next Steps\n\nYou should prioritize items above. The ROI is estimated at $5,000/month.";
        let score = scorer.score_output(output, &[]);
        assert!(score.overall > 0.5, "Well-structured output should score > 0.5, got {}", score.overall);
        assert_eq!(score.criteria.len(), 5);
    }

    #[test]
    fn test_quality_scorer_length_short() {
        let scorer = QualityScorer::new();
        let score = scorer.score_output("Hi", &["length_ok"]);
        assert!(score.overall < 0.5, "Very short output should score low");
    }

    #[test]
    fn test_quality_scorer_length_good() {
        let scorer = QualityScorer::new();
        let output = "a ".repeat(100);
        let score = scorer.score_output(&output, &["length_ok"]);
        assert!((score.overall - 1.0).abs() < 0.01, "Good length should score 1.0");
    }

    #[test]
    fn test_quality_scorer_structure() {
        let scorer = QualityScorer::new();

        // No structure
        let flat = "Just some plain text without any formatting at all.";
        let s1 = scorer.score_output(flat, &["has_structure"]);

        // With structure
        let structured = "# Title\n\n- Point 1\n- Point 2\n\n1. Step one\n2. Step two\n\n```code```";
        let s2 = scorer.score_output(structured, &["has_structure"]);

        assert!(s2.overall > s1.overall, "Structured output should score higher");
    }

    #[test]
    fn test_quality_scorer_repetition() {
        let scorer = QualityScorer::new();

        let repetitive = "This is a very important point that we must consider. This is a very important point that we must consider. This is a very important point that we must consider. Another different sentence here.";
        let score = scorer.score_output(repetitive, &["no_repetition"]);
        assert!(score.overall < 0.8, "Repetitive output should score lower, got {}", score.overall);
    }

    #[test]
    fn test_quality_scorer_no_repetition() {
        let scorer = QualityScorer::new();
        let unique = "First point about marketing strategy. Second insight about customer acquisition. Third recommendation for product development. Fourth analysis of competitive landscape.";
        let score = scorer.score_output(unique, &["no_repetition"]);
        assert!((score.overall - 1.0).abs() < 0.01, "Non-repetitive output should score 1.0");
    }

    #[test]
    fn test_quality_scorer_factual_density() {
        let scorer = QualityScorer::new();

        let vague = "Things are generally going well and we should continue doing stuff to improve.";
        let s1 = scorer.score_output(vague, &["factual_density"]);

        let factual = "Revenue increased 25% to $150,000 in Q3 2025. Customer Acquisition Cost dropped from $45 to $32 per user. See https://example.com/report for details.";
        let s2 = scorer.score_output(factual, &["factual_density"]);

        assert!(s2.overall > s1.overall, "Factual output should score higher than vague output");
    }

    #[test]
    fn test_quality_scorer_actionable() {
        let scorer = QualityScorer::new();

        let passive = "The market is large and there are opportunities available for exploration.";
        let s1 = scorer.score_output(passive, &["actionable"]);

        let actionable = "You should implement the new API first. Step 1: Deploy the staging server. Then create the database. Consider using PostgreSQL for production.";
        let s2 = scorer.score_output(actionable, &["actionable"]);

        assert!(s2.overall > s1.overall, "Actionable output should score higher");
    }

    #[test]
    fn test_quality_scorer_specific_criteria() {
        let scorer = QualityScorer::new();
        let output = "Some test output for scoring purposes only.";
        let score = scorer.score_output(output, &["length_ok", "has_structure"]);
        assert_eq!(score.criteria.len(), 2);
        assert_eq!(score.criteria[0].name, "length_ok");
        assert_eq!(score.criteria[1].name, "has_structure");
    }

    // -----------------------------------------------------------------------
    // ImprovementTracker tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_improvement_tracker_record_and_avg() {
        let (db_str, db_path) = temp_db("it_basic");
        let tracker = ImprovementTracker::new(&db_str).unwrap();

        tracker.record_quality("content", 0.6, 1000).unwrap();
        tracker.record_quality("content", 0.8, 2000).unwrap();
        tracker.record_quality("content", 0.7, 3000).unwrap();

        let avg = tracker.avg_quality("content").unwrap();
        assert!((avg - 0.7).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_improvement_tracker_trend_improving() {
        let (db_str, db_path) = temp_db("it_improving");
        let tracker = ImprovementTracker::new(&db_str).unwrap();

        // Steadily improving scores
        for i in 0..10 {
            tracker.record_quality("seo", 0.3 + (i as f64 * 0.07), i as i64 * 1000).unwrap();
        }

        let trend = tracker.quality_trend("seo", 10).unwrap();
        assert_eq!(trend.direction, TrendDirection::Improving);
        assert!(trend.slope > 0.0);
        assert_eq!(trend.data_points, 10);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_improvement_tracker_trend_declining() {
        let (db_str, db_path) = temp_db("it_declining");
        let tracker = ImprovementTracker::new(&db_str).unwrap();

        // Steadily declining scores
        for i in 0..8 {
            tracker.record_quality("lead", 0.9 - (i as f64 * 0.1), i as i64 * 1000).unwrap();
        }

        let trend = tracker.quality_trend("lead", 8).unwrap();
        assert_eq!(trend.direction, TrendDirection::Declining);
        assert!(trend.slope < 0.0);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_improvement_tracker_trend_stable() {
        let (db_str, db_path) = temp_db("it_stable");
        let tracker = ImprovementTracker::new(&db_str).unwrap();

        // Flat scores
        for i in 0..6 {
            tracker.record_quality("stable", 0.75, i as i64 * 1000).unwrap();
        }

        let trend = tracker.quality_trend("stable", 6).unwrap();
        assert_eq!(trend.direction, TrendDirection::Stable);
        assert!(trend.slope.abs() < 0.02);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_improvement_tracker_trend_insufficient_data() {
        let (db_str, db_path) = temp_db("it_insufficient");
        let tracker = ImprovementTracker::new(&db_str).unwrap();

        tracker.record_quality("single", 0.5, 1000).unwrap();

        let trend = tracker.quality_trend("single", 10).unwrap();
        assert_eq!(trend.direction, TrendDirection::Stable);
        assert_eq!(trend.data_points, 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_improvement_tracker_suggest_improvements() {
        let (db_str, db_path) = temp_db("it_suggest");
        let tracker = ImprovementTracker::new(&db_str).unwrap();

        // Record quality with criteria breakdown — "has_structure" is consistently low
        let mut criteria = HashMap::new();
        criteria.insert("length_ok".to_string(), 0.9);
        criteria.insert("has_structure".to_string(), 0.3);
        criteria.insert("no_repetition".to_string(), 0.95);
        criteria.insert("factual_density".to_string(), 0.4);
        criteria.insert("actionable".to_string(), 0.85);

        for i in 0..5 {
            tracker
                .record_quality_with_criteria("content", 0.68, i as i64 * 1000, &criteria)
                .unwrap();
        }

        let suggestions = tracker.suggest_improvements("content", 5).unwrap();
        assert!(!suggestions.is_empty(), "Should have suggestions for low-scoring criteria");

        // has_structure (0.3) should appear first (lowest)
        assert_eq!(suggestions[0].criterion, "has_structure");
        assert!(suggestions[0].avg_score < 0.5);
        assert!(suggestions[0].recommendation.contains("structure"));

        // factual_density (0.4) should also appear
        assert!(suggestions.iter().any(|s| s.criterion == "factual_density"));

        // length_ok (0.9), no_repetition (0.95), actionable (0.85) should NOT appear
        assert!(!suggestions.iter().any(|s| s.criterion == "length_ok"));
        assert!(!suggestions.iter().any(|s| s.criterion == "no_repetition"));
        assert!(!suggestions.iter().any(|s| s.criterion == "actionable"));

        let _ = std::fs::remove_file(&db_path);
    }

    // -----------------------------------------------------------------------
    // A/B Testing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_ab_test_no_data() {
        let test = AbTest::new("prompt_test", "Write concisely", "Write in detail");
        assert!(test.winner().is_none());
    }

    #[test]
    fn test_ab_test_insufficient_data() {
        let mut test = AbTest::new("test", "A", "B");
        test.record_result("a", 0.8);
        test.record_result("a", 0.9);
        test.record_result("b", 0.5);
        // Need at least 5 per variant
        assert!(test.winner().is_none());
    }

    #[test]
    fn test_ab_test_clear_winner_a() {
        let mut test = AbTest::new("test", "Detailed prompt", "Short prompt");

        // Variant A consistently scores higher
        for _ in 0..20 {
            test.record_result("a", 0.85 + (rand::random::<f64>() * 0.1));
            test.record_result("b", 0.3 + (rand::random::<f64>() * 0.1));
        }

        assert_eq!(test.winner(), Some("a"), "Variant A should win with significantly higher scores");
    }

    #[test]
    fn test_ab_test_clear_winner_b() {
        let mut test = AbTest::new("test", "Prompt A", "Prompt B");

        // Variant B consistently scores higher
        for _ in 0..20 {
            test.record_result("a", 0.2 + (rand::random::<f64>() * 0.1));
            test.record_result("b", 0.8 + (rand::random::<f64>() * 0.1));
        }

        assert_eq!(test.winner(), Some("b"));
    }

    #[test]
    fn test_ab_test_no_significant_difference() {
        let mut test = AbTest::new("test", "A", "B");

        // Both variants perform similarly
        for _ in 0..20 {
            test.record_result("a", 0.7);
            test.record_result("b", 0.7);
        }

        assert!(test.winner().is_none(), "No winner when scores are identical");
    }

    #[test]
    fn test_ab_test_summary() {
        let mut test = AbTest::new("seo_prompt", "Version 1", "Version 2");
        test.record_result("a", 0.8);
        test.record_result("a", 0.7);
        test.record_result("b", 0.9);
        test.record_result("b", 0.85);

        let summary = test.summary();
        assert_eq!(summary.name, "seo_prompt");
        assert_eq!(summary.count_a, 2);
        assert_eq!(summary.count_b, 2);
        assert!((summary.mean_a - 0.75).abs() < 0.01);
        assert!((summary.mean_b - 0.875).abs() < 0.01);
    }

    #[test]
    fn test_ab_test_case_insensitive_variant() {
        let mut test = AbTest::new("test", "A", "B");
        test.record_result("A", 0.8);
        test.record_result("a", 0.9);
        test.record_result("B", 0.5);
        test.record_result("b", 0.6);

        assert_eq!(test.results_a.len(), 2);
        assert_eq!(test.results_b.len(), 2);
    }

    #[test]
    fn test_ab_test_invalid_variant() {
        let mut test = AbTest::new("test", "A", "B");
        test.record_result("c", 0.8); // Should be ignored
        assert!(test.results_a.is_empty());
        assert!(test.results_b.is_empty());
    }

    // -----------------------------------------------------------------------
    // mean_and_variance tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mean_and_variance_basic() {
        let (mean, var) = mean_and_variance(&[2.0, 4.0, 6.0]);
        assert!((mean - 4.0).abs() < 0.01);
        assert!((var - 4.0).abs() < 0.01); // sample variance of [2,4,6] = 4.0
    }

    #[test]
    fn test_mean_and_variance_single() {
        let (mean, var) = mean_and_variance(&[5.0]);
        assert!((mean - 5.0).abs() < 0.01);
        assert!((var - 0.0).abs() < 0.01); // single element => 0 variance
    }

    #[test]
    fn test_mean_and_variance_empty() {
        let (mean, var) = mean_and_variance(&[]);
        assert!((mean - 0.0).abs() < 0.01);
        assert!((var - 0.0).abs() < 0.01);
    }

    // -----------------------------------------------------------------------
    // Integration: QualityScorer -> ImprovementTracker
    // -----------------------------------------------------------------------

    #[test]
    fn test_scorer_to_tracker_integration() {
        let (db_str, db_path) = temp_db("integration");
        let scorer = QualityScorer::new();
        let tracker = ImprovementTracker::new(&db_str).unwrap();

        let outputs = vec![
            "Short.",
            "# Report\n\n## Section 1\n\n- Item 1\n- Item 2\n\nWe recommend implementing the API with 15 endpoints. Step 1: Deploy to staging. Consider using Docker for production.\n\n## Conclusion\n\nRevenue target: $50,000/month.",
            "# Updated Report\n\n## Analysis\n\n1. Market size: $2.5B\n2. Growth rate: 12% annually\n\nYou should focus on the enterprise segment first. Then expand to SMB.\n\n## Action Items\n\n- Deploy MVP by March 2026\n- Target 100 users in first month",
        ];

        for (i, output) in outputs.iter().enumerate() {
            let score = scorer.score_output(output, &[]);
            let criteria_map: HashMap<String, f64> = score
                .criteria
                .iter()
                .map(|c| (c.name.clone(), c.score))
                .collect();
            tracker
                .record_quality_with_criteria("test_hand", score.overall, (i as i64) * 1000, &criteria_map)
                .unwrap();
        }

        let avg = tracker.avg_quality("test_hand").unwrap();
        assert!(avg > 0.0, "Average quality should be positive");

        let trend = tracker.quality_trend("test_hand", 3).unwrap();
        assert_eq!(trend.data_points, 3);

        let _ = std::fs::remove_file(&db_path);
    }
}
