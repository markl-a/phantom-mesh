//! Revenue Tracker — records revenue from all income routes (A-J) in Clawtex.
//! Persisted to SQLite. Queryable by route, source, date, and client.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::debug;

// ---------------------------------------------------------------------------
// Route constants (A-J) — descriptive names for each income route
// ---------------------------------------------------------------------------

pub const ROUTE_A: &str = "A:freelance_dev";
pub const ROUTE_B: &str = "B:saas_products";
pub const ROUTE_C: &str = "C:content_monetization";
pub const ROUTE_D: &str = "D:consulting";
pub const ROUTE_E: &str = "E:api_services";
pub const ROUTE_F: &str = "F:affiliate_marketing";
pub const ROUTE_G: &str = "G:digital_products";
pub const ROUTE_H: &str = "H:automation_services";
pub const ROUTE_I: &str = "I:data_services";
pub const ROUTE_J: &str = "J:training_education";

/// All route constants for iteration / validation
pub const ALL_ROUTES: &[&str] = &[
    ROUTE_A, ROUTE_B, ROUTE_C, ROUTE_D, ROUTE_E,
    ROUTE_F, ROUTE_G, ROUTE_H, ROUTE_I, ROUTE_J,
];

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Status of a revenue record
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RevenueStatus {
    Pending,
    Confirmed,
    Paid,
}

impl RevenueStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RevenueStatus::Pending => "pending",
            RevenueStatus::Confirmed => "confirmed",
            RevenueStatus::Paid => "paid",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "confirmed" => RevenueStatus::Confirmed,
            "paid" => RevenueStatus::Paid,
            _ => RevenueStatus::Pending,
        }
    }
}

/// A single revenue record
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueRecord {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    /// Route identifier (A-J), use ROUTE_* constants
    pub route: String,
    /// Source of revenue, e.g. "upwork", "email_outreach", "blog_ads"
    pub source: String,
    /// Client or customer name
    pub client_name: String,
    /// Amount in USD (or converted to USD equivalent)
    pub amount_usd: f64,
    /// Original currency code (e.g. "USD", "TWD", "EUR")
    pub currency: String,
    /// Payment status
    pub status: RevenueStatus,
    /// Free-form notes
    pub notes: Option<String>,
    /// Optional invoice identifier
    pub invoice_id: Option<String>,
}

/// Summary of revenue grouped by a dimension
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueSummary {
    pub group: String,
    pub total_usd: f64,
    pub count: u32,
}

// ---------------------------------------------------------------------------
// RevenueTracker — SQLite-backed persistence
// ---------------------------------------------------------------------------

/// Revenue tracker with SQLite persistence
pub struct RevenueTracker {
    db_path: String,
}

