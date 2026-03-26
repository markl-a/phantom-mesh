//! Optimizer policy store.
//!
//! Phase P0 goal:
//! - persist versioned optimizer policies
//! - persist optimization runs
//! - provide a stable SQLite-backed foundation for the governor

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::str::FromStr;
use std::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyType {
    Prompt,
    Routing,
    Workflow,
    ToolCapability,
    RuntimeTuning,
}

impl std::fmt::Display for PolicyType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PolicyType::Prompt => "prompt",
            PolicyType::Routing => "routing",
            PolicyType::Workflow => "workflow",
            PolicyType::ToolCapability => "tool_capability",
            PolicyType::RuntimeTuning => "runtime_tuning",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for PolicyType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "prompt" => Ok(Self::Prompt),
            "routing" => Ok(Self::Routing),
            "workflow" => Ok(Self::Workflow),
            "tool_capability" => Ok(Self::ToolCapability),
            "runtime_tuning" => Ok(Self::RuntimeTuning),
            _ => Err(anyhow::anyhow!("unknown policy type '{}'", s)),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyStatus {
    Draft,
    Canary,
    Active,
    RolledBack,
    Rejected,
}

impl std::fmt::Display for PolicyStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            PolicyStatus::Draft => "draft",
            PolicyStatus::Canary => "canary",
            PolicyStatus::Active => "active",
            PolicyStatus::RolledBack => "rolled_back",
            PolicyStatus::Rejected => "rejected",
        };
        write!(f, "{}", s)
    }
}

