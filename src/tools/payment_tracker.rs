//! Payment Tracker tool — manage invoices and payments backed by SQLite.
//! Supports: create_invoice, record_payment, list_outstanding, revenue_by_period.
//! DB: ~/.phantom-mesh/payments.db (auto-created).

use anyhow::Result;
use async_trait::async_trait;
use rusqlite::params;
use serde_json::{json, Value};
use tracing::debug;

use super::{Tool, ToolResult};

/// Default path for the payments database.
fn default_db_path() -> String {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    format!("{}/.phantom-mesh/payments.db", home)
}

/// Validate a date string is YYYY-MM-DD format with valid ranges.
fn validate_date(date: &str) -> bool {
    if date.len() != 10 {
        return false;
    }
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    match (
        parts[0].parse::<u32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) {
        (Ok(y), Ok(m), Ok(d)) => {
            y >= 2000 && y <= 2100 && m >= 1 && m <= 12 && d >= 1 && d <= 31
        }
        _ => false,
    }
}

/// Generate a simple unique ID based on timestamp + random suffix.
fn generate_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let rand_suffix: u32 = (ts as u32).wrapping_mul(2654435761); // simple hash
    format!("{}_{:x}_{:04x}", prefix, ts, rand_suffix & 0xFFFF)
}

/// Payment tracker tool — manages invoices and payments via SQLite.
pub struct PaymentTrackerTool {
    db_path: String,
}

impl PaymentTrackerTool {
    /// Create a new PaymentTrackerTool with the default DB path (~/.phantom-mesh/payments.db).
    pub fn new() -> Result<Self> {
        Self::with_db_path(&default_db_path())
    }

    /// Create a new PaymentTrackerTool with a custom DB path (useful for testing).
    pub fn with_db_path(db_path: &str) -> Result<Self> {
        let conn = rusqlite::Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS invoices (
                id TEXT PRIMARY KEY,
                client TEXT NOT NULL,
                amount REAL NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                due_date TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'unpaid',
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_invoices_client ON invoices(client);
            CREATE INDEX IF NOT EXISTS idx_invoices_status ON invoices(status);
            CREATE INDEX IF NOT EXISTS idx_invoices_due_date ON invoices(due_date);

            CREATE TABLE IF NOT EXISTS payments (
                id TEXT PRIMARY KEY,
                invoice_id TEXT NOT NULL,
                amount REAL NOT NULL,
                method TEXT NOT NULL DEFAULT '',
                paid_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (invoice_id) REFERENCES invoices(id)
            );
            CREATE INDEX IF NOT EXISTS idx_payments_invoice ON payments(invoice_id);
            CREATE INDEX IF NOT EXISTS idx_payments_paid_at ON payments(paid_at);",
        )?;
        Ok(Self {
            db_path: db_path.to_string(),
        })
    }

    /// Create a new invoice.
    fn create_invoice(
        &self,
        client: &str,
        amount: f64,
        description: &str,
        due_date: &str,
    ) -> Result<String> {
        let id = generate_id("inv");
        let conn = rusqlite::Connection::open(&self.db_path)?;
        conn.execute(
            "INSERT INTO invoices (id, client, amount, description, due_date, status)
             VALUES (?1, ?2, ?3, ?4, ?5, 'unpaid')",
            params![id, client, amount, description, due_date],
        )?;
        debug!(
            "payment_tracker: created invoice {} for {} — ${:.2} due {}",
            id, client, amount, due_date
        );
        Ok(json!({
            "message": "Invoice created",
            "invoice_id": id,
            "client": client,
            "amount": amount,
            "description": description,
            "due_date": due_date,
            "status": "unpaid"
        })
        .to_string())
    }

