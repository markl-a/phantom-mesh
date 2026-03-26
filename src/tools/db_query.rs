// DbQueryTool — read-only SQLite query tool for internal analytics
// Security: only SELECT queries allowed, database allowlist enforced

use anyhow::Result;
use async_trait::async_trait;
use rusqlite::Connection;
use serde_json::{json, Value};
use tracing::info;

use super::{Tool, ToolResult};

/// Allowed database names and their default paths under ~/.phantom-mesh/
const ALLOWED_DBS: &[(&str, &str)] = &[
    ("costs", "costs.db"),
    ("revenue", "revenue.db"),
    ("core", "core.db"),
    ("memory", "memory.db"),
];

/// Maximum number of rows returned by a single query to prevent runaway results.
const MAX_ROWS: usize = 500;

pub struct DbQueryTool;

impl DbQueryTool {
    pub fn new() -> Self {
        Self
    }

    /// Resolve ~db name~ to an absolute path under ~/.phantom-mesh/.
    fn resolve_db_path(db: &str) -> Result<String> {
        let entry = ALLOWED_DBS.iter().find(|(name, _)| *name == db);
        match entry {
            Some((_, filename)) => {
                let home = std::env::var("USERPROFILE")
                    .or_else(|_| std::env::var("HOME"))
                    .unwrap_or_else(|_| ".".to_string());
                Ok(format!("{}/.phantom-mesh/{}", home, filename))
            }
            None => {
                let names: Vec<&str> = ALLOWED_DBS.iter().map(|(n, _)| *n).collect();
                Err(anyhow::anyhow!(
                    "Unknown database '{}'. Allowed: {:?}",
                    db,
                    names
                ))
            }
        }
    }

    /// Validate that a SQL string is a read-only SELECT statement.
    /// Blocks INSERT, UPDATE, DELETE, DROP, ALTER, CREATE, ATTACH, DETACH, PRAGMA (write), etc.
    fn validate_sql(sql: &str) -> Result<()> {
        let trimmed = sql.trim();
        if trimmed.is_empty() {
            return Err(anyhow::anyhow!("SQL query is empty"));
        }

        // Must start with SELECT (case-insensitive)
        let upper = trimmed.to_uppercase();
        if !upper.starts_with("SELECT") {
            return Err(anyhow::anyhow!(
                "Only SELECT queries are allowed. Got: {}",
                &trimmed[..trimmed.len().min(40)]
            ));
        }

        // Block dangerous keywords anywhere in the query (even in subqueries / CTEs)
        let blocked_keywords = [
            "INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "CREATE",
            "ATTACH", "DETACH", "REPLACE", "VACUUM", "REINDEX",
        ];
        // Tokenize on whitespace/parens/semicolons for word-boundary matching
        let words: Vec<String> = upper
            .split(|c: char| c.is_whitespace() || c == '(' || c == ')' || c == ';')
            .filter(|w| !w.is_empty())
            .map(String::from)
            .collect();

        for word in &words {
            if blocked_keywords.contains(&word.as_str()) && word.as_str() != "SELECT" {
                return Err(anyhow::anyhow!(
                    "Blocked keyword '{}' detected. Only read-only SELECT queries are allowed.",
                    word
                ));
            }
        }

        // Block PRAGMA that could write (allow PRAGMA table_info etc. as read-only)
        if upper.contains("PRAGMA") && !upper.contains("PRAGMA TABLE_INFO")
            && !upper.contains("PRAGMA TABLE_LIST")
            && !upper.contains("PRAGMA DATABASE_LIST")
        {
            return Err(anyhow::anyhow!(
                "PRAGMA statements are restricted. Only PRAGMA table_info/table_list are allowed."
            ));
        }

        // Block semicolons that could chain statements (allow trailing semicolons)
        let without_trailing = trimmed.trim_end_matches(';').trim();
        if without_trailing.contains(';') {
            return Err(anyhow::anyhow!(
                "Multiple SQL statements (semicolons) are not allowed."
            ));
        }

        Ok(())
    }

