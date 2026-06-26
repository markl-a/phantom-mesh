//! F400 integration tests — skill RPC endpoints.
//!
//! Exercises the three new GET routes (`/api/skills`,
//! `/api/skills/:id`, `/api/skill-timeline`) end-to-end
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
//!  11. service-unavailable when skill_memory is None
//!  12. perf gate: list 1000 skills < 500ms
//!
//! All tests run under `--features experimental-memory` per
//! Cargo.toml `[[test]]` requirement below — there is no such entry yet,
//! but `#![cfg(feature = ...)]` makes the file compile-empty without the
//! flag, which is enough for `cargo test --no-default-features` to skip it.

#![cfg(feature = "experimental-memory")]

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use phantom_mesh::skillbank::memory::{SkillMemory, NewMemory};
use phantom_mesh::mesh::{ClusterConfig, ClusterManager};
use phantom_mesh::AppState;
use serde_json::Value;
use std::sync::Arc;
use tower::ServiceExt;

const SECRET: &str = "f400-test-secret";

/// Build an `AppState` with an in-memory skill DB and a fixed cluster
/// secret. Returns the state plus the bare `SkillMemory` handle so the
/// test can seed rows directly.
async fn make_state_with_memory() -> (Arc<AppState>, SkillMemory) {
    let td = tempfile::tempdir().unwrap();
    let db = td.path().join("f400.db");
    let mem = SkillMemory::open_at(db).expect("open skill db");

    let mut st = AppState::new();
    let mut cfg = ClusterConfig::default();
    cfg.cluster_secret = Some(SECRET.into());
    st.cluster_manager = ClusterManager::new(cfg);
    st.skill_memory = Some(mem.clone());

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
async fn seed_varied(mem: &SkillMemory) -> Vec<i64> {
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

    let (status, body) = get_signed_json(app, &state, "/api/skills").await;
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

    let (status, body) = get_signed_json(app, &state, "/api/skills?limit=2&offset=1").await;
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

    let (status, body) = get_signed_json(app, &state, "/api/skills?q=rebase").await;
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
    let uri = format!("/api/skills/{id}");
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

    let (status, body) = get_signed_json(app, &state, "/api/skills/99999").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    assert!(body["error"].as_str().unwrap_or("").contains("99999"));
}

// ── 6. timeline endpoint — chronological order ───────────────────────────

#[tokio::test]
async fn timeline_endpoint_orders_chronologically() {
    let (state, mem) = make_state_with_memory().await;
    let ids = seed_varied(&mem).await;
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/skill-timeline").await;
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
        get_signed_json(app, &state, "/api/skill-timeline?since=9999999999").await;
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
        .uri("/api/skills")
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
        .uri(format!("/api/skills/{}", ids[0]))
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
        .uri("/api/skill-timeline")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── 11. service-unavailable when memory not wired ────────────────────────

#[tokio::test]
async fn list_endpoint_returns_503_when_skill_memory_not_wired() {
    let mut st = AppState::new();
    let mut cfg = ClusterConfig::default();
    cfg.cluster_secret = Some(SECRET.into());
    st.cluster_manager = ClusterManager::new(cfg);
    // skill_memory deliberately left as None
    let state = Arc::new(st);
    let app = phantom_mesh::serve::router(state.clone());

    let (status, body) = get_signed_json(app, &state, "/api/skills").await;
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
    let (status, body) = get_signed_json(app, &state, "/api/skills?limit=200").await;
    let elapsed = start.elapsed();

    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["total"], 1000);
    assert_eq!(body["items"].as_array().unwrap().len(), 200);
    assert!(
        elapsed.as_millis() < 500,
        "F400 perf gate: list endpoint must serve 1000-skill bank in <500ms, took {elapsed:?}"
    );
}

// ── 13. E005 cut evidence: FTS5 search p99 latency ───────────────────────

/// Seed ~1000 rows, then time 100 FTS5 searches through the full router
/// (auth + handler + `search_by_kind_paginated`) and assert the p99 stays
/// under 200ms. Queries rotate across tokens with very different
/// selectivity (1 hit … all 1000 hits) so we are not measuring one hot
/// statement, and the timing covers the worst realistic shape (bm25 rank
/// over the whole bank).
#[tokio::test]
async fn search_fts5_p99_latency_under_200ms_with_1000_rows() {
    let (state, mem) = make_state_with_memory().await;
    for i in 0..1000 {
        mem.insert(NewMemory {
            kind: "skill",
            source: "hermes_skills",
            text: &format!(
                "skill-{i}\nDescription {i} alpha{} beta{}.\nbody token{i}\n",
                i % 7,
                i % 13
            ),
            tags: if i % 2 == 0 { "recipe" } else { "lesson" },
        })
        .await
        .unwrap();
    }
    let app = phantom_mesh::serve::router(state.clone());

    // Selectivity spread: token42 → 1 row, beta3 → ~77, alpha1 → ~143,
    // Description/skill → all 1000 rows (bm25 over the full bank).
    let queries = ["token42", "beta3", "alpha1", "Description", "skill"];
    let mut samples_ms: Vec<u128> = Vec::with_capacity(100);
    for i in 0..100 {
        let uri = format!("/api/skills?q={}&limit=50", queries[i % queries.len()]);
        let start = std::time::Instant::now();
        let (status, body) = get_signed_json(app.clone(), &state, &uri).await;
        samples_ms.push(start.elapsed().as_millis());
        assert_eq!(status, StatusCode::OK, "query {uri} failed: {body}");
    }
    samples_ms.sort_unstable();
    // p99 over 100 samples = the 99th order statistic (index 98).
    let p99 = samples_ms[98];
    assert!(
        p99 < 200,
        "E005 cut evidence: FTS5 search p99 must stay <200ms over a ~1000-row \
         bank, got p99={p99}ms (min={}ms max={}ms)",
        samples_ms[0],
        samples_ms[99]
    );
}

// ── 14. E005 cut evidence: CJK query behaviour probe (documented) ────────
//
// The index uses FTS5's default tokenizer (`unicode61 remove_diacritics 2`,
// see migrations/0007_hermes_fts5.sql), which does NOT segment CJK text:
// CJK ideographs are Unicode-alphanumeric, and unicode61 only splits on
// non-alphanumerics, so any contiguous run of ideographs indexes as ONE
// token. Observed behaviour for the query 焦點 (asserted below):
//   • it DOES match a row where 焦點 appears whitespace-delimited
//     ("保持 焦點 模式" → indexed as the exact token 焦點);
//   • it does NOT match a row where 焦點 is embedded in a longer CJK run
//     ("焦點工作階段紀錄" → a single token; the phrase query "焦點"
//     produced by escape_fts5_query carries no prefix `*`, so a sub-token
//     can never match).
// This is the known FTS5 default-tokenizer CJK limitation and is accepted
// for the v0.6.0 cut — full CJK recall needs a segmenting tokenizer
// (trigram / ICU), tracked as a follow-up, not a ship blocker.

#[tokio::test]
async fn search_cjk_query_behaviour_documented_probe() {
    let (state, mem) = make_state_with_memory().await;
    // Row 1: 焦點 as a standalone, whitespace-delimited token.
    mem.insert(NewMemory {
        kind: "skill",
        source: "hermes_skills",
        text: "cjk-standalone\n保持 焦點 模式\nbody\n",
        tags: "cjk",
    })
    .await
    .unwrap();
    // Row 2: 焦點 embedded in a longer undelimited CJK run.
    mem.insert(NewMemory {
        kind: "skill",
        source: "hermes_skills",
        text: "cjk-embedded\n焦點工作階段紀錄\nbody\n",
        tags: "cjk",
    })
    .await
    .unwrap();
    let app = phantom_mesh::serve::router(state.clone());

    // q=焦點, percent-encoded (UTF-8 bytes E7 84 A6 E9 BB 9E).
    let (status, body) =
        get_signed_json(app, &state, "/api/skills?q=%E7%84%A6%E9%BB%9E").await;
    assert_eq!(status, StatusCode::OK, "CJK query must not error: {body}");

    let names: Vec<&str> = body["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|i| i["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"cjk-standalone"),
        "whitespace-delimited CJK token must match q=焦點, got {names:?}"
    );
    assert!(
        !names.contains(&"cjk-embedded"),
        "unicode61 does not segment CJK — embedded 焦點 is expected NOT to \
         match (documented limitation, see comment above); got {names:?}"
    );
}
