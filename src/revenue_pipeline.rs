//! Revenue Pipeline Orchestrator — chains multiple hands together for end-to-end
//! revenue generation lifecycle: lead generation -> content creation -> payment collection.
//!
//! Provides 8 built-in pipeline definitions, execution tracking, conversion rate
//! analysis, and performance reporting across all revenue pipelines.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

// ── Stage Type ──────────────────────────────────────────────────────────────

/// Classifies what a pipeline stage does in the revenue lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageType {
    /// Creates content, leads, or other raw material
    Generate,
    /// Filters or scores results for quality
    Qualify,
    /// Takes action: send email, publish post, submit proposal
    Execute,
    /// Records metrics and conversion data
    Track,
    /// Records revenue and payment information
    Collect,
}

impl std::fmt::Display for StageType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StageType::Generate => write!(f, "generate"),
            StageType::Qualify => write!(f, "qualify"),
            StageType::Execute => write!(f, "execute"),
            StageType::Track => write!(f, "track"),
            StageType::Collect => write!(f, "collect"),
        }
    }
}

// ── Pipeline Stage ──────────────────────────────────────────────────────────

/// A single step within a pipeline, mapped to a hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStage {
    /// Name of the hand to execute for this stage
    pub hand_name: String,
    /// What this stage does in the lifecycle
    pub stage_type: StageType,
    /// Maximum seconds to wait for this stage to complete
    pub timeout_secs: u64,
    /// Whether to retry on failure (single retry)
    pub retry_on_fail: bool,
    /// Optional jq-like expression to transform input between stages
    pub input_transform: Option<String>,
}

// ── Pipeline Definition ─────────────────────────────────────────────────────

/// A complete pipeline definition: sequence of stages that produce revenue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineDef {
    /// Unique pipeline name
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Ordered list of stages to execute
    pub stages: Vec<PipelineStage>,
    /// Expected USD revenue per full execution cycle
    pub expected_revenue_per_cycle: f64,
    /// Target execution frequency as a cron expression
    pub target_frequency: String,
}

// ── Pipeline Status ─────────────────────────────────────────────────────────

/// Outcome of a pipeline execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PipelineStatus {
    /// Currently running
    Running,
    /// All stages completed successfully
    Completed,
    /// Failed at a specific stage
    Failed {
        stage: String,
        error: String,
    },
    /// Some stages completed before failure
    PartialSuccess {
        completed_stages: usize,
    },
}

impl std::fmt::Display for PipelineStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PipelineStatus::Running => write!(f, "running"),
            PipelineStatus::Completed => write!(f, "completed"),
            PipelineStatus::Failed { stage, error } => {
                write!(f, "failed at '{}': {}", stage, error)
            }
            PipelineStatus::PartialSuccess { completed_stages } => {
                write!(f, "partial ({} stages done)", completed_stages)
            }
        }
    }
}

// ── Pipeline Execution ──────────────────────────────────────────────────────

/// Tracks a single run of a pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineExecution {
    /// Unique execution identifier
    pub id: String,
    /// Name of the pipeline being executed
    pub pipeline_name: String,
    /// When execution started
    pub started_at: DateTime<Utc>,
    /// When execution completed (None if still running)
    pub completed_at: Option<DateTime<Utc>>,
    /// Number of stages completed so far
    pub stages_completed: usize,
    /// Total number of stages in the pipeline
    pub stages_total: usize,
    /// USD revenue recorded during this execution
    pub revenue_generated: f64,
    /// Current status
    pub status: PipelineStatus,
    /// Per-stage completion flags (true = completed)
    pub stage_results: Vec<bool>,
}

// ── Pipeline Stats ──────────────────────────────────────────────────────────

/// Aggregated statistics for a single pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineStats {
    /// Total number of executions
    pub executions: usize,
    /// Number of fully successful executions
    pub successes: usize,
    /// Number of failed executions
    pub failures: usize,
    /// Average revenue per execution (USD)
    pub avg_revenue: f64,
    /// Total revenue from all executions (USD)
    pub total_revenue: f64,
    /// Average execution duration in seconds
    pub avg_duration_secs: f64,
    /// Stage-to-stage conversion rates (stage_index -> rate)
    pub conversion_rates: HashMap<usize, f64>,
}

// ── Revenue Pipeline ────────────────────────────────────────────────────────

