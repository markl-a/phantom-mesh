//! Integration test: Full Efficiency Engine cycle
//!
//! Verifies the closed loop:
//! 1. ROI Gate checks hand profitability
//! 2. RoiScheduler records execution results
//! 3. Governor promotes canary policies
//! 4. FeedbackLoop analyzes trajectories and creates draft policies
//! 5. Pipeline definitions are accessible

use phantom_mesh::governor::{Governor, GovernorConfig};
use phantom_mesh::optimizer_store::{OptimizerStore, PolicyStatus, PolicyType};
use phantom_mesh::pipeline::PipelineOrchestrator;
use phantom_mesh::roi_gate::{GateDecision, RoiGate, RoiGateConfig};
use phantom_mesh::roi_scheduler::RoiScheduler;
use phantom_mesh::trajectory::TrajectoryLogger;
use phantom_mesh::unit_economics::UnitEconomics;
use std::sync::Arc;
use uuid::Uuid;

fn setup_engine() -> (
    Arc<RoiGate>,
    Arc<Governor>,
    Arc<OptimizerStore>,
    Arc<RoiScheduler>,
    Arc<UnitEconomics>,
    Arc<TrajectoryLogger>,
) {
    let dir = std::env::temp_dir().join(format!("engine-integration-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();

    let store = Arc::new(
        OptimizerStore::new(dir.join("policies.db").to_str().unwrap()).unwrap(),
    );
    let traj = Arc::new(
        TrajectoryLogger::new(dir.join("traj.db").to_str().unwrap()).unwrap(),
    );
    let roi = Arc::new(RoiScheduler::new());
    let econ = Arc::new(UnitEconomics::new());

    let governor = Arc::new(Governor::new(
        store.clone(),
        GovernorConfig {
            canary_min_runs: 2,
            ..Default::default()
        },
    ));

    let gate = Arc::new(RoiGate::new(
        roi.clone(),
        econ.clone(),
        RoiGateConfig::default(),
    ));

    (gate, governor, store, roi, econ, traj)
}

#[test]
fn test_roi_gate_allows_first_execution() {
    let (gate, _, _, _, _, _) = setup_engine();
    let decision = gate.check("freelancer", false);
    assert!(
        decision.is_allowed(),
        "First execution of unknown hand should be allowed"
    );
}

#[test]
fn test_roi_gate_user_triggered_always_passes() {
    let (gate, _, _, _, _, _) = setup_engine();
    gate.record_spend(100.0); // exceed any budget
    let decision = gate.check("any-hand", true);
    assert!(
        decision.is_allowed(),
        "User-triggered should always be allowed"
    );
}

#[test]
fn test_roi_gate_budget_exhaustion() {
    let (gate, _, _, _, _, _) = setup_engine();
    gate.record_spend(5.01); // exceed default $5 budget
    let decision = gate.check("freelancer", false);
    assert!(
        !decision.is_allowed(),
        "Should deny when budget exhausted"
    );
}

#[test]
fn test_record_execution_updates_roi() {
    let (_, _, _, roi, econ, _) = setup_engine();

    // Record profitable execution
    roi.record_execution("freelancer", 50.0, 2.0, true);
    econ.record_execution("freelancer", 50.0, 2.0, 120.0);

    let economics = econ.get_economics("freelancer").unwrap();
    assert!(economics.revenue_usd > 0.0);
    assert!(economics.margin_pct > 0.0);
}

#[tokio::test]
async fn test_governor_canary_promotion_cycle() {
    let (_, governor, store, _, _, _) = setup_engine();

    // Create Active + Canary policies
    store
        .insert_policy_version(
            "prompt-freelancer",
            PolicyType::Prompt,
            1,
            r#"{"prompt":"original"}"#,
            PolicyStatus::Active,
            Some(chrono::Utc::now().to_rfc3339()),
            None,
        )
        .unwrap();
    store
        .insert_policy_version(
            "prompt-freelancer",
            PolicyType::Prompt,
            2,
            r#"{"prompt":"improved"}"#,
            PolicyStatus::Canary,
            None,
            None,
        )
        .unwrap();

    // Record successful canary runs (need 2 per config)
    governor.record_canary_result("prompt-freelancer", true, 0.9);
    governor.record_canary_result("prompt-freelancer", true, 0.85);

    // Governor should promote
    let actions = governor.check_and_promote().await.unwrap();
    assert!(
        !actions.is_empty(),
        "Governor should produce at least one action"
    );

    // Verify latest policy is now Active (promoted version)
    let latest = store.latest_policy("prompt-freelancer").unwrap().unwrap();
    assert_eq!(
        latest.status,
        PolicyStatus::Active,
        "Canary should be promoted to Active"
    );
    assert!(
        latest.version > 2,
        "A new version should be created on promotion"
    );
}

#[test]
fn test_pipeline_orchestrator_builtins() {
    let orch = PipelineOrchestrator::new();
    let pipelines = orch.list_pipelines();

    let names: Vec<&str> = pipelines.iter().map(|p| p.name.as_str()).collect();
    assert!(
        names.contains(&"revenue-hunt"),
        "Should have revenue-hunt pipeline"
    );
    assert!(
        names.contains(&"content-publish"),
        "Should have content-publish pipeline"
    );
}

#[test]
fn test_trajectory_hand_query_helpers() {
    let dir = std::env::temp_dir().join(format!("traj-helpers-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let logger =
        TrajectoryLogger::new(dir.join("traj.db").to_str().unwrap()).unwrap();

    // Log some entries
    let now = chrono::Utc::now();
    for hand in &["freelancer", "freelancer", "seo_content"] {
        let entry = phantom_mesh::trajectory::TrajectoryEntry {
            id: Uuid::new_v4().to_string(),
            session_id: None,
            agent_name: "master".to_string(),
            hand_name: Some(hand.to_string()),
            phase_name: None,
            provider: "ollama".to_string(),
            model: "qwen3:8b".to_string(),
            prompt: "test".to_string(),
            output: "test output".to_string(),
            tool_calls: 0,
            tool_names: vec![],
            total_tokens: 100,
            duration_secs: 1.0,
            estimated_cost_usd: 0.001,
            quality_score: Some(4),
            guardrail_issues: vec![],
            success: true,
            error_message: None,
            worker_name: None,
            worker_latency_ms: None,
            created_at: now.to_rfc3339(),
            date_key: now.format("%Y-%m-%d").to_string(),
        };
        logger.log_run(&entry).unwrap();
    }

    // Test list_hand_names
    let names = logger.list_hand_names().unwrap();
    assert!(names.contains(&"freelancer".to_string()));
    assert!(names.contains(&"seo_content".to_string()));

    // Test count_for_hand
    assert_eq!(logger.count_for_hand("freelancer").unwrap(), 2);
    assert_eq!(logger.count_for_hand("seo_content").unwrap(), 1);
    assert_eq!(logger.count_for_hand("nonexistent").unwrap(), 0);
}

#[test]
fn test_optimizer_store_list_by_status() {
    let dir = std::env::temp_dir().join(format!("opt-store-status-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let store =
        OptimizerStore::new(dir.join("policies.db").to_str().unwrap()).unwrap();

    store
        .insert_policy_version("p1", PolicyType::Prompt, 1, "{}", PolicyStatus::Active, None, None)
        .unwrap();
    store
        .insert_policy_version("p2", PolicyType::Prompt, 1, "{}", PolicyStatus::Canary, None, None)
        .unwrap();
    store
        .insert_policy_version("p3", PolicyType::Routing, 1, "{}", PolicyStatus::Draft, None, None)
        .unwrap();

    let active = store
        .list_policies_by_status(PolicyStatus::Active)
        .unwrap();
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].policy_id, "p1");

    let canary = store
        .list_policies_by_status(PolicyStatus::Canary)
        .unwrap();
    assert_eq!(canary.len(), 1);
    assert_eq!(canary[0].policy_id, "p2");
}

#[tokio::test]
async fn test_full_engine_cycle() {
    let (gate, governor, store, roi, econ, _traj) = setup_engine();

    // 1. Gate allows first execution (unknown hand, allow_unknown = true)
    let decision = gate.check("freelancer", false);
    assert!(decision.is_allowed());

    // 2. Record execution results
    roi.record_execution("freelancer", 50.0, 2.0, true);
    econ.record_execution("freelancer", 50.0, 2.0, 120.0);
    gate.record_spend(2.0);

    // 3. Create a canary policy (simulating optimizer output)
    store
        .insert_policy_version(
            "prompt-freelancer",
            PolicyType::Prompt,
            1,
            r#"{"prompt":"original"}"#,
            PolicyStatus::Active,
            None,
            None,
        )
        .unwrap();
    store
        .insert_policy_version(
            "prompt-freelancer",
            PolicyType::Prompt,
            2,
            r#"{"prompt":"improved"}"#,
            PolicyStatus::Canary,
            None,
            None,
        )
        .unwrap();

    // 4. Record successful canary runs
    governor.record_canary_result("prompt-freelancer", true, 0.9);
    governor.record_canary_result("prompt-freelancer", true, 0.85);

    // 5. Governor promotes canary
    let actions = governor.check_and_promote().await.unwrap();
    assert!(!actions.is_empty());

    // 6. Verify daily spend tracking
    assert!((gate.current_spend() - 2.0).abs() < 0.01);

    // 7. Verify economics recorded
    let economics = econ.get_economics("freelancer").unwrap();
    assert!(economics.margin_pct > 0.0);
}
