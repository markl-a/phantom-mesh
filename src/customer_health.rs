//! Customer Health Score & Churn Risk Detection
//!
//! Two features in one module:
//! 1. **CustomerHealthManager** — weighted health scoring per customer (efficiency, quality, speed, satisfaction)
//! 2. **ChurnDetector** — signal-based churn risk detection with alert lifecycle management
//!
//! Both backed by SQLite (~/.clawtex/customer_health.db).

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

// ── Health Grade ────────────────────────────────────────────────────────────────

/// Customer health classification based on overall score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthGrade {
    /// Score 80+
    Healthy,
    /// Score 60-79
    Watch,
    /// Score 40-59
    Risk,
    /// Score < 40
    Danger,
}

impl HealthGrade {
    pub fn from_score(score: f64) -> Self {
        if score >= 80.0 {
            HealthGrade::Healthy
        } else if score >= 60.0 {
            HealthGrade::Watch
        } else if score >= 40.0 {
            HealthGrade::Risk
        } else {
            HealthGrade::Danger
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            HealthGrade::Healthy => "Healthy",
            HealthGrade::Watch => "Watch",
            HealthGrade::Risk => "Risk",
            HealthGrade::Danger => "Danger",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "healthy" => Some(HealthGrade::Healthy),
            "watch" => Some(HealthGrade::Watch),
            "risk" => Some(HealthGrade::Risk),
            "danger" => Some(HealthGrade::Danger),
            _ => None,
        }
    }
}

impl std::fmt::Display for HealthGrade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ── Customer Health ─────────────────────────────────────────────────────────────

/// A customer's health snapshot with individual dimension scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomerHealth {
    pub customer_id: String,
    pub name: String,
    pub efficiency_score: f64,
    pub quality_score: f64,
    pub speed_score: f64,
    pub satisfaction_score: f64,
    pub overall_score: f64,
    pub grade: HealthGrade,
    pub updated_at: DateTime<Utc>,
}

impl CustomerHealth {
    /// Compute overall score: 30% efficiency + 25% quality + 25% speed + 20% satisfaction
    pub fn compute_overall(efficiency: f64, quality: f64, speed: f64, satisfaction: f64) -> f64 {
        let raw = 0.30 * efficiency + 0.25 * quality + 0.25 * speed + 0.20 * satisfaction;
        // Clamp to 0..100
        raw.max(0.0).min(100.0)
    }
}

// ── Churn Risk Level ────────────────────────────────────────────────────────────

/// Severity of churn risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChurnRiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl ChurnRiskLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            ChurnRiskLevel::Low => "Low",
            ChurnRiskLevel::Medium => "Medium",
            ChurnRiskLevel::High => "High",
            ChurnRiskLevel::Critical => "Critical",
        }
    }

    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" => Some(ChurnRiskLevel::Low),
            "medium" => Some(ChurnRiskLevel::Medium),
            "high" => Some(ChurnRiskLevel::High),
            "critical" => Some(ChurnRiskLevel::Critical),
            _ => None,
        }
    }
}

impl std::fmt::Display for ChurnRiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ── Churn Alert ─────────────────────────────────────────────────────────────────

/// A single churn risk alert for a customer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChurnAlert {
    pub id: String,
    pub customer_id: String,
    pub risk_level: ChurnRiskLevel,
    pub signals: Vec<String>,
    pub recommended_action: String,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
}

// ── Churn Summary ───────────────────────────────────────────────────────────────

/// Aggregate counts of active churn alerts by risk level.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChurnSummary {
    pub total_active: u32,
    pub low: u32,
    pub medium: u32,
    pub high: u32,
    pub critical: u32,
}

// ── Customer Health Manager ─────────────────────────────────────────────────────

/// Manages customer health scores in SQLite.
pub struct CustomerHealthManager {
    db_path: String,
}

impl CustomerHealthManager {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS customer_health (
                customer_id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                efficiency REAL NOT NULL DEFAULT 0.0,
                quality REAL NOT NULL DEFAULT 0.0,
                speed REAL NOT NULL DEFAULT 0.0,
                satisfaction REAL NOT NULL DEFAULT 0.0,
                overall REAL NOT NULL DEFAULT 0.0,
                grade TEXT NOT NULL DEFAULT 'Danger',
                updated_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS customer_activity (
                customer_id TEXT PRIMARY KEY,
                last_activity TEXT NOT NULL,
                complaint_count INTEGER NOT NULL DEFAULT 0,
                renewal_due_date TEXT
            );
            CREATE TABLE IF NOT EXISTS churn_alerts (
                id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                signals_json TEXT NOT NULL,
                recommended_action TEXT NOT NULL,
                created_at TEXT NOT NULL,
                resolved INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_churn_customer ON churn_alerts(customer_id);
            CREATE INDEX IF NOT EXISTS idx_churn_resolved ON churn_alerts(resolved);
            CREATE INDEX IF NOT EXISTS idx_health_grade ON customer_health(grade);"
        )?;
        debug!("CustomerHealthManager initialized (db: {})", db_path);
        Ok(Self { db_path: db_path.to_string() })
    }