/// Orchestrates the complete revenue generation lifecycle.
///
/// Manages pipeline definitions, tracks executions, calculates conversion rates,
/// and produces performance reports.
pub struct RevenuePipeline {
    /// All registered pipeline definitions
    pipelines: Vec<PipelineDef>,
    /// All execution records (active + historical)
    executions: Mutex<Vec<PipelineExecution>>,
}

impl RevenuePipeline {
    /// Create a new empty pipeline orchestrator.
    pub fn new() -> Self {
        Self {
            pipelines: Vec::new(),
            executions: Mutex::new(Vec::new()),
        }
    }

    /// Create a pipeline orchestrator pre-loaded with 8 built-in pipeline definitions.
    pub fn with_defaults() -> Self {
        let mut p = Self::new();
        for def in default_pipelines() {
            p.pipelines.push(def);
        }
        p
    }

    /// Register a custom pipeline definition.
    pub fn add_pipeline(&mut self, def: PipelineDef) {
        self.pipelines.push(def);
    }

    /// List all registered pipeline definitions.
    pub fn list_pipelines(&self) -> Vec<&PipelineDef> {
        self.pipelines.iter().collect()
    }

    /// Start a new execution of the named pipeline. Returns the execution ID.
    pub fn start_execution(&self, pipeline_name: &str) -> Result<String, String> {
        let def = self
            .pipelines
            .iter()
            .find(|p| p.name == pipeline_name)
            .ok_or_else(|| format!("Pipeline '{}' not found", pipeline_name))?;

        let id = Uuid::new_v4().to_string();
        let stages_total = def.stages.len();
        let execution = PipelineExecution {
            id: id.clone(),
            pipeline_name: pipeline_name.to_string(),
            started_at: Utc::now(),
            completed_at: None,
            stages_completed: 0,
            stages_total,
            revenue_generated: 0.0,
            status: PipelineStatus::Running,
            stage_results: vec![false; stages_total],
        };

        let mut execs = self.executions.lock().map_err(|e| format!("lock: {}", e))?;
        execs.push(execution);
        Ok(id)
    }

    /// Mark a stage as successfully completed. Optionally record revenue (USD).
    /// If all stages are complete, the execution status transitions to `Completed`.
    pub fn record_stage_complete(
        &self,
        execution_id: &str,
        stage_index: usize,
        revenue: f64,
    ) -> Result<(), String> {
        let mut execs = self.executions.lock().map_err(|e| format!("lock: {}", e))?;
        let exec = execs
            .iter_mut()
            .find(|e| e.id == execution_id)
            .ok_or_else(|| format!("Execution '{}' not found", execution_id))?;

        if stage_index >= exec.stages_total {
            return Err(format!(
                "Stage index {} out of range (total {})",
                stage_index, exec.stages_total
            ));
        }

        exec.stage_results[stage_index] = true;
        exec.stages_completed = exec.stage_results.iter().filter(|&&b| b).count();
        exec.revenue_generated += revenue;

        // If all stages done, mark completed
        if exec.stages_completed == exec.stages_total {
            exec.status = PipelineStatus::Completed;
            exec.completed_at = Some(Utc::now());
        }

        Ok(())
    }

    /// Mark a stage as failed. Sets execution status to `Failed` and records the error.
    pub fn record_stage_failed(
        &self,
        execution_id: &str,
        stage_index: usize,
        error: &str,
    ) -> Result<(), String> {
        let mut execs = self.executions.lock().map_err(|e| format!("lock: {}", e))?;
        let exec = execs
            .iter_mut()
            .find(|e| e.id == execution_id)
            .ok_or_else(|| format!("Execution '{}' not found", execution_id))?;

        if stage_index >= exec.stages_total {
            return Err(format!(
                "Stage index {} out of range (total {})",
                stage_index, exec.stages_total
            ));
        }

        let stage_name = format!("stage_{}", stage_index);

        // If some stages completed, mark as partial success; otherwise mark as failed
        if exec.stages_completed > 0 {
            exec.status = PipelineStatus::PartialSuccess {
                completed_stages: exec.stages_completed,
            };
        } else {
            exec.status = PipelineStatus::Failed {
                stage: stage_name,
                error: error.to_string(),
            };
        }
        exec.completed_at = Some(Utc::now());

        Ok(())
    }

    /// Return all currently running pipeline executions.
    pub fn active_executions(&self) -> Vec<PipelineExecution> {
        let execs = self.executions.lock().unwrap_or_else(|e| e.into_inner());
        execs
            .iter()
            .filter(|e| e.status == PipelineStatus::Running)
            .cloned()
            .collect()
    }