    /// Execute a validated SELECT query and return results as JSON rows.
    fn run_query(db_path: &str, sql: &str) -> Result<ToolResult> {
        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        let mut stmt = conn.prepare(sql)?;
        let column_count = stmt.column_count();
        let column_names: Vec<String> = (0..column_count)
            .map(|i| stmt.column_name(i).unwrap_or("?").to_string())
            .collect();

        let mut rows_out: Vec<Value> = Vec::new();
        let mut rows = stmt.query([])?;

        while let Some(row) = rows.next()? {
            if rows_out.len() >= MAX_ROWS {
                break;
            }
            let mut obj = serde_json::Map::new();
            for (i, col_name) in column_names.iter().enumerate() {
                let val: Value = match row.get_ref(i)? {
                    rusqlite::types::ValueRef::Null => Value::Null,
                    rusqlite::types::ValueRef::Integer(n) => json!(n),
                    rusqlite::types::ValueRef::Real(f) => json!(f),
                    rusqlite::types::ValueRef::Text(s) => {
                        let text = std::str::from_utf8(s).unwrap_or("");
                        json!(text)
                    }
                    rusqlite::types::ValueRef::Blob(b) => {
                        json!(format!("<blob {} bytes>", b.len()))
                    }
                };
                obj.insert(col_name.clone(), val);
            }
            rows_out.push(Value::Object(obj));
        }

        let truncated = rows_out.len() >= MAX_ROWS;
        let result = json!({
            "columns": column_names,
            "rows": rows_out,
            "row_count": rows_out.len(),
            "truncated": truncated,
        });

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result)?,
        })
    }

    /// List all tables and their columns in a database (schema action).
    fn run_schema(db_path: &str) -> Result<ToolResult> {
        let conn = Connection::open_with_flags(
            db_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;

        // Get all table names
        let mut stmt = conn.prepare(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        )?;
        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();

        let mut tables = Vec::new();
        for table_name in &table_names {
            let mut col_stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", table_name))?;
            let columns: Vec<Value> = col_stmt
                .query_map([], |row| {
                    let name: String = row.get(1)?;
                    let col_type: String = row.get(2)?;
                    let notnull: bool = row.get(3)?;
                    let pk: bool = row.get(5)?;
                    Ok(json!({
                        "name": name,
                        "type": col_type,
                        "not_null": notnull,
                        "primary_key": pk,
                    }))
                })?
                .filter_map(|r| r.ok())
                .collect();

            // Get row count
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM \"{}\"", table_name),
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0);

            tables.push(json!({
                "table": table_name,
                "columns": columns,
                "row_count": count,
            }));
        }

        let result = json!({
            "database": db_path,
            "tables": tables,
            "table_count": tables.len(),
        });

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result)?,
        })
    }

    /// Pre-built aggregate queries for revenue/cost rollups.
    fn run_aggregate(db: &str, metric: &str) -> Result<ToolResult> {
        let db_path = Self::resolve_db_path(db)?;

        let sql = match (db, metric) {
            // --- Cost aggregates ---
            ("costs", "daily") => {
                "SELECT date_key, SUM(total_tokens) as total_tokens, \
                 SUM(estimated_cost_usd) as total_cost, COUNT(*) as call_count \
                 FROM cost_records GROUP BY date_key ORDER BY date_key DESC LIMIT 30"
            }
            ("costs", "by_agent") => {
                "SELECT agent, SUM(total_tokens) as total_tokens, \
                 SUM(estimated_cost_usd) as total_cost, COUNT(*) as call_count \
                 FROM cost_records GROUP BY agent ORDER BY total_cost DESC"
            }
            ("costs", "by_provider") => {
                "SELECT provider, SUM(total_tokens) as total_tokens, \
                 SUM(estimated_cost_usd) as total_cost, COUNT(*) as call_count \
                 FROM cost_records GROUP BY provider ORDER BY total_cost DESC"
            }
            ("costs", "by_model") => {
                "SELECT model, SUM(total_tokens) as total_tokens, \
                 SUM(estimated_cost_usd) as total_cost, COUNT(*) as call_count \
                 FROM cost_records GROUP BY model ORDER BY total_cost DESC"
            }
            ("costs", "summary") => {
                "SELECT COUNT(*) as total_calls, SUM(total_tokens) as total_tokens, \
                 SUM(estimated_cost_usd) as total_cost, \
                 AVG(estimated_cost_usd) as avg_cost, \
                 MAX(estimated_cost_usd) as max_cost, \
                 MIN(date_key) as first_date, MAX(date_key) as last_date \
                 FROM cost_records"
            }
            // --- Revenue aggregates ---
            ("revenue", "daily") => {
                "SELECT date_key, SUM(amount_usd) as total_revenue, COUNT(*) as txn_count \
                 FROM revenue_records GROUP BY date_key ORDER BY date_key DESC LIMIT 30"
            }
            ("revenue", "by_route") => {
                "SELECT route, SUM(amount_usd) as total_revenue, COUNT(*) as txn_count \
                 FROM revenue_records GROUP BY route ORDER BY total_revenue DESC"
            }
            ("revenue", "by_source") => {
                "SELECT source, SUM(amount_usd) as total_revenue, COUNT(*) as txn_count \
                 FROM revenue_records GROUP BY source ORDER BY total_revenue DESC"
            }
            ("revenue", "by_status") => {
                "SELECT status, SUM(amount_usd) as total_revenue, COUNT(*) as txn_count \
                 FROM revenue_records GROUP BY status ORDER BY total_revenue DESC"
            }
            ("revenue", "summary") => {
                "SELECT COUNT(*) as total_txns, SUM(amount_usd) as total_revenue, \
                 AVG(amount_usd) as avg_revenue, MAX(amount_usd) as max_revenue, \
                 MIN(date_key) as first_date, MAX(date_key) as last_date \
                 FROM revenue_records"
            }
            // --- Profit (revenue - cost) ---
            ("revenue", "profit") | ("costs", "profit") => {
                // This is a special case: we need both DBs. Handle in the caller.
                return Self::run_profit_aggregate();
            }
            _ => {
                let available = match db {
                    "costs" => "daily, by_agent, by_provider, by_model, summary, profit",
                    "revenue" => "daily, by_route, by_source, by_status, summary, profit",
                    _ => "summary",
                };
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Unknown metric '{}' for db '{}'. Available: {}",
                        metric, db, available
                    ),
                });
            }
        };

        Self::run_query(&db_path, sql)
    }

    /// Cross-database profit calculation (revenue - costs).
    fn run_profit_aggregate() -> Result<ToolResult> {
        let revenue_path = Self::resolve_db_path("revenue")?;
        let costs_path = Self::resolve_db_path("costs")?;

        let mut revenue_total = 0.0f64;
        let mut cost_total = 0.0f64;

        // Revenue total
        if let Ok(conn) = Connection::open_with_flags(
            &revenue_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            revenue_total = conn
                .query_row(
                    "SELECT COALESCE(SUM(amount_usd), 0.0) FROM revenue_records",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0.0);
        }

        // Cost total
        if let Ok(conn) = Connection::open_with_flags(
            &costs_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        ) {
            cost_total = conn
                .query_row(
                    "SELECT COALESCE(SUM(estimated_cost_usd), 0.0) FROM cost_records",
                    [],
                    |row| row.get(0),
                )
                .unwrap_or(0.0);
        }

        let profit = revenue_total - cost_total;
        let result = json!({
            "revenue_total_usd": revenue_total,
            "cost_total_usd": cost_total,
            "profit_usd": profit,
            "margin_pct": if revenue_total > 0.0 { (profit / revenue_total) * 100.0 } else { 0.0 },
        });

        Ok(ToolResult {
            success: true,
            output: serde_json::to_string_pretty(&result)?,
        })
    }
}

