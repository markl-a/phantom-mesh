//! F400 integration tests — Hermes skill RPC endpoints.
//!
//! Exercises the three new GET routes (`/api/hermes/skills`,
//! `/api/hermes/skills/:id`, `/api/hermes/skill-timeline`) end-to-end
//! through `phantom_mesh::serve::router`, using `tower::ServiceExt::oneshot`
//! exactly like `core/tests/test_security_t7.rs` does.
//!
//! Coverage (≥ 8 tests as required by the spec):
//!   1. list endpoint returns all seeded skills (no pagination)
//!   2. list endpoint paginates correctly (limit + offset)
//!   3. list endpoint supports FTS5 search via `?q=`
//!   4. detail endpoint returns full provenance for a known id
//!   5. detail endpoint returns 404 for a missing id
//!   6. timeline endpoint orders entries chronologically (ASC by created_at)
//!   7. timeline endpoint respects `?since=`
//!   8. auth: missing X-Cluster-Auth on list endpoint → 401
//!   9. auth: missing X-Cluster-Auth on detail endpoint → 401
//!  10. auth: missing X-Cluster-Auth on timeline endpoint → 401
//!  11. service-unavailable when hermes_memory is None
//!  12. perf gate: list 1000 skills < 500ms
//!
//! All tests run under `--features experimental-hermes-memory` per
//! Cargo.toml `[[test]]` requirement below — there is no such entry yet,
//! but `#![cfg(feature = ...)]` makes the file compile-empty without the
//! flag, which is enough for `cargo test --no-default-features` to skip it.

#![cfg(feature = "experimental-hermes-memory")]

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use phantom_mesh::hermes::memory::{HermesMemory, NewMemory};
use phantom_mesh::mesh::{ClusterConfig, ClusterManager};
use phantom_mesh::AppState;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &str = "f400-test-secret";

/// Build an `AppState` with an in-memory hermes DB and a fixed cluster
/// secret. Returns the state plus the bare `HermesMemory` handle so the
/// test can seed rows directly.
async fn make_state_with_memory() -> (Arc<AppState>, HermesMemory) {
    let td = tempfile::tempdir().unwrap();
    let db = td.path().join("f400.db");
    let mem = HermesMemory::open_at(db).expect("open hermes db");

    let mut st = AppState::new();
    let mut cfg = ClusterConfig::default();
    cfg.cluster_secret = Some(SECRET.into());
    st.cluster_manager = ClusterManager::new(cfg);
    st.hermes_memory = Some(mem.clone());

    // Keep tempdir alive for the test duration by leaking it — we never
    // delete the file. Tests are single-process; the tempdir's drop would
    // race with the sqlite handle anyway.
    std::mem::forget(td);

    (Arc::new(st), mem)
}

/// Seed five varied skills covering both polarity flavours and a range of
/// created_at timestamps. Returns the inserted ids in insertion order.
///
/// Layout (oldest → newest):
///   1. rebase-onto-main (success, recipe)       — seed-style text
///   2. handle-rate-limit (failure, lesson)      — frontmatter-style text
///   3. dedupe-prs (success, workflow_pattern)   — seed-style
///   4. unknown-incident (no markers)            — seed-style
///   5. retry-merge (failure, retry_loop)        — seed-style
async fn seed_varied(mem: &HermesMemory) -> Vec<i64> {
    let mut ids = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // sqlite's `insert` uses SystemTime; we want predictable ordering for
    // pagination tests so seed in real time-order with a tiny sleep so the
    // unix-seconds tick.
    for (text, tags) in [
        (
            "rebase-onto-main\nRebase the current branch onto main.\nstep 1: fetch\nstep 2: rebase\n",
            "recipe git rebase",
        ),
        (
            "---\nname: handle-rate-limit\nversion: 0.1.0\ndescription: Back off on 429.\ntriggers:\n  - rate-limit\n---\nBody.\n",
            "lesson http",
        ),
        (
            "dedupe-prs\nDe-duplicate PRs by hash.\nbody\n",
            "workflow_pattern git",
        ),
        (
            "unknown-incident\nAn ambiguous one.\nbody\n",
            "",
        ),
        (
            "retry-merge\nRetry merging on conflict.\nbody\n",
            "retry_loop lesson",
        ),
    ] {
        let id = mem
            .insert(NewMemory {
                kind: "skill",
                source: "hermes_skills",
                text,
                tags,
            })
            .await
            .unwrap();
        ids.push(id);
        // sub-second granularity is fine for ordering by id; created_at
        // ties are broken by id in `list_by_kind`.
        let _ = now;
    }
    ids
}