    /// Return the most recent completed/failed executions, up to `limit`.
    /// Ordered by start time descending (most recent first).
    pub fn execution_history(&self, limit: usize) -> Vec<PipelineExecution> {
        let execs = self.executions.lock().unwrap_or_else(|e| e.into_inner());
        let mut finished: Vec<PipelineExecution> = execs
            .iter()
            .filter(|e| e.status != PipelineStatus::Running)
            .cloned()
            .collect();
        finished.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        finished.truncate(limit);
        finished
    }

    /// Compute aggregated statistics for a named pipeline.
    pub fn pipeline_stats(&self, name: &str) -> PipelineStats {
        let execs = self.executions.lock().unwrap_or_else(|e| e.into_inner());
        let pipeline_execs: Vec<&PipelineExecution> = execs
            .iter()
            .filter(|e| e.pipeline_name == name)
            .collect();

        let executions = pipeline_execs.len();
        let successes = pipeline_execs
            .iter()
            .filter(|e| e.status == PipelineStatus::Completed)
            .count();
        let failures = pipeline_execs
            .iter()
            .filter(|e| matches!(e.status, PipelineStatus::Failed { .. }))
            .count();

        let total_revenue: f64 = pipeline_execs.iter().map(|e| e.revenue_generated).sum();
        let avg_revenue = if executions > 0 {
            total_revenue / executions as f64
        } else {
            0.0
        };

        // Average duration for completed executions
        let durations: Vec<f64> = pipeline_execs
            .iter()
            .filter_map(|e| {
                e.completed_at
                    .map(|end| (end - e.started_at).num_seconds() as f64)
            })
            .collect();
        let avg_duration_secs = if !durations.is_empty() {
            durations.iter().sum::<f64>() / durations.len() as f64
        } else {
            0.0
        };

        // Per-stage conversion rates
        let mut conversion_rates = HashMap::new();
        if let Some(first) = pipeline_execs.first() {
            for i in 0..first.stages_total {
                let reached = pipeline_execs
                    .iter()
                    .filter(|e| {
                        if i == 0 {
                            true // all executions reach stage 0
                        } else {
                            e.stage_results.get(i.saturating_sub(1)).copied().unwrap_or(false)
                        }
                    })
                    .count();
                let completed = pipeline_execs
                    .iter()
                    .filter(|e| e.stage_results.get(i).copied().unwrap_or(false))
                    .count();
                let rate = if reached > 0 {
                    completed as f64 / reached as f64
                } else {
                    0.0
                };
                conversion_rates.insert(i, rate);
            }
        }

        PipelineStats {
            executions,
            successes,
            failures,
            avg_revenue,
            total_revenue,
            avg_duration_secs,
            conversion_rates,
        }
    }

    /// Return the top `n` pipelines ranked by total revenue generated.
    pub fn best_performing_pipelines(&self, n: usize) -> Vec<(String, f64)> {
        let execs = self.executions.lock().unwrap_or_else(|e| e.into_inner());

        let mut totals: HashMap<String, f64> = HashMap::new();
        for exec in execs.iter() {
            *totals.entry(exec.pipeline_name.clone()).or_default() += exec.revenue_generated;
        }

        let mut ranked: Vec<(String, f64)> = totals.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked.truncate(n);
        ranked
    }

    /// Compute the conversion rate between two stages of a pipeline.
    /// Returns the fraction of executions that completed `to_stage` among those that
    /// completed `from_stage`.
    pub fn conversion_rate(
        &self,
        pipeline_name: &str,
        from_stage: usize,
        to_stage: usize,
    ) -> f64 {
        let execs = self.executions.lock().unwrap_or_else(|e| e.into_inner());
        let pipeline_execs: Vec<&PipelineExecution> = execs
            .iter()
            .filter(|e| e.pipeline_name == pipeline_name)
            .collect();

        let from_count = pipeline_execs
            .iter()
            .filter(|e| e.stage_results.get(from_stage).copied().unwrap_or(false))
            .count();

        if from_count == 0 {
            return 0.0;
        }

        let to_count = pipeline_execs
            .iter()
            .filter(|e| e.stage_results.get(to_stage).copied().unwrap_or(false))
            .count();

        to_count as f64 / from_count as f64
    }

