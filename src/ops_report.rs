//! Operations Report Generator
//! Generates daily/weekly operational reports from available data sources:
//! - costs.db: cost tracking data
//! - cluster registry: node health status
//! - task queue: task completion stats
//! Formats reports for Telegram delivery with emoji indicators.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::cluster::ClusterRegistry;
use crate::cost_tracker::CostTracker;
use crate::task_queue::TaskQueue;

/// Report type discriminant
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ReportType {
    Daily,
    Weekly,
}

impl std::fmt::Display for ReportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReportType::Daily => write!(f, "Daily"),
            ReportType::Weekly => write!(f, "Weekly"),
        }
    }
}

/// Health status of a cluster node
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NodeStatus {
    Online,
    Degraded,
    Offline,
}

impl std::fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NodeStatus::Online => write!(f, "Online"),
            NodeStatus::Degraded => write!(f, "Degraded"),
            NodeStatus::Offline => write!(f, "Offline"),
        }
    }
}

/// Health info for a single cluster node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHealth {
    pub node_id: String,
    pub status: NodeStatus,
    pub cpu_load: f64,
    pub uptime_hours: f64,
}

/// Cost section of the report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostSection {
    pub today_usd: f64,
    pub mtd_usd: f64,
    pub daily_budget: f64,
    pub pct_used: f64,
    pub top_provider: (String, f64),
}

/// Task section of the report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSection {
    pub completed_today: u32,
    pub failed_today: u32,
    pub success_rate: f64,
    pub in_flight: u32,
}

/// Pipeline section (optional, for revenue tracking)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSection {
    pub active_cases: u32,
    pub total_value: f64,
    pub overdue: u32,
}

/// The full operations report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpsReport {
    pub report_type: ReportType,
    pub generated_at: DateTime<Utc>,
    pub cluster_health: Vec<NodeHealth>,
    pub cost_summary: CostSection,
    pub task_summary: TaskSection,
    pub alerts: Vec<String>,
    pub pipeline_summary: Option<PipelineSection>,
}

/// Operations report generator
pub struct OpsReporter {
    daily_budget: f64,
}

impl OpsReporter {
    pub fn new(daily_budget: f64) -> Self {
        Self { daily_budget }
    }

    /// Generate a daily operations report from live data sources
    pub async fn generate_daily_report(
        &self,
        cost_tracker: Option<&CostTracker>,
        cluster: Option<&ClusterRegistry>,
        task_queue: Option<&TaskQueue>,
    ) -> OpsReport {
        let now = Utc::now();
        let mut alerts = Vec::new();

        // -- Cluster health --
        let cluster_health = self.collect_cluster_health(cluster, &mut alerts).await;

        // -- Cost summary --
        let cost_summary = self.collect_cost_summary(cost_tracker, 1, &mut alerts);

        // -- Task summary --
        let task_summary = self.collect_task_summary(task_queue, &mut alerts).await;

        OpsReport {
            report_type: ReportType::Daily,
            generated_at: now,
            cluster_health,
            cost_summary,
            task_summary,
            alerts,
            pipeline_summary: None,
        }
    }

    /// Generate a weekly operations report from live data sources
    pub async fn generate_weekly_report(
        &self,
        cost_tracker: Option<&CostTracker>,
        cluster: Option<&ClusterRegistry>,
        task_queue: Option<&TaskQueue>,
    ) -> OpsReport {
        let now = Utc::now();
        let mut alerts = Vec::new();

        let cluster_health = self.collect_cluster_health(cluster, &mut alerts).await;
        let cost_summary = self.collect_cost_summary(cost_tracker, 7, &mut alerts);
        let task_summary = self.collect_task_summary(task_queue, &mut alerts).await;

        OpsReport {
            report_type: ReportType::Weekly,
            generated_at: now,
            cluster_health,
            cost_summary,
            task_summary,
            alerts,
            pipeline_summary: None,
        }
    }