impl FromStr for PolicyStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "draft" => Ok(Self::Draft),
            "canary" => Ok(Self::Canary),
            "active" => Ok(Self::Active),
            "rolled_back" => Ok(Self::RolledBack),
            "rejected" => Ok(Self::Rejected),
            _ => Err(anyhow::anyhow!("unknown policy status '{}'", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyVersion {
    pub policy_ref: String,
    pub policy_id: String,
    pub policy_type: PolicyType,
    pub version: i64,
    pub content_json: String,
    pub status: PolicyStatus,
    pub created_at: String,
    pub activated_at: Option<String>,
    pub replaced_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationRun {
    pub run_id: String,
    pub run_type: String,
    pub target_scope: String,
    pub input_window: String,
    pub baseline_policy_ref: Option<String>,
    pub candidate_policy_ref: Option<String>,
    pub decision: String,
    pub summary: String,
    pub created_at: String,
}

pub struct OptimizerStore {
    conn: Mutex<Connection>,
}

impl OptimizerStore {
    pub fn new(db_path: &str) -> Result<Self> {
        if let Some(parent) = Path::new(db_path).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS policy_versions (
                policy_id      TEXT NOT NULL,
                policy_type    TEXT NOT NULL,
                version        INTEGER NOT NULL,
                content_json   TEXT NOT NULL,
                status         TEXT NOT NULL,
                created_at     TEXT NOT NULL,
                activated_at   TEXT,
                replaced_by    TEXT,
                PRIMARY KEY (policy_id, version)
            );
            CREATE INDEX IF NOT EXISTS idx_policy_versions_created
                ON policy_versions(created_at DESC);
            CREATE INDEX IF NOT EXISTS idx_policy_versions_status
                ON policy_versions(status);

            CREATE TABLE IF NOT EXISTS optimization_runs (
                run_id                TEXT PRIMARY KEY,
                run_type              TEXT NOT NULL,
                target_scope          TEXT NOT NULL,
                input_window          TEXT NOT NULL,
                baseline_policy_ref   TEXT,
                candidate_policy_ref  TEXT,
                decision              TEXT NOT NULL,
                summary               TEXT NOT NULL,
                created_at            TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_optimization_runs_created
                ON optimization_runs(created_at DESC);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn ensure_baseline_policy(
        &self,
        policy_id: &str,
        policy_type: PolicyType,
        content_json: &str,
    ) -> Result<PolicyVersion> {
        if let Some(existing) = self.latest_policy(policy_id)? {
            return Ok(existing);
        }

        self.insert_policy_version(
            policy_id,
            policy_type,
            1,
            content_json,
            PolicyStatus::Active,
            Some(Utc::now().to_rfc3339()),
            None,
        )
    }

    pub fn insert_policy_version(
        &self,
        policy_id: &str,
        policy_type: PolicyType,
        version: i64,
        content_json: &str,
        status: PolicyStatus,
        activated_at: Option<String>,
        replaced_by: Option<String>,
    ) -> Result<PolicyVersion> {
        let created_at = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO policy_versions
             (policy_id, policy_type, version, content_json, status, created_at, activated_at, replaced_by)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                policy_id,
                policy_type.to_string(),
                version,
                content_json,
                status.to_string(),
                created_at,
                activated_at,
                replaced_by,
            ],
        )?;

        Ok(PolicyVersion {
            policy_ref: format!("{}@v{}", policy_id, version),
            policy_id: policy_id.to_string(),
            policy_type,
            version,
            content_json: content_json.to_string(),
            status,
            created_at,
            activated_at,
            replaced_by,
        })
    }

    pub fn latest_policy(&self, policy_id: &str) -> Result<Option<PolicyVersion>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT policy_id, policy_type, version, content_json, status, created_at, activated_at, replaced_by
             FROM policy_versions
             WHERE policy_id = ?1
             ORDER BY version DESC
             LIMIT 1",
        )?;

        let row = stmt.query_row(params![policy_id], |row| {
            let policy_id: String = row.get(0)?;
            let policy_type: String = row.get(1)?;
            let version: i64 = row.get(2)?;
            let content_json: String = row.get(3)?;
            let status: String = row.get(4)?;
            let created_at: String = row.get(5)?;
            let activated_at: Option<String> = row.get(6)?;
            let replaced_by: Option<String> = row.get(7)?;
            Ok((
                policy_id,
                policy_type,
                version,
                content_json,
                status,
                created_at,
                activated_at,
                replaced_by,
            ))
        });

        match row {
            Ok((
                policy_id,
                policy_type,
                version,
                content_json,
                status,
                created_at,
                activated_at,
                replaced_by,
            )) => Ok(Some(PolicyVersion {
                policy_ref: format!("{}@v{}", policy_id, version),
                policy_id,
                policy_type: PolicyType::from_str(&policy_type)?,
                version,
                content_json,
                status: PolicyStatus::from_str(&status)?,
                created_at,
                activated_at,
                replaced_by,
            })),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn list_policies(&self, limit: usize) -> Result<Vec<PolicyVersion>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT policy_id, policy_type, version, content_json, status, created_at, activated_at, replaced_by
             FROM policy_versions
             ORDER BY created_at DESC, version DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;

        let mut policies = Vec::new();
        for row in rows {
            let (policy_id, policy_type, version, content_json, status, created_at, activated_at, replaced_by) = row?;
            policies.push(PolicyVersion {
                policy_ref: format!("{}@v{}", policy_id, version),
                policy_id,
                policy_type: PolicyType::from_str(&policy_type)?,
                version,
                content_json,
                status: PolicyStatus::from_str(&status)?,
                created_at,
                activated_at,
                replaced_by,
            });
        }
        Ok(policies)
    }

    pub fn record_optimization_run(
        &self,
        run_type: &str,
        target_scope: &str,
        input_window: &str,
        baseline_policy_ref: Option<&str>,
        candidate_policy_ref: Option<&str>,
        decision: &str,
        summary: &str,
    ) -> Result<OptimizationRun> {
        let run_id = Uuid::new_v4().to_string();
        let created_at = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO optimization_runs
             (run_id, run_type, target_scope, input_window, baseline_policy_ref,
              candidate_policy_ref, decision, summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                run_id,
                run_type,
                target_scope,
                input_window,
                baseline_policy_ref,
                candidate_policy_ref,
                decision,
                summary,
                created_at,
            ],
        )?;

        Ok(OptimizationRun {
            run_id,
            run_type: run_type.to_string(),
            target_scope: target_scope.to_string(),
            input_window: input_window.to_string(),
            baseline_policy_ref: baseline_policy_ref.map(|s| s.to_string()),
            candidate_policy_ref: candidate_policy_ref.map(|s| s.to_string()),
            decision: decision.to_string(),
            summary: summary.to_string(),
            created_at,
        })
    }

    /// List policies filtered by status.
    pub fn list_policies_by_status(&self, status: PolicyStatus) -> Result<Vec<PolicyVersion>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT policy_id, policy_type, version, content_json, status, created_at, activated_at, replaced_by
             FROM policy_versions
             WHERE status = ?1
             ORDER BY created_at DESC",
        )?;

        let rows = stmt.query_map(params![status.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
            ))
        })?;

        let mut policies = Vec::new();
        for row in rows {
            let (policy_id, policy_type, version, content_json, status_str, created_at, activated_at, replaced_by) = row?;
            policies.push(PolicyVersion {
                policy_ref: format!("{}@v{}", policy_id, version),
                policy_id,
                policy_type: PolicyType::from_str(&policy_type)?,
                version,
                content_json,
                status: PolicyStatus::from_str(&status_str)?,
                created_at,
                activated_at,
                replaced_by,
            });
        }
        Ok(policies)
    }

    /// Update a policy's status in-place (for Governor promotions/rollbacks).
    /// Creates a new version with the new status, preserving the append-only pattern.
    pub fn promote_policy(&self, policy_id: &str, current_version: i64, new_status: PolicyStatus) -> Result<PolicyVersion> {
        // Read the current version's content
        let conn = self.conn.lock().unwrap();
        let content_json: String = conn.query_row(
            "SELECT content_json FROM policy_versions WHERE policy_id = ?1 AND version = ?2",
            params![policy_id, current_version],
            |row| row.get(0),
        )?;
        let policy_type_str: String = conn.query_row(
            "SELECT policy_type FROM policy_versions WHERE policy_id = ?1 AND version = ?2",
            params![policy_id, current_version],
            |row| row.get(0),
        )?;
        drop(conn);

        let policy_type = PolicyType::from_str(&policy_type_str)?;
        let new_version = current_version + 1;
        let activated_at = if new_status == PolicyStatus::Active {
            Some(Utc::now().to_rfc3339())
        } else {
            None
        };

        self.insert_policy_version(
            policy_id,
            policy_type,
            new_version,
            &content_json,
            new_status,
            activated_at,
            None,
        )
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<OptimizationRun>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT run_id, run_type, target_scope, input_window, baseline_policy_ref,
                    candidate_policy_ref, decision, summary, created_at
             FROM optimization_runs
             ORDER BY created_at DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(OptimizationRun {
                run_id: row.get(0)?,
                run_type: row.get(1)?,
                target_scope: row.get(2)?,
                input_window: row.get(3)?,
                baseline_policy_ref: row.get(4)?,
                candidate_policy_ref: row.get(5)?,
                decision: row.get(6)?,
                summary: row.get(7)?,
                created_at: row.get(8)?,
            })
        })?;

        let mut runs = Vec::new();
        for row in rows {
            runs.push(row?);
        }
        Ok(runs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> String {
        let unique = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("phantom_mesh_optimizer_store_{}_{}", name, unique));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("optimizer.db").to_string_lossy().to_string()
    }

    #[test]
    fn test_ensure_baseline_policy_is_idempotent() {
        let db = temp_db("baseline");
        let store = OptimizerStore::new(&db).unwrap();

        let first = store
            .ensure_baseline_policy("prompt.default", PolicyType::Prompt, r#"{"hands":{}}"#)
            .unwrap();
        let second = store
            .ensure_baseline_policy("prompt.default", PolicyType::Prompt, r#"{"hands":{}}"#)
            .unwrap();

        assert_eq!(first.policy_ref, second.policy_ref);
        assert_eq!(first.status, PolicyStatus::Active);
    }

    #[test]
    fn test_insert_and_latest_policy() {
        let db = temp_db("latest");
        let store = OptimizerStore::new(&db).unwrap();

        store
            .insert_policy_version(
                "routing.default",
                PolicyType::Routing,
                1,
                r#"{"preferred_nodes":{}}"#,
                PolicyStatus::Active,
                Some(Utc::now().to_rfc3339()),
                None,
            )
            .unwrap();
        store
            .insert_policy_version(
                "routing.default",
                PolicyType::Routing,
                2,
                r#"{"preferred_nodes":{"code":["Z13"]}}"#,
                PolicyStatus::Canary,
                None,
                None,
            )
            .unwrap();

        let latest = store.latest_policy("routing.default").unwrap().unwrap();
        assert_eq!(latest.version, 2);
        assert_eq!(latest.status, PolicyStatus::Canary);
    }

    #[test]
    fn test_list_policies() {
        let db = temp_db("list_policies");
        let store = OptimizerStore::new(&db).unwrap();

        store
            .ensure_baseline_policy("prompt.default", PolicyType::Prompt, r#"{}"#)
            .unwrap();
        store
            .ensure_baseline_policy("workflow.default", PolicyType::Workflow, r#"{}"#)
            .unwrap();

        let policies = store.list_policies(10).unwrap();
        assert!(policies.len() >= 2);
    }

    #[test]
    fn test_record_and_list_runs() {
        let db = temp_db("runs");
        let store = OptimizerStore::new(&db).unwrap();

        let run = store
            .record_optimization_run(
                "daily_prompt_opt",
                "seo_content.phase_2",
                "7d",
                Some("prompt.default@v1"),
                Some("prompt.default@v2"),
                "canary",
                "Promoted to 10% canary after offline replay.",
            )
            .unwrap();

        let runs = store.list_runs(10).unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, run.run_id);
        assert_eq!(runs[0].decision, "canary");
    }
}