    /// Total revenue generated across all pipelines and all executions.
    pub fn total_revenue_generated(&self) -> f64 {
        let execs = self.executions.lock().unwrap_or_else(|e| e.into_inner());
        execs.iter().map(|e| e.revenue_generated).sum()
    }

    /// Generate a human-readable performance report for all pipelines.
    pub fn generate_report(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push("=== Revenue Pipeline Performance Report ===".to_string());
        lines.push(format!("Generated: {}", Utc::now().format("%Y-%m-%d %H:%M UTC")));
        lines.push(format!(
            "Total Revenue (all pipelines): ${:.2}",
            self.total_revenue_generated()
        ));
        lines.push(String::new());

        for def in &self.pipelines {
            let stats = self.pipeline_stats(&def.name);
            lines.push(format!("--- {} ---", def.name));
            lines.push(format!("  Description: {}", def.description));
            lines.push(format!("  Stages: {}", def.stages.len()));
            lines.push(format!("  Frequency: {}", def.target_frequency));
            lines.push(format!(
                "  Expected Revenue/Cycle: ${:.2}",
                def.expected_revenue_per_cycle
            ));
            lines.push(format!("  Executions: {}", stats.executions));
            lines.push(format!("  Successes: {}", stats.successes));
            lines.push(format!("  Failures: {}", stats.failures));
            lines.push(format!("  Total Revenue: ${:.2}", stats.total_revenue));
            lines.push(format!("  Avg Revenue: ${:.2}", stats.avg_revenue));
            lines.push(format!("  Avg Duration: {:.0}s", stats.avg_duration_secs));
            if !stats.conversion_rates.is_empty() {
                let rates: Vec<String> = stats
                    .conversion_rates
                    .iter()
                    .map(|(k, v)| format!("stage_{}: {:.0}%", k, v * 100.0))
                    .collect();
                lines.push(format!("  Conversion: {}", rates.join(", ")));
            }
            lines.push(String::new());
        }

        // Top performers
        let top = self.best_performing_pipelines(3);
        if !top.is_empty() {
            lines.push("--- Top Performers ---".to_string());
            for (i, (name, rev)) in top.iter().enumerate() {
                lines.push(format!("  {}. {} — ${:.2}", i + 1, name, rev));
            }
            lines.push(String::new());
        }

        // Active executions
        let active = self.active_executions();
        if !active.is_empty() {
            lines.push(format!("--- Active Executions ({}) ---", active.len()));
            for exec in &active {
                lines.push(format!(
                    "  {} — {}/{} stages, ${:.2} rev so far",
                    exec.pipeline_name,
                    exec.stages_completed,
                    exec.stages_total,
                    exec.revenue_generated
                ));
            }
        }

        lines.join("\n")
    }
}

