//! CUJ-04 / CUJ-02 coach-review LLM-path integration tests (wiremock).
//!
//! Complements the offline degraded test in `cuj04_stats_only_fallback.rs` by
//! exercising the paths that REACH a provider, via a mock HTTP server:
//!
//!   • MAC-CUJ02-COA-001  happy path — provider answers → status = Completed,
//!     next_action + model_used populated.
//!   • partial fallback — first provider 500s, second answers → Completed via
//!     the second (proves `complete_with_fallback` walks the chain).
//!   • lint-reject degrade — provider answers with a banned shame phrase →
//!     SPEC-23 §11.1 lint rejects → status = Degraded, no next_action.
//!
//! ## Runtime model (why this is tricky)
//!
//! `run_daily_review` is SYNC and drives HTTP through `block_on_async`, which
//! calls `tokio::task::block_in_place` + `Handle::block_on` IF an ambient tokio
//! runtime is present, else spins its own `current_thread` runtime. wiremock's
//! `MockServer` needs a LIVE runtime to serve requests.
//!
//! So: a single long-lived MULTI-THREAD runtime (`rt`) hosts every MockServer
//! AND drives `run_daily_review` via `rt.block_on(spawn_blocking(...))`. Inside
//! `spawn_blocking` there IS an ambient handle, so `block_on_async` takes the
//! `block_in_place` branch on a multi-thread runtime (allowed), and the mock
//! servers — started on the same `rt` — stay reachable for the whole call. A
//! `current_thread` runtime would forbid `block_in_place`; a throwaway
//! per-helper runtime would be torn down mid-request (the original bug:
//! "A Tokio 1.x context was found, but it is being shutdown").
//!
//! Env mutation (`HOME`, per-provider keys + base URLs) is process-global, so
//! the three tests are folded into ONE `#[test]` run sequentially under a single
//! runtime rather than racing across cargo's test threads.