    /// Insert or update a customer's health scores. Returns the computed health.
    pub fn update_scores(
        &self,
        customer_id: &str,
        name: &str,
        efficiency: f64,
        quality: f64,
        speed: f64,
        satisfaction: f64,
    ) -> Result<CustomerHealth> {
        let overall = CustomerHealth::compute_overall(efficiency, quality, speed, satisfaction);
        let grade = HealthGrade::from_score(overall);
        let now = Utc::now();

        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO customer_health (customer_id, name, efficiency, quality, speed, satisfaction, overall, grade, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(customer_id) DO UPDATE SET
                name = excluded.name,
                efficiency = excluded.efficiency,
                quality = excluded.quality,
                speed = excluded.speed,
                satisfaction = excluded.satisfaction,
                overall = excluded.overall,
                grade = excluded.grade,
                updated_at = excluded.updated_at",
            rusqlite::params![
                customer_id,
                name,
                efficiency,
                quality,
                speed,
                satisfaction,
                overall,
                grade.as_str(),
                now.to_rfc3339(),
            ],
        )?;

        info!(
            "Customer health updated: {} ({}) — overall={:.1}, grade={}",
            name, customer_id, overall, grade
        );

        Ok(CustomerHealth {
            customer_id: customer_id.to_string(),
            name: name.to_string(),
            efficiency_score: efficiency,
            quality_score: quality,
            speed_score: speed,
            satisfaction_score: satisfaction,
            overall_score: overall,
            grade,
            updated_at: now,
        })
    }