    /// Record a payment against an invoice.
    fn record_payment(
        &self,
        invoice_id: &str,
        amount: f64,
        method: &str,
    ) -> Result<String> {
        let conn = rusqlite::Connection::open(&self.db_path)?;

        // Verify the invoice exists
        let invoice_exists: bool = conn.query_row(
            "SELECT COUNT(*) FROM invoices WHERE id = ?1",
            params![invoice_id],
            |row| row.get::<_, i64>(0),
        )? > 0;

        if !invoice_exists {
            anyhow::bail!("Invoice '{}' not found", invoice_id);
        }

        let payment_id = generate_id("pay");
        conn.execute(
            "INSERT INTO payments (id, invoice_id, amount, method)
             VALUES (?1, ?2, ?3, ?4)",
            params![payment_id, invoice_id, amount, method],
        )?;

        // Calculate total paid for this invoice
        let total_paid: f64 = conn.query_row(
            "SELECT COALESCE(SUM(amount), 0.0) FROM payments WHERE invoice_id = ?1",
            params![invoice_id],
            |row| row.get(0),
        )?;

        // Get the invoice amount
        let invoice_amount: f64 = conn.query_row(
            "SELECT amount FROM invoices WHERE id = ?1",
            params![invoice_id],
            |row| row.get(0),
        )?;

        // Update invoice status based on total payments
        let new_status = if total_paid >= invoice_amount {
            "paid"
        } else {
            "partial"
        };
        conn.execute(
            "UPDATE invoices SET status = ?1 WHERE id = ?2",
            params![new_status, invoice_id],
        )?;

        debug!(
            "payment_tracker: recorded payment {} — ${:.2} via {} for invoice {}",
            payment_id, amount, method, invoice_id
        );

        Ok(json!({
            "message": "Payment recorded",
            "payment_id": payment_id,
            "invoice_id": invoice_id,
            "amount_paid": amount,
            "method": method,
            "total_paid": total_paid,
            "invoice_amount": invoice_amount,
            "invoice_status": new_status
        })
        .to_string())
    }

