//! Node Capability Scoring — scores and ranks cluster nodes by performance.
//!
//! Each node gets scored on 4 dimensions:
//! - Stability: based on success/failure ratio
//! - Speed: based on average latency
//! - Cost efficiency: based on cost per successful task
//! - Quality: based on quality scores from evaluations
//!
//! Overall = 0.30*stability + 0.25*speed + 0.25*cost_efficiency + 0.20*quality

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

// ── Data Structures ──────────────────────────────────────────────────────────

/// Performance metrics for a node, used to calculate scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeMetrics {
    pub success_count: u64,
    pub failure_count: u64,
    pub avg_latency_ms: f64,
    pub total_cost: f64,
    pub quality_score: f64,
}

impl Default for NodeMetrics {
    fn default() -> Self {
        Self {
            success_count: 0,
            failure_count: 0,
            avg_latency_ms: 0.0,
            total_cost: 0.0,
            quality_score: 0.0,
        }
    }
}

/// Computed scores for a node (each 0-100).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeScore {
    pub stability: f64,
    pub speed: f64,
    pub cost_efficiency: f64,
    pub quality: f64,
    pub overall: f64,
    pub grade: NodeGrade,
}

impl Default for NodeScore {
    fn default() -> Self {
        Self {
            stability: 0.0,
            speed: 0.0,
            cost_efficiency: 0.0,
            quality: 0.0,
            overall: 0.0,
            grade: NodeGrade::D,
        }
    }
}

/// Letter grade for a node.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum NodeGrade {
    A, // 90+
    B, // 75-89
    C, // 60-74
    D, // <60
}

impl std::fmt::Display for NodeGrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeGrade::A => write!(f, "A"),
            NodeGrade::B => write!(f, "B"),
            NodeGrade::C => write!(f, "C"),
            NodeGrade::D => write!(f, "D"),
        }
    }
}

impl NodeGrade {
    pub fn from_str(s: &str) -> Self {
        match s {
            "A" => NodeGrade::A,
            "B" => NodeGrade::B,
            "C" => NodeGrade::C,
            _ => NodeGrade::D,
        }
    }
}

// ── Scoring Constants ────────────────────────────────────────────────────────

/// Weight for stability in overall score
const W_STABILITY: f64 = 0.30;
/// Weight for speed in overall score
const W_SPEED: f64 = 0.25;
/// Weight for cost efficiency in overall score
const W_COST_EFFICIENCY: f64 = 0.25;
/// Weight for quality in overall score
const W_QUALITY: f64 = 0.20;

/// Latency baseline: at this latency (ms) the speed score is 50
const LATENCY_BASELINE_MS: f64 = 5000.0;
/// Cost baseline: at this cost-per-task the efficiency score is 50
const COST_BASELINE: f64 = 0.10;

// ── NodeScorer ───────────────────────────────────────────────────────────────

/// Scores and ranks cluster nodes. Persists scores to SQLite.
pub struct NodeScorer {
    conn: Mutex<Connection>,
}