#[async_trait]
impl Tool for DbQueryTool {
    fn name(&self) -> &str {
        "db_query"
    }

    fn description(&self) -> &str {
        "Read-only SQLite query tool for internal analytics. \
         Actions: 'query' (run SELECT), 'schema' (list tables/columns), \
         'aggregate' (pre-built revenue/cost rollups). \
         Databases: costs, revenue, core, memory."
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        match action {
            "query" => {
                // Validate db name
                let db = args.get("db").and_then(|v| v.as_str()).unwrap_or("");
                if db.is_empty() {
                    return Err(anyhow::anyhow!("Preflight: missing 'db' parameter"));
                }
                Self::resolve_db_path(db)?;

                // Validate SQL
                let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                Self::validate_sql(sql)?;
            }
            "schema" => {
                let db = args.get("db").and_then(|v| v.as_str()).unwrap_or("");
                if db.is_empty() {
                    return Err(anyhow::anyhow!("Preflight: missing 'db' parameter"));
                }
                Self::resolve_db_path(db)?;
            }
            "aggregate" => {
                let db = args.get("db").and_then(|v| v.as_str()).unwrap_or("");
                if db.is_empty() {
                    return Err(anyhow::anyhow!("Preflight: missing 'db' parameter"));
                }
                // aggregate 'profit' works on both costs and revenue
                let metric = args.get("metric").and_then(|v| v.as_str()).unwrap_or("");
                if metric != "profit" {
                    Self::resolve_db_path(db)?;
                }
                if metric.is_empty() {
                    return Err(anyhow::anyhow!("Preflight: missing 'metric' parameter"));
                }
            }
            "" => {
                return Err(anyhow::anyhow!(
                    "Preflight: missing 'action' parameter. Use 'query', 'schema', or 'aggregate'."
                ));
            }
            other => {
                return Err(anyhow::anyhow!(
                    "Preflight: unknown action '{}'. Use 'query', 'schema', or 'aggregate'.",
                    other
                ));
            }
        }

        Ok(())
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["query", "schema", "aggregate"],
                    "description": "Action to perform: 'query' runs a SELECT, 'schema' lists tables/columns, 'aggregate' runs pre-built rollups."
                },
                "db": {
                    "type": "string",
                    "enum": ["costs", "revenue", "core", "memory"],
                    "description": "Which database to query."
                },
                "sql": {
                    "type": "string",
                    "description": "The SELECT query to run (only for action='query')."
                },
                "metric": {
                    "type": "string",
                    "description": "Pre-built aggregate metric (only for action='aggregate'). Costs: daily, by_agent, by_provider, by_model, summary, profit. Revenue: daily, by_route, by_source, by_status, summary, profit."
                }
            },
            "required": ["action", "db"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let db = args.get("db").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "query" => {
                let sql = args.get("sql").and_then(|v| v.as_str()).unwrap_or("");
                if sql.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Error: missing 'sql' parameter".to_string(),
                    });
                }

                // Re-validate in execute (defense in depth)
                if let Err(e) = Self::validate_sql(sql) {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("SQL validation failed: {}", e),
                    });
                }

                let db_path = match Self::resolve_db_path(db) {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(ToolResult {
                            success: false,
                            output: format!("Error: {}", e),
                        });
                    }
                };

                info!("db_query: [{}] {}", db, sql);

                match Self::run_query(&db_path, sql) {
                    Ok(r) => Ok(r),
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Query error: {}", e),
                    }),
                }
            }
            "schema" => {
                let db_path = match Self::resolve_db_path(db) {
                    Ok(p) => p,
                    Err(e) => {
                        return Ok(ToolResult {
                            success: false,
                            output: format!("Error: {}", e),
                        });
                    }
                };

                info!("db_query: schema for [{}]", db);

                match Self::run_schema(&db_path) {
                    Ok(r) => Ok(r),
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Schema error: {}", e),
                    }),
                }
            }
            "aggregate" => {
                let metric = args
                    .get("metric")
                    .and_then(|v| v.as_str())
                    .unwrap_or("summary");

                info!("db_query: aggregate [{}] metric={}", db, metric);

                match Self::run_aggregate(db, metric) {
                    Ok(r) => Ok(r),
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Aggregate error: {}", e),
                    }),
                }
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!(
                    "Unknown action '{}'. Use 'query', 'schema', or 'aggregate'.",
                    action
                ),
            }),
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Helper: create a temp DB with cost_records and revenue_records tables.
    fn create_test_db(suffix: &str) -> String {
        let dir = std::env::temp_dir().join(format!("phantom_mesh_test_dbq_{}", suffix));
        let _ = fs::create_dir_all(&dir);
        let db_path = dir.join("test.db");
        let path_str = db_path.to_string_lossy().to_string();

        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cost_records (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                agent TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                tokens_in INTEGER NOT NULL DEFAULT 0,
                tokens_out INTEGER NOT NULL DEFAULT 0,
                total_tokens INTEGER NOT NULL DEFAULT 0,
                estimated_cost_usd REAL NOT NULL DEFAULT 0.0,
                duration_secs REAL NOT NULL DEFAULT 0.0,
                context TEXT,
                date_key TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS revenue_records (
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
            );",
        )
        .unwrap();

        // Insert sample cost data
        conn.execute_batch(
            "INSERT INTO cost_records (id, timestamp, agent, provider, model, tokens_in, tokens_out, total_tokens, estimated_cost_usd, duration_secs, date_key)
             VALUES ('c1', '2026-03-01T00:00:00Z', 'agent1', 'gemini', 'gemini-2.0-flash', 100, 50, 150, 0.001, 1.2, '2026-03-01'),
                    ('c2', '2026-03-01T01:00:00Z', 'agent2', 'groq', 'llama-3.3-70b', 200, 100, 300, 0.002, 0.8, '2026-03-01'),
                    ('c3', '2026-03-02T00:00:00Z', 'agent1', 'gemini', 'gemini-2.0-flash', 300, 150, 450, 0.003, 1.5, '2026-03-02');",
        )
        .unwrap();

        // Insert sample revenue data
        conn.execute_batch(
            "INSERT INTO revenue_records (id, timestamp, route, source, client_name, amount_usd, status, date_key)
             VALUES ('r1', '2026-03-01T00:00:00Z', 'A:freelance_dev', 'upwork', 'ClientA', 150.0, 'confirmed', '2026-03-01'),
                    ('r2', '2026-03-02T00:00:00Z', 'B:saas_products', 'stripe', 'ClientB', 29.99, 'paid', '2026-03-02');",
        )
        .unwrap();

        path_str
    }

    fn cleanup_test_db(suffix: &str) {
        let dir = std::env::temp_dir().join(format!("phantom_mesh_test_dbq_{}", suffix));
        let _ = fs::remove_dir_all(&dir);
    }

    // ── SQL Validation Tests ──────────────────────────────────────────────

    #[test]
    fn test_validate_sql_select_allowed() {
        assert!(DbQueryTool::validate_sql("SELECT * FROM cost_records").is_ok());
        assert!(DbQueryTool::validate_sql("select count(*) from revenue_records").is_ok());
        assert!(DbQueryTool::validate_sql("  SELECT id, agent FROM cost_records WHERE agent = 'x'  ").is_ok());
    }

    #[test]
    fn test_validate_sql_blocks_insert() {
        let err = DbQueryTool::validate_sql("INSERT INTO cost_records VALUES ('x')").unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_validate_sql_blocks_update() {
        let err = DbQueryTool::validate_sql("UPDATE cost_records SET agent='hacked'").unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_validate_sql_blocks_delete() {
        let err = DbQueryTool::validate_sql("DELETE FROM cost_records").unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_validate_sql_blocks_drop() {
        let err = DbQueryTool::validate_sql("DROP TABLE cost_records").unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_validate_sql_blocks_embedded_delete() {
        // Even if it starts with SELECT, embedded write keywords are blocked
        let err = DbQueryTool::validate_sql(
            "SELECT * FROM cost_records; DELETE FROM cost_records"
        ).unwrap_err();
        assert!(err.to_string().contains("Blocked keyword") || err.to_string().contains("DELETE"));
    }

    #[test]
    fn test_validate_sql_blocks_attach() {
        let err = DbQueryTool::validate_sql("ATTACH DATABASE '/tmp/evil.db' AS evil").unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_validate_sql_empty() {
        let err = DbQueryTool::validate_sql("").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn test_validate_sql_allows_trailing_semicolon() {
        assert!(DbQueryTool::validate_sql("SELECT 1;").is_ok());
    }

    // ── Database Allowlist Tests ──────────────────────────────────────────

    #[test]
    fn test_resolve_db_path_known() {
        let path = DbQueryTool::resolve_db_path("costs").unwrap();
        assert!(path.contains("costs.db"));
        assert!(path.contains(".phantom-mesh"));
    }

    #[test]
    fn test_resolve_db_path_unknown() {
        let err = DbQueryTool::resolve_db_path("malicious").unwrap_err();
        assert!(err.to_string().contains("Unknown database"));
        assert!(err.to_string().contains("Allowed"));
    }

    // ── Preflight Tests ──────────────────────────────────────────────────

    #[test]
    fn test_preflight_missing_action() {
        let tool = DbQueryTool::new();
        let err = tool.preflight(&json!({"db": "costs"})).unwrap_err();
        assert!(err.to_string().contains("missing 'action'"));
    }

    #[test]
    fn test_preflight_unknown_action() {
        let tool = DbQueryTool::new();
        let err = tool.preflight(&json!({"action": "hack", "db": "costs"})).unwrap_err();
        assert!(err.to_string().contains("unknown action"));
    }

    #[test]
    fn test_preflight_query_missing_db() {
        let tool = DbQueryTool::new();
        let err = tool.preflight(&json!({"action": "query", "sql": "SELECT 1"})).unwrap_err();
        assert!(err.to_string().contains("missing 'db'"));
    }

    #[test]
    fn test_preflight_query_bad_sql() {
        let tool = DbQueryTool::new();
        let err = tool.preflight(&json!({
            "action": "query",
            "db": "costs",
            "sql": "DROP TABLE cost_records"
        })).unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_preflight_query_valid() {
        let tool = DbQueryTool::new();
        assert!(tool.preflight(&json!({
            "action": "query",
            "db": "costs",
            "sql": "SELECT * FROM cost_records"
        })).is_ok());
    }

    #[test]
    fn test_preflight_schema_valid() {
        let tool = DbQueryTool::new();
        assert!(tool.preflight(&json!({"action": "schema", "db": "revenue"})).is_ok());
    }

    #[test]
    fn test_preflight_aggregate_missing_metric() {
        let tool = DbQueryTool::new();
        let err = tool.preflight(&json!({"action": "aggregate", "db": "costs"})).unwrap_err();
        assert!(err.to_string().contains("missing 'metric'"));
    }

    // ── Execute: Query Action Tests ──────────────────────────────────────

    #[tokio::test]
    async fn test_execute_query_select() {
        let db_path = create_test_db("query_select");
        let result = DbQueryTool::run_query(&db_path, "SELECT agent, total_tokens FROM cost_records ORDER BY id").unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["row_count"].as_u64().unwrap(), 3);
        assert_eq!(parsed["columns"][0].as_str().unwrap(), "agent");
        assert_eq!(parsed["rows"][0]["agent"].as_str().unwrap(), "agent1");
        assert_eq!(parsed["rows"][0]["total_tokens"].as_i64().unwrap(), 150);
        cleanup_test_db("query_select");
    }

    #[tokio::test]
    async fn test_execute_query_aggregate_sql() {
        let db_path = create_test_db("query_agg_sql");
        let result = DbQueryTool::run_query(
            &db_path,
            "SELECT SUM(estimated_cost_usd) as total_cost FROM cost_records",
        ).unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        let total = parsed["rows"][0]["total_cost"].as_f64().unwrap();
        assert!((total - 0.006).abs() < 1e-9);
        cleanup_test_db("query_agg_sql");
    }

    #[tokio::test]
    async fn test_execute_query_where_clause() {
        let db_path = create_test_db("query_where");
        let result = DbQueryTool::run_query(
            &db_path,
            "SELECT id FROM cost_records WHERE agent = 'agent2'",
        ).unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["row_count"].as_u64().unwrap(), 1);
        assert_eq!(parsed["rows"][0]["id"].as_str().unwrap(), "c2");
        cleanup_test_db("query_where");
    }

    // ── Execute: Schema Action Tests ─────────────────────────────────────

    #[tokio::test]
    async fn test_execute_schema() {
        let db_path = create_test_db("schema");
        let result = DbQueryTool::run_schema(&db_path).unwrap();
        assert!(result.success);
        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["table_count"].as_u64().unwrap(), 2);
        // Check table names
        let tables: Vec<&str> = parsed["tables"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["table"].as_str().unwrap())
            .collect();
        assert!(tables.contains(&"cost_records"));
        assert!(tables.contains(&"revenue_records"));
        // Check columns exist
        let cost_table = parsed["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["table"].as_str().unwrap() == "cost_records")
            .unwrap();
        let col_names: Vec<&str> = cost_table["columns"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["name"].as_str().unwrap())
            .collect();
        assert!(col_names.contains(&"id"));
        assert!(col_names.contains(&"agent"));
        assert!(col_names.contains(&"estimated_cost_usd"));
        // Check row count
        assert_eq!(cost_table["row_count"].as_i64().unwrap(), 3);
        cleanup_test_db("schema");
    }

    // ── Execute: Full Tool Integration Tests ─────────────────────────────

    #[tokio::test]
    async fn test_execute_unknown_action() {
        let tool = DbQueryTool::new();
        let result = tool
            .execute(json!({"action": "hack", "db": "costs"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_execute_missing_sql() {
        let tool = DbQueryTool::new();
        let result = tool
            .execute(json!({"action": "query", "db": "costs"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("missing 'sql'"));
    }

    #[tokio::test]
    async fn test_execute_blocked_sql_at_execute_level() {
        let tool = DbQueryTool::new();
        let result = tool
            .execute(json!({
                "action": "query",
                "db": "costs",
                "sql": "INSERT INTO cost_records VALUES ('evil')"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("validation failed"));
    }

    // ── Name / Description / Schema Tests ────────────────────────────────

    #[test]
    fn test_tool_metadata() {
        let tool = DbQueryTool::new();
        assert_eq!(tool.name(), "db_query");
        assert!(tool.description().contains("read-only") || tool.description().contains("Read-only"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["action"].is_object());
        assert!(schema["properties"]["db"].is_object());
        assert!(schema["properties"]["sql"].is_object());
        assert!(schema["properties"]["metric"].is_object());
    }

    // ── SQL Injection / Edge Cases ───────────────────────────────────────

    #[test]
    fn test_validate_sql_select_with_subquery_containing_delete() {
        // A SELECT that embeds DELETE in a subquery-like string
        let err = DbQueryTool::validate_sql(
            "SELECT * FROM cost_records WHERE id IN (DELETE FROM cost_records)"
        ).unwrap_err();
        assert!(err.to_string().contains("Blocked keyword"));
    }

    #[test]
    fn test_validate_sql_create_blocked() {
        let err = DbQueryTool::validate_sql("CREATE TABLE evil (id TEXT)").unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[test]
    fn test_validate_sql_replace_blocked() {
        let err = DbQueryTool::validate_sql("REPLACE INTO cost_records VALUES ('x')").unwrap_err();
        assert!(err.to_string().contains("Only SELECT"));
    }

    #[tokio::test]
    async fn test_run_query_nonexistent_table() {
        let db_path = create_test_db("bad_table");
        let result = DbQueryTool::run_query(&db_path, "SELECT * FROM nonexistent_table");
        assert!(result.is_err() || !result.unwrap().success);
        cleanup_test_db("bad_table");
    }

    // ── Aggregate Metric Error ───────────────────────────────────────────

    #[test]
    fn test_aggregate_unknown_metric() {
        let db_path = create_test_db("agg_unknown");
        // We call run_aggregate directly — it returns ToolResult with success=false
        // But run_aggregate uses resolve_db_path which points to ~/.phantom-mesh/,
        // so we test the error path via the metric name:
        let result = DbQueryTool::run_aggregate("costs", "nonexistent").unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown metric"));
        cleanup_test_db("agg_unknown");
    }
}