    /// Format the report for Telegram delivery with emoji indicators
    pub fn format_telegram(report: &OpsReport) -> String {
        let mut lines = Vec::new();

        // Header
        let type_emoji = match report.report_type {
            ReportType::Daily => "\u{1F4CA}", // chart_bar
            ReportType::Weekly => "\u{1F4C8}", // chart_increasing
        };
        lines.push(format!(
            "{} *Clawtex {} Operations Report*",
            type_emoji,
            report.report_type
        ));
        lines.push(format!(
            "_Generated: {}_",
            report.generated_at.format("%Y-%m-%d %H:%M UTC")
        ));
        lines.push(String::new());

        // Cluster Health
        lines.push("\u{1F5A5} *Cluster Health*".to_string()); // desktop
        if report.cluster_health.is_empty() {
            lines.push("  No nodes registered".to_string());
        } else {
            for node in &report.cluster_health {
                let status_icon = match node.status {
                    NodeStatus::Online => "\u{2705}",   // check
                    NodeStatus::Degraded => "\u{26A0}",  // warning
                    NodeStatus::Offline => "\u{274C}",   // cross
                };
                lines.push(format!(
                    "  {} {} | CPU: {:.0}% | Up: {:.1}h",
                    status_icon, node.node_id, node.cpu_load * 100.0, node.uptime_hours
                ));
            }
        }
        lines.push(String::new());

        // Cost Summary
        lines.push("\u{1F4B0} *Cost Summary*".to_string()); // money bag
        let budget_icon = if report.cost_summary.pct_used > 90.0 {
            "\u{1F534}" // red circle
        } else if report.cost_summary.pct_used > 70.0 {
            "\u{1F7E1}" // yellow circle
        } else {
            "\u{1F7E2}" // green circle
        };
        lines.push(format!(
            "  {} Today: ${:.4} / ${:.2} ({:.1}%)",
            budget_icon,
            report.cost_summary.today_usd,
            report.cost_summary.daily_budget,
            report.cost_summary.pct_used
        ));
        lines.push(format!(
            "  MTD: ${:.4}",
            report.cost_summary.mtd_usd
        ));
        if !report.cost_summary.top_provider.0.is_empty() {
            lines.push(format!(
                "  Top provider: {} (${:.4})",
                report.cost_summary.top_provider.0,
                report.cost_summary.top_provider.1
            ));
        }
        lines.push(String::new());

        // Task Summary
        lines.push("\u{1F4CB} *Task Summary*".to_string()); // clipboard
        let rate_icon = if report.task_summary.success_rate >= 95.0 {
            "\u{1F7E2}" // green circle
        } else if report.task_summary.success_rate >= 80.0 {
            "\u{1F7E1}" // yellow circle
        } else {
            "\u{1F534}" // red circle
        };
        lines.push(format!(
            "  {} Success rate: {:.1}%",
            rate_icon, report.task_summary.success_rate
        ));
        lines.push(format!(
            "  Completed: {} | Failed: {} | In-flight: {}",
            report.task_summary.completed_today,
            report.task_summary.failed_today,
            report.task_summary.in_flight
        ));
        lines.push(String::new());

        // Pipeline (if present)
        if let Some(ref pipeline) = report.pipeline_summary {
            lines.push("\u{1F4E6} *Pipeline*".to_string()); // package
            lines.push(format!(
                "  Active: {} | Value: ${:.2} | Overdue: {}",
                pipeline.active_cases, pipeline.total_value, pipeline.overdue
            ));
            lines.push(String::new());
        }

        // Alerts
        if !report.alerts.is_empty() {
            lines.push("\u{26A0}\u{FE0F} *Alerts*".to_string()); // warning
            for alert in &report.alerts {
                lines.push(format!("  \u{2022} {}", alert));
            }
        }

        lines.join("\n")
    }

    // -- Internal helpers --

