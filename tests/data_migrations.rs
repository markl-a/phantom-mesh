//! Integration tests for the data layer: migrations + sync index.

use phantom_mesh::data::migrations::MigrationRunner;
use phantom_mesh::data::sync::{ConversationSummary, SyncIndex, TaskSummary};
use rusqlite::Connection;

// ── Migration tests ─────────────────────────────────────────────────────────────

#[test]
fn test_migrations_create_tracking_table() {
    let conn = Connection::open_in_memory().unwrap();
    MigrationRunner::run(&conn).unwrap();

    // _migrations table must exist and contain entries
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
        .unwrap();
    assert!(count > 0, "expected at least one migration recorded");
}

#[test]
fn test_migrations_add_node_id() {
    let conn = Connection::open_in_memory().unwrap();

    // Mimic ConversationStore's table creation
    conn.execute_batch(
        "CREATE TABLE sessions (
            chat_id    TEXT PRIMARY KEY,
            messages   TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )
    .unwrap();

    MigrationRunner::run(&conn).unwrap();

    // node_id column should be usable
    conn.execute(
        "INSERT INTO sessions (chat_id, messages, node_id) VALUES ('c1', '[]', 'node-abc')",
        [],
    )
    .unwrap();

    let nid: String = conn
        .query_row(
            "SELECT node_id FROM sessions WHERE chat_id = 'c1'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(nid, "node-abc");
}

#[test]
fn test_migrations_idempotent() {
    let conn = Connection::open_in_memory().unwrap();

    conn.execute_batch(
        "CREATE TABLE sessions (
            chat_id    TEXT PRIMARY KEY,
            messages   TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );
        CREATE TABLE tasks (
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
        );",
    )
    .unwrap();

    // Run twice — must not error
    MigrationRunner::run(&conn).unwrap();
    MigrationRunner::run(&conn).unwrap();

    // Verify migration count stayed the same
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM _migrations", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 5);
}

// ── SyncIndex tests ─────────────────────────────────────────────────────────────

fn conv(chat_id: &str, node_id: &str, ts: u64) -> ConversationSummary {
    ConversationSummary {
        chat_id: chat_id.to_string(),
        node_id: node_id.to_string(),
        last_message_at: ts,
        message_count: 1,
        title: Some(format!("conv-{}", chat_id)),
    }
}

fn task(task_id: &str, node_id: &str, ts: u64) -> TaskSummary {
    TaskSummary {
        task_id: task_id.to_string(),
        node_id: node_id.to_string(),
        title: format!("task-{}", task_id),
        status: "pending".to_string(),
        updated_at: ts,
    }
}

#[test]
fn test_sync_index_update_and_find() {
    let idx = SyncIndex::new();
    idx.update_from_node("node-a", vec![conv("c1", "node-a", 100)], vec![]);
    idx.update_from_node("node-b", vec![conv("c2", "node-b", 200)], vec![]);

    let found = idx.find_conversation("c1").expect("c1 should exist");
    assert_eq!(found.node_id, "node-a");

    let found = idx.find_conversation("c2").expect("c2 should exist");
    assert_eq!(found.node_id, "node-b");

    // Non-existent chat
    assert!(idx.find_conversation("c999").is_none());
}

#[test]
fn test_sync_index_all_conversations() {
    let idx = SyncIndex::new();
    idx.update_from_node(
        "node-a",
        vec![conv("c1", "node-a", 10), conv("c2", "node-a", 20)],
        vec![task("t1", "node-a", 100)],
    );
    idx.update_from_node("node-b", vec![conv("c3", "node-b", 30)], vec![]);
    idx.update_from_node("node-c", vec![conv("c4", "node-c", 40)], vec![]);

    let all_convs = idx.all_conversations();
    assert_eq!(all_convs.len(), 4);

    let all_tasks = idx.all_tasks();
    assert_eq!(all_tasks.len(), 1);
}