use spectyn_mesh::coach_wire::{run_daily_review, DailyReviewRequest, DailyReviewOutcome, ReviewStatus};
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn unique_home(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "spectyn-cuj04llm-{}-{}-{}",
        tag,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

fn seed_home(tag: &str, agents_toml: &str) -> std::path::PathBuf {
    let home = unique_home(tag);
    let pm = home.join(".spectyn-mesh");
    std::fs::create_dir_all(&pm).expect("create .spectyn-mesh");
    std::fs::write(pm.join("agents.toml"), agents_toml).expect("write agents.toml");
    home
}

/// Anthropic `/v1/messages` success body: one text content block + usage.
fn anthropic_ok(text: &str) -> serde_json::Value {
    serde_json::json!({
        "content": [{ "type": "text", "text": text }],
        "model": "claude-test",
        "usage": { "input_tokens": 11, "output_tokens": 7 }
    })
}

fn set_event_key() {
    spectyn_mesh::encryption_wire::install_event_key_from_seed(&[9u8; 32])
        .expect("install test EventKey");
}

/// Run `run_daily_review` on `rt`'s blocking pool. Inside `spawn_blocking`,
/// `block_on_async`'s `Handle::try_current()` finds `rt` (a LIVE multi-thread
/// runtime) and uses `block_in_place` + `handle.block_on` to drive reqwest —
/// so the HTTP client and its timeout timer run on the same long-lived runtime
/// that hosts the mock servers. (A throwaway `current_thread` runtime, which is
/// what `run_daily_review` would otherwise spin up, gets dropped while reqwest's
/// timer still references it → "A Tokio 1.x context … is being shutdown".)
fn review_on(
    rt: &tokio::runtime::Runtime,
    date: &str,
) -> Result<DailyReviewOutcome, spectyn_mesh::coach_wire::CoachError> {
    let date = date.to_string();
    rt.block_on(async move {
        tokio::task::spawn_blocking(move || {
            run_daily_review(&DailyReviewRequest { date, recall_k: 0 }, None)
        })
        .await
        .expect("review spawn_blocking join")
    })
}

#[test]
fn cuj04_coach_review_llm_paths() {
    // One multi-thread runtime for the whole test: hosts the mock servers AND
    // permits block_in_place inside run_daily_review's block_on_async.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("multi-thread tokio runtime");

    // ─── 1. happy path — provider answers → Completed ────────────────────────
    {
        let server = rt.block_on(async {
            let s = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(anthropic_ok("明天起床後先喝一杯 250ml 的水。")),
                )
                .mount(&s)
                .await;
            s
        });

        let home = seed_home(
            "happy",
            r#"
[routing]
fallback_chain = ["anthropic"]

[providers.anthropic]
default_model = "claude-test"
"#,
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_API_KEY", "test-key");
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_BASE_URL", server.uri());
        set_event_key();

        let outcome = review_on(&rt, "2026-05-30").expect("happy review returns Ok");
        assert_eq!(
            outcome.status,
            ReviewStatus::Completed,
            "provider answered → status must be Completed"
        );
        assert!(
            outcome.next_action.contains('水'),
            "next_action should carry the LLM's answer, got {:?}",
            outcome.next_action
        );
        assert_eq!(outcome.model_used, "claude-test");
        assert!(!outcome.event_id.is_empty(), "completed review is persisted");

        std::env::remove_var("SPECTYN_MESH_ANTHROPIC_BASE_URL");
        let _ = std::fs::remove_dir_all(&home);
    }

    // ─── 2. partial fallback — first 500s, second answers → Completed ────────
    {
        let (gemini, anthropic) = rt.block_on(async {
            let g = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(500).set_body_string("upstream boom"))
                .mount(&g)
                .await;
            let a = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(anthropic_ok("明天午餐後散步 10 分鐘。")),
                )
                .mount(&a)
                .await;
            (g, a)
        });

        let home = seed_home(
            "fallback",
            r#"
[routing]
fallback_chain = ["gemini", "anthropic"]

[providers.gemini]
default_model = "gemini-test"

[providers.anthropic]
default_model = "claude-test"
"#,
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("SPECTYN_MESH_GEMINI_API_KEY", "g-key");
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_API_KEY", "a-key");
        std::env::set_var("SPECTYN_MESH_GEMINI_BASE_URL", gemini.uri());
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_BASE_URL", anthropic.uri());
        set_event_key();

        let outcome = review_on(&rt, "2026-05-30").expect("fallback review returns Ok");
        assert_eq!(
            outcome.status,
            ReviewStatus::Completed,
            "second provider answered → Completed"
        );
        assert_eq!(
            outcome.model_used, "claude-test",
            "the SECOND (anthropic) provider is the one that answered"
        );
        assert!(outcome.next_action.contains("散步"));

        std::env::remove_var("SPECTYN_MESH_GEMINI_BASE_URL");
        std::env::remove_var("SPECTYN_MESH_ANTHROPIC_BASE_URL");
        let _ = std::fs::remove_dir_all(&home);
    }

    // ─── 2b. per-status-code fallback (MAC-CUJ02-PROV-002/003/004/005) ────────
    // The fallback chain must skip the primary on EACH error class — not just
    // 500 — and reach the second provider. Drive 429 (quota), 404
    // (model-not-found), 401 (auth-fail), 503 (5xx) one at a time: primary
    // (gemini) returns the code, second (anthropic) returns 200, assert the run
    // Completes via the SECOND provider (model_used == claude-test).
    for (code, tag) in [
        (429u16, "prov002-429"),
        (404u16, "prov003-404"),
        (401u16, "prov004-401"),
        (503u16, "prov005-503"),
    ] {
        let (bad, good) = rt.block_on(async {
            let b = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(code))
                .mount(&b)
                .await;
            let a = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(anthropic_ok("喝水提醒：明早先喝 250ml。")),
                )
                .mount(&a)
                .await;
            (b, a)
        });

        let home = seed_home(
            tag,
            r#"
[routing]
fallback_chain = ["gemini", "anthropic"]

[providers.gemini]
default_model = "gemini-test"

[providers.anthropic]
default_model = "claude-test"
"#,
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("SPECTYN_MESH_GEMINI_API_KEY", "g-key");
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_API_KEY", "a-key");
        std::env::set_var("SPECTYN_MESH_GEMINI_BASE_URL", bad.uri());
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_BASE_URL", good.uri());
        set_event_key();

        let outcome = review_on(&rt, "2026-05-23")
            .unwrap_or_else(|e| panic!("review on HTTP {code}: {e:?}"));
        assert_eq!(
            outcome.status,
            ReviewStatus::Completed,
            "HTTP {code} from the primary must skip to the second provider (Completed)"
        );
        assert_eq!(
            outcome.model_used, "claude-test",
            "after HTTP {code}, the SECOND (anthropic) provider must be the one that answered"
        );

        std::env::remove_var("SPECTYN_MESH_GEMINI_BASE_URL");
        std::env::remove_var("SPECTYN_MESH_ANTHROPIC_BASE_URL");
        std::env::remove_var("SPECTYN_MESH_GEMINI_API_KEY");
        std::env::remove_var("SPECTYN_MESH_ANTHROPIC_API_KEY");
        let _ = std::fs::remove_dir_all(&home);
    }

    // ─── 3. lint-reject — shaming LLM output → Degraded, not surfaced ────────
    {
        let server = rt.block_on(async {
            let s = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(anthropic_ok(
                    "You failed again today. Shame on you.",
                )))
                .mount(&s)
                .await;
            s
        });

        let home = seed_home(
            "lint",
            r#"
[routing]
fallback_chain = ["anthropic"]

[providers.anthropic]
default_model = "claude-test"
"#,
        );
        std::env::set_var("HOME", &home);
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_API_KEY", "test-key");
        std::env::set_var("SPECTYN_MESH_ANTHROPIC_BASE_URL", server.uri());
        set_event_key();

        let outcome = review_on(&rt, "2026-05-30").expect("lint-reject still returns Ok (degraded)");
        assert_eq!(
            outcome.status,
            ReviewStatus::Degraded,
            "shaming LLM output must be lint-rejected → Degraded, not surfaced"
        );
        assert!(
            outcome.next_action.is_empty(),
            "degraded review must not surface the shaming action, got {:?}",
            outcome.next_action
        );
        assert!(
            !outcome.events_summary.is_empty(),
            "stats-only brief still present on the degraded path"
        );

        std::env::remove_var("SPECTYN_MESH_ANTHROPIC_BASE_URL");
        let _ = std::fs::remove_dir_all(&home);
    }
}