impl NodeScorer {
    /// Create a new NodeScorer, creating the `node_scores` table if needed.
    pub async fn new(db_path: &str) -> Result<Self> {
        let path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)?;
                }
            }

            let conn = Connection::open(&path)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA busy_timeout=5000;")?;

            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS node_scores (
                    node_id          TEXT PRIMARY KEY,
                    success_count    INTEGER NOT NULL DEFAULT 0,
                    failure_count    INTEGER NOT NULL DEFAULT 0,
                    avg_latency_ms   REAL NOT NULL DEFAULT 0.0,
                    total_cost       REAL NOT NULL DEFAULT 0.0,
                    quality_score    REAL NOT NULL DEFAULT 0.0,
                    stability        REAL NOT NULL DEFAULT 0.0,
                    speed            REAL NOT NULL DEFAULT 0.0,
                    cost_efficiency  REAL NOT NULL DEFAULT 0.0,
                    quality          REAL NOT NULL DEFAULT 0.0,
                    overall          REAL NOT NULL DEFAULT 0.0,
                    grade            TEXT NOT NULL DEFAULT 'D',
                    updated_at       TEXT NOT NULL
                );",
            )?;

            Ok(conn)
        }).await.map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))??;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Update raw metrics for a node, recalculate scores, and persist.
    pub fn update_metrics(&self, node_id: &str, metrics: NodeMetrics) -> Result<NodeScore> {
        let score = Self::compute_score(&metrics);
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO node_scores
             (node_id, success_count, failure_count, avg_latency_ms, total_cost, quality_score,
              stability, speed, cost_efficiency, quality, overall, grade, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                node_id,
                metrics.success_count as i64,
                metrics.failure_count as i64,
                metrics.avg_latency_ms,
                metrics.total_cost,
                metrics.quality_score,
                score.stability,
                score.speed,
                score.cost_efficiency,
                score.quality,
                score.overall,
                score.grade.to_string(),
                now,
            ],
        )?;

        Ok(score)
    }

    /// Calculate the score for a node from its stored metrics.
    /// Returns default score if node has no stored data.
    pub fn calculate_score(&self, node_id: &str) -> NodeScore {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT success_count, failure_count, avg_latency_ms, total_cost, quality_score
             FROM node_scores WHERE node_id = ?1",
        ) {
            Ok(s) => s,
            Err(_) => return NodeScore::default(),
        };

        match stmt.query_row(params![node_id], |row| {
            Ok(NodeMetrics {
                success_count: row.get::<_, i64>(0)? as u64,
                failure_count: row.get::<_, i64>(1)? as u64,
                avg_latency_ms: row.get(2)?,
                total_cost: row.get(3)?,
                quality_score: row.get(4)?,
            })
        }) {
            Ok(metrics) => Self::compute_score(&metrics),
            Err(_) => NodeScore::default(),
        }
    }

    /// Get rankings for all nodes, sorted by overall score descending.
    pub fn get_rankings(&self) -> Vec<(String, NodeScore)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare(
            "SELECT node_id, stability, speed, cost_efficiency, quality, overall, grade
             FROM node_scores ORDER BY overall DESC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let rows = stmt
            .query_map([], |row| {
                let node_id: String = row.get(0)?;
                let grade_str: String = row.get(6)?;
                let score = NodeScore {
                    stability: row.get(1)?,
                    speed: row.get(2)?,
                    cost_efficiency: row.get(3)?,
                    quality: row.get(4)?,
                    overall: row.get(5)?,
                    grade: NodeGrade::from_str(&grade_str),
                };
                Ok((node_id, score))
            })
            .ok();

        match rows {
            Some(r) => r.filter_map(|x| x.ok()).collect(),
            None => Vec::new(),
        }
    }

    /// Get the grade for a given overall score.
    pub fn get_grade(score: f64) -> NodeGrade {
        if score >= 90.0 {
            NodeGrade::A
        } else if score >= 75.0 {
            NodeGrade::B
        } else if score >= 60.0 {
            NodeGrade::C
        } else {
            NodeGrade::D
        }
    }

    /// Compute all scores from raw metrics.
    ///
    /// Stability (0-100): success_rate * 100. Zero tasks = 0.
    /// Speed (0-100): 100 * baseline / (baseline + latency). Fast = high.
    /// Cost efficiency (0-100): 100 * baseline / (baseline + cost_per_task). Cheap = high.
    /// Quality (0-100): quality_score clamped to 0-100.
    /// Overall: weighted sum.
    fn compute_score(metrics: &NodeMetrics) -> NodeScore {
        let total_tasks = metrics.success_count + metrics.failure_count;

        // Stability: success rate * 100
        let stability = if total_tasks == 0 {
            0.0
        } else {
            (metrics.success_count as f64 / total_tasks as f64) * 100.0
        };

        // Speed: inverse latency mapped to 0-100
        // score = 100 * baseline / (baseline + latency)
        // 0 latency => 100, baseline latency => 50, very high latency => ~0
        let speed = if metrics.avg_latency_ms <= 0.0 {
            if total_tasks > 0 {
                100.0 // Zero latency is perfect
            } else {
                0.0 // No data
            }
        } else {
            100.0 * LATENCY_BASELINE_MS / (LATENCY_BASELINE_MS + metrics.avg_latency_ms)
        };

        // Cost efficiency: inverse cost-per-task mapped to 0-100
        let cost_per_task = if metrics.success_count == 0 {
            0.0
        } else {
            metrics.total_cost / metrics.success_count as f64
        };

        let cost_efficiency = if total_tasks == 0 {
            0.0
        } else if cost_per_task <= 0.0 {
            100.0 // Free is maximally efficient
        } else {
            100.0 * COST_BASELINE / (COST_BASELINE + cost_per_task)
        };

        // Quality: direct score clamped to 0-100
        let quality = metrics.quality_score.clamp(0.0, 100.0);

        // Overall weighted score
        let overall = W_STABILITY * stability
            + W_SPEED * speed
            + W_COST_EFFICIENCY * cost_efficiency
            + W_QUALITY * quality;

        let grade = Self::get_grade(overall);

        NodeScore {
            stability,
            speed,
            cost_efficiency,
            quality,
            overall,
            grade,
        }
    }

    /// Delete a node's scores (e.g. when decommissioned).
    pub fn remove_node(&self, node_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM node_scores WHERE node_id = ?1", params![node_id])?;
        Ok(())
    }

    /// Get metrics and score for a specific node (returns None if unknown).
    pub fn get_node_details(&self, node_id: &str) -> Option<(NodeMetrics, NodeScore)> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT success_count, failure_count, avg_latency_ms, total_cost, quality_score,
                        stability, speed, cost_efficiency, quality, overall, grade
                 FROM node_scores WHERE node_id = ?1",
            )
            .ok()?;

        stmt.query_row(params![node_id], |row| {
            let metrics = NodeMetrics {
                success_count: row.get::<_, i64>(0)? as u64,
                failure_count: row.get::<_, i64>(1)? as u64,
                avg_latency_ms: row.get(2)?,
                total_cost: row.get(3)?,
                quality_score: row.get(4)?,
            };
            let grade_str: String = row.get(10)?;
            let score = NodeScore {
                stability: row.get(5)?,
                speed: row.get(6)?,
                cost_efficiency: row.get(7)?,
                quality: row.get(8)?,
                overall: row.get(9)?,
                grade: NodeGrade::from_str(&grade_str),
            };
            Ok((metrics, score))
        })
        .ok()
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    async fn make_scorer() -> NodeScorer {
        let tmp = NamedTempFile::new().unwrap();
        NodeScorer::new(tmp.path().to_str().unwrap()).await.unwrap()
    }

    #[test]
    fn test_grade_boundaries() {
        assert_eq!(NodeScorer::get_grade(100.0), NodeGrade::A);
        assert_eq!(NodeScorer::get_grade(90.0), NodeGrade::A);
        assert_eq!(NodeScorer::get_grade(89.9), NodeGrade::B);
        assert_eq!(NodeScorer::get_grade(75.0), NodeGrade::B);
        assert_eq!(NodeScorer::get_grade(74.9), NodeGrade::C);
        assert_eq!(NodeScorer::get_grade(60.0), NodeGrade::C);
        assert_eq!(NodeScorer::get_grade(59.9), NodeGrade::D);
        assert_eq!(NodeScorer::get_grade(0.0), NodeGrade::D);
    }

    #[tokio::test]
    async fn test_perfect_node_gets_grade_a() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 100,
            failure_count: 0,
            avg_latency_ms: 100.0, // very fast
            total_cost: 0.0,       // free
            quality_score: 100.0,
        };
        let score = scorer.update_metrics("perfect-node", metrics).unwrap();
        assert_eq!(score.grade, NodeGrade::A);
        assert!(score.overall >= 90.0);
        assert_eq!(score.stability, 100.0);
    }

    #[tokio::test]
    async fn test_terrible_node_gets_grade_d() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 10,
            failure_count: 90,
            avg_latency_ms: 60000.0, // 60 seconds
            total_cost: 50.0,        // $50 for 10 successes = $5/task
            quality_score: 10.0,
        };
        let score = scorer.update_metrics("bad-node", metrics).unwrap();
        assert_eq!(score.grade, NodeGrade::D);
        assert!(score.overall < 60.0);
    }

    #[tokio::test]
    async fn test_stability_100_percent_success() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 50,
            failure_count: 0,
            avg_latency_ms: 5000.0,
            total_cost: 5.0,
            quality_score: 50.0,
        };
        let score = scorer.update_metrics("stable", metrics).unwrap();
        assert_eq!(score.stability, 100.0);
    }

    #[tokio::test]
    async fn test_stability_50_percent() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 50,
            failure_count: 50,
            avg_latency_ms: 5000.0,
            total_cost: 5.0,
            quality_score: 50.0,
        };
        let score = scorer.update_metrics("half", metrics).unwrap();
        assert!((score.stability - 50.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_speed_zero_latency_is_perfect() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 10,
            failure_count: 0,
            avg_latency_ms: 0.0,
            total_cost: 0.0,
            quality_score: 50.0,
        };
        let score = scorer.update_metrics("instant", metrics).unwrap();
        assert_eq!(score.speed, 100.0);
    }

    #[tokio::test]
    async fn test_speed_baseline_latency_is_50() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 10,
            failure_count: 0,
            avg_latency_ms: LATENCY_BASELINE_MS,
            total_cost: 0.0,
            quality_score: 50.0,
        };
        let score = scorer.update_metrics("baseline", metrics).unwrap();
        assert!((score.speed - 50.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_cost_efficiency_free_is_100() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 10,
            failure_count: 0,
            avg_latency_ms: 1000.0,
            total_cost: 0.0, // free
            quality_score: 50.0,
        };
        let score = scorer.update_metrics("free-node", metrics).unwrap();
        assert_eq!(score.cost_efficiency, 100.0);
    }

    #[tokio::test]
    async fn test_overall_formula_weights() {
        // Verify: overall = 0.30*stability + 0.25*speed + 0.25*cost_efficiency + 0.20*quality
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 80,
            failure_count: 20,
            avg_latency_ms: 2000.0,
            total_cost: 8.0,       // $0.10/task
            quality_score: 70.0,
        };
        let score = scorer.update_metrics("weighted", metrics).unwrap();

        let expected_stability = 80.0; // 80/100
        let expected_speed = 100.0 * 5000.0 / (5000.0 + 2000.0); // ~71.43
        let expected_cost_eff = 100.0 * 0.10 / (0.10 + 0.10); // 50.0
        let expected_quality = 70.0;
        let expected_overall = 0.30 * expected_stability
            + 0.25 * expected_speed
            + 0.25 * expected_cost_eff
            + 0.20 * expected_quality;

        assert!((score.stability - expected_stability).abs() < 0.01);
        assert!((score.speed - expected_speed).abs() < 0.01);
        assert!((score.cost_efficiency - expected_cost_eff).abs() < 0.01);
        assert!((score.quality - expected_quality).abs() < 0.01);
        assert!((score.overall - expected_overall).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_rankings_sorted_by_overall() {
        let scorer = make_scorer().await;

        // Add 3 nodes with different quality levels
        scorer
            .update_metrics(
                "best",
                NodeMetrics {
                    success_count: 100,
                    failure_count: 0,
                    avg_latency_ms: 100.0,
                    total_cost: 0.0,
                    quality_score: 95.0,
                },
            )
            .unwrap();
        scorer
            .update_metrics(
                "mid",
                NodeMetrics {
                    success_count: 70,
                    failure_count: 30,
                    avg_latency_ms: 3000.0,
                    total_cost: 7.0,
                    quality_score: 60.0,
                },
            )
            .unwrap();
        scorer
            .update_metrics(
                "worst",
                NodeMetrics {
                    success_count: 20,
                    failure_count: 80,
                    avg_latency_ms: 30000.0,
                    total_cost: 100.0,
                    quality_score: 10.0,
                },
            )
            .unwrap();

        let rankings = scorer.get_rankings();
        assert_eq!(rankings.len(), 3);
        assert_eq!(rankings[0].0, "best");
        assert_eq!(rankings[1].0, "mid");
        assert_eq!(rankings[2].0, "worst");
        assert!(rankings[0].1.overall >= rankings[1].1.overall);
        assert!(rankings[1].1.overall >= rankings[2].1.overall);
    }

    #[tokio::test]
    async fn test_calculate_score_unknown_node() {
        let scorer = make_scorer().await;
        let score = scorer.calculate_score("nonexistent");
        assert_eq!(score.overall, 0.0);
        assert_eq!(score.grade, NodeGrade::D);
    }

    #[tokio::test]
    async fn test_update_and_retrieve() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 90,
            failure_count: 10,
            avg_latency_ms: 1500.0,
            total_cost: 4.5,
            quality_score: 85.0,
        };
        let written = scorer.update_metrics("z13", metrics).unwrap();
        let read = scorer.calculate_score("z13");

        assert!((written.overall - read.overall).abs() < 0.01);
        assert_eq!(written.grade, read.grade);
    }

    #[tokio::test]
    async fn test_remove_node() {
        let scorer = make_scorer().await;
        scorer
            .update_metrics(
                "to-remove",
                NodeMetrics {
                    success_count: 10,
                    failure_count: 0,
                    avg_latency_ms: 1000.0,
                    total_cost: 1.0,
                    quality_score: 50.0,
                },
            )
            .unwrap();
        assert_eq!(scorer.get_rankings().len(), 1);

        scorer.remove_node("to-remove").unwrap();
        assert_eq!(scorer.get_rankings().len(), 0);
        assert!(scorer.get_node_details("to-remove").is_none());
    }

    #[tokio::test]
    async fn test_no_tasks_gives_zero_scores() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 0,
            failure_count: 0,
            avg_latency_ms: 0.0,
            total_cost: 0.0,
            quality_score: 0.0,
        };
        let score = scorer.update_metrics("empty-node", metrics).unwrap();
        assert_eq!(score.stability, 0.0);
        assert_eq!(score.speed, 0.0);
        assert_eq!(score.cost_efficiency, 0.0);
        assert_eq!(score.quality, 0.0);
        assert_eq!(score.overall, 0.0);
        assert_eq!(score.grade, NodeGrade::D);
    }

    #[tokio::test]
    async fn test_quality_clamped_to_100() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 10,
            failure_count: 0,
            avg_latency_ms: 100.0,
            total_cost: 0.0,
            quality_score: 150.0, // over 100 — should be clamped
        };
        let score = scorer.update_metrics("over-quality", metrics).unwrap();
        assert_eq!(score.quality, 100.0);
    }

    #[tokio::test]
    async fn test_get_node_details() {
        let scorer = make_scorer().await;
        let metrics = NodeMetrics {
            success_count: 42,
            failure_count: 8,
            avg_latency_ms: 2500.0,
            total_cost: 3.0,
            quality_score: 77.0,
        };
        scorer.update_metrics("detail-node", metrics.clone()).unwrap();

        let (stored_metrics, stored_score) = scorer.get_node_details("detail-node").unwrap();
        assert_eq!(stored_metrics.success_count, 42);
        assert_eq!(stored_metrics.failure_count, 8);
        assert!((stored_metrics.avg_latency_ms - 2500.0).abs() < 0.01);
        assert!(stored_score.overall > 0.0);
    }
}
