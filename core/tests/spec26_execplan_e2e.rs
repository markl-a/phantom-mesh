//! SPEC-26 §8 — live-peer e2e for `execute_plan` + `refresh_capabilities`.
//!
//! Before this, the only test touching either was the connection-refused
//! error path (`v4_chaos.rs`). The SUCCESS path (assign → poll → Completed),
//! the reassign-to-fallback hop, and `refresh_capabilities` parsing were all
//! untested — a fake-green risk on real reqwest/HMAC code. This drives them
//! through wiremock via the per-peer URL seam (`SPECTYN_PEER_<ID>_URL`) and the
//! poll seam (`SPECTYN_POLL_URL`).
//!
//! All scenarios run sequentially in ONE test fn: `SPECTYN_CLUSTER_SECRET` and
//! `SPECTYN_POLL_URL` are process-global, and this is a dedicated test binary,
//! so a single fn avoids any cross-test env race. Unique peer_ids keep each
//! `SPECTYN_PEER_<ID>_URL` key non-colliding.

use spectyn_mesh::cluster_dispatch_wire::{
    execute_plan, refresh_capabilities, DispatchPlan, DispatchStatus,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn plan(task_id: &str, selected: &str, fallback: Vec<String>) -> DispatchPlan {
    DispatchPlan {
        task_id: task_id.to_string(),
        selected_peer_id: selected.to_string(),
        fallback_peer_ids: fallback,
        scoring_reason: "spec26-e2e".to_string(),
        planned_at_ms: 1_700_000_000_000,
    }
}

fn set_peer_url(peer_id: &str, url: &str) {
    let key = format!(
        "SPECTYN_PEER_{}_URL",
        peer_id.to_uppercase().replace('-', "_")
    );
    std::env::set_var(key, url);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spec26_execplan_e2e_caps_success_and_reassign() {
    std::env::set_var("SPECTYN_CLUSTER_SECRET", "test-cluster-secret-spec26-e2e");

    // ── (1) refresh_capabilities success: GET /node/capabilities → typed caps ──
    {
        let caps_mock = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/node/capabilities"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"capability_ids":["shell","gpu_compute:metal"]}"#),
            )
            .mount(&caps_mock)
            .await;
        set_peer_url("e2e-caps", &caps_mock.uri());

        let caps = refresh_capabilities("e2e-caps")
            .await
            .expect("refresh_capabilities on a 200 peer must be Ok");
        assert_eq!(caps.peer_id, "e2e-caps");
        assert_eq!(caps.tags.len(), 2, "both capability_ids parsed into tags");
        let metal = caps
            .tags
            .iter()
            .find(|t| t.slug == "gpu_compute")
            .expect("parametric gpu_compute tag present");
        assert_eq!(
            metal.value.as_deref(),
            Some("metal"),
            "the ':value' suffix splits into the tag value"
        );
        assert!(caps.last_reported_at > 0, "local receipt timestamp is stamped");
    }

    // ── (2) execute_plan success on the selected peer ──
    {
        let assign_mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rpc/task/assign"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&assign_mock)
            .await;
        let poll_mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status":"completed"}"#))
            .mount(&poll_mock)
            .await;
        set_peer_url("e2e-sel", &assign_mock.uri());
        std::env::set_var("SPECTYN_POLL_URL", poll_mock.uri());

        let out = execute_plan(&plan("t-ok", "e2e-sel", vec![]))
            .await
            .expect("assign 200 + poll completed must be Ok");
        assert_eq!(out.executed_by_peer_id, "e2e-sel", "executed by the selected peer");
        assert!(
            matches!(out.status, DispatchStatus::Completed),
            "terminal status Completed, got {:?}",
            out.status
        );
    }

    // ── (3) execute_plan reassigns to the fallback when the selected peer 5xx's ──
    {
        let sel_mock = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&sel_mock)
            .await;
        let fb_mock = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/rpc/task/assign"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&fb_mock)
            .await;
        let poll_mock = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status":"completed"}"#))
            .mount(&poll_mock)
            .await;
        set_peer_url("e2e-bad", &sel_mock.uri());
        set_peer_url("e2e-fb", &fb_mock.uri());
        std::env::set_var("SPECTYN_POLL_URL", poll_mock.uri());

        let out = execute_plan(&plan("t-reassign", "e2e-bad", vec!["e2e-fb".to_string()]))
            .await
            .expect("selected 503 then fallback 200 must still be Ok");
        assert_eq!(
            out.executed_by_peer_id, "e2e-fb",
            "the SPEC-26 §8 one-hop reassign must land on the fallback peer"
        );
        assert!(matches!(out.status, DispatchStatus::Completed));
    }

    std::env::remove_var("SPECTYN_POLL_URL");
    std::env::remove_var("SPECTYN_CLUSTER_SECRET");
}