/// Helper: HMAC-token an empty-body request, send through router, collect
/// JSON.
async fn get_signed_json(app: axum::Router, state: &AppState, uri: &str) -> (StatusCode, Value) {
    let token = state.cluster_manager.make_auth_token("");
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header("X-Cluster-Auth", token)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

// ── 1. list endpoint — no pagination ─────────────────────────────────────

#[tokio::test]
async fn list_endpoint_returns_all_seeded_skills() {
    let (state, mem) = make_state_with_memory().await;
    seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/hermes/skills").await;
    assert_eq!(status, StatusCode::OK, "got body={body}");

    let items = body["items"].as_array().expect("items array");
    assert_eq!(items.len(), 5, "expected all 5 skills, got {}", items.len());
    assert_eq!(body["total"], 5);
    assert_eq!(body["offset"], 0);

    // First item is newest (DESC) → "retry-merge"
    let first_name = items[0]["name"].as_str().unwrap();
    assert_eq!(
        first_name, "retry-merge",
        "DESC order broken: got {first_name}"
    );

    // Polarity is parsed.
    let polarities: Vec<&str> = items
        .iter()
        .map(|i| i["polarity"].as_str().unwrap())
        .collect();
    assert!(polarities.contains(&"success"));
    assert!(polarities.contains(&"failure"));
    assert!(polarities.contains(&"unknown"));
}

// ── 2. list endpoint — paginated ─────────────────────────────────────────

#[tokio::test]
async fn list_endpoint_paginates_with_limit_and_offset() {
    let (state, mem) = make_state_with_memory().await;
    seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/hermes/skills?limit=2&offset=1").await;
    assert_eq!(status, StatusCode::OK);

    assert_eq!(body["total"], 5, "total ignores limit/offset");
    assert_eq!(body["limit"], 2);
    assert_eq!(body["offset"], 1);
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
}

// ── 3. list endpoint — FTS5 search ───────────────────────────────────────

#[tokio::test]
async fn list_endpoint_supports_fts5_search() {
    let (state, mem) = make_state_with_memory().await;
    seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/hermes/skills?q=rebase").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 1, "got {items:?}");
    assert_eq!(items[0]["name"], "rebase-onto-main");
    assert_eq!(body["total"], 1);
}

// ── 4. detail endpoint — full provenance ─────────────────────────────────

#[tokio::test]
async fn detail_endpoint_returns_full_provenance() {
    let (state, mem) = make_state_with_memory().await;
    let ids = seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    let id = ids[1]; // handle-rate-limit (frontmatter-style)
    let uri = format!("/api/hermes/skills/{id}");
    let (status, body) = get_signed_json(app, &state, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["id"], id);
    assert_eq!(body["name"], "handle-rate-limit");
    assert_eq!(body["description"], "Back off on 429.");
    assert_eq!(body["polarity"], "failure");
    assert_eq!(body["source"], "hermes_skills");
    let raw = body["raw_text"].as_str().expect("raw_text present");
    assert!(raw.contains("---"), "raw_text must include frontmatter");
    assert!(raw.contains("Body.\n"), "raw_text must include body");
}

// ── 5. detail endpoint — 404 ─────────────────────────────────────────────

#[tokio::test]
async fn detail_endpoint_returns_404_for_unknown_id() {
    let (state, mem) = make_state_with_memory().await;
    seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/hermes/skills/99999").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    assert!(body["error"].as_str().unwrap_or("").contains("99999"));
}

// ── 6. timeline endpoint — chronological order ───────────────────────────

#[tokio::test]
async fn timeline_endpoint_orders_chronologically() {
    let (state, mem) = make_state_with_memory().await;
    let ids = seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/hermes/skill-timeline").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let items = body["items"].as_array().expect("items");
    assert_eq!(items.len(), 5);
    // Timeline is ASC by created_at, ties broken by id ASC. We seeded in
    // a single tick (usually), so ids are the tie-breaker. First id ⇒
    // first item.
    assert_eq!(items[0]["id"], ids[0]);
    assert_eq!(items[4]["id"], ids[4]);
    // Verify ids are non-decreasing.
    let id_seq: Vec<i64> = items.iter().map(|i| i["id"].as_i64().unwrap()).collect();
    let mut sorted = id_seq.clone();
    sorted.sort_unstable();
    assert_eq!(id_seq, sorted, "timeline must be chronological (asc)");
}

// ── 7. timeline endpoint — `since=` cutoff ───────────────────────────────

#[tokio::test]
async fn timeline_endpoint_respects_since_cutoff() {
    let (state, mem) = make_state_with_memory().await;
    seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    // Future timestamp ⇒ zero matches.
    let (status, body) =
        get_signed_json(app, &state, "/api/hermes/skill-timeline?since=9999999999").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let items = body["items"].as_array().unwrap();
    assert!(items.is_empty(), "expected no items, got {items:?}");
    assert_eq!(body["since"], 9_999_999_999_i64);
}

// ── 8/9/10. auth: missing token → 401 ────────────────────────────────────

#[tokio::test]
async fn list_endpoint_rejects_missing_auth_token() {
    let (state, _mem) = make_state_with_memory().await;
    let app = phantom_mesh::serve::router(state.clone());

    let req = Request::builder()
        .method("GET")
        .uri("/api/hermes/skills")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn detail_endpoint_rejects_missing_auth_token() {
    let (state, mem) = make_state_with_memory().await;
    let ids = seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state);

    let req = Request::builder()
        .method("GET")
        .uri(format!("/api/hermes/skills/{}", ids[0]))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn timeline_endpoint_rejects_missing_auth_token() {
    let (state, _mem) = make_state_with_memory().await;
    let app = phantom_mesh::serve::router(state);

    let req = Request::builder()
        .method("GET")
        .uri("/api/hermes/skill-timeline")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── 11. service-unavailable when memory not wired ────────────────────────

#[tokio::test]
async fn list_endpoint_returns_503_when_hermes_memory_not_wired() {
    let mut st = AppState::new();
    let mut cfg = ClusterConfig::default();
    cfg.cluster_secret = Some(SECRET.into());
    st.cluster_manager = ClusterManager::new(cfg);
    // hermes_memory deliberately left as None
    let state = Arc::new(st);
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/hermes/skills").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body={body}");
}

// ── 12. perf gate ────────────────────────────────────────────────────────

#[tokio::test]
async fn list_endpoint_meets_500ms_perf_gate_for_1000_skills() {
    let (state, mem) = make_state_with_memory().await;
    for i in 0..1000 {
        mem.insert(NewMemory {
            kind: "skill",
            source: "hermes_skills",
            text: &format!("skill-{i}\nDescription {i}.\nbody {i}\n"),
            tags: if i % 2 == 0 { "recipe" } else { "lesson" },
        })
        .await
        .unwrap();
    }
    let app = phantom_mesh::serve::router(state.clone());

    let start = std::time::Instant::now();
    let (status, body) = get_signed_json(app, &state, "/api/hermes/skills?limit=200").await;
    let elapsed = start.elapsed();

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["total"], 1000);
    assert_eq!(body["items"].as_array().unwrap().len(), 200);
    assert!(
        elapsed.as_millis() < 500,
        "F400 perf gate: list endpoint must serve 1000-skill bank in <500ms, took {elapsed:?}"
    );
}
