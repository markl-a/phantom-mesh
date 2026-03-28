//! Schema migration runner for peer-to-peer data columns.
//!
//! Each migration adds `node_id` / `updated_at` columns needed for
//! cross-node sync.  Migrations are idempotent and tracked in a
//! `_migrations` table per database file.

use anyhow::Result;
use rusqlite::Connection;
use tracing::info;

/// Run all pending migrations on a SQLite connection.
pub struct MigrationRunner;

/// Describes a single migration step.
struct Migration {
    id: &'static str,
    apply: fn(&Connection) -> Result<()>,
}

/// All known migrations in order.
fn all_migrations() -> Vec<Migration> {
    vec![
        Migration {
            id: "v001_add_node_id_to_sessions",
            apply: |conn| {
                add_column_if_not_exists(conn, "sessions", "node_id", "TEXT", "''")?;
                Ok(())
            },
        },
        Migration {
            id: "v002_add_node_id_to_tasks",
            apply: |conn| {
                add_column_if_not_exists(conn, "tasks", "node_id", "TEXT", "''")?;
                add_column_if_not_exists(conn, "tasks", "assigned_node_id", "TEXT", "''")?;
                Ok(())
            },
        },
        Migration {
            id: "v003_add_node_id_to_cost_records",
            apply: |conn| {
                // cost_records may already have `node_id` — safe to call anyway.
                add_column_if_not_exists(conn, "cost_records", "node_id", "TEXT", "NULL")?;
                Ok(())
            },
        },
        Migration {
            id: "v004_add_updated_at_to_sessions",
            apply: |conn| {
                add_column_if_not_exists(conn, "sessions", "updated_at", "INTEGER", "0")?;
                Ok(())
            },
        },
        Migration {
            id: "v005_add_updated_at_to_tasks",
            apply: |conn| {
                add_column_if_not_exists(conn, "tasks", "updated_at", "INTEGER", "0")?;
                Ok(())
            },
        },
    ]
}

impl MigrationRunner {
    /// Run all pending migrations on the given connection.
    ///
    /// Creates a `_migrations` tracking table if it doesn't exist, then
    /// applies each migration that hasn't already been recorded.
    ///
    /// Migrations that reference tables not present in this database are
    /// silently skipped (e.g. running task migrations against
    /// `conversations.db` is a no-op because the `ALTER TABLE tasks …`
    /// will fail with "no such table" and we treat that as non-applicable).
    pub fn run(conn: &Connection) -> Result<()> {
        // Ensure tracking table exists.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS _migrations (
                id         TEXT PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );"
        )?;

        for m in all_migrations() {
            if Self::already_applied(conn, m.id)? {
                continue;
            }

            match (m.apply)(conn) {
                Ok(()) => {
                    Self::record(conn, m.id)?;
                    info!("Migration {} applied", m.id);
                }
                Err(e) => {
                    let msg = e.to_string();
                    // "no such table" means the migration doesn't apply to
                    // this database file — skip silently.
                    if msg.contains("no such table") {
                        info!("Migration {} skipped (table not in this db)", m.id);
                        // Record it so we don't retry every startup.
                        Self::record(conn, m.id)?;
                    } else {
                        return Err(e);
                    }
                }
            }
        }

        Ok(())
    }

    fn already_applied(conn: &Connection, id: &str) -> Result<bool> {
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM _migrations WHERE id = ?1",
            [id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    fn record(conn: &Connection, id: &str) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        conn.execute(
            "INSERT OR IGNORE INTO _migrations (id, applied_at) VALUES (?1, ?2)",
            rusqlite::params![id, now],
        )?;
        Ok(())
    }
}

/// Attempt to add a column; silently ignore "duplicate column" errors.
fn add_column_if_not_exists(
    conn: &Connection,
    table: &str,
    column: &str,
    col_type: &str,
    default: &str,
) -> Result<()> {
    let sql = format!(
        "ALTER TABLE {} ADD COLUMN {} {} DEFAULT {}",
        table, column, col_type, default
    );
    match conn.execute(&sql, []) {
        Ok(_) => Ok(()),
        Err(e) if e.to_string().contains("duplicate column") => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_conn() -> Connection {
        Connection::open_in_memory().unwrap()
    }

    #[test]
    fn tracking_table_created() {
        let conn = memory_conn();
        MigrationRunner::run(&conn).unwrap();

        // _migrations table should exist
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        // All 5 migrations recorded (even skipped ones)
        assert_eq!(count, 5);
    }

    #[test]
    fn adds_node_id_to_sessions() {
        let conn = memory_conn();
        // Create the sessions table as ConversationStore does
        conn.execute_batch(
            "CREATE TABLE sessions (
                chat_id    TEXT PRIMARY KEY,
                messages   TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );"
        ).unwrap();

        MigrationRunner::run(&conn).unwrap();

        // node_id column should exist
        conn.execute(
            "INSERT INTO sessions (chat_id, messages, node_id) VALUES ('c1', '[]', 'node-a')",
            [],
        ).unwrap();

        let nid: String = conn
            .query_row("SELECT node_id FROM sessions WHERE chat_id = 'c1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(nid, "node-a");
    }

    #[test]
    fn adds_columns_to_tasks() {
        let conn = memory_conn();
        conn.execute_batch(
            "CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY,
                title   TEXT NOT NULL,
                prompt  TEXT NOT NULL,
                status  TEXT NOT NULL DEFAULT 'pending',
                result  TEXT,
                strategy_used TEXT,
                feedback_score REAL,
                priority INTEGER NOT NULL DEFAULT 2,
                idempotency_key TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );"
        ).unwrap();

        MigrationRunner::run(&conn).unwrap();

        // Insert with new columns
        conn.execute(
            "INSERT INTO tasks (task_id, title, prompt, created_at, updated_at, node_id, assigned_node_id)
             VALUES ('t1', 'test', 'do it', '2025-01-01', '2025-01-01', 'node-a', 'node-b')",
            [],
        ).unwrap();

        let (nid, anid): (String, String) = conn
            .query_row(
                "SELECT node_id, assigned_node_id FROM tasks WHERE task_id = 't1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(nid, "node-a");
        assert_eq!(anid, "node-b");
    }

    #[test]
    fn idempotent_run() {
        let conn = memory_conn();
        conn.execute_batch(
            "CREATE TABLE sessions (
                chat_id    TEXT PRIMARY KEY,
                messages   TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );"
        ).unwrap();

        MigrationRunner::run(&conn).unwrap();
        // Second run should not error
        MigrationRunner::run(&conn).unwrap();

        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 5);
    }
}