impl Default for RevenuePipeline {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ── Built-in Pipeline Definitions ───────────────────────────────────────────

/// Returns the 8 built-in revenue pipeline definitions.
pub fn default_pipelines() -> Vec<PipelineDef> {
    vec![
        // 1. Freelance Pipeline
        PipelineDef {
            name: "freelance".to_string(),
            description: "End-to-end freelance workflow: discover jobs, write proposals, execute, invoice, collect payment".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "upwork_proposal".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 300,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "freelancer".to_string(),
                    stage_type: StageType::Execute,
                    timeout_secs: 600,
                    retry_on_fail: false,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "invoice".to_string(),
                    stage_type: StageType::Collect,
                    timeout_secs: 120,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "payment_tracker".to_string(),
                    stage_type: StageType::Track,
                    timeout_secs: 60,
                    retry_on_fail: false,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 500.0,
            target_frequency: "0 9 * * MON-FRI".to_string(),
        },
        // 2. Content Monetization Pipeline
        PipelineDef {
            name: "content_monetization".to_string(),
            description: "SEO content creation, affiliate integration, blog publishing, and performance tracking".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "seo_content".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 600,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "affiliate_content".to_string(),
                    stage_type: StageType::Qualify,
                    timeout_secs: 300,
                    retry_on_fail: true,
                    input_transform: Some(".content | {text, keywords}".to_string()),
                },
                PipelineStage {
                    hand_name: "blog_publish".to_string(),
                    stage_type: StageType::Execute,
                    timeout_secs: 180,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "tracking".to_string(),
                    stage_type: StageType::Track,
                    timeout_secs: 60,
                    retry_on_fail: false,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 50.0,
            target_frequency: "0 8 * * TUE,THU".to_string(),
        },
        // 3. Outreach Pipeline
        PipelineDef {
            name: "outreach".to_string(),
            description: "Lead generation, cold outreach, consulting preparation, and invoicing".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "lead".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 300,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "cold_outreach".to_string(),
                    stage_type: StageType::Execute,
                    timeout_secs: 300,
                    retry_on_fail: true,
                    input_transform: Some(".leads[] | select(.score > 7)".to_string()),
                },
                PipelineStage {
                    hand_name: "consulting_prep".to_string(),
                    stage_type: StageType::Qualify,
                    timeout_secs: 600,
                    retry_on_fail: false,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "invoice".to_string(),
                    stage_type: StageType::Collect,
                    timeout_secs: 120,
                    retry_on_fail: true,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 1000.0,
            target_frequency: "0 10 * * MON,WED,FRI".to_string(),
        },
        // 4. Newsletter Pipeline
        PipelineDef {
            name: "newsletter".to_string(),
            description: "Newsletter content creation, social scheduling, and subscriber tracking".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "newsletter".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 600,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "content".to_string(),
                    stage_type: StageType::Qualify,
                    timeout_secs: 300,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "social_scheduler".to_string(),
                    stage_type: StageType::Execute,
                    timeout_secs: 180,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "tracking".to_string(),
                    stage_type: StageType::Track,
                    timeout_secs: 60,
                    retry_on_fail: false,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 25.0,
            target_frequency: "0 7 * * WED".to_string(),
        },
        // 5. Ebook Pipeline
        PipelineDef {
            name: "ebook".to_string(),
            description: "Ebook creation, product listing, marketing push, and sales tracking".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "ebook_writer".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 1800,
                    retry_on_fail: false,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "product_review".to_string(),
                    stage_type: StageType::Qualify,
                    timeout_secs: 600,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "marketing".to_string(),
                    stage_type: StageType::Execute,
                    timeout_secs: 300,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "tracking".to_string(),
                    stage_type: StageType::Track,
                    timeout_secs: 60,
                    retry_on_fail: false,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 200.0,
            target_frequency: "0 6 1 * *".to_string(),
        },
        // 6. SaaS Pipeline
        PipelineDef {
            name: "saas".to_string(),
            description: "SaaS landing page generation, API monetization setup, and Stripe revenue tracking".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "saas_landing".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 900,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "api_monetize".to_string(),
                    stage_type: StageType::Execute,
                    timeout_secs: 600,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "stripe_tracking".to_string(),
                    stage_type: StageType::Collect,
                    timeout_secs: 120,
                    retry_on_fail: true,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 300.0,
            target_frequency: "0 10 * * MON".to_string(),
        },
        // 7. Translation Pipeline
        PipelineDef {
            name: "translation".to_string(),
            description: "Translation service execution, invoicing, and payment tracking".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "translation_service".to_string(),
                    stage_type: StageType::Execute,
                    timeout_secs: 900,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "invoice".to_string(),
                    stage_type: StageType::Collect,
                    timeout_secs: 120,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "payment_tracker".to_string(),
                    stage_type: StageType::Track,
                    timeout_secs: 60,
                    retry_on_fail: false,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 150.0,
            target_frequency: "0 9 * * MON-FRI".to_string(),
        },
        // 8. Data Services Pipeline
        PipelineDef {
            name: "data_services".to_string(),
            description: "Data report generation, consulting preparation, invoicing, and payment tracking".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "data_report".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 900,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "consulting_prep".to_string(),
                    stage_type: StageType::Qualify,
                    timeout_secs: 600,
                    retry_on_fail: false,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "invoice".to_string(),
                    stage_type: StageType::Collect,
                    timeout_secs: 120,
                    retry_on_fail: true,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "payment_tracker".to_string(),
                    stage_type: StageType::Track,
                    timeout_secs: 60,
                    retry_on_fail: false,
                    input_transform: None,
                },
            ],
            expected_revenue_per_cycle: 250.0,
            target_frequency: "0 14 * * WED".to_string(),
        },
    ]
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_pipelines_created() {
        let rp = RevenuePipeline::with_defaults();
        let pipelines = rp.list_pipelines();
        assert_eq!(pipelines.len(), 8);

        let names: Vec<&str> = pipelines.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"freelance"));
        assert!(names.contains(&"content_monetization"));
        assert!(names.contains(&"outreach"));
        assert!(names.contains(&"newsletter"));
        assert!(names.contains(&"ebook"));
        assert!(names.contains(&"saas"));
        assert!(names.contains(&"translation"));
        assert!(names.contains(&"data_services"));
    }

    #[test]
    fn test_start_execution_creates_entry() {
        let rp = RevenuePipeline::with_defaults();
        let id = rp.start_execution("freelance").unwrap();
        assert!(!id.is_empty());

        let active = rp.active_executions();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pipeline_name, "freelance");
        assert_eq!(active[0].status, PipelineStatus::Running);
        assert_eq!(active[0].stages_completed, 0);
        assert_eq!(active[0].stages_total, 4);
    }

    #[test]
    fn test_start_execution_unknown_pipeline() {
        let rp = RevenuePipeline::with_defaults();
        let result = rp.start_execution("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_stage_completion_updates_status() {
        let rp = RevenuePipeline::with_defaults();
        let id = rp.start_execution("freelance").unwrap();

        rp.record_stage_complete(&id, 0, 0.0).unwrap();
        let active = rp.active_executions();
        assert_eq!(active[0].stages_completed, 1);
        assert_eq!(active[0].status, PipelineStatus::Running);
    }

    #[test]
    fn test_stage_failure_records_error() {
        let rp = RevenuePipeline::with_defaults();
        let id = rp.start_execution("freelance").unwrap();

        rp.record_stage_failed(&id, 0, "API timeout").unwrap();
        let history = rp.execution_history(10);
        assert_eq!(history.len(), 1);
        match &history[0].status {
            PipelineStatus::Failed { stage, error } => {
                assert_eq!(stage, "stage_0");
                assert_eq!(error, "API timeout");
            }
            other => panic!("Expected Failed, got {:?}", other),
        }
    }

    #[test]
    fn test_pipeline_completion_with_revenue() {
        let rp = RevenuePipeline::with_defaults();
        let id = rp.start_execution("freelance").unwrap();

        rp.record_stage_complete(&id, 0, 0.0).unwrap();
        rp.record_stage_complete(&id, 1, 0.0).unwrap();
        rp.record_stage_complete(&id, 2, 450.0).unwrap();
        rp.record_stage_complete(&id, 3, 0.0).unwrap();

        let history = rp.execution_history(10);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].status, PipelineStatus::Completed);
        assert!((history[0].revenue_generated - 450.0).abs() < f64::EPSILON);
        assert!(history[0].completed_at.is_some());
    }

    #[test]
    fn test_execution_history_ordering() {
        let rp = RevenuePipeline::with_defaults();

        // Start two executions
        let id1 = rp.start_execution("freelance").unwrap();
        let id2 = rp.start_execution("outreach").unwrap();

        // Fail the first
        rp.record_stage_failed(&id1, 0, "err1").unwrap();
        // Complete the second quickly
        for i in 0..4 {
            rp.record_stage_complete(&id2, i, 0.0).unwrap();
        }

        let history = rp.execution_history(10);
        assert_eq!(history.len(), 2);
        // Most recent start first — id2 was started after id1
        assert_eq!(history[0].pipeline_name, "outreach");
        assert_eq!(history[1].pipeline_name, "freelance");
    }

    #[test]
    fn test_pipeline_stats_calculation() {
        let rp = RevenuePipeline::with_defaults();

        // Run freelance twice: one success, one failure
        let id1 = rp.start_execution("freelance").unwrap();
        for i in 0..4 {
            rp.record_stage_complete(&id1, i, if i == 2 { 500.0 } else { 0.0 })
                .unwrap();
        }

        let id2 = rp.start_execution("freelance").unwrap();
        rp.record_stage_failed(&id2, 1, "timeout").unwrap();

        let stats = rp.pipeline_stats("freelance");
        assert_eq!(stats.executions, 2);
        assert_eq!(stats.successes, 1);
        // id2 had 0 stages completed before failure, so it's a Failed, not PartialSuccess
        assert_eq!(stats.failures, 1);
        assert!((stats.total_revenue - 500.0).abs() < f64::EPSILON);
        assert!((stats.avg_revenue - 250.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_best_performing_sorting() {
        let rp = RevenuePipeline::with_defaults();

        // Freelance generates $500
        let id1 = rp.start_execution("freelance").unwrap();
        for i in 0..4 {
            rp.record_stage_complete(&id1, i, if i == 2 { 500.0 } else { 0.0 })
                .unwrap();
        }

        // Outreach generates $1000
        let id2 = rp.start_execution("outreach").unwrap();
        for i in 0..4 {
            rp.record_stage_complete(&id2, i, if i == 3 { 1000.0 } else { 0.0 })
                .unwrap();
        }

        // SaaS generates $200
        let id3 = rp.start_execution("saas").unwrap();
        for i in 0..3 {
            rp.record_stage_complete(&id3, i, if i == 2 { 200.0 } else { 0.0 })
                .unwrap();
        }

        let top = rp.best_performing_pipelines(2);
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].0, "outreach");
        assert!((top[0].1 - 1000.0).abs() < f64::EPSILON);
        assert_eq!(top[1].0, "freelance");
        assert!((top[1].1 - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_conversion_rate_calculation() {
        let rp = RevenuePipeline::with_defaults();

        // Execution 1: completes stages 0, 1, 2, 3
        let id1 = rp.start_execution("freelance").unwrap();
        for i in 0..4 {
            rp.record_stage_complete(&id1, i, 0.0).unwrap();
        }

        // Execution 2: completes stages 0, 1 only, then fails at 2
        let id2 = rp.start_execution("freelance").unwrap();
        rp.record_stage_complete(&id2, 0, 0.0).unwrap();
        rp.record_stage_complete(&id2, 1, 0.0).unwrap();
        rp.record_stage_failed(&id2, 2, "err").unwrap();

        // Conversion from stage 1 to stage 2: 1 out of 2 = 0.5
        let rate = rp.conversion_rate("freelance", 1, 2);
        assert!((rate - 0.5).abs() < f64::EPSILON);

        // Conversion from stage 0 to stage 1: 2 out of 2 = 1.0
        let rate01 = rp.conversion_rate("freelance", 0, 1);
        assert!((rate01 - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_total_revenue_aggregation() {
        let rp = RevenuePipeline::with_defaults();

        let id1 = rp.start_execution("freelance").unwrap();
        rp.record_stage_complete(&id1, 0, 100.0).unwrap();
        rp.record_stage_complete(&id1, 1, 200.0).unwrap();

        let id2 = rp.start_execution("outreach").unwrap();
        rp.record_stage_complete(&id2, 0, 300.0).unwrap();

        let total = rp.total_revenue_generated();
        assert!((total - 600.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_partial_success_status() {
        let rp = RevenuePipeline::with_defaults();
        let id = rp.start_execution("freelance").unwrap();

        // Complete stage 0, then fail at stage 1
        rp.record_stage_complete(&id, 0, 0.0).unwrap();
        rp.record_stage_failed(&id, 1, "network error").unwrap();

        let history = rp.execution_history(10);
        assert_eq!(history.len(), 1);
        match &history[0].status {
            PipelineStatus::PartialSuccess { completed_stages } => {
                assert_eq!(*completed_stages, 1);
            }
            other => panic!("Expected PartialSuccess, got {:?}", other),
        }
    }

    #[test]
    fn test_custom_pipeline_creation() {
        let mut rp = RevenuePipeline::new();
        assert_eq!(rp.list_pipelines().len(), 0);

        rp.add_pipeline(PipelineDef {
            name: "custom_test".to_string(),
            description: "A custom test pipeline".to_string(),
            stages: vec![
                PipelineStage {
                    hand_name: "step_one".to_string(),
                    stage_type: StageType::Generate,
                    timeout_secs: 60,
                    retry_on_fail: false,
                    input_transform: None,
                },
                PipelineStage {
                    hand_name: "step_two".to_string(),
                    stage_type: StageType::Collect,
                    timeout_secs: 30,
                    retry_on_fail: true,
                    input_transform: Some(".result".to_string()),
                },
            ],
            expected_revenue_per_cycle: 100.0,
            target_frequency: "0 12 * * *".to_string(),
        });

        assert_eq!(rp.list_pipelines().len(), 1);
        assert_eq!(rp.list_pipelines()[0].name, "custom_test");
        assert_eq!(rp.list_pipelines()[0].stages.len(), 2);

        // Execute it
        let id = rp.start_execution("custom_test").unwrap();
        rp.record_stage_complete(&id, 0, 0.0).unwrap();
        rp.record_stage_complete(&id, 1, 100.0).unwrap();

        let history = rp.execution_history(10);
        assert_eq!(history[0].status, PipelineStatus::Completed);
        assert!((history[0].revenue_generated - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_report_generation() {
        let rp = RevenuePipeline::with_defaults();

        let id = rp.start_execution("freelance").unwrap();
        for i in 0..4 {
            rp.record_stage_complete(&id, i, if i == 2 { 500.0 } else { 0.0 })
                .unwrap();
        }

        let report = rp.generate_report();
        assert!(report.contains("Revenue Pipeline Performance Report"));
        assert!(report.contains("freelance"));
        assert!(report.contains("$500.00"));
        assert!(report.contains("Total Revenue (all pipelines)"));
    }

    #[test]
    fn test_concurrent_executions() {
        let rp = RevenuePipeline::with_defaults();

        // Start multiple pipelines simultaneously
        let id1 = rp.start_execution("freelance").unwrap();
        let id2 = rp.start_execution("outreach").unwrap();
        let id3 = rp.start_execution("saas").unwrap();

        let active = rp.active_executions();
        assert_eq!(active.len(), 3);

        // Complete one
        for i in 0..4 {
            rp.record_stage_complete(&id1, i, 0.0).unwrap();
        }

        let active = rp.active_executions();
        assert_eq!(active.len(), 2);

        // Fail another
        rp.record_stage_failed(&id2, 0, "err").unwrap();
        let active = rp.active_executions();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].pipeline_name, "saas");

        // Verify history
        let history = rp.execution_history(10);
        assert_eq!(history.len(), 2);
    }

    #[test]
    fn test_stage_out_of_range() {
        let rp = RevenuePipeline::with_defaults();
        let id = rp.start_execution("freelance").unwrap();

        let result = rp.record_stage_complete(&id, 99, 0.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("out of range"));
    }

    #[test]
    fn test_empty_pipeline_stats() {
        let rp = RevenuePipeline::with_defaults();
        let stats = rp.pipeline_stats("freelance");
        assert_eq!(stats.executions, 0);
        assert_eq!(stats.successes, 0);
        assert_eq!(stats.failures, 0);
        assert!((stats.avg_revenue - 0.0).abs() < f64::EPSILON);
        assert!((stats.total_revenue - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_stage_type_display() {
        assert_eq!(format!("{}", StageType::Generate), "generate");
        assert_eq!(format!("{}", StageType::Qualify), "qualify");
        assert_eq!(format!("{}", StageType::Execute), "execute");
        assert_eq!(format!("{}", StageType::Track), "track");
        assert_eq!(format!("{}", StageType::Collect), "collect");
    }

    #[test]
    fn test_pipeline_status_display() {
        assert_eq!(format!("{}", PipelineStatus::Running), "running");
        assert_eq!(format!("{}", PipelineStatus::Completed), "completed");
        assert_eq!(
            format!(
                "{}",
                PipelineStatus::Failed {
                    stage: "s1".to_string(),
                    error: "boom".to_string()
                }
            ),
            "failed at 's1': boom"
        );
        assert_eq!(
            format!(
                "{}",
                PipelineStatus::PartialSuccess {
                    completed_stages: 2
                }
            ),
            "partial (2 stages done)"
        );
    }

    #[test]
    fn test_default_trait() {
        let rp = RevenuePipeline::default();
        assert_eq!(rp.list_pipelines().len(), 8);
    }

    #[test]
    fn test_conversion_rate_no_executions() {
        let rp = RevenuePipeline::with_defaults();
        let rate = rp.conversion_rate("freelance", 0, 1);
        assert!((rate - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_best_performing_empty() {
        let rp = RevenuePipeline::with_defaults();
        let top = rp.best_performing_pipelines(5);
        assert!(top.is_empty());
    }

    #[test]
    fn test_multiple_revenue_accumulation() {
        let rp = RevenuePipeline::with_defaults();

        // Two freelance runs
        let id1 = rp.start_execution("freelance").unwrap();
        for i in 0..4 {
            rp.record_stage_complete(&id1, i, if i == 2 { 500.0 } else { 0.0 })
                .unwrap();
        }

        let id2 = rp.start_execution("freelance").unwrap();
        for i in 0..4 {
            rp.record_stage_complete(&id2, i, if i == 2 { 700.0 } else { 0.0 })
                .unwrap();
        }

        let stats = rp.pipeline_stats("freelance");
        assert_eq!(stats.executions, 2);
        assert_eq!(stats.successes, 2);
        assert!((stats.total_revenue - 1200.0).abs() < f64::EPSILON);
        assert!((stats.avg_revenue - 600.0).abs() < f64::EPSILON);
    }
}