    /// Get a single customer's health.
    pub fn get_health(&self, customer_id: &str) -> Result<Option<CustomerHealth>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT customer_id, name, efficiency, quality, speed, satisfaction, overall, grade, updated_at
             FROM customer_health WHERE customer_id = ?1"
        )?;
        let mut rows = stmt.query_map(rusqlite::params![customer_id], |row| {
            Self::row_to_health(row)
        })?;
        match rows.next() {
            Some(Ok(h)) => Ok(Some(h)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// List all customers' health records.
    pub fn list_all(&self) -> Result<Vec<CustomerHealth>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT customer_id, name, efficiency, quality, speed, satisfaction, overall, grade, updated_at
             FROM customer_health ORDER BY overall DESC"
        )?;
        let results = stmt
            .query_map([], |row| Self::row_to_health(row))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// List customers filtered by a specific grade.
    pub fn list_by_grade(&self, grade: HealthGrade) -> Result<Vec<CustomerHealth>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT customer_id, name, efficiency, quality, speed, satisfaction, overall, grade, updated_at
             FROM customer_health WHERE grade = ?1 ORDER BY overall DESC"
        )?;
        let results = stmt
            .query_map(rusqlite::params![grade.as_str()], |row| Self::row_to_health(row))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Get customers at risk (Risk + Danger grades).
    pub fn get_at_risk(&self) -> Result<Vec<CustomerHealth>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT customer_id, name, efficiency, quality, speed, satisfaction, overall, grade, updated_at
             FROM customer_health WHERE grade IN ('Risk', 'Danger') ORDER BY overall ASC"
        )?;
        let results = stmt
            .query_map([], |row| Self::row_to_health(row))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Average overall health score across all customers.
    pub fn average_health(&self) -> Result<f64> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT COALESCE(AVG(overall), 0.0) FROM customer_health"
        )?;
        let avg: f64 = stmt.query_row([], |row| row.get(0))?;
        Ok(avg)
    }

    fn row_to_health(row: &rusqlite::Row<'_>) -> rusqlite::Result<CustomerHealth> {
        let grade_str: String = row.get(7)?;
        let ts_str: String = row.get(8)?;
        Ok(CustomerHealth {
            customer_id: row.get(0)?,
            name: row.get(1)?,
            efficiency_score: row.get(2)?,
            quality_score: row.get(3)?,
            speed_score: row.get(4)?,
            satisfaction_score: row.get(5)?,
            overall_score: row.get(6)?,
            grade: HealthGrade::from_str_loose(&grade_str).unwrap_or(HealthGrade::Danger),
            updated_at: DateTime::parse_from_rfc3339(&ts_str)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }
}

// ── Churn Detector ──────────────────────────────────────────────────────────────

/// Detects churn risk signals and manages alert lifecycle.
pub struct ChurnDetector {
    db_path: String,
}

impl ChurnDetector {
    /// Create a new ChurnDetector sharing the same DB as CustomerHealthManager.
    pub fn new(db_path: &str) -> Result<Self> {
        // Tables are created by CustomerHealthManager::new, but ensure they exist
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS customer_activity (
                customer_id TEXT PRIMARY KEY,
                last_activity TEXT NOT NULL,
                complaint_count INTEGER NOT NULL DEFAULT 0,
                renewal_due_date TEXT
            );
            CREATE TABLE IF NOT EXISTS churn_alerts (
                id TEXT PRIMARY KEY,
                customer_id TEXT NOT NULL,
                risk_level TEXT NOT NULL,
                signals_json TEXT NOT NULL,
                recommended_action TEXT NOT NULL,
                created_at TEXT NOT NULL,
                resolved INTEGER NOT NULL DEFAULT 0
            );"
        )?;
        debug!("ChurnDetector initialized (db: {})", db_path);
        Ok(Self { db_path: db_path.to_string() })
    }

    /// Analyze a set of customers and generate churn risk alerts.
    /// Checks multiple signals and produces alerts for at-risk customers.
    pub fn detect_churn_risks(&self, customers: &[CustomerHealth]) -> Result<Vec<ChurnAlert>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let now = Utc::now();
        let mut alerts = Vec::new();

        for customer in customers {
            let mut signals: Vec<String> = Vec::new();
            let mut max_level = ChurnRiskLevel::Low;

            // Signal 1: health_grade == Danger -> Critical
            if customer.grade == HealthGrade::Danger {
                signals.push(format!("Health grade is Danger (overall score: {:.1})", customer.overall_score));
                max_level = ChurnRiskLevel::Critical;
            }

            // Signal 2: satisfaction_score < 30 -> High
            if customer.satisfaction_score < 30.0 {
                signals.push(format!("Satisfaction score critically low: {:.1}", customer.satisfaction_score));
                if matches!(max_level, ChurnRiskLevel::Low | ChurnRiskLevel::Medium) {
                    max_level = ChurnRiskLevel::High;
                }
            }

            // Signal 3: days_since_last_activity > 14 -> Medium
            let days_inactive = self.days_since_last_activity(&conn, &customer.customer_id);
            if days_inactive > 14 {
                signals.push(format!("Inactive for {} days (threshold: 14)", days_inactive));
                if matches!(max_level, ChurnRiskLevel::Low) {
                    max_level = ChurnRiskLevel::Medium;
                }
            }

            // Signal 4: complaint_count_30d >= 3 -> High
            let complaints = self.get_complaint_count(&conn, &customer.customer_id);
            if complaints >= 3 {
                signals.push(format!("{} complaints in last 30 days (threshold: 3)", complaints));
                if matches!(max_level, ChurnRiskLevel::Low | ChurnRiskLevel::Medium) {
                    max_level = ChurnRiskLevel::High;
                }
            }

            // Signal 5: renewal_pending > 14 days -> Medium
            let renewal_days = self.days_until_renewal(&conn, &customer.customer_id);
            if let Some(days) = renewal_days {
                if days <= 14 && days > 0 {
                    signals.push(format!("Renewal pending in {} days", days));
                    if matches!(max_level, ChurnRiskLevel::Low) {
                        max_level = ChurnRiskLevel::Medium;
                    }
                }
            }

            // Only create alert if there are signals
            if !signals.is_empty() {
                let recommended_action = Self::recommend_action(&max_level, &signals);
                let alert = ChurnAlert {
                    id: uuid::Uuid::new_v4().to_string(),
                    customer_id: customer.customer_id.clone(),
                    risk_level: max_level,
                    signals: signals.clone(),
                    recommended_action,
                    created_at: now,
                    resolved: false,
                };

                // Persist the alert
                let signals_json = serde_json::to_string(&alert.signals).unwrap_or_default();
                conn.execute(
                    "INSERT INTO churn_alerts (id, customer_id, risk_level, signals_json, recommended_action, created_at, resolved)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    rusqlite::params![
                        alert.id,
                        alert.customer_id,
                        alert.risk_level.as_str(),
                        signals_json,
                        alert.recommended_action,
                        alert.created_at.to_rfc3339(),
                        0,
                    ],
                )?;

                info!(
                    "Churn alert created: customer={}, risk={}, signals={}",
                    alert.customer_id,
                    alert.risk_level,
                    signals.len()
                );
                alerts.push(alert);
            }
        }

        Ok(alerts)
    }

    /// Record customer activity (resets inactivity timer).
    pub fn record_activity(&self, customer_id: &str) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO customer_activity (customer_id, last_activity, complaint_count)
             VALUES (?1, ?2, 0)
             ON CONFLICT(customer_id) DO UPDATE SET last_activity = excluded.last_activity",
            rusqlite::params![customer_id, now],
        )?;
        debug!("Activity recorded for customer '{}'", customer_id);
        Ok(())
    }

    /// Record a complaint for a customer (increments complaint_count).
    pub fn record_complaint(&self, customer_id: &str, description: &str) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let now = Utc::now().to_rfc3339();
        // Ensure activity row exists first
        conn.execute(
            "INSERT INTO customer_activity (customer_id, last_activity, complaint_count)
             VALUES (?1, ?2, 1)
             ON CONFLICT(customer_id) DO UPDATE SET complaint_count = complaint_count + 1",
            rusqlite::params![customer_id, now],
        )?;
        info!("Complaint recorded for customer '{}': {}", customer_id, description);
        Ok(())
    }

    /// Set renewal due date for a customer.
    pub fn set_renewal_date(&self, customer_id: &str, date: &str) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO customer_activity (customer_id, last_activity, complaint_count, renewal_due_date)
             VALUES (?1, ?2, 0, ?3)
             ON CONFLICT(customer_id) DO UPDATE SET renewal_due_date = excluded.renewal_due_date",
            rusqlite::params![customer_id, now, date],
        )?;
        debug!("Renewal date set for customer '{}': {}", customer_id, date);
        Ok(())
    }

    /// Get all alerts for a specific customer.
    pub fn get_alerts(&self, customer_id: &str) -> Result<Vec<ChurnAlert>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, customer_id, risk_level, signals_json, recommended_action, created_at, resolved
             FROM churn_alerts WHERE customer_id = ?1 ORDER BY created_at DESC"
        )?;
        let results = stmt
            .query_map(rusqlite::params![customer_id], |row| Self::row_to_alert(row))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Get all active (unresolved) alerts across all customers.
    pub fn get_all_active_alerts(&self) -> Result<Vec<ChurnAlert>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, customer_id, risk_level, signals_json, recommended_action, created_at, resolved
             FROM churn_alerts WHERE resolved = 0 ORDER BY created_at DESC"
        )?;
        let results = stmt
            .query_map([], |row| Self::row_to_alert(row))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Resolve (close) an alert by ID.
    pub fn resolve_alert(&self, alert_id: &str) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let updated = conn.execute(
            "UPDATE churn_alerts SET resolved = 1 WHERE id = ?1",
            rusqlite::params![alert_id],
        )?;
        if updated == 0 {
            anyhow::bail!("Alert '{}' not found", alert_id);
        }
        info!("Churn alert resolved: {}", alert_id);
        Ok(())
    }

    /// Get a summary of active alerts grouped by risk level.
    pub fn churn_summary(&self) -> Result<ChurnSummary> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT risk_level, COUNT(*) FROM churn_alerts WHERE resolved = 0 GROUP BY risk_level"
        )?;
        let mut summary = ChurnSummary {
            total_active: 0,
            low: 0,
            medium: 0,
            high: 0,
            critical: 0,
        };
        let rows = stmt.query_map([], |row| {
            let level: String = row.get(0)?;
            let count: u32 = row.get(1)?;
            Ok((level, count))
        })?;
        for row in rows.flatten() {
            let (level, count) = row;
            match level.to_lowercase().as_str() {
                "low" => summary.low = count,
                "medium" => summary.medium = count,
                "high" => summary.high = count,
                "critical" => summary.critical = count,
                _ => {}
            }
            summary.total_active += count;
        }
        Ok(summary)
    }

    // ── Private helpers ─────────────────────────────────────────────────────────

    fn days_since_last_activity(&self, conn: &rusqlite::Connection, customer_id: &str) -> i64 {
        let result: rusqlite::Result<String> = conn.query_row(
            "SELECT last_activity FROM customer_activity WHERE customer_id = ?1",
            rusqlite::params![customer_id],
            |row| row.get(0),
        );
        match result {
            Ok(ts_str) => {
                if let Ok(ts) = DateTime::parse_from_rfc3339(&ts_str) {
                    let diff = Utc::now().signed_duration_since(ts.with_timezone(&Utc));
                    diff.num_days()
                } else {
                    999 // Unparseable = treat as very inactive
                }
            }
            Err(_) => 999, // No record = never active
        }
    }

    fn get_complaint_count(&self, conn: &rusqlite::Connection, customer_id: &str) -> i32 {
        conn.query_row(
            "SELECT complaint_count FROM customer_activity WHERE customer_id = ?1",
            rusqlite::params![customer_id],
            |row| row.get(0),
        )
        .unwrap_or(0)
    }

    fn days_until_renewal(&self, conn: &rusqlite::Connection, customer_id: &str) -> Option<i64> {
        let result: rusqlite::Result<Option<String>> = conn.query_row(
            "SELECT renewal_due_date FROM customer_activity WHERE customer_id = ?1",
            rusqlite::params![customer_id],
            |row| row.get(0),
        );
        match result {
            Ok(Some(date_str)) => {
                // Parse as date (YYYY-MM-DD) or full datetime
                if let Ok(dt) = DateTime::parse_from_rfc3339(&date_str) {
                    Some(dt.signed_duration_since(Utc::now()).num_days())
                } else if let Ok(nd) = chrono::NaiveDate::parse_from_str(&date_str, "%Y-%m-%d") {
                    let today = Utc::now().date_naive();
                    Some((nd - today).num_days())
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn recommend_action(level: &ChurnRiskLevel, signals: &[String]) -> String {
        match level {
            ChurnRiskLevel::Critical => {
                "URGENT: Schedule executive outreach call immediately. Prepare retention offer with significant discount or service upgrade.".to_string()
            }
            ChurnRiskLevel::High => {
                if signals.iter().any(|s| s.contains("complaint")) {
                    "Assign dedicated support rep. Review and resolve all open complaints. Schedule check-in call within 48h.".to_string()
                } else {
                    "Initiate proactive outreach. Offer personalized demo of new features. Consider loyalty incentive.".to_string()
                }
            }
            ChurnRiskLevel::Medium => {
                if signals.iter().any(|s| s.contains("Inactive")) {
                    "Send re-engagement campaign. Share relevant case studies or feature updates. Monitor for 7 more days.".to_string()
                } else if signals.iter().any(|s| s.contains("Renewal")) {
                    "Begin renewal conversation early. Prepare ROI summary and success metrics for customer review.".to_string()
                } else {
                    "Monitor closely. Add to weekly review list. Prepare engagement plan.".to_string()
                }
            }
            ChurnRiskLevel::Low => {
                "Continue standard monitoring. No immediate action required.".to_string()
            }
        }
    }

    fn row_to_alert(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChurnAlert> {
        let risk_str: String = row.get(2)?;
        let signals_json: String = row.get(3)?;
        let ts_str: String = row.get(5)?;
        let resolved_int: i32 = row.get(6)?;
        Ok(ChurnAlert {
            id: row.get(0)?,
            customer_id: row.get(1)?,
            risk_level: ChurnRiskLevel::from_str_loose(&risk_str).unwrap_or(ChurnRiskLevel::Low),
            signals: serde_json::from_str(&signals_json).unwrap_or_default(),
            recommended_action: row.get(4)?,
            created_at: DateTime::parse_from_rfc3339(&ts_str)
                .map(|d| d.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            resolved: resolved_int != 0,
        })
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("clawtex_test_customer_health");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    // ── HealthGrade Tests ───────────────────────────────────────────────────

    #[test]
    fn test_health_grade_from_score_healthy() {
        assert_eq!(HealthGrade::from_score(100.0), HealthGrade::Healthy);
        assert_eq!(HealthGrade::from_score(80.0), HealthGrade::Healthy);
        assert_eq!(HealthGrade::from_score(95.5), HealthGrade::Healthy);
    }

    #[test]
    fn test_health_grade_from_score_watch() {
        assert_eq!(HealthGrade::from_score(79.9), HealthGrade::Watch);
        assert_eq!(HealthGrade::from_score(60.0), HealthGrade::Watch);
        assert_eq!(HealthGrade::from_score(70.0), HealthGrade::Watch);
    }

    #[test]
    fn test_health_grade_from_score_risk() {
        assert_eq!(HealthGrade::from_score(59.9), HealthGrade::Risk);
        assert_eq!(HealthGrade::from_score(40.0), HealthGrade::Risk);
        assert_eq!(HealthGrade::from_score(50.0), HealthGrade::Risk);
    }

    #[test]
    fn test_health_grade_from_score_danger() {
        assert_eq!(HealthGrade::from_score(39.9), HealthGrade::Danger);
        assert_eq!(HealthGrade::from_score(0.0), HealthGrade::Danger);
        assert_eq!(HealthGrade::from_score(10.0), HealthGrade::Danger);
    }

    #[test]
    fn test_health_grade_string_roundtrip() {
        for grade in &[HealthGrade::Healthy, HealthGrade::Watch, HealthGrade::Risk, HealthGrade::Danger] {
            let s = grade.as_str();
            let back = HealthGrade::from_str_loose(s).unwrap();
            assert_eq!(*grade, back);
        }
    }

    // ── CustomerHealth Computation Tests ────────────────────────────────────

    #[test]
    fn test_compute_overall_perfect() {
        let overall = CustomerHealth::compute_overall(100.0, 100.0, 100.0, 100.0);
        assert!((overall - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_overall_zero() {
        let overall = CustomerHealth::compute_overall(0.0, 0.0, 0.0, 0.0);
        assert!((overall - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_compute_overall_weighted() {
        // 0.30*80 + 0.25*60 + 0.25*70 + 0.20*90 = 24 + 15 + 17.5 + 18 = 74.5
        let overall = CustomerHealth::compute_overall(80.0, 60.0, 70.0, 90.0);
        assert!((overall - 74.5).abs() < 0.01);
    }

    #[test]
    fn test_compute_overall_clamped() {
        // Values above 100 or below 0 should be clamped in result
        let overall = CustomerHealth::compute_overall(120.0, 120.0, 120.0, 120.0);
        assert_eq!(overall, 100.0);
    }

    // ── CustomerHealthManager DB Tests ──────────────────────────────────────

    #[test]
    fn test_manager_update_and_get() {
        let (db_str, db_path) = temp_db("update_get");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();

        let health = mgr.update_scores("c1", "Acme Corp", 85.0, 90.0, 75.0, 80.0).unwrap();
        assert_eq!(health.customer_id, "c1");
        assert_eq!(health.name, "Acme Corp");
        assert_eq!(health.grade, HealthGrade::Healthy);
        // 0.30*85 + 0.25*90 + 0.25*75 + 0.20*80 = 25.5 + 22.5 + 18.75 + 16.0 = 82.75
        assert!((health.overall_score - 82.75).abs() < 0.01);

        let fetched = mgr.get_health("c1").unwrap().unwrap();
        assert_eq!(fetched.name, "Acme Corp");
        assert!((fetched.overall_score - 82.75).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_manager_get_nonexistent() {
        let (db_str, db_path) = temp_db("get_none");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();
        assert!(mgr.get_health("nonexistent").unwrap().is_none());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_manager_update_overwrites() {
        let (db_str, db_path) = temp_db("update_overwrite");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();

        mgr.update_scores("c1", "Acme", 90.0, 90.0, 90.0, 90.0).unwrap();
        let h1 = mgr.get_health("c1").unwrap().unwrap();
        assert_eq!(h1.grade, HealthGrade::Healthy);

        mgr.update_scores("c1", "Acme", 20.0, 20.0, 20.0, 20.0).unwrap();
        let h2 = mgr.get_health("c1").unwrap().unwrap();
        assert_eq!(h2.grade, HealthGrade::Danger);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_manager_list_all() {
        let (db_str, db_path) = temp_db("list_all");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();

        mgr.update_scores("c1", "Alpha", 90.0, 90.0, 90.0, 90.0).unwrap();
        mgr.update_scores("c2", "Beta", 50.0, 50.0, 50.0, 50.0).unwrap();
        mgr.update_scores("c3", "Gamma", 30.0, 30.0, 30.0, 30.0).unwrap();

        let all = mgr.list_all().unwrap();
        assert_eq!(all.len(), 3);
        // Ordered by overall DESC
        assert_eq!(all[0].customer_id, "c1");
        assert_eq!(all[2].customer_id, "c3");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_manager_list_by_grade() {
        let (db_str, db_path) = temp_db("list_grade");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();

        mgr.update_scores("c1", "A", 90.0, 90.0, 90.0, 90.0).unwrap();
        mgr.update_scores("c2", "B", 50.0, 50.0, 50.0, 50.0).unwrap();
        mgr.update_scores("c3", "C", 30.0, 30.0, 30.0, 30.0).unwrap();

        let healthy = mgr.list_by_grade(HealthGrade::Healthy).unwrap();
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].customer_id, "c1");

        let risk = mgr.list_by_grade(HealthGrade::Risk).unwrap();
        assert_eq!(risk.len(), 1);
        assert_eq!(risk[0].customer_id, "c2");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_manager_get_at_risk() {
        let (db_str, db_path) = temp_db("at_risk");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();

        mgr.update_scores("c1", "Healthy Co", 90.0, 90.0, 90.0, 90.0).unwrap();
        mgr.update_scores("c2", "Risky Co", 45.0, 45.0, 45.0, 45.0).unwrap();
        mgr.update_scores("c3", "Danger Co", 20.0, 20.0, 20.0, 20.0).unwrap();

        let at_risk = mgr.get_at_risk().unwrap();
        assert_eq!(at_risk.len(), 2);
        // Danger first (lower overall), then Risk
        assert_eq!(at_risk[0].customer_id, "c3");
        assert_eq!(at_risk[1].customer_id, "c2");

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_manager_average_health() {
        let (db_str, db_path) = temp_db("avg_health");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();

        mgr.update_scores("c1", "A", 80.0, 80.0, 80.0, 80.0).unwrap();
        mgr.update_scores("c2", "B", 60.0, 60.0, 60.0, 60.0).unwrap();

        let avg = mgr.average_health().unwrap();
        assert!((avg - 70.0).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_manager_average_health_empty() {
        let (db_str, db_path) = temp_db("avg_empty");
        let mgr = CustomerHealthManager::new(&db_str).unwrap();
        let avg = mgr.average_health().unwrap();
        assert!((avg - 0.0).abs() < 0.01);
        let _ = std::fs::remove_file(&db_path);
    }

    // ── ChurnDetector Tests ─────────────────────────────────────────────────

    #[test]
    fn test_churn_record_activity() {
        let (db_str, db_path) = temp_db("churn_activity");
        let detector = ChurnDetector::new(&db_str).unwrap();

        assert!(detector.record_activity("c1").is_ok());
        // Record again to update
        assert!(detector.record_activity("c1").is_ok());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_record_complaint() {
        let (db_str, db_path) = temp_db("churn_complaint");
        let detector = ChurnDetector::new(&db_str).unwrap();

        detector.record_complaint("c1", "Slow response").unwrap();
        detector.record_complaint("c1", "Wrong output").unwrap();
        detector.record_complaint("c1", "Crash on run").unwrap();

        let conn = rusqlite::Connection::open(&db_str).unwrap();
        let count: i32 = conn.query_row(
            "SELECT complaint_count FROM customer_activity WHERE customer_id = 'c1'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(count, 3);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_detect_danger_grade() {
        let (db_str, db_path) = temp_db("churn_danger");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();

        let customer = CustomerHealth {
            customer_id: "c1".into(),
            name: "Danger Corp".into(),
            efficiency_score: 10.0,
            quality_score: 10.0,
            speed_score: 10.0,
            satisfaction_score: 10.0,
            overall_score: 10.0,
            grade: HealthGrade::Danger,
            updated_at: Utc::now(),
        };

        let alerts = detector.detect_churn_risks(&[customer]).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].risk_level, ChurnRiskLevel::Critical);
        assert!(alerts[0].signals.iter().any(|s| s.contains("Danger")));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_detect_low_satisfaction() {
        let (db_str, db_path) = temp_db("churn_low_sat");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();
        // Record activity so inactivity signal doesn't fire
        detector.record_activity("c1").unwrap();

        let customer = CustomerHealth {
            customer_id: "c1".into(),
            name: "Unhappy Corp".into(),
            efficiency_score: 70.0,
            quality_score: 70.0,
            speed_score: 70.0,
            satisfaction_score: 25.0,
            overall_score: 60.0,
            grade: HealthGrade::Watch,
            updated_at: Utc::now(),
        };

        let alerts = detector.detect_churn_risks(&[customer]).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].risk_level, ChurnRiskLevel::High);
        assert!(alerts[0].signals.iter().any(|s| s.contains("Satisfaction")));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_detect_complaints() {
        let (db_str, db_path) = temp_db("churn_complaints_detect");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();

        // Record activity and 3 complaints
        detector.record_activity("c1").unwrap();
        detector.record_complaint("c1", "issue 1").unwrap();
        detector.record_complaint("c1", "issue 2").unwrap();
        detector.record_complaint("c1", "issue 3").unwrap();

        let customer = CustomerHealth {
            customer_id: "c1".into(),
            name: "Complainer Co".into(),
            efficiency_score: 70.0,
            quality_score: 70.0,
            speed_score: 70.0,
            satisfaction_score: 50.0,
            overall_score: 65.0,
            grade: HealthGrade::Watch,
            updated_at: Utc::now(),
        };

        let alerts = detector.detect_churn_risks(&[customer]).unwrap();
        assert!(!alerts.is_empty());
        assert!(alerts[0].signals.iter().any(|s| s.contains("complaint")));

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_no_signals_healthy() {
        let (db_str, db_path) = temp_db("churn_no_signals");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();
        // Record recent activity
        detector.record_activity("c1").unwrap();

        let customer = CustomerHealth {
            customer_id: "c1".into(),
            name: "Happy Corp".into(),
            efficiency_score: 90.0,
            quality_score: 90.0,
            speed_score: 90.0,
            satisfaction_score: 95.0,
            overall_score: 91.25,
            grade: HealthGrade::Healthy,
            updated_at: Utc::now(),
        };

        let alerts = detector.detect_churn_risks(&[customer]).unwrap();
        assert!(alerts.is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_resolve_alert() {
        let (db_str, db_path) = temp_db("churn_resolve");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();

        let customer = CustomerHealth {
            customer_id: "c1".into(),
            name: "Risk Co".into(),
            efficiency_score: 10.0,
            quality_score: 10.0,
            speed_score: 10.0,
            satisfaction_score: 10.0,
            overall_score: 10.0,
            grade: HealthGrade::Danger,
            updated_at: Utc::now(),
        };

        let alerts = detector.detect_churn_risks(&[customer]).unwrap();
        assert_eq!(alerts.len(), 1);
        let alert_id = alerts[0].id.clone();

        // Resolve it
        detector.resolve_alert(&alert_id).unwrap();

        let active = detector.get_all_active_alerts().unwrap();
        assert!(active.is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_resolve_nonexistent() {
        let (db_str, db_path) = temp_db("churn_resolve_none");
        let detector = ChurnDetector::new(&db_str).unwrap();
        assert!(detector.resolve_alert("nonexistent-id").is_err());
        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_summary() {
        let (db_str, db_path) = temp_db("churn_summary");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();

        // Create alerts of different levels
        let danger_customer = CustomerHealth {
            customer_id: "c1".into(), name: "Danger".into(),
            efficiency_score: 10.0, quality_score: 10.0, speed_score: 10.0, satisfaction_score: 10.0,
            overall_score: 10.0, grade: HealthGrade::Danger, updated_at: Utc::now(),
        };
        detector.record_activity("c2").unwrap();
        let low_sat_customer = CustomerHealth {
            customer_id: "c2".into(), name: "Low Sat".into(),
            efficiency_score: 70.0, quality_score: 70.0, speed_score: 70.0, satisfaction_score: 20.0,
            overall_score: 57.0, grade: HealthGrade::Risk, updated_at: Utc::now(),
        };

        detector.detect_churn_risks(&[danger_customer, low_sat_customer]).unwrap();

        let summary = detector.churn_summary().unwrap();
        assert!(summary.total_active >= 2);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_get_alerts_for_customer() {
        let (db_str, db_path) = temp_db("churn_get_alerts");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();

        let customer = CustomerHealth {
            customer_id: "c1".into(), name: "Test".into(),
            efficiency_score: 10.0, quality_score: 10.0, speed_score: 10.0, satisfaction_score: 10.0,
            overall_score: 10.0, grade: HealthGrade::Danger, updated_at: Utc::now(),
        };
        detector.detect_churn_risks(&[customer]).unwrap();

        let alerts = detector.get_alerts("c1").unwrap();
        assert!(!alerts.is_empty());
        assert_eq!(alerts[0].customer_id, "c1");

        // No alerts for c2
        let alerts_c2 = detector.get_alerts("c2").unwrap();
        assert!(alerts_c2.is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_churn_renewal_signal() {
        let (db_str, db_path) = temp_db("churn_renewal");
        let _mgr = CustomerHealthManager::new(&db_str).unwrap();
        let detector = ChurnDetector::new(&db_str).unwrap();

        // Set renewal in 10 days (< 14 threshold)
        let renewal_date = (Utc::now() + chrono::Duration::days(10)).format("%Y-%m-%d").to_string();
        detector.record_activity("c1").unwrap();
        detector.set_renewal_date("c1", &renewal_date).unwrap();

        let customer = CustomerHealth {
            customer_id: "c1".into(), name: "Renewing Co".into(),
            efficiency_score: 70.0, quality_score: 70.0, speed_score: 70.0, satisfaction_score: 70.0,
            overall_score: 70.0, grade: HealthGrade::Watch, updated_at: Utc::now(),
        };

        let alerts = detector.detect_churn_risks(&[customer]).unwrap();
        assert!(!alerts.is_empty());
        assert!(alerts[0].signals.iter().any(|s| s.contains("Renewal")));

        let _ = std::fs::remove_file(&db_path);
    }

    // ── Serialization Tests ─────────────────────────────────────────────────

    #[test]
    fn test_health_grade_serialize() {
        let grade = HealthGrade::Watch;
        let json = serde_json::to_string(&grade).unwrap();
        assert!(json.contains("Watch"));
        let back: HealthGrade = serde_json::from_str(&json).unwrap();
        assert_eq!(back, HealthGrade::Watch);
    }

    #[test]
    fn test_churn_risk_level_serialize() {
        let level = ChurnRiskLevel::Critical;
        let json = serde_json::to_string(&level).unwrap();
        assert!(json.contains("Critical"));
        let back: ChurnRiskLevel = serde_json::from_str(&json).unwrap();
        assert_eq!(back, ChurnRiskLevel::Critical);
    }

    #[test]
    fn test_customer_health_serialize() {
        let health = CustomerHealth {
            customer_id: "c1".into(),
            name: "Test".into(),
            efficiency_score: 80.0,
            quality_score: 70.0,
            speed_score: 60.0,
            satisfaction_score: 90.0,
            overall_score: 74.5,
            grade: HealthGrade::Watch,
            updated_at: Utc::now(),
        };
        let json = serde_json::to_string(&health).unwrap();
        assert!(json.contains("c1"));
        assert!(json.contains("Watch"));
    }

    #[test]
    fn test_churn_summary_serialize() {
        let summary = ChurnSummary {
            total_active: 5, low: 1, medium: 2, high: 1, critical: 1,
        };
        let json = serde_json::to_string(&summary).unwrap();
        let back: ChurnSummary = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total_active, 5);
        assert_eq!(back.critical, 1);
    }
}