    async fn collect_cluster_health(
        &self,
        cluster: Option<&ClusterRegistry>,
        alerts: &mut Vec<String>,
    ) -> Vec<NodeHealth> {
        let Some(registry) = cluster else {
            return Vec::new();
        };

        let nodes = registry.status().await;
        let mut health = Vec::new();
        let mut offline_count = 0u32;

        for node in &nodes {
            let status = match node.status.as_str() {
                "online" => {
                    if node.cpu_load > 0.9 {
                        NodeStatus::Degraded
                    } else {
                        NodeStatus::Online
                    }
                }
                "offline" => {
                    offline_count += 1;
                    NodeStatus::Offline
                }
                _ => NodeStatus::Offline,
            };

            health.push(NodeHealth {
                node_id: node.name.clone(),
                status,
                cpu_load: node.cpu_load as f64,
                uptime_hours: 0.0, // Not tracked per-node currently
            });
        }

        if offline_count > 0 {
            alerts.push(format!("{} node(s) offline", offline_count));
        }

        // Check for high CPU nodes
        let high_cpu: Vec<&str> = nodes
            .iter()
            .filter(|n| n.cpu_load > 0.9 && n.status == "online")
            .map(|n| n.name.as_str())
            .collect();
        if !high_cpu.is_empty() {
            alerts.push(format!("High CPU: {}", high_cpu.join(", ")));
        }

        health
    }

    fn collect_cost_summary(
        &self,
        cost_tracker: Option<&CostTracker>,
        days: u32,
        alerts: &mut Vec<String>,
    ) -> CostSection {
        let Some(ct) = cost_tracker else {
            return CostSection {
                today_usd: 0.0,
                mtd_usd: 0.0,
                daily_budget: self.daily_budget,
                pct_used: 0.0,
                top_provider: (String::new(), 0.0),
            };
        };

        let today_total = ct.today_total().ok();
        let today_usd = today_total.as_ref().map(|t| t.total_cost_usd).unwrap_or(0.0);

        // MTD: sum of all days this month (use by_day to approximate)
        let by_day = ct.by_day(days.max(30)).unwrap_or_default();
        let now = Utc::now();
        let current_month = now.format("%Y-%m").to_string();
        let mtd_usd: f64 = by_day
            .iter()
            .filter(|d| d.group.starts_with(&current_month))
            .map(|d| d.total_cost_usd)
            .sum();

        let pct_used = if self.daily_budget > 0.0 {
            (today_usd / self.daily_budget) * 100.0
        } else {
            0.0
        };

        // Top provider (last N days)
        let by_provider = ct.by_provider(days).unwrap_or_default();
        let top_provider = by_provider
            .first()
            .map(|p| (p.group.clone(), p.total_cost_usd))
            .unwrap_or_else(|| (String::new(), 0.0));

        // Budget alerts
        if pct_used > 90.0 {
            alerts.push(format!("Budget CRITICAL: {:.1}% used", pct_used));
        } else if pct_used > 70.0 {
            alerts.push(format!("Budget WARNING: {:.1}% used", pct_used));
        }

        CostSection {
            today_usd,
            mtd_usd,
            daily_budget: self.daily_budget,
            pct_used,
            top_provider,
        }
    }

