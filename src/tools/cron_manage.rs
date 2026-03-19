// Cron management tool — programmatic CRUD for cron jobs via agent tool calling
// Actions: list_jobs, create_job, update_job, delete_job, trigger_job
// Uses ~/.clawtex/core.db SQLite table `cron_jobs` (CronStore)

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::cron::{
    next_cron_run, CronJob, CronStore, JobAction, JobStatus, Schedule,
};
use super::{Tool, ToolResult};

/// Local helper: compute next run for a Schedule (mirrors the private cron::compute_next_run)
fn compute_next_run(schedule: &Schedule, after: &chrono::DateTime<Utc>) -> Option<chrono::DateTime<Utc>> {
    match schedule {
        Schedule::Cron { expr } => next_cron_run(expr, *after),
        Schedule::At { at } => {
            if at > after { Some(*at) } else { None }
        }
        Schedule::Every { interval_secs } => {
            Some(*after + chrono::Duration::seconds(*interval_secs as i64))
        }
    }
}

/// Allowed actions for the cron_manage tool
const ALLOWED_ACTIONS: &[&str] = &[
    "list_jobs",
    "create_job",
    "update_job",
    "delete_job",
    "trigger_job",
];

/// Validate that a cron expression is syntactically valid (5-field)
fn validate_cron_expr(expr: &str) -> Result<(), String> {
    let fields: Vec<&str> = expr.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "Cron expression must have exactly 5 fields (minute hour day month weekday), got {}",
            fields.len()
        ));
    }
    // Use the project's own parser to validate — if it can compute a next run, the expression is valid
    let now = Utc::now();
    match next_cron_run(expr, now) {
        Some(_) => Ok(()),
        None => Err(format!(
            "Invalid cron expression '{}': could not compute next run time",
            expr
        )),
    }
}

/// Tool for programmatic cron job management
pub struct CronManageTool {
    store: Arc<CronStore>,
}

impl CronManageTool {
    pub fn new(store: Arc<CronStore>) -> Self {
        Self { store }
    }

    /// List all cron jobs
    fn action_list_jobs(&self) -> Result<ToolResult> {
        let jobs = self.store.list_all()?;
        if jobs.is_empty() {
            return Ok(ToolResult {
                success: true,
                output: "No cron jobs found.".into(),
            });
        }
        let lines: Vec<String> = jobs
            .iter()
            .map(|j| {
                let schedule_str = match &j.schedule {
                    Schedule::Cron { expr } => format!("cron({})", expr),
                    Schedule::At { at } => format!("at({})", at.to_rfc3339()),
                    Schedule::Every { interval_secs } => format!("every({}s)", interval_secs),
                };
                let enabled = matches!(j.status, JobStatus::Active);
                let hand = match &j.action {
                    JobAction::Hand { hand_name, .. } => hand_name.clone(),
                    JobAction::Shell { command } => format!("shell:{}", command),
                    JobAction::Agent { agent, .. } => format!("agent:{}", agent),
                    JobAction::Notify { chat_id, .. } => format!("notify:{}", chat_id),
                };
                format!(
                    "- [{}] name={}, schedule={}, hand/action={}, enabled={}, runs={}, last_run={}, next_run={}",
                    j.id,
                    j.name,
                    schedule_str,
                    hand,
                    enabled,
                    j.run_count,
                    j.last_run
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_else(|| "never".into()),
                    j.next_run
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_else(|| "none".into()),
                )
            })
            .collect();
        Ok(ToolResult {
            success: true,
            output: format!("Found {} cron jobs:\n{}", jobs.len(), lines.join("\n")),
        })
    }

