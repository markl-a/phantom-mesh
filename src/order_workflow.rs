//! Order Workflow — sales pipeline / case management system.
//!
//! Manages orders through a state machine:
//! Lead -> Demo -> Quote -> Contract -> Delivery -> Acceptance -> Renewal
//! with Cancelled and OnHold side-states.
//!
//! Backed by SQLite (~/.phantom-mesh/orders.db).

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use tracing::{debug, info};
use uuid::Uuid;

// ── Order Status ─────────────────────────────────────────────────────────────

/// Pipeline status with state-machine transitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    Lead,
    Demo,
    Quote,
    Contract,
    Delivery,
    Acceptance,
    Renewal,
    Cancelled,
    OnHold,
}

impl fmt::Display for OrderStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OrderStatus::Lead => write!(f, "lead"),
            OrderStatus::Demo => write!(f, "demo"),
            OrderStatus::Quote => write!(f, "quote"),
            OrderStatus::Contract => write!(f, "contract"),
            OrderStatus::Delivery => write!(f, "delivery"),
            OrderStatus::Acceptance => write!(f, "acceptance"),
            OrderStatus::Renewal => write!(f, "renewal"),
            OrderStatus::Cancelled => write!(f, "cancelled"),
            OrderStatus::OnHold => write!(f, "on_hold"),
        }
    }
}

impl OrderStatus {
    /// Parse from string (case-insensitive).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "lead" => Some(OrderStatus::Lead),
            "demo" => Some(OrderStatus::Demo),
            "quote" => Some(OrderStatus::Quote),
            "contract" => Some(OrderStatus::Contract),
            "delivery" => Some(OrderStatus::Delivery),
            "acceptance" => Some(OrderStatus::Acceptance),
            "renewal" => Some(OrderStatus::Renewal),
            "cancelled" | "canceled" => Some(OrderStatus::Cancelled),
            "on_hold" | "onhold" => Some(OrderStatus::OnHold),
            _ => None,
        }
    }

    /// SLA hours for each active status. Returns None for terminal/side states.
    pub fn sla_hours(&self) -> Option<u64> {
        match self {
            OrderStatus::Lead => Some(48),
            OrderStatus::Demo => Some(72),
            OrderStatus::Quote => Some(120),
            OrderStatus::Contract => Some(168),
            OrderStatus::Delivery => Some(336),
            _ => None,
        }
    }

    /// Valid transitions from this status.
    pub fn valid_transitions(&self) -> Vec<OrderStatus> {
        match self {
            OrderStatus::Lead => vec![OrderStatus::Demo, OrderStatus::Cancelled],
            OrderStatus::Demo => vec![OrderStatus::Quote, OrderStatus::Cancelled],
            OrderStatus::Quote => vec![
                OrderStatus::Contract,
                OrderStatus::Cancelled,
                OrderStatus::Lead, // re-qualify
            ],
            OrderStatus::Contract => vec![OrderStatus::Delivery, OrderStatus::OnHold],
            OrderStatus::Delivery => vec![OrderStatus::Acceptance, OrderStatus::OnHold],
            OrderStatus::Acceptance => vec![OrderStatus::Renewal, OrderStatus::Cancelled],
            OrderStatus::Renewal => vec![OrderStatus::Lead], // new cycle
            OrderStatus::OnHold => vec![
                // OnHold can return to any main pipeline stage
                OrderStatus::Lead,
                OrderStatus::Demo,
                OrderStatus::Quote,
                OrderStatus::Contract,
                OrderStatus::Delivery,
                OrderStatus::Acceptance,
            ],
            OrderStatus::Cancelled => vec![], // terminal
        }
    }

    /// Check if transitioning to `target` is valid.
    pub fn can_transition_to(&self, target: OrderStatus) -> bool {
        self.valid_transitions().contains(&target)
    }
}

// ── Order ────────────────────────────────────────────────────────────────────

/// A single order in the pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
    pub id: String,
    pub customer_name: String,
    pub customer_email: String,
    pub service_tier: String,
    pub status: OrderStatus,
    pub amount_usd: f64,
    pub created_at: String,
    pub updated_at: String,
    pub notes: String,
    pub assigned_agent: String,
    /// Previous status before OnHold (for restoring)
    pub previous_status: Option<String>,
}

