//! Integration test for the partner's Todoist goal model + DO-actions.
//!
//! Hermetic: stands up a wiremock server that speaks the Todoist API v1 shape,
//! points the client at it via `TODOIST_API_BASE`, and asserts the full path —
//! `list_tasks` + `list_projects` → `format_goal_model` → a block containing the
//! user's REAL goal text (not the old `<unknown>` placeholder) — plus the
//! `add_task` DO-action. Makes zero real network calls.
//!
//! `TODOIST_API_BASE` / `TODOIST_API_TOKEN` are process-global env vars, so this
//! test runs single-threaded relative to itself; we serialize the two test fns
//! behind one async lock and restore the env afterwards.

use spectyn_mesh::todoist;
use serde_json::json;
use std::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[tokio::test]
async fn goal_model_contains_real_todoist_tasks_not_unknown() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let server = MockServer::start().await;

    // Realistic Todoist API v1 task + project payloads (subset of fields),
    // wrapped in the v1 `{"results": [...], "next_cursor": ...}` envelope.
    // Tasks use v1's `checked` key to exercise the is_completed alias end-to-end.
    let tasks = json!({
        "results": [
            {
                "id": "1001",
                "content": "接 Todoist 目標模型",
                "priority": 4,
                "project_id": "p1",
                "due": {"string": "今天", "date": "2026-06-05"},
                "checked": false
            },
            {
                "id": "1002",
                "content": "買牛奶",
                "priority": 1,
                "project_id": "p2",
                "checked": false
            },
            {
                "id": "1003",
                "content": "ignore me — completed",
                "priority": 1,
                "checked": true
            }
        ],
        "next_cursor": null
    });
    let projects = json!({
        "results": [
            {"id": "p1", "name": "🤝 spectyn-mesh"},
            {"id": "p2", "name": "暫存"}
        ],
        "next_cursor": null
    });

    Mock::given(method("GET"))
        .and(path("/tasks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tasks))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/projects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(projects))
        .mount(&server)
        .await;

    std::env::set_var("TODOIST_API_BASE", server.uri());
    std::env::set_var("TODOIST_API_TOKEN", "test-token");

    let fetched_tasks = todoist::list_tasks("test-token", None)
        .await
        .expect("list_tasks against mock should succeed");
    let fetched_projects = todoist::list_projects("test-token")
        .await
        .expect("list_projects against mock should succeed");

    let block = todoist::format_goal_model(&fetched_tasks, &fetched_projects, 12)
        .expect("two open tasks → a non-empty goal model");

    // The whole point of the step: real goals, NOT the old placeholder.
    assert!(
        !block.contains("<unknown>") && !block.to_lowercase().contains("not yet integrated"),
        "goal model must not be the placeholder:\n{block}"
    );
    // Real task content shows up.
    assert!(block.contains("接 Todoist 目標模型"), "p4 task present:\n{block}");
    assert!(block.contains("買牛奶"), "second task present:\n{block}");
    // Completed task is excluded.
    assert!(!block.contains("ignore me"), "completed task excluded:\n{block}");
    // p4 task is listed before the p1 task.
    let hi = block.find("接 Todoist").unwrap();
    let lo = block.find("買牛奶").unwrap();
    assert!(hi < lo, "higher-priority goal listed first:\n{block}");
    // Project label + due string surfaced.
    assert!(block.contains("🤝 spectyn-mesh"), "project label present:\n{block}");
    assert!(block.contains("今天") || block.contains("2026-06-05"), "due surfaced:\n{block}");

    std::env::remove_var("TODOIST_API_BASE");
    std::env::remove_var("TODOIST_API_TOKEN");
}

#[tokio::test]
async fn add_task_do_action_posts_to_todoist() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let server = MockServer::start().await;

    // Todoist echoes back the created task. API v1 POST /tasks returns the task
    // object directly (no envelope); it uses v1's `checked` key.
    let created = json!({
        "id": "2001",
        "content": "記得回信給房東",
        "priority": 3,
        "due": {"string": "tomorrow", "date": "2026-06-06"},
        "checked": false
    });
    Mock::given(method("POST"))
        .and(path("/tasks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(created))
        .expect(1) // assert the DO-action actually hits the API exactly once
        .mount(&server)
        .await;

    std::env::set_var("TODOIST_API_BASE", server.uri());

    let task = todoist::add_task("test-token", "記得回信給房東", Some("tomorrow"), Some(3), None)
        .await
        .expect("add_task against mock should succeed");
    assert_eq!(task.id, "2001");
    assert_eq!(task.content, "記得回信給房東");

    std::env::remove_var("TODOIST_API_BASE");
    // server.expect(1) is verified on drop.
}

#[tokio::test]
async fn complete_task_do_action_closes_the_task() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let server = MockServer::start().await;

    // Todoist API v1 `POST /tasks/{id}/close` returns 204 No Content on success.
    // Asserting on the exact path proves the DO-action targets the *close*
    // endpoint for the right task id, not some other mutation.
    Mock::given(method("POST"))
        .and(path("/tasks/2001/close"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1) // the action must hit the close endpoint exactly once
        .mount(&server)
        .await;

    std::env::set_var("TODOIST_API_BASE", server.uri());

    todoist::complete_task("test-token", "2001")
        .await
        .expect("complete_task against mock should succeed (204)");

    std::env::remove_var("TODOIST_API_BASE");
    // server.expect(1) is verified on drop: the request hit /tasks/2001/close.
}

#[tokio::test]
async fn complete_task_surfaces_api_error() {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let server = MockServer::start().await;

    // A non-2xx (e.g. unknown task id) must surface as an Err with the status,
    // so the partner can report the failure instead of silently "succeeding".
    Mock::given(method("POST"))
        .and(path("/tasks/9999/close"))
        .respond_with(ResponseTemplate::new(404).set_body_string("Task not found"))
        .mount(&server)
        .await;

    std::env::set_var("TODOIST_API_BASE", server.uri());

    let err = todoist::complete_task("test-token", "9999")
        .await
        .expect_err("404 must surface as an error");
    assert!(
        err.to_string().contains("404"),
        "error carries the HTTP status: {err}"
    );

    std::env::remove_var("TODOIST_API_BASE");
}