    /// Create a new cron job that runs a hand workflow
    fn action_create_job(
        &self,
        name: &str,
        schedule: &str,
        hand_name: &str,
        input: Option<&str>,
    ) -> Result<ToolResult> {
        if name.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required field 'name'".into(),
            });
        }
        if schedule.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required field 'schedule'".into(),
            });
        }
        if hand_name.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required field 'hand_name'".into(),
            });
        }

        // Validate cron expression
        if let Err(e) = validate_cron_expr(schedule) {
            return Ok(ToolResult {
                success: false,
                output: format!("Invalid schedule: {}", e),
            });
        }

        // Check for duplicate name
        let existing = self.store.list_all()?;
        if existing.iter().any(|j| j.name == name) {
            return Ok(ToolResult {
                success: false,
                output: format!("A cron job named '{}' already exists", name),
            });
        }

        let now = Utc::now();
        let sched = Schedule::Cron {
            expr: schedule.to_string(),
        };
        let next_run = compute_next_run(&sched, &now);

        let job = CronJob {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            schedule: sched,
            action: JobAction::Hand {
                hand_name: hand_name.to_string(),
                input: input.unwrap_or("").to_string(),
            },
            status: JobStatus::Active,
            next_run,
            last_run: None,
            last_result: None,
            run_count: 0,
            max_runs: None,
            created_at: now,
        };

        let id = job.id.clone();
        self.store.save_job(&job)?;
        Ok(ToolResult {
            success: true,
            output: format!(
                "Created cron job '{}' (id={}) schedule='{}' hand='{}' next_run={}",
                name,
                id,
                schedule,
                hand_name,
                next_run
                    .map(|t| t.to_rfc3339())
                    .unwrap_or_else(|| "none".into()),
            ),
        })
    }

    /// Update an existing cron job (schedule and/or enabled status)
    fn action_update_job(
        &self,
        name: &str,
        new_schedule: Option<&str>,
        enabled: Option<bool>,
    ) -> Result<ToolResult> {
        if name.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required field 'name'".into(),
            });
        }

        // Validate new schedule if provided
        if let Some(sched) = new_schedule {
            if let Err(e) = validate_cron_expr(sched) {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Invalid schedule: {}", e),
                });
            }
        }

        let jobs = self.store.list_all()?;
        let job = jobs.iter().find(|j| j.name == name);
        let mut job = match job {
            Some(j) => j.clone(),
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Cron job '{}' not found", name),
                });
            }
        };

        let mut changes = Vec::new();

        if let Some(sched) = new_schedule {
            job.schedule = Schedule::Cron {
                expr: sched.to_string(),
            };
            job.next_run = compute_next_run(&job.schedule, &Utc::now());
            changes.push(format!("schedule='{}'", sched));
        }

        if let Some(en) = enabled {
            if en {
                job.status = JobStatus::Active;
                // Recompute next_run when re-enabling
                if job.next_run.is_none() {
                    job.next_run = compute_next_run(&job.schedule, &Utc::now());
                }
            } else {
                job.status = JobStatus::Paused;
            }
            changes.push(format!("enabled={}", en));
        }

        if changes.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "No updates specified. Provide 'new_schedule' and/or 'enabled'.".into(),
            });
        }

        self.store.save_job(&job)?;
        Ok(ToolResult {
            success: true,
            output: format!(
                "Updated cron job '{}' (id={}): {}",
                name,
                job.id,
                changes.join(", ")
            ),
        })
    }

    /// Delete a cron job by name
    fn action_delete_job(&self, name: &str) -> Result<ToolResult> {
        if name.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required field 'name'".into(),
            });
        }

        let jobs = self.store.list_all()?;
        let job = jobs.iter().find(|j| j.name == name);
        match job {
            Some(j) => {
                let id = j.id.clone();
                let deleted = self.store.delete_job(&id)?;
                if deleted {
                    Ok(ToolResult {
                        success: true,
                        output: format!("Deleted cron job '{}' (id={})", name, id),
                    })
                } else {
                    Ok(ToolResult {
                        success: false,
                        output: format!("Failed to delete cron job '{}'", name),
                    })
                }
            }
            None => Ok(ToolResult {
                success: false,
                output: format!("Cron job '{}' not found", name),
            }),
        }
    }

    /// Trigger a job for immediate execution by setting next_run to now
    fn action_trigger_job(&self, name: &str) -> Result<ToolResult> {
        if name.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing required field 'name'".into(),
            });
        }

        let jobs = self.store.list_all()?;
        let job = jobs.iter().find(|j| j.name == name);
        let mut job = match job {
            Some(j) => j.clone(),
            None => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Cron job '{}' not found", name),
                });
            }
        };

        // Must be active or paused to trigger
        if job.status == JobStatus::Completed || job.status == JobStatus::Failed {
            return Ok(ToolResult {
                success: false,
                output: format!(
                    "Cannot trigger job '{}': status is {:?}",
                    name, job.status
                ),
            });
        }

        // Ensure it is active so the scheduler picks it up
        job.status = JobStatus::Active;
        // Set next_run to now so the scheduler will execute it on the next tick
        let now = Utc::now();
        job.next_run = Some(now);
        self.store.save_job(&job)?;

        Ok(ToolResult {
            success: true,
            output: format!(
                "Triggered cron job '{}' (id={}) for immediate execution at {}",
                name,
                job.id,
                now.to_rfc3339()
            ),
        })
    }
}