    /// List outstanding (unpaid/partial) invoices, optionally filtered by client or date range.
    fn list_outstanding(
        &self,
        client: Option<&str>,
        start_date: Option<&str>,
        end_date: Option<&str>,
    ) -> Result<String> {
        let conn = rusqlite::Connection::open(&self.db_path)?;

        let mut sql = String::from(
            "SELECT id, client, amount, description, due_date, status, created_at
             FROM invoices WHERE status IN ('unpaid', 'partial')",
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        let mut param_idx = 1;

        if let Some(c) = client {
            if !c.is_empty() {
                sql.push_str(&format!(" AND client = ?{}", param_idx));
                param_values.push(Box::new(c.to_string()));
                param_idx += 1;
            }
        }
        if let Some(sd) = start_date {
            if !sd.is_empty() {
                sql.push_str(&format!(" AND due_date >= ?{}", param_idx));
                param_values.push(Box::new(sd.to_string()));
                param_idx += 1;
            }
        }
        if let Some(ed) = end_date {
            if !ed.is_empty() {
                sql.push_str(&format!(" AND due_date <= ?{}", param_idx));
                param_values.push(Box::new(ed.to_string()));
                let _ = param_idx; // suppress unused warning
            }
        }

        sql.push_str(" ORDER BY due_date ASC");

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let invoices: Vec<Value> = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(json!({
                    "id": row.get::<_, String>(0)?,
                    "client": row.get::<_, String>(1)?,
                    "amount": row.get::<_, f64>(2)?,
                    "description": row.get::<_, String>(3)?,
                    "due_date": row.get::<_, String>(4)?,
                    "status": row.get::<_, String>(5)?,
                    "created_at": row.get::<_, String>(6)?,
                }))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(json!({
            "count": invoices.len(),
            "invoices": invoices
        })
        .to_string())
    }

    /// Calculate revenue for a date period based on payments received.
    fn revenue_by_period(&self, start_date: &str, end_date: &str) -> Result<String> {
        let conn = rusqlite::Connection::open(&self.db_path)?;

        // Total payments received in the period
        let mut stmt = conn.prepare(
            "SELECT COALESCE(SUM(p.amount), 0.0), COUNT(*)
             FROM payments p
             WHERE date(p.paid_at) >= ?1 AND date(p.paid_at) <= ?2",
        )?;
        let (total_revenue, payment_count): (f64, i64) =
            stmt.query_row(params![start_date, end_date], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?;

        // Revenue broken down by client
        let mut stmt2 = conn.prepare(
            "SELECT i.client, SUM(p.amount) as client_total, COUNT(p.id)
             FROM payments p
             JOIN invoices i ON p.invoice_id = i.id
             WHERE date(p.paid_at) >= ?1 AND date(p.paid_at) <= ?2
             GROUP BY i.client
             ORDER BY client_total DESC",
        )?;
        let by_client: Vec<Value> = stmt2
            .query_map(params![start_date, end_date], |row| {
                Ok(json!({
                    "client": row.get::<_, String>(0)?,
                    "total": row.get::<_, f64>(1)?,
                    "payments": row.get::<_, i64>(2)?,
                }))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Revenue broken down by payment method
        let mut stmt3 = conn.prepare(
            "SELECT p.method, SUM(p.amount) as method_total, COUNT(p.id)
             FROM payments p
             WHERE date(p.paid_at) >= ?1 AND date(p.paid_at) <= ?2
             GROUP BY p.method
             ORDER BY method_total DESC",
        )?;
        let by_method: Vec<Value> = stmt3
            .query_map(params![start_date, end_date], |row| {
                Ok(json!({
                    "method": row.get::<_, String>(0)?,
                    "total": row.get::<_, f64>(1)?,
                    "payments": row.get::<_, i64>(2)?,
                }))
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(json!({
            "start_date": start_date,
            "end_date": end_date,
            "total_revenue": total_revenue,
            "payment_count": payment_count,
            "by_client": by_client,
            "by_method": by_method
        })
        .to_string())
    }
}

#[async_trait]
impl Tool for PaymentTrackerTool {
    fn name(&self) -> &str {
        "payment_tracker"
    }

    fn description(&self) -> &str {
        "Track invoices and payments. Actions: create_invoice, record_payment, \
         list_outstanding, revenue_by_period. Backed by SQLite at ~/.phantom-mesh/payments.db."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create_invoice", "record_payment", "list_outstanding", "revenue_by_period"],
                    "description": "The payment tracking action to perform"
                },
                "client": {
                    "type": "string",
                    "description": "Client name (required for create_invoice, optional filter for list_outstanding)"
                },
                "amount": {
                    "type": "number",
                    "description": "Amount in USD (required for create_invoice and record_payment)"
                },
                "description": {
                    "type": "string",
                    "description": "Invoice description (for create_invoice)"
                },
                "due_date": {
                    "type": "string",
                    "description": "Due date in YYYY-MM-DD format (required for create_invoice)"
                },
                "invoice_id": {
                    "type": "string",
                    "description": "Invoice ID (required for record_payment)"
                },
                "method": {
                    "type": "string",
                    "description": "Payment method, e.g. 'stripe', 'bank_transfer', 'paypal' (for record_payment)"
                },
                "start_date": {
                    "type": "string",
                    "description": "Start date in YYYY-MM-DD (for revenue_by_period and list_outstanding filter)"
                },
                "end_date": {
                    "type": "string",
                    "description": "End date in YYYY-MM-DD (for revenue_by_period and list_outstanding filter)"
                }
            },
            "required": ["action"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match action {
            "create_invoice" => {
                let client = args.get("client").and_then(|v| v.as_str()).unwrap_or("");
                let amount = args.get("amount").and_then(|v| v.as_f64());
                let due_date = args.get("due_date").and_then(|v| v.as_str()).unwrap_or("");
                if client.is_empty() {
                    anyhow::bail!("create_invoice requires 'client'");
                }
                if amount.is_none() || amount.unwrap() <= 0.0 {
                    anyhow::bail!("create_invoice requires a positive 'amount'");
                }
                if due_date.is_empty() || !validate_date(due_date) {
                    anyhow::bail!(
                        "create_invoice requires a valid 'due_date' in YYYY-MM-DD format"
                    );
                }
                Ok(())
            }
            "record_payment" => {
                let invoice_id = args
                    .get("invoice_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let amount = args.get("amount").and_then(|v| v.as_f64());
                if invoice_id.is_empty() {
                    anyhow::bail!("record_payment requires 'invoice_id'");
                }
                if amount.is_none() || amount.unwrap() <= 0.0 {
                    anyhow::bail!("record_payment requires a positive 'amount'");
                }
                Ok(())
            }
            "list_outstanding" => Ok(()),
            "revenue_by_period" => {
                let start = args
                    .get("start_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let end = args
                    .get("end_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if start.is_empty() || !validate_date(start) {
                    anyhow::bail!(
                        "revenue_by_period requires a valid 'start_date' in YYYY-MM-DD format"
                    );
                }
                if end.is_empty() || !validate_date(end) {
                    anyhow::bail!(
                        "revenue_by_period requires a valid 'end_date' in YYYY-MM-DD format"
                    );
                }
                Ok(())
            }
            "" => anyhow::bail!("Missing required parameter: 'action'"),
            _ => anyhow::bail!(
                "Unknown action '{}'. Available: create_invoice, record_payment, list_outstanding, revenue_by_period",
                action
            ),
        }
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let result = match action {
            "create_invoice" => {
                let client = args.get("client").and_then(|v| v.as_str()).unwrap_or("");
                let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let description = args
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let due_date = args
                    .get("due_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if client.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'client' is required for create_invoice".into(),
                    });
                }
                if amount <= 0.0 {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'amount' must be a positive number".into(),
                    });
                }
                if due_date.is_empty() || !validate_date(due_date) {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'due_date' must be a valid YYYY-MM-DD date".into(),
                    });
                }

                self.create_invoice(client, amount, description, due_date)
            }
            "record_payment" => {
                let invoice_id = args
                    .get("invoice_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let amount = args.get("amount").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let method = args
                    .get("method")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");

                if invoice_id.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'invoice_id' is required for record_payment".into(),
                    });
                }
                if amount <= 0.0 {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'amount' must be a positive number".into(),
                    });
                }