// ── Pipeline Summary ─────────────────────────────────────────────────────────

/// Aggregated pipeline view: count and total value per status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineSummary {
    /// (count, total_value_usd) per status
    pub by_status: HashMap<String, (u64, f64)>,
    pub total_orders: u64,
    pub total_value: f64,
}

// ── Order Workflow ───────────────────────────────────────────────────────────

/// SQLite-backed order workflow manager.
pub struct OrderWorkflow {
    conn: Mutex<Connection>,
}

impl OrderWorkflow {
    /// Create a new OrderWorkflow backed by the given SQLite path.
    /// Creates tables if they don't exist.
    pub async fn new(db_path: &str) -> Result<Self> {
        let path = db_path.to_string();
        let conn = tokio::task::spawn_blocking(move || -> Result<Connection> {
            let conn = Connection::open(&path)?;
            conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")?;
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS orders (
                    id TEXT PRIMARY KEY,
                    customer_name TEXT NOT NULL,
                    customer_email TEXT NOT NULL DEFAULT '',
                    service_tier TEXT NOT NULL DEFAULT 'standard',
                    status TEXT NOT NULL DEFAULT 'lead',
                    amount_usd REAL NOT NULL DEFAULT 0.0,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                    notes TEXT NOT NULL DEFAULT '',
                    assigned_agent TEXT NOT NULL DEFAULT '',
                    previous_status TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_orders_status ON orders(status);
                CREATE INDEX IF NOT EXISTS idx_orders_updated ON orders(updated_at);"
            )?;

            Ok(conn)
        }).await.map_err(|e| anyhow::anyhow!("spawn_blocking join error: {}", e))??;

        info!("OrderWorkflow initialized (db: {})", db_path);
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Create a new order in Lead status.
    pub fn create_order(
        &self,
        customer_name: &str,
        customer_email: &str,
        service_tier: &str,
    ) -> Result<Order> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO orders (id, customer_name, customer_email, service_tier, status, amount_usd, created_at, updated_at, notes, assigned_agent)
             VALUES (?1, ?2, ?3, ?4, 'lead', 0.0, ?5, ?5, '', '')",
            params![id, customer_name, customer_email, service_tier, now],
        )?;
        info!("Order created: {} for {}", id, customer_name);
        Ok(Order {
            id,
            customer_name: customer_name.to_string(),
            customer_email: customer_email.to_string(),
            service_tier: service_tier.to_string(),
            status: OrderStatus::Lead,
            amount_usd: 0.0,
            created_at: now.clone(),
            updated_at: now,
            notes: String::new(),
            assigned_agent: String::new(),
            previous_status: None,
        })
    }

    /// Transition an order to a new status. Validates the state machine.
    pub fn transition(&self, id: &str, new_status: OrderStatus) -> Result<Order> {
        let conn = self.conn.lock().unwrap();

        // Fetch current order
        let current = Self::get_order_inner(&conn, id)?
            .ok_or_else(|| anyhow!("Order '{}' not found", id))?;

        // Validate transition
        if !current.status.can_transition_to(new_status) {
            return Err(anyhow!(
                "Invalid transition: {} -> {} (allowed: {:?})",
                current.status,
                new_status,
                current.status.valid_transitions()
            ));
        }

        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();

        // When going to OnHold, store previous status
        let previous = if new_status == OrderStatus::OnHold {
            Some(current.status.to_string())
        } else {
            None
        };

        conn.execute(
            "UPDATE orders SET status = ?1, updated_at = ?2, previous_status = ?3 WHERE id = ?4",
            params![new_status.to_string(), now, previous, id],
        )?;

        debug!("Order {} transitioned: {} -> {}", id, current.status, new_status);

        // Re-fetch updated order
        Self::get_order_inner(&conn, id)?
            .ok_or_else(|| anyhow!("Order '{}' disappeared after update", id))
    }

    /// Get an order by ID.
    pub fn get_order(&self, id: &str) -> Result<Option<Order>> {
        let conn = self.conn.lock().unwrap();
        Self::get_order_inner(&conn, id)
    }

    /// Internal: fetch order using an already-held connection.
    fn get_order_inner(conn: &Connection, id: &str) -> Result<Option<Order>> {
        let mut stmt = conn.prepare(
            "SELECT id, customer_name, customer_email, service_tier, status, amount_usd,
                    created_at, updated_at, notes, assigned_agent, previous_status
             FROM orders WHERE id = ?1"
        )?;
        let mut rows = stmt.query_map(params![id], Self::row_to_order)?;
        match rows.next() {
            Some(Ok(order)) => Ok(Some(order)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }

    /// List orders filtered by status.
    pub fn list_by_status(&self, status: OrderStatus) -> Result<Vec<Order>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, customer_name, customer_email, service_tier, status, amount_usd,
                    created_at, updated_at, notes, assigned_agent, previous_status
             FROM orders WHERE status = ?1 ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map(params![status.to_string()], Self::row_to_order)?;
        let mut orders = Vec::new();
        for row in rows {
            orders.push(row?);
        }
        Ok(orders)
    }

    /// List all orders.
    pub fn list_all(&self) -> Result<Vec<Order>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, customer_name, customer_email, service_tier, status, amount_usd,
                    created_at, updated_at, notes, assigned_agent, previous_status
             FROM orders ORDER BY updated_at DESC"
        )?;
        let rows = stmt.query_map([], Self::row_to_order)?;
        let mut orders = Vec::new();
        for row in rows {
            orders.push(row?);
        }
        Ok(orders)
    }

    /// Find orders that have been stuck in a status longer than the SLA.
    /// `sla_hours` overrides the default per-status SLA if provided (0 = use defaults).
    pub fn overdue_orders(&self, sla_hours: u64) -> Result<Vec<Order>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, customer_name, customer_email, service_tier, status, amount_usd,
                    created_at, updated_at, notes, assigned_agent, previous_status
             FROM orders
             WHERE status NOT IN ('cancelled', 'on_hold', 'renewal')
             ORDER BY updated_at ASC"
        )?;
        let rows = stmt.query_map([], Self::row_to_order)?;
        let now = Utc::now();
        let mut overdue = Vec::new();
        for row in rows {
            let order = row?;
            let threshold_hours = if sla_hours > 0 {
                sla_hours
            } else {
                order.status.sla_hours().unwrap_or(168)
            };

            if let Ok(updated) = DateTime::parse_from_str(
                &format!("{} +0000", order.updated_at),
                "%Y-%m-%d %H:%M:%S %z",
            ) {
                let elapsed_hours = (now - updated.with_timezone(&Utc)).num_hours();
                if elapsed_hours >= threshold_hours as i64 {
                    overdue.push(order);
                }
            }
        }
        Ok(overdue)
    }

    /// Generate a pipeline summary with count and total value per status.
    pub fn pipeline_summary(&self) -> Result<PipelineSummary> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT status, COUNT(*), COALESCE(SUM(amount_usd), 0.0)
             FROM orders GROUP BY status"
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u64>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;

        let mut by_status = HashMap::new();
        let mut total_orders = 0u64;
        let mut total_value = 0.0f64;
        for row in rows {
            let (status, count, value) = row?;
            total_orders += count;
            total_value += value;
            by_status.insert(status, (count, value));
        }

        Ok(PipelineSummary {
            by_status,
            total_orders,
            total_value,
        })
    }

    /// Append a note to an order (separated by newline).
    pub fn add_note(&self, id: &str, note: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let existing: Option<String> = conn
            .query_row(
                "SELECT notes FROM orders WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .ok();
        let existing = existing.ok_or_else(|| anyhow!("Order '{}' not found", id))?;

        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let timestamped = format!("[{}] {}", now, note);
        let new_notes = if existing.is_empty() {
            timestamped
        } else {
            format!("{}\n{}", existing, timestamped)
        };

        conn.execute(
            "UPDATE orders SET notes = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_notes, now, id],
        )?;
        debug!("Note added to order {}", id);
        Ok(())
    }

    /// Update the amount_usd for an order.
    pub fn set_amount(&self, id: &str, amount: f64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let rows = conn.execute(
            "UPDATE orders SET amount_usd = ?1, updated_at = ?2 WHERE id = ?3",
            params![amount, now, id],
        )?;
        if rows == 0 {
            return Err(anyhow!("Order '{}' not found", id));
        }
        Ok(())
    }

    /// Update the assigned agent for an order.
    pub fn assign_agent(&self, id: &str, agent: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let rows = conn.execute(
            "UPDATE orders SET assigned_agent = ?1, updated_at = ?2 WHERE id = ?3",
            params![agent, now, id],
        )?;
        if rows == 0 {
            return Err(anyhow!("Order '{}' not found", id));
        }
        Ok(())
    }

    /// Row mapper: SQLite row -> Order
    fn row_to_order(row: &rusqlite::Row<'_>) -> rusqlite::Result<Order> {
        let status_str: String = row.get(4)?;
        let status = OrderStatus::from_str_loose(&status_str)
            .unwrap_or(OrderStatus::Lead);
        let prev: Option<String> = row.get(10)?;
        Ok(Order {
            id: row.get(0)?,
            customer_name: row.get(1)?,
            customer_email: row.get(2)?,
            service_tier: row.get(3)?,
            status,
            amount_usd: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            notes: row.get(8)?,
            assigned_agent: row.get(9)?,
            previous_status: prev,
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    async fn test_workflow() -> (OrderWorkflow, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let wf = OrderWorkflow::new(tmp.path().to_str().unwrap()).await.unwrap();
        (wf, tmp)
    }

    #[tokio::test]
    async fn test_create_order() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("Alice Corp", "alice@example.com", "pro").unwrap();
        assert_eq!(order.customer_name, "Alice Corp");
        assert_eq!(order.customer_email, "alice@example.com");
        assert_eq!(order.service_tier, "pro");
        assert_eq!(order.status, OrderStatus::Lead);
        assert_eq!(order.amount_usd, 0.0);
        assert!(!order.id.is_empty());
    }

    #[tokio::test]
    async fn test_get_order() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("Bob Inc", "bob@test.com", "standard").unwrap();
        let fetched = wf.get_order(&order.id).unwrap().unwrap();
        assert_eq!(fetched.customer_name, "Bob Inc");
        assert_eq!(fetched.status, OrderStatus::Lead);
    }

    #[tokio::test]
    async fn test_get_order_not_found() {
        let (wf, _tmp) = test_workflow().await;
        let result = wf.get_order("nonexistent").unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_transition_lead_to_demo() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("C Corp", "c@test.com", "pro").unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Demo).unwrap();
        assert_eq!(updated.status, OrderStatus::Demo);
    }

    #[tokio::test]
    async fn test_transition_demo_to_quote() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("D Corp", "d@test.com", "pro").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Quote).unwrap();
        assert_eq!(updated.status, OrderStatus::Quote);
    }

    #[tokio::test]
    async fn test_transition_quote_to_contract() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("E Corp", "e@test.com", "pro").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Contract).unwrap();
        assert_eq!(updated.status, OrderStatus::Contract);
    }

    #[tokio::test]
    async fn test_transition_contract_to_delivery() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("F Corp", "f@test.com", "team").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        wf.transition(&order.id, OrderStatus::Contract).unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Delivery).unwrap();
        assert_eq!(updated.status, OrderStatus::Delivery);
    }

    #[tokio::test]
    async fn test_transition_delivery_to_acceptance() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("G Corp", "g@test.com", "team").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        wf.transition(&order.id, OrderStatus::Contract).unwrap();
        wf.transition(&order.id, OrderStatus::Delivery).unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Acceptance).unwrap();
        assert_eq!(updated.status, OrderStatus::Acceptance);
    }

    #[tokio::test]
    async fn test_transition_acceptance_to_renewal() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("H Corp", "h@test.com", "team").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        wf.transition(&order.id, OrderStatus::Contract).unwrap();
        wf.transition(&order.id, OrderStatus::Delivery).unwrap();
        wf.transition(&order.id, OrderStatus::Acceptance).unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Renewal).unwrap();
        assert_eq!(updated.status, OrderStatus::Renewal);
    }

    #[tokio::test]
    async fn test_transition_renewal_to_lead() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("I Corp", "i@test.com", "standard").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        wf.transition(&order.id, OrderStatus::Contract).unwrap();
        wf.transition(&order.id, OrderStatus::Delivery).unwrap();
        wf.transition(&order.id, OrderStatus::Acceptance).unwrap();
        wf.transition(&order.id, OrderStatus::Renewal).unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Lead).unwrap();
        assert_eq!(updated.status, OrderStatus::Lead);
    }

    #[tokio::test]
    async fn test_transition_quote_back_to_lead() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("J Corp", "j@test.com", "standard").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        // Re-qualify: Quote -> Lead
        let updated = wf.transition(&order.id, OrderStatus::Lead).unwrap();
        assert_eq!(updated.status, OrderStatus::Lead);
    }

    #[tokio::test]
    async fn test_transition_to_cancelled() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("K Corp", "k@test.com", "lite").unwrap();
        let updated = wf.transition(&order.id, OrderStatus::Cancelled).unwrap();
        assert_eq!(updated.status, OrderStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_invalid_transition_lead_to_contract() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("L Corp", "l@test.com", "pro").unwrap();
        let result = wf.transition(&order.id, OrderStatus::Contract);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid transition"));
    }

    #[tokio::test]
    async fn test_invalid_transition_cancelled_to_anything() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("M Corp", "m@test.com", "pro").unwrap();
        wf.transition(&order.id, OrderStatus::Cancelled).unwrap();
        // Cancelled is terminal — no transitions out
        let result = wf.transition(&order.id, OrderStatus::Lead);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_transition_demo_to_delivery() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("N Corp", "n@test.com", "pro").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        let result = wf.transition(&order.id, OrderStatus::Delivery);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_on_hold_and_resume() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("O Corp", "o@test.com", "team").unwrap();
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        wf.transition(&order.id, OrderStatus::Contract).unwrap();
        // Put on hold
        let held = wf.transition(&order.id, OrderStatus::OnHold).unwrap();
        assert_eq!(held.status, OrderStatus::OnHold);
        assert_eq!(held.previous_status.as_deref(), Some("contract"));
        // Resume to Contract
        let resumed = wf.transition(&order.id, OrderStatus::Contract).unwrap();
        assert_eq!(resumed.status, OrderStatus::Contract);
    }

    #[tokio::test]
    async fn test_transition_nonexistent_order() {
        let (wf, _tmp) = test_workflow().await;
        let result = wf.transition("does-not-exist", OrderStatus::Demo);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_list_by_status() {
        let (wf, _tmp) = test_workflow().await;
        wf.create_order("A1", "a1@test.com", "pro").unwrap();
        wf.create_order("A2", "a2@test.com", "pro").unwrap();
        let o3 = wf.create_order("A3", "a3@test.com", "pro").unwrap();
        wf.transition(&o3.id, OrderStatus::Demo).unwrap();

        let leads = wf.list_by_status(OrderStatus::Lead).unwrap();
        assert_eq!(leads.len(), 2);
        let demos = wf.list_by_status(OrderStatus::Demo).unwrap();
        assert_eq!(demos.len(), 1);
    }

    #[tokio::test]
    async fn test_list_all() {
        let (wf, _tmp) = test_workflow().await;
        wf.create_order("B1", "b1@test.com", "lite").unwrap();
        wf.create_order("B2", "b2@test.com", "lite").unwrap();
        wf.create_order("B3", "b3@test.com", "lite").unwrap();
        let all = wf.list_all().unwrap();
        assert_eq!(all.len(), 3);
    }

    #[tokio::test]
    async fn test_pipeline_summary() {
        let (wf, _tmp) = test_workflow().await;
        let o1 = wf.create_order("C1", "c1@test.com", "pro").unwrap();
        let o2 = wf.create_order("C2", "c2@test.com", "pro").unwrap();
        wf.set_amount(&o1.id, 500.0).unwrap();
        wf.set_amount(&o2.id, 300.0).unwrap();
        wf.transition(&o2.id, OrderStatus::Demo).unwrap();

        let summary = wf.pipeline_summary().unwrap();
        assert_eq!(summary.total_orders, 2);
        assert!((summary.total_value - 800.0).abs() < 0.01);
        let (lead_count, lead_val) = summary.by_status.get("lead").unwrap();
        assert_eq!(*lead_count, 1);
        assert!((*lead_val - 500.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_add_note() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("D1", "d1@test.com", "standard").unwrap();
        wf.add_note(&order.id, "Initial contact made").unwrap();
        wf.add_note(&order.id, "Follow-up scheduled").unwrap();
        let fetched = wf.get_order(&order.id).unwrap().unwrap();
        assert!(fetched.notes.contains("Initial contact made"));
        assert!(fetched.notes.contains("Follow-up scheduled"));
    }

    #[tokio::test]
    async fn test_add_note_not_found() {
        let (wf, _tmp) = test_workflow().await;
        let result = wf.add_note("ghost", "test");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_amount() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("E1", "e1@test.com", "pro").unwrap();
        wf.set_amount(&order.id, 1500.0).unwrap();
        let fetched = wf.get_order(&order.id).unwrap().unwrap();
        assert!((fetched.amount_usd - 1500.0).abs() < 0.01);
    }

    #[tokio::test]
    async fn test_assign_agent() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("F1", "f1@test.com", "team").unwrap();
        wf.assign_agent(&order.id, "sales-bot").unwrap();
        let fetched = wf.get_order(&order.id).unwrap().unwrap();
        assert_eq!(fetched.assigned_agent, "sales-bot");
    }

    #[tokio::test]
    async fn test_full_pipeline_journey() {
        let (wf, _tmp) = test_workflow().await;
        let order = wf.create_order("FullCycle Corp", "full@test.com", "team").unwrap();

        // Lead -> Demo -> Quote -> Contract -> Delivery -> Acceptance -> Renewal -> Lead
        wf.transition(&order.id, OrderStatus::Demo).unwrap();
        wf.transition(&order.id, OrderStatus::Quote).unwrap();
        wf.transition(&order.id, OrderStatus::Contract).unwrap();
        wf.transition(&order.id, OrderStatus::Delivery).unwrap();
        wf.transition(&order.id, OrderStatus::Acceptance).unwrap();
        wf.transition(&order.id, OrderStatus::Renewal).unwrap();
        let final_order = wf.transition(&order.id, OrderStatus::Lead).unwrap();
        assert_eq!(final_order.status, OrderStatus::Lead);
    }

    #[test]
    fn test_order_status_sla_hours() {
        assert_eq!(OrderStatus::Lead.sla_hours(), Some(48));
        assert_eq!(OrderStatus::Demo.sla_hours(), Some(72));
        assert_eq!(OrderStatus::Quote.sla_hours(), Some(120));
        assert_eq!(OrderStatus::Contract.sla_hours(), Some(168));
        assert_eq!(OrderStatus::Delivery.sla_hours(), Some(336));
        assert_eq!(OrderStatus::Acceptance.sla_hours(), None);
        assert_eq!(OrderStatus::Cancelled.sla_hours(), None);
        assert_eq!(OrderStatus::OnHold.sla_hours(), None);
    }

    #[test]
    fn test_order_status_display() {
        assert_eq!(OrderStatus::Lead.to_string(), "lead");
        assert_eq!(OrderStatus::OnHold.to_string(), "on_hold");
        assert_eq!(OrderStatus::Cancelled.to_string(), "cancelled");
    }

    #[test]
    fn test_order_status_from_str_loose() {
        assert_eq!(OrderStatus::from_str_loose("LEAD"), Some(OrderStatus::Lead));
        assert_eq!(OrderStatus::from_str_loose("on_hold"), Some(OrderStatus::OnHold));
        assert_eq!(OrderStatus::from_str_loose("OnHold"), Some(OrderStatus::OnHold));
        assert_eq!(OrderStatus::from_str_loose("canceled"), Some(OrderStatus::Cancelled));
        assert_eq!(OrderStatus::from_str_loose("bogus"), None);
    }

    #[test]
    fn test_valid_transitions_completeness() {
        // Lead can go to Demo or Cancelled
        let lt = OrderStatus::Lead.valid_transitions();
        assert!(lt.contains(&OrderStatus::Demo));
        assert!(lt.contains(&OrderStatus::Cancelled));
        assert!(!lt.contains(&OrderStatus::Contract));

        // OnHold can return to any main stage
        let oht = OrderStatus::OnHold.valid_transitions();
        assert!(oht.contains(&OrderStatus::Lead));
        assert!(oht.contains(&OrderStatus::Contract));
        assert!(oht.contains(&OrderStatus::Delivery));

        // Cancelled is terminal
        assert!(OrderStatus::Cancelled.valid_transitions().is_empty());
    }
}