#[async_trait]
impl Tool for CronManageTool {
    fn name(&self) -> &str {
        "cron_manage"
    }

    fn description(&self) -> &str {
        "Manage cron/scheduled jobs: list, create, update, delete, or trigger immediate execution. \
         Jobs run Hand workflows on a cron schedule."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list_jobs", "create_job", "update_job", "delete_job", "trigger_job"],
                    "description": "The action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Name of the cron job (required for create/update/delete/trigger)"
                },
                "schedule": {
                    "type": "string",
                    "description": "5-field cron expression, e.g. '0 9 * * *' for daily at 9AM (required for create, optional for update)"
                },
                "hand_name": {
                    "type": "string",
                    "description": "Name of the Hand workflow to run (required for create)"
                },
                "input": {
                    "type": "string",
                    "description": "Input text passed to the Hand workflow (optional, for create)"
                },
                "new_schedule": {
                    "type": "string",
                    "description": "New 5-field cron expression (for update action)"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "Enable (true) or disable/pause (false) the job (for update action)"
                }
            },
            "required": ["action"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        // Validate action is in the allowlist
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !ALLOWED_ACTIONS.contains(&action) {
            anyhow::bail!(
                "Invalid action '{}'. Allowed: {}",
                action,
                ALLOWED_ACTIONS.join(", ")
            );
        }

        // Validate cron expression if provided (for create or update)
        if action == "create_job" {
            if let Some(sched) = args.get("schedule").and_then(|v| v.as_str()) {
                if !sched.is_empty() {
                    if let Err(e) = validate_cron_expr(sched) {
                        anyhow::bail!("Preflight: {}", e);
                    }
                }
            }
        }
        if action == "update_job" {
            if let Some(sched) = args.get("new_schedule").and_then(|v| v.as_str()) {
                if !sched.is_empty() {
                    if let Err(e) = validate_cron_expr(sched) {
                        anyhow::bail!("Preflight: {}", e);
                    }
                }
            }
        }

        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match action {
            "list_jobs" => self.action_list_jobs(),
            "create_job" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let schedule = args.get("schedule").and_then(|v| v.as_str()).unwrap_or("");
                let hand_name = args
                    .get("hand_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let input = args.get("input").and_then(|v| v.as_str());
                self.action_create_job(name, schedule, hand_name, input)
            }
            "update_job" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let new_schedule = args.get("new_schedule").and_then(|v| v.as_str());
                let enabled = args.get("enabled").and_then(|v| v.as_bool());
                self.action_update_job(name, new_schedule, enabled)
            }
            "delete_job" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                self.action_delete_job(name)
            }
            "trigger_job" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                self.action_trigger_job(name)
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!(
                    "Unknown action '{}'. Allowed: {}",
                    action,
                    ALLOWED_ACTIONS.join(", ")
                ),
            }),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a temp CronStore backed by a fresh SQLite DB
    fn make_store() -> (Arc<CronStore>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_cron_manage.db");
        let store = Arc::new(CronStore::new(db_path.to_str().unwrap()).unwrap());
        (store, dir)
    }

    /// Convenience: build the tool with a fresh store
    fn make_tool() -> (CronManageTool, Arc<CronStore>, tempfile::TempDir) {
        let (store, dir) = make_store();
        let tool = CronManageTool::new(store.clone());
        (tool, store, dir)
    }

    // ── 1. Trait basics ─────────────────────────────────────────────────

    #[test]
    fn test_tool_name_and_description() {
        let (tool, _store, _dir) = make_tool();
        assert_eq!(tool.name(), "cron_manage");
        assert!(!tool.description().is_empty());
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
    }

    // ── 2. Preflight validation ─────────────────────────────────────────

    #[test]
    fn test_preflight_rejects_invalid_action() {
        let (tool, _store, _dir) = make_tool();
        let result = tool.preflight(&json!({"action": "hack_system"}));
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("Invalid action"));
    }

    #[test]
    fn test_preflight_rejects_bad_cron_expr_create() {
        let (tool, _store, _dir) = make_tool();
        let result = tool.preflight(&json!({
            "action": "create_job",
            "schedule": "bad expr"
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_preflight_rejects_bad_cron_expr_update() {
        let (tool, _store, _dir) = make_tool();
        let result = tool.preflight(&json!({
            "action": "update_job",
            "new_schedule": "99 99 99 99 99"
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_preflight_passes_valid_action() {
        let (tool, _store, _dir) = make_tool();
        assert!(tool.preflight(&json!({"action": "list_jobs"})).is_ok());
        assert!(tool
            .preflight(&json!({"action": "create_job", "schedule": "0 9 * * *"}))
            .is_ok());
        assert!(tool.preflight(&json!({"action": "delete_job"})).is_ok());
        assert!(tool.preflight(&json!({"action": "trigger_job"})).is_ok());
        assert!(tool
            .preflight(&json!({"action": "update_job", "new_schedule": "*/5 * * * *"}))
            .is_ok());
    }

    // ── 3. list_jobs (empty) ────────────────────────────────────────────

    #[tokio::test]
    async fn test_list_jobs_empty() {
        let (tool, _store, _dir) = make_tool();
        let result = tool.execute(json!({"action": "list_jobs"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No cron jobs found"));
    }

    // ── 4. create_job ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_create_job_success() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_job",
                "name": "daily_freelancer",
                "schedule": "0 9 * * *",
                "hand_name": "freelancer",
                "input": "AI automation jobs"
            }))
            .await
            .unwrap();
        assert!(result.success, "Create failed: {}", result.output);
        assert!(result.output.contains("daily_freelancer"));
        assert!(result.output.contains("freelancer"));
    }

    #[tokio::test]
    async fn test_create_job_missing_name() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_job",
                "schedule": "0 9 * * *",
                "hand_name": "freelancer"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_create_job_missing_schedule() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_job",
                "name": "test",
                "hand_name": "freelancer"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_create_job_missing_hand_name() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_job",
                "name": "test",
                "schedule": "0 9 * * *"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_create_job_invalid_schedule() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_job",
                "name": "bad_job",
                "schedule": "not a cron",
                "hand_name": "freelancer"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Invalid schedule"));
    }

    #[tokio::test]
    async fn test_create_job_duplicate_name() {
        let (tool, _store, _dir) = make_tool();
        // Create first
        let r = tool
            .execute(json!({
                "action": "create_job",
                "name": "dup_test",
                "schedule": "0 9 * * *",
                "hand_name": "freelancer"
            }))
            .await
            .unwrap();
        assert!(r.success);

        // Create duplicate
        let r2 = tool
            .execute(json!({
                "action": "create_job",
                "name": "dup_test",
                "schedule": "0 10 * * *",
                "hand_name": "seo_content"
            }))
            .await
            .unwrap();
        assert!(!r2.success);
        assert!(r2.output.contains("already exists"));
    }

    // ── 5. list_jobs (with data) ────────────────────────────────────────

    #[tokio::test]
    async fn test_list_jobs_with_entries() {
        let (tool, _store, _dir) = make_tool();
        // Create two jobs
        tool.execute(json!({
            "action": "create_job",
            "name": "job_alpha",
            "schedule": "0 9 * * *",
            "hand_name": "freelancer"
        }))
        .await
        .unwrap();
        tool.execute(json!({
            "action": "create_job",
            "name": "job_beta",
            "schedule": "30 14 * * 1-5",
            "hand_name": "seo_content"
        }))
        .await
        .unwrap();

        let result = tool.execute(json!({"action": "list_jobs"})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Found 2 cron jobs"));
        assert!(result.output.contains("job_alpha"));
        assert!(result.output.contains("job_beta"));
    }

    // ── 6. update_job ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_update_job_schedule() {
        let (tool, _store, _dir) = make_tool();
        tool.execute(json!({
            "action": "create_job",
            "name": "upd_test",
            "schedule": "0 9 * * *",
            "hand_name": "freelancer"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({
                "action": "update_job",
                "name": "upd_test",
                "new_schedule": "30 10 * * *"
            }))
            .await
            .unwrap();
        assert!(result.success, "Update failed: {}", result.output);
        assert!(result.output.contains("schedule='30 10 * * *'"));
    }

    #[tokio::test]
    async fn test_update_job_disable_enable() {
        let (tool, _store, _dir) = make_tool();
        tool.execute(json!({
            "action": "create_job",
            "name": "toggle_test",
            "schedule": "0 9 * * *",
            "hand_name": "freelancer"
        }))
        .await
        .unwrap();

        // Disable
        let r = tool
            .execute(json!({
                "action": "update_job",
                "name": "toggle_test",
                "enabled": false
            }))
            .await
            .unwrap();
        assert!(r.success);
        assert!(r.output.contains("enabled=false"));

        // Re-enable
        let r2 = tool
            .execute(json!({
                "action": "update_job",
                "name": "toggle_test",
                "enabled": true
            }))
            .await
            .unwrap();
        assert!(r2.success);
        assert!(r2.output.contains("enabled=true"));
    }

    #[tokio::test]
    async fn test_update_job_not_found() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "update_job",
                "name": "nonexistent",
                "enabled": false
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_update_job_no_changes() {
        let (tool, _store, _dir) = make_tool();
        tool.execute(json!({
            "action": "create_job",
            "name": "noop_test",
            "schedule": "0 9 * * *",
            "hand_name": "freelancer"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({
                "action": "update_job",
                "name": "noop_test"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("No updates specified"));
    }

    // ── 7. delete_job ───────────────────────────────────────────────────

    #[tokio::test]
    async fn test_delete_job_success() {
        let (tool, _store, _dir) = make_tool();
        tool.execute(json!({
            "action": "create_job",
            "name": "del_me",
            "schedule": "0 9 * * *",
            "hand_name": "freelancer"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({
                "action": "delete_job",
                "name": "del_me"
            }))
            .await
            .unwrap();
        assert!(result.success, "Delete failed: {}", result.output);
        assert!(result.output.contains("Deleted"));

        // Verify it's gone
        let list = tool.execute(json!({"action": "list_jobs"})).await.unwrap();
        assert!(list.output.contains("No cron jobs found"));
    }

    #[tokio::test]
    async fn test_delete_job_not_found() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "delete_job",
                "name": "ghost"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }

    // ── 8. trigger_job ──────────────────────────────────────────────────

    #[tokio::test]
    async fn test_trigger_job_success() {
        let (tool, store, _dir) = make_tool();
        tool.execute(json!({
            "action": "create_job",
            "name": "trig_test",
            "schedule": "0 3 * * *",
            "hand_name": "freelancer"
        }))
        .await
        .unwrap();

        let before = Utc::now();
        let result = tool
            .execute(json!({
                "action": "trigger_job",
                "name": "trig_test"
            }))
            .await
            .unwrap();
        assert!(result.success, "Trigger failed: {}", result.output);
        assert!(result.output.contains("Triggered"));
        assert!(result.output.contains("immediate execution"));

        // Verify next_run was set to approximately now
        let jobs = store.list_all().unwrap();
        let job = jobs.iter().find(|j| j.name == "trig_test").unwrap();
        let next = job.next_run.unwrap();
        let diff = (next - before).num_seconds().abs();
        assert!(diff < 5, "next_run should be ~now, diff was {}s", diff);
    }

    #[tokio::test]
    async fn test_trigger_job_not_found() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "trigger_job",
                "name": "missing"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_trigger_completed_job_fails() {
        let (tool, store, _dir) = make_tool();
        // Create a job, then mark it completed via the store
        tool.execute(json!({
            "action": "create_job",
            "name": "done_job",
            "schedule": "0 9 * * *",
            "hand_name": "freelancer"
        }))
        .await
        .unwrap();

        // Manually mark as completed
        let jobs = store.list_all().unwrap();
        let mut job = jobs.iter().find(|j| j.name == "done_job").unwrap().clone();
        job.status = JobStatus::Completed;
        store.save_job(&job).unwrap();

        let result = tool
            .execute(json!({
                "action": "trigger_job",
                "name": "done_job"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Cannot trigger"));
    }

    // ── 9. Unknown action ───────────────────────────────────────────────

    #[tokio::test]
    async fn test_unknown_action() {
        let (tool, _store, _dir) = make_tool();
        let result = tool
            .execute(json!({"action": "destroy_all"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    // ── 10. validate_cron_expr unit tests ───────────────────────────────

    #[test]
    fn test_validate_cron_expr_valid() {
        assert!(validate_cron_expr("0 9 * * *").is_ok());
        assert!(validate_cron_expr("*/5 * * * *").is_ok());
        assert!(validate_cron_expr("30 14 * * 1-5").is_ok());
        assert!(validate_cron_expr("0 0 1 * *").is_ok());
    }

    #[test]
    fn test_validate_cron_expr_invalid() {
        assert!(validate_cron_expr("bad").is_err());
        assert!(validate_cron_expr("* * *").is_err()); // only 3 fields
        assert!(validate_cron_expr("99 99 99 99 99").is_err());
        assert!(validate_cron_expr("").is_err());
    }

    // ── 11. create_job with input ───────────────────────────────────────

    #[tokio::test]
    async fn test_create_job_with_and_without_input() {
        let (tool, store, _dir) = make_tool();

        // With input
        tool.execute(json!({
            "action": "create_job",
            "name": "with_input",
            "schedule": "0 9 * * *",
            "hand_name": "freelancer",
            "input": "Search for Rust jobs"
        }))
        .await
        .unwrap();

        // Without input
        tool.execute(json!({
            "action": "create_job",
            "name": "without_input",
            "schedule": "0 10 * * *",
            "hand_name": "seo_content"
        }))
        .await
        .unwrap();

        let jobs = store.list_all().unwrap();
        let with = jobs.iter().find(|j| j.name == "with_input").unwrap();
        let without = jobs.iter().find(|j| j.name == "without_input").unwrap();

        match &with.action {
            JobAction::Hand { input, .. } => assert_eq!(input, "Search for Rust jobs"),
            _ => panic!("Expected Hand action"),
        }
        match &without.action {
            JobAction::Hand { input, .. } => assert_eq!(input, ""),
            _ => panic!("Expected Hand action"),
        }
    }

    // ── 12. Full lifecycle: create -> list -> update -> trigger -> delete

    #[tokio::test]
    async fn test_full_lifecycle() {
        let (tool, _store, _dir) = make_tool();

        // Create
        let r = tool
            .execute(json!({
                "action": "create_job",
                "name": "lifecycle",
                "schedule": "0 8 * * *",
                "hand_name": "content",
                "input": "daily content generation"
            }))
            .await
            .unwrap();
        assert!(r.success);

        // List — should find it
        let r = tool.execute(json!({"action": "list_jobs"})).await.unwrap();
        assert!(r.success);
        assert!(r.output.contains("lifecycle"));
        assert!(r.output.contains("content"));

        // Update schedule
        let r = tool
            .execute(json!({
                "action": "update_job",
                "name": "lifecycle",
                "new_schedule": "30 9 * * 1-5"
            }))
            .await
            .unwrap();
        assert!(r.success);

        // Trigger immediate
        let r = tool
            .execute(json!({
                "action": "trigger_job",
                "name": "lifecycle"
            }))
            .await
            .unwrap();
        assert!(r.success);

        // Delete
        let r = tool
            .execute(json!({
                "action": "delete_job",
                "name": "lifecycle"
            }))
            .await
            .unwrap();
        assert!(r.success);

        // Verify gone
        let r = tool.execute(json!({"action": "list_jobs"})).await.unwrap();
        assert!(r.output.contains("No cron jobs found"));
    }
}