impl RevenueTracker {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS revenue_records (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                route TEXT NOT NULL,
                source TEXT NOT NULL,
                client_name TEXT NOT NULL,
                amount_usd REAL NOT NULL DEFAULT 0.0,
                currency TEXT NOT NULL DEFAULT 'USD',
                status TEXT NOT NULL DEFAULT 'pending',
                notes TEXT,
                invoice_id TEXT,
                date_key TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_revenue_date ON revenue_records(date_key);
            CREATE INDEX IF NOT EXISTS idx_revenue_route ON revenue_records(route);
            CREATE INDEX IF NOT EXISTS idx_revenue_source ON revenue_records(source);
            CREATE INDEX IF NOT EXISTS idx_revenue_client ON revenue_records(client_name);
            CREATE INDEX IF NOT EXISTS idx_revenue_status ON revenue_records(status);"
        )?;
        Ok(Self { db_path: db_path.to_string() })
    }

    /// Record a new revenue entry
    pub fn record(&self, record: &RevenueRecord) -> Result<()> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let date_key = record.timestamp.format("%Y-%m-%d").to_string();
        conn.execute(
            "INSERT INTO revenue_records (id, timestamp, route, source, client_name, amount_usd, currency, status, notes, invoice_id, date_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                record.id,
                record.timestamp.to_rfc3339(),
                record.route,
                record.source,
                record.client_name,
                record.amount_usd,
                record.currency,
                record.status.as_str(),
                record.notes,
                record.invoice_id,
                date_key,
            ],
        )?;
        debug!(
            "Revenue recorded: ${:.2} from {} via {} [{}]",
            record.amount_usd, record.client_name, record.source, record.route
        );
        Ok(())
    }

    /// Get total revenue for today
    pub fn today_total(&self) -> Result<RevenueSummary> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        self.summary_for_date(&today)
    }

    /// Get total revenue for a specific date
    pub fn summary_for_date(&self, date: &str) -> Result<RevenueSummary> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT COALESCE(SUM(amount_usd), 0.0), COUNT(*)
             FROM revenue_records WHERE date_key = ?1"
        )?;
        let (total, count) = stmt.query_row([date], |row| {
            Ok((row.get::<_, f64>(0)?, row.get::<_, u32>(1)?))
        })?;
        Ok(RevenueSummary {
            group: date.to_string(),
            total_usd: total,
            count,
        })
    }

    /// Get revenue grouped by route (last N days)
    pub fn by_route(&self, days: u32) -> Result<Vec<RevenueSummary>> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT route, SUM(amount_usd), COUNT(*)
             FROM revenue_records WHERE date_key >= ?1
             GROUP BY route ORDER BY SUM(amount_usd) DESC"
        )?;
        let summaries = stmt
            .query_map([&cutoff], |row| {
                Ok(RevenueSummary {
                    group: row.get(0)?,
                    total_usd: row.get(1)?,
                    count: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(summaries)
    }

    /// Get revenue grouped by source (last N days)
    pub fn by_source(&self, days: u32) -> Result<Vec<RevenueSummary>> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT source, SUM(amount_usd), COUNT(*)
             FROM revenue_records WHERE date_key >= ?1
             GROUP BY source ORDER BY SUM(amount_usd) DESC"
        )?;
        let summaries = stmt
            .query_map([&cutoff], |row| {
                Ok(RevenueSummary {
                    group: row.get(0)?,
                    total_usd: row.get(1)?,
                    count: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(summaries)
    }

    /// Get revenue grouped by day (last N days)
    pub fn by_day(&self, days: u32) -> Result<Vec<RevenueSummary>> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64))
            .format("%Y-%m-%d")
            .to_string();
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT date_key, SUM(amount_usd), COUNT(*)
             FROM revenue_records WHERE date_key >= ?1
             GROUP BY date_key ORDER BY date_key DESC"
        )?;
        let summaries = stmt
            .query_map([&cutoff], |row| {
                Ok(RevenueSummary {
                    group: row.get(0)?,
                    total_usd: row.get(1)?,
                    count: row.get(2)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(summaries)
    }

    /// Get all records for a date range (for export / detailed view)
    pub fn records_between(&self, start: &str, end: &str) -> Result<Vec<RevenueRecord>> {
        let conn = rusqlite::Connection::open(&self.db_path)?;
        let mut stmt = conn.prepare(
            "SELECT id, timestamp, route, source, client_name, amount_usd, currency, status, notes, invoice_id
             FROM revenue_records WHERE date_key >= ?1 AND date_key <= ?2
             ORDER BY timestamp DESC"
        )?;
        let records = stmt
            .query_map(rusqlite::params![start, end], |row| {
                let ts_str: String = row.get(1)?;
                let status_str: String = row.get(7)?;
                Ok(RevenueRecord {
                    id: row.get(0)?,
                    timestamp: DateTime::parse_from_rfc3339(&ts_str)
                        .map(|d| d.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    route: row.get(2)?,
                    source: row.get(3)?,
                    client_name: row.get(4)?,
                    amount_usd: row.get(5)?,
                    currency: row.get(6)?,
                    status: RevenueStatus::from_str(&status_str),
                    notes: row.get(8)?,
                    invoice_id: row.get(9)?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();
        Ok(records)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db(name: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join("clawtex_test_revenue");
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join(format!("{}.db", name));
        let _ = std::fs::remove_file(&db_path);
        (db_path.to_str().unwrap().to_string(), db_path)
    }

    fn sample_record(route: &str, source: &str, client: &str, amount: f64) -> RevenueRecord {
        RevenueRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            route: route.to_string(),
            source: source.to_string(),
            client_name: client.to_string(),
            amount_usd: amount,
            currency: "USD".to_string(),
            status: RevenueStatus::Confirmed,
            notes: None,
            invoice_id: None,
        }
    }

    #[test]
    fn test_revenue_record_and_query() {
        let (db_str, db_path) = temp_db("record_query");
        let tracker = RevenueTracker::new(&db_str).unwrap();

        tracker.record(&sample_record(ROUTE_A, "upwork", "Acme Corp", 500.0)).unwrap();
        tracker.record(&sample_record(ROUTE_C, "blog_ads", "AdNetwork", 150.0)).unwrap();

        let today = tracker.today_total().unwrap();
        assert_eq!(today.count, 2);
        assert!((today.total_usd - 650.0).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_revenue_by_route() {
        let (db_str, db_path) = temp_db("by_route");
        let tracker = RevenueTracker::new(&db_str).unwrap();

        tracker.record(&sample_record(ROUTE_A, "upwork", "Client A", 300.0)).unwrap();
        tracker.record(&sample_record(ROUTE_A, "email_outreach", "Client B", 200.0)).unwrap();
        tracker.record(&sample_record(ROUTE_C, "blog_ads", "AdNetwork", 100.0)).unwrap();
        tracker.record(&sample_record(ROUTE_E, "api_billing", "ApiUser", 50.0)).unwrap();

        let by_route = tracker.by_route(7).unwrap();
        assert_eq!(by_route.len(), 3);

        // Route A should be first (highest total: $500)
        assert_eq!(by_route[0].group, ROUTE_A);
        assert!((by_route[0].total_usd - 500.0).abs() < 0.01);
        assert_eq!(by_route[0].count, 2);

        // Route C: $100
        let route_c = by_route.iter().find(|s| s.group == ROUTE_C).unwrap();
        assert!((route_c.total_usd - 100.0).abs() < 0.01);
        assert_eq!(route_c.count, 1);

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_revenue_tracker_empty() {
        let (db_str, db_path) = temp_db("empty");
        let tracker = RevenueTracker::new(&db_str).unwrap();

        let today = tracker.today_total().unwrap();
        assert_eq!(today.count, 0);
        assert_eq!(today.total_usd, 0.0);

        let by_route = tracker.by_route(30).unwrap();
        assert!(by_route.is_empty());

        let by_source = tracker.by_source(30).unwrap();
        assert!(by_source.is_empty());

        let by_day = tracker.by_day(30).unwrap();
        assert!(by_day.is_empty());

        let _ = std::fs::remove_file(&db_path);
    }

    #[test]
    fn test_revenue_status_tracking() {
        let (db_str, db_path) = temp_db("status");
        let tracker = RevenueTracker::new(&db_str).unwrap();

        let mut rec = sample_record(ROUTE_B, "stripe", "SaaS Customer", 99.0);
        rec.status = RevenueStatus::Pending;
        rec.invoice_id = Some("INV-2026-001".to_string());
        tracker.record(&rec).unwrap();

        let today = Utc::now().format("%Y-%m-%d").to_string();
        let records = tracker.records_between(&today, &today).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, RevenueStatus::Pending);
        assert_eq!(records[0].invoice_id.as_deref(), Some("INV-2026-001"));
        assert!((records[0].amount_usd - 99.0).abs() < 0.01);

        let _ = std::fs::remove_file(&db_path);
    }
}