    async fn collect_task_summary(
        &self,
        task_queue: Option<&TaskQueue>,
        alerts: &mut Vec<String>,
    ) -> TaskSection {
        let Some(tq) = task_queue else {
            return TaskSection {
                completed_today: 0,
                failed_today: 0,
                success_rate: 100.0,
                in_flight: 0,
            };
        };

        // Get recent tasks (up to 200 for statistics)
        let tasks = tq.history(200).await.unwrap_or_default();

        let completed = tasks
            .iter()
            .filter(|t| t.status == crate::task_queue::TaskStatus::Done)
            .count() as u32;
        let failed = tasks
            .iter()
            .filter(|t| t.status == crate::task_queue::TaskStatus::Failed)
            .count() as u32;
        let in_flight = tasks
            .iter()
            .filter(|t| {
                t.status == crate::task_queue::TaskStatus::Running
                    || t.status == crate::task_queue::TaskStatus::Pending
            })
            .count() as u32;

        let total_finished = completed + failed;
        let success_rate = if total_finished > 0 {
            (completed as f64 / total_finished as f64) * 100.0
        } else {
            100.0
        };

        // Alert on low success rate
        if total_finished > 5 && success_rate < 80.0 {
            alerts.push(format!(
                "Task success rate low: {:.1}% ({} failed)",
                success_rate, failed
            ));
        }

        TaskSection {
            completed_today: completed,
            failed_today: failed,
            success_rate,
            in_flight,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== Unit tests for structs and formatting =====

    fn sample_report(report_type: ReportType) -> OpsReport {
        OpsReport {
            report_type,
            generated_at: Utc::now(),
            cluster_health: vec![
                NodeHealth {
                    node_id: "z13".to_string(),
                    status: NodeStatus::Online,
                    cpu_load: 0.35,
                    uptime_hours: 48.0,
                },
                NodeHealth {
                    node_id: "m1-mac".to_string(),
                    status: NodeStatus::Degraded,
                    cpu_load: 0.92,
                    uptime_hours: 24.0,
                },
                NodeHealth {
                    node_id: "acer".to_string(),
                    status: NodeStatus::Offline,
                    cpu_load: 0.0,
                    uptime_hours: 0.0,
                },
            ],
            cost_summary: CostSection {
                today_usd: 0.0523,
                mtd_usd: 1.234,
                daily_budget: 5.0,
                pct_used: 1.046,
                top_provider: ("gemini".to_string(), 0.03),
            },
            task_summary: TaskSection {
                completed_today: 42,
                failed_today: 3,
                success_rate: 93.3,
                in_flight: 2,
            },
            alerts: vec![
                "1 node(s) offline".to_string(),
                "High CPU: m1-mac".to_string(),
            ],
            pipeline_summary: Some(PipelineSection {
                active_cases: 5,
                total_value: 1500.0,
                overdue: 1,
            }),
        }
    }

    #[test]
    fn test_report_type_display() {
        assert_eq!(ReportType::Daily.to_string(), "Daily");
        assert_eq!(ReportType::Weekly.to_string(), "Weekly");
    }

    #[test]
    fn test_node_status_display() {
        assert_eq!(NodeStatus::Online.to_string(), "Online");
        assert_eq!(NodeStatus::Degraded.to_string(), "Degraded");
        assert_eq!(NodeStatus::Offline.to_string(), "Offline");
    }

    #[test]
    fn test_format_telegram_daily_has_header() {
        let report = sample_report(ReportType::Daily);
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("Daily Operations Report"));
        assert!(formatted.contains("Generated:"));
    }

    #[test]
    fn test_format_telegram_weekly_has_header() {
        let report = sample_report(ReportType::Weekly);
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("Weekly Operations Report"));
    }