                self.record_payment(invoice_id, amount, method)
            }
            "list_outstanding" => {
                let client = args.get("client").and_then(|v| v.as_str());
                let start_date = args.get("start_date").and_then(|v| v.as_str());
                let end_date = args.get("end_date").and_then(|v| v.as_str());

                self.list_outstanding(client, start_date, end_date)
            }
            "revenue_by_period" => {
                let start_date = args
                    .get("start_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let end_date = args
                    .get("end_date")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if start_date.is_empty() || !validate_date(start_date) {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'start_date' must be a valid YYYY-MM-DD date".into(),
                    });
                }
                if end_date.is_empty() || !validate_date(end_date) {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: 'end_date' must be a valid YYYY-MM-DD date".into(),
                    });
                }

                self.revenue_by_period(start_date, end_date)
            }
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Unknown action '{}'. Available: create_invoice, record_payment, \
                         list_outstanding, revenue_by_period",
                        action
                    ),
                });
            }
        };

        match result {
            Ok(output) => Ok(ToolResult {
                success: true,
                output,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Payment tracker error: {}", e),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a PaymentTrackerTool backed by a temporary DB file.
    fn make_tool() -> (PaymentTrackerTool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_payments.db");
        let tool =
            PaymentTrackerTool::with_db_path(db_path.to_str().unwrap()).unwrap();
        (tool, dir) // keep dir alive so the DB file persists
    }

    // ── Tool metadata ────────────────────────────────────────────────

    #[test]
    fn test_name() {
        let (tool, _dir) = make_tool();
        assert_eq!(tool.name(), "payment_tracker");
    }

    #[test]
    fn test_description_contains_keywords() {
        let (tool, _dir) = make_tool();
        let desc = tool.description();
        assert!(desc.contains("invoice"), "desc: {}", desc);
        assert!(desc.contains("payment"), "desc: {}", desc);
    }

    #[test]
    fn test_parameters_schema_has_action() {
        let (tool, _dir) = make_tool();
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("action")));
    }

    // ── Preflight validation ─────────────────────────────────────────

    #[test]
    fn test_preflight_missing_action() {
        let (tool, _dir) = make_tool();
        let err = tool.preflight(&json!({}));
        assert!(err.is_err());
        assert!(
            err.unwrap_err().to_string().contains("action"),
            "should mention missing action"
        );
    }

    #[test]
    fn test_preflight_unknown_action() {
        let (tool, _dir) = make_tool();
        let err = tool.preflight(&json!({"action": "explode"}));
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("Unknown action"));
    }

    #[test]
    fn test_preflight_create_invoice_missing_client() {
        let (tool, _dir) = make_tool();
        let err = tool.preflight(&json!({
            "action": "create_invoice",
            "amount": 100.0,
            "due_date": "2026-04-01"
        }));
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("client"));
    }

    #[test]
    fn test_preflight_create_invoice_invalid_date() {
        let (tool, _dir) = make_tool();
        let err = tool.preflight(&json!({
            "action": "create_invoice",
            "client": "Acme",
            "amount": 100.0,
            "due_date": "not-a-date"
        }));
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("due_date"));
    }

    #[test]
    fn test_preflight_create_invoice_zero_amount() {
        let (tool, _dir) = make_tool();
        let err = tool.preflight(&json!({
            "action": "create_invoice",
            "client": "Acme",
            "amount": 0,
            "due_date": "2026-04-01"
        }));
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("amount"));
    }

    #[test]
    fn test_preflight_record_payment_missing_invoice_id() {
        let (tool, _dir) = make_tool();
        let err = tool.preflight(&json!({
            "action": "record_payment",
            "amount": 50.0
        }));
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("invoice_id"));
    }

    #[test]
    fn test_preflight_revenue_by_period_missing_dates() {
        let (tool, _dir) = make_tool();
        let err = tool.preflight(&json!({
            "action": "revenue_by_period"
        }));
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("start_date"));
    }

    #[test]
    fn test_preflight_valid_create_invoice() {
        let (tool, _dir) = make_tool();
        let result = tool.preflight(&json!({
            "action": "create_invoice",
            "client": "Acme Corp",
            "amount": 500.0,
            "due_date": "2026-04-15"
        }));
        assert!(result.is_ok());
    }

    // ── Execute: create_invoice ──────────────────────────────────────

    #[tokio::test]
    async fn test_create_invoice_success() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_invoice",
                "client": "Acme Corp",
                "amount": 1500.00,
                "description": "Website redesign phase 1",
                "due_date": "2026-04-15"
            }))
            .await
            .unwrap();
        assert!(result.success, "output: {}", result.output);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["client"], "Acme Corp");
        assert_eq!(v["amount"], 1500.0);
        assert_eq!(v["status"], "unpaid");
        assert!(v["invoice_id"].as_str().unwrap().starts_with("inv_"));
    }

    #[tokio::test]
    async fn test_create_invoice_missing_client() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_invoice",
                "amount": 100.0,
                "due_date": "2026-04-01"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("client"));
    }

    #[tokio::test]
    async fn test_create_invoice_invalid_date() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_invoice",
                "client": "Test",
                "amount": 100.0,
                "due_date": "bad-date"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("due_date"));
    }

    // ── Execute: record_payment ──────────────────────────────────────

    #[tokio::test]
    async fn test_record_payment_success() {
        let (tool, _dir) = make_tool();

        // First create an invoice
        let inv_result = tool
            .execute(json!({
                "action": "create_invoice",
                "client": "Beta LLC",
                "amount": 1000.0,
                "description": "Consulting",
                "due_date": "2026-05-01"
            }))
            .await
            .unwrap();
        assert!(inv_result.success);
        let inv: Value = serde_json::from_str(&inv_result.output).unwrap();
        let invoice_id = inv["invoice_id"].as_str().unwrap();

        // Record a partial payment
        let pay_result = tool
            .execute(json!({
                "action": "record_payment",
                "invoice_id": invoice_id,
                "amount": 500.0,
                "method": "bank_transfer"
            }))
            .await
            .unwrap();
        assert!(pay_result.success, "output: {}", pay_result.output);
        let pay: Value = serde_json::from_str(&pay_result.output).unwrap();
        assert_eq!(pay["invoice_status"], "partial");
        assert_eq!(pay["total_paid"], 500.0);
    }

    #[tokio::test]
    async fn test_record_payment_full_marks_paid() {
        let (tool, _dir) = make_tool();

        // Create invoice
        let inv_result = tool
            .execute(json!({
                "action": "create_invoice",
                "client": "Gamma Inc",
                "amount": 200.0,
                "description": "Logo design",
                "due_date": "2026-04-20"
            }))
            .await
            .unwrap();
        let inv: Value = serde_json::from_str(&inv_result.output).unwrap();
        let invoice_id = inv["invoice_id"].as_str().unwrap();

        // Pay in full
        let pay_result = tool
            .execute(json!({
                "action": "record_payment",
                "invoice_id": invoice_id,
                "amount": 200.0,
                "method": "stripe"
            }))
            .await
            .unwrap();
        assert!(pay_result.success);
        let pay: Value = serde_json::from_str(&pay_result.output).unwrap();
        assert_eq!(pay["invoice_status"], "paid");
    }

    #[tokio::test]
    async fn test_record_payment_nonexistent_invoice() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "record_payment",
                "invoice_id": "inv_nonexistent",
                "amount": 100.0,
                "method": "cash"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("not found"));
    }

    #[tokio::test]
    async fn test_record_payment_missing_invoice_id() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "record_payment",
                "amount": 50.0
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("invoice_id"));
    }

    // ── Execute: list_outstanding ────────────────────────────────────

    #[tokio::test]
    async fn test_list_outstanding_empty() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({"action": "list_outstanding"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["count"], 0);
    }

    #[tokio::test]
    async fn test_list_outstanding_with_invoices() {
        let (tool, _dir) = make_tool();

        // Create two invoices
        tool.execute(json!({
            "action": "create_invoice",
            "client": "Alpha Co",
            "amount": 300.0,
            "description": "Service A",
            "due_date": "2026-04-10"
        }))
        .await
        .unwrap();

        tool.execute(json!({
            "action": "create_invoice",
            "client": "Beta Co",
            "amount": 700.0,
            "description": "Service B",
            "due_date": "2026-04-20"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({"action": "list_outstanding"}))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["count"], 2);
    }

    #[tokio::test]
    async fn test_list_outstanding_filter_by_client() {
        let (tool, _dir) = make_tool();

        tool.execute(json!({
            "action": "create_invoice",
            "client": "Alpha Co",
            "amount": 300.0,
            "due_date": "2026-04-10"
        }))
        .await
        .unwrap();

        tool.execute(json!({
            "action": "create_invoice",
            "client": "Beta Co",
            "amount": 700.0,
            "due_date": "2026-04-20"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({
                "action": "list_outstanding",
                "client": "Alpha Co"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["invoices"][0]["client"], "Alpha Co");
    }

    #[tokio::test]
    async fn test_list_outstanding_excludes_paid() {
        let (tool, _dir) = make_tool();

        // Create and fully pay one invoice
        let inv = tool
            .execute(json!({
                "action": "create_invoice",
                "client": "Paid Co",
                "amount": 100.0,
                "due_date": "2026-04-01"
            }))
            .await
            .unwrap();
        let inv_v: Value = serde_json::from_str(&inv.output).unwrap();
        let inv_id = inv_v["invoice_id"].as_str().unwrap();

        tool.execute(json!({
            "action": "record_payment",
            "invoice_id": inv_id,
            "amount": 100.0,
            "method": "stripe"
        }))
        .await
        .unwrap();

        // Create an unpaid invoice
        tool.execute(json!({
            "action": "create_invoice",
            "client": "Unpaid Co",
            "amount": 500.0,
            "due_date": "2026-04-15"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(json!({"action": "list_outstanding"}))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["invoices"][0]["client"], "Unpaid Co");
    }

    // ── Execute: revenue_by_period ───────────────────────────────────

    #[tokio::test]
    async fn test_revenue_by_period_empty() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "revenue_by_period",
                "start_date": "2026-01-01",
                "end_date": "2026-12-31"
            }))
            .await
            .unwrap();
        assert!(result.success);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["total_revenue"], 0.0);
        assert_eq!(v["payment_count"], 0);
    }

    #[tokio::test]
    async fn test_revenue_by_period_with_payments() {
        let (tool, _dir) = make_tool();

        // Create an invoice and make a payment
        let inv = tool
            .execute(json!({
                "action": "create_invoice",
                "client": "Revenue Client",
                "amount": 1000.0,
                "description": "Big project",
                "due_date": "2026-06-01"
            }))
            .await
            .unwrap();
        let inv_v: Value = serde_json::from_str(&inv.output).unwrap();
        let inv_id = inv_v["invoice_id"].as_str().unwrap();

        tool.execute(json!({
            "action": "record_payment",
            "invoice_id": inv_id,
            "amount": 600.0,
            "method": "paypal"
        }))
        .await
        .unwrap();

        // Query revenue for a wide range that includes today
        let result = tool
            .execute(json!({
                "action": "revenue_by_period",
                "start_date": "2026-01-01",
                "end_date": "2026-12-31"
            }))
            .await
            .unwrap();
        assert!(result.success, "output: {}", result.output);
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["total_revenue"], 600.0);
        assert_eq!(v["payment_count"], 1);
        // by_client breakdown
        assert_eq!(v["by_client"][0]["client"], "Revenue Client");
        assert_eq!(v["by_client"][0]["total"], 600.0);
        // by_method breakdown
        assert_eq!(v["by_method"][0]["method"], "paypal");
    }

    #[tokio::test]
    async fn test_revenue_by_period_missing_dates() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({
                "action": "revenue_by_period"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("start_date"));
    }

    // ── Execute: unknown action ──────────────────────────────────────

    #[tokio::test]
    async fn test_unknown_action() {
        let (tool, _dir) = make_tool();
        let result = tool
            .execute(json!({"action": "destroy"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        let (tool, _dir) = make_tool();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    // ── Date validation ──────────────────────────────────────────────

    #[test]
    fn test_validate_date_valid() {
        assert!(validate_date("2026-01-01"));
        assert!(validate_date("2026-12-31"));
        assert!(validate_date("2030-06-15"));
    }

    #[test]
    fn test_validate_date_invalid() {
        assert!(!validate_date(""));
        assert!(!validate_date("not-a-date"));
        assert!(!validate_date("2026-13-01"));
        assert!(!validate_date("2026-00-15"));
        assert!(!validate_date("20260101"));
    }

    // ── Multiple payments on same invoice ────────────────────────────

    #[tokio::test]
    async fn test_multiple_partial_payments() {
        let (tool, _dir) = make_tool();

        let inv = tool
            .execute(json!({
                "action": "create_invoice",
                "client": "Multi-Pay Corp",
                "amount": 900.0,
                "description": "Three installments",
                "due_date": "2026-07-01"
            }))
            .await
            .unwrap();
        let inv_v: Value = serde_json::from_str(&inv.output).unwrap();
        let inv_id = inv_v["invoice_id"].as_str().unwrap();

        // First payment — partial
        let p1 = tool
            .execute(json!({
                "action": "record_payment",
                "invoice_id": inv_id,
                "amount": 300.0,
                "method": "bank_transfer"
            }))
            .await
            .unwrap();
        let p1v: Value = serde_json::from_str(&p1.output).unwrap();
        assert_eq!(p1v["invoice_status"], "partial");
        assert_eq!(p1v["total_paid"], 300.0);

        // Second payment — still partial
        let p2 = tool
            .execute(json!({
                "action": "record_payment",
                "invoice_id": inv_id,
                "amount": 300.0,
                "method": "bank_transfer"
            }))
            .await
            .unwrap();
        let p2v: Value = serde_json::from_str(&p2.output).unwrap();
        assert_eq!(p2v["invoice_status"], "partial");
        assert_eq!(p2v["total_paid"], 600.0);

        // Third payment — now fully paid
        let p3 = tool
            .execute(json!({
                "action": "record_payment",
                "invoice_id": inv_id,
                "amount": 300.0,
                "method": "bank_transfer"
            }))
            .await
            .unwrap();
        let p3v: Value = serde_json::from_str(&p3.output).unwrap();
        assert_eq!(p3v["invoice_status"], "paid");
        assert_eq!(p3v["total_paid"], 900.0);

        // Verify it no longer shows in outstanding
        let outstanding = tool
            .execute(json!({"action": "list_outstanding"}))
            .await
            .unwrap();
        let ov: Value = serde_json::from_str(&outstanding.output).unwrap();
        assert_eq!(ov["count"], 0);
    }
}