    #[test]
    fn test_format_telegram_includes_cluster_health() {
        let report = sample_report(ReportType::Daily);
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("Cluster Health"));
        assert!(formatted.contains("z13"));
        assert!(formatted.contains("m1-mac"));
        assert!(formatted.contains("acer"));
    }

    #[test]
    fn test_format_telegram_includes_cost_summary() {
        let report = sample_report(ReportType::Daily);
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("Cost Summary"));
        assert!(formatted.contains("$0.0523"));
        assert!(formatted.contains("gemini"));
    }

    #[test]
    fn test_format_telegram_includes_task_summary() {
        let report = sample_report(ReportType::Daily);
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("Task Summary"));
        assert!(formatted.contains("93.3%"));
        assert!(formatted.contains("Completed: 42"));
        assert!(formatted.contains("Failed: 3"));
    }

    #[test]
    fn test_format_telegram_includes_pipeline() {
        let report = sample_report(ReportType::Daily);
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("Pipeline"));
        assert!(formatted.contains("Active: 5"));
        assert!(formatted.contains("$1500.00"));
    }

    #[test]
    fn test_format_telegram_no_pipeline() {
        let mut report = sample_report(ReportType::Daily);
        report.pipeline_summary = None;
        let formatted = OpsReporter::format_telegram(&report);
        assert!(!formatted.contains("Pipeline"));
    }

    #[test]
    fn test_format_telegram_includes_alerts() {
        let report = sample_report(ReportType::Daily);
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("Alerts"));
        assert!(formatted.contains("1 node(s) offline"));
        assert!(formatted.contains("High CPU: m1-mac"));
    }

    #[test]
    fn test_format_telegram_no_alerts() {
        let mut report = sample_report(ReportType::Daily);
        report.alerts.clear();
        let formatted = OpsReporter::format_telegram(&report);
        assert!(!formatted.contains("Alerts"));
    }

    #[test]
    fn test_format_telegram_empty_cluster() {
        let mut report = sample_report(ReportType::Daily);
        report.cluster_health.clear();
        let formatted = OpsReporter::format_telegram(&report);
        assert!(formatted.contains("No nodes registered"));
    }

    #[test]
    fn test_format_telegram_budget_critical_indicator() {
        let mut report = sample_report(ReportType::Daily);
        report.cost_summary.pct_used = 95.0;
        let formatted = OpsReporter::format_telegram(&report);
        // Should contain red circle indicator
        assert!(formatted.contains("\u{1F534}"));
    }

    #[test]
    fn test_format_telegram_budget_warning_indicator() {
        let mut report = sample_report(ReportType::Daily);
        report.cost_summary.pct_used = 75.0;
        let formatted = OpsReporter::format_telegram(&report);
        // Should contain yellow circle indicator
        assert!(formatted.contains("\u{1F7E1}"));
    }

    #[test]
    fn test_format_telegram_budget_ok_indicator() {
        let mut report = sample_report(ReportType::Daily);
        report.cost_summary.pct_used = 30.0;
        let formatted = OpsReporter::format_telegram(&report);
        // Should contain green circle indicator
        assert!(formatted.contains("\u{1F7E2}"));
    }

    #[test]
    fn test_ops_reporter_new() {
        let reporter = OpsReporter::new(10.0);
        assert_eq!(reporter.daily_budget, 10.0);
    }

    #[test]
    fn test_report_serialization() {
        let report = sample_report(ReportType::Daily);
        let json = serde_json::to_string(&report).unwrap();
        let back: OpsReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.report_type, ReportType::Daily);
        assert_eq!(back.cluster_health.len(), 3);
        assert_eq!(back.alerts.len(), 2);
    }

    // ===== Integration tests with real data sources =====

    #[tokio::test]
    async fn test_generate_daily_no_sources() {
        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_daily_report(None, None, None)
            .await;
        assert_eq!(report.report_type, ReportType::Daily);
        assert!(report.cluster_health.is_empty());
        assert_eq!(report.cost_summary.today_usd, 0.0);
        assert_eq!(report.task_summary.completed_today, 0);
        assert_eq!(report.task_summary.success_rate, 100.0);
    }

    #[tokio::test]
    async fn test_generate_weekly_no_sources() {
        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_weekly_report(None, None, None)
            .await;
        assert_eq!(report.report_type, ReportType::Weekly);
    }

    #[tokio::test]
    async fn test_generate_daily_with_cluster() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("worker1", "10.0.0.2", 7879).await.unwrap();
        registry.register("worker2", "10.0.0.3", 7880).await.unwrap();

        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_daily_report(None, Some(&registry), None)
            .await;

        // Should include local + worker1 + worker2
        assert!(report.cluster_health.len() >= 3);
    }

    #[tokio::test]
    async fn test_generate_daily_offline_alert() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("stale", "10.0.0.5", 7879).await.unwrap();

        // Force the node to be offline
        {
            let conn = registry.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'stale'",
                rusqlite::params![old],
            )
            .unwrap();
        }
        registry.mark_offline_stale(60).await;

        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_daily_report(None, Some(&registry), None)
            .await;

        assert!(report.alerts.iter().any(|a| a.contains("offline")));
    }

    #[tokio::test]
    async fn test_generate_daily_high_cpu_alert() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("hot-node", "10.0.0.6", 7879).await.unwrap();
        registry.heartbeat("hot-node", 0.95).await.unwrap();

        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_daily_report(None, Some(&registry), None)
            .await;

        assert!(report.alerts.iter().any(|a| a.contains("High CPU")));
    }

    #[tokio::test]
    async fn test_generate_daily_with_cost_tracker() {
        let dir = std::env::temp_dir().join("clawtex_test_ops_cost");
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("ops_cost.db");
        let _ = std::fs::remove_file(&db);

        let ct = CostTracker::new(db.to_str().unwrap()).unwrap();
        let rec = crate::cost_tracker::CostRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            agent: "master".to_string(),
            provider: "gemini".to_string(),
            model: "flash".to_string(),
            tokens_in: 500,
            tokens_out: 500,
            total_tokens: 1000,
            estimated_cost_usd: 0.05,
            duration_secs: 1.0,
            context: None,
        };
        ct.record(&rec).unwrap();

        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_daily_report(Some(&ct), None, None)
            .await;

        assert!(report.cost_summary.today_usd >= 0.05);
        assert!(report.cost_summary.pct_used > 0.0);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_generate_daily_with_task_queue() {
        let tq = TaskQueue::new(":memory:").await.unwrap();
        let id1 = tq.add("task1", "prompt1").await.unwrap();
        let id2 = tq.add("task2", "prompt2").await.unwrap();
        let id3 = tq.add("task3", "prompt3").await.unwrap();

        tq.set_status(&id1, crate::task_queue::TaskStatus::Done, Some("ok"), None)
            .unwrap();
        tq.set_status(&id2, crate::task_queue::TaskStatus::Done, Some("ok"), None)
            .unwrap();
        tq.set_status(
            &id3,
            crate::task_queue::TaskStatus::Failed,
            Some("error"),
            None,
        )
        .unwrap();

        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_daily_report(None, None, Some(&tq))
            .await;

        assert_eq!(report.task_summary.completed_today, 2);
        assert_eq!(report.task_summary.failed_today, 1);
        assert!((report.task_summary.success_rate - 66.7).abs() < 1.0);
    }

    #[tokio::test]
    async fn test_generate_daily_budget_alert() {
        let dir = std::env::temp_dir().join("clawtex_test_ops_budget_alert");
        let _ = std::fs::create_dir_all(&dir);
        let db = dir.join("ops_cost_alert.db");
        let _ = std::fs::remove_file(&db);

        let ct = CostTracker::new(db.to_str().unwrap()).unwrap();
        let rec = crate::cost_tracker::CostRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            agent: "master".to_string(),
            provider: "anthropic".to_string(),
            model: "opus".to_string(),
            tokens_in: 10000,
            tokens_out: 10000,
            total_tokens: 20000,
            estimated_cost_usd: 4.8,
            duration_secs: 5.0,
            context: None,
        };
        ct.record(&rec).unwrap();

        let reporter = OpsReporter::new(5.0);
        let report = reporter
            .generate_daily_report(Some(&ct), None, None)
            .await;

        // 4.8/5.0 = 96% -> CRITICAL alert
        assert!(report.alerts.iter().any(|a| a.contains("CRITICAL")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn test_full_report_format_telegram() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("w1", "10.0.0.2", 7879).await.unwrap();
        registry.heartbeat("w1", 0.45).await.unwrap();

        let tq = TaskQueue::new(":memory:").await.unwrap();
        let id = tq.add("test", "prompt").await.unwrap();
        tq.set_status(&id, crate::task_queue::TaskStatus::Done, Some("ok"), None)
            .unwrap();

        let reporter = OpsReporter::new(10.0);
        let report = reporter
            .generate_daily_report(None, Some(&registry), Some(&tq))
            .await;

        let telegram = OpsReporter::format_telegram(&report);
        assert!(!telegram.is_empty());
        assert!(telegram.contains("Daily Operations Report"));
        assert!(telegram.contains("Cluster Health"));
        assert!(telegram.contains("Cost Summary"));
        assert!(telegram.contains("Task Summary"));
    }
}
