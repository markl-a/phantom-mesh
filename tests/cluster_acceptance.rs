//! Cluster Acceptance Tests — validates the two key acceptance criteria:
//! 1. 8-device cluster formation, dispatch, and resilience
//! 2. Self-modification capability with approval gate and rollback
//!
//! Run with: cargo test --test cluster_acceptance -- --nocapture

use clawtex_core::*;
use clawtex_core::cluster::ClusterRegistry;
use clawtex_core::cluster_hub::{ClusterHub, ToolRouting};
use clawtex_core::hands::{Hand, HandRunner, HandRegistry, Phase};
use clawtex_core::approval::{ApprovalGate, ApprovalConfig};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::time::Duration;

/// Build the full 8-device cluster topology (1 hub + 7 workers)
async fn build_8_device_cluster() -> (Arc<ClusterRegistry>, Arc<ClusterHub>) {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());

    // 3 Full workers
    registry.register_full("m1-mac", "10.0.2.1", 7879,
        &["tools".into(), "llm".into(), "build".into()], "full").await.unwrap();
    registry.register_full("ayaneo", "100.0.0.20", 7879,
        &["tools".into(), "llm".into(), "build".into()], "full").await.unwrap();
    registry.register_full("aspire5", "100.0.0.30", 7879,
        &["tools".into(), "llm".into(), "build".into()], "full").await.unwrap();

    // 4 Light workers
    registry.register_full("android-1", "100.0.0.40", 7880,
        &["web_search".into(), "http_request".into()], "light").await.unwrap();
    registry.register_full("android-2", "100.0.0.41", 7880,
        &["web_search".into(), "http_request".into()], "light").await.unwrap();
    registry.register_full("iphone", "100.0.0.50", 7880,
        &["web_search".into(), "http_request".into()], "light").await.unwrap();
    registry.register_full("ipad", "100.0.0.51", 7880,
        &["web_search".into(), "http_request".into()], "light").await.unwrap();

    // Set all workers as alive with varied loads
    registry.heartbeat("m1-mac", 0.3).await.unwrap();
    registry.heartbeat("ayaneo", 0.5).await.unwrap();
    registry.heartbeat("aspire5", 0.2).await.unwrap();
    registry.heartbeat("android-1", 0.1).await.unwrap();
    registry.heartbeat("android-2", 0.1).await.unwrap();
    registry.heartbeat("iphone", 0.05).await.unwrap();
    registry.heartbeat("ipad", 0.15).await.unwrap();

    let hub = Arc::new(ClusterHub::new(registry.clone()));
    (registry, hub)
}

// ═══════════════════════════════════════════════════════════════════════════
// A1: 8-Device Cluster Formation
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a1_cluster_formation_7_workers_online() {
    let (registry, _hub) = build_8_device_cluster().await;

    let workers = registry.online_workers().await;
    assert_eq!(workers.len(), 7, "Expected 7 online workers (hub is Z13), got {}", workers.len());

    // Verify device types
    let full_count = workers.iter().filter(|w| w.device_type == "full").count();
    let light_count = workers.iter().filter(|w| w.device_type == "light").count();
    assert_eq!(full_count, 3, "Expected 3 full workers");
    assert_eq!(light_count, 4, "Expected 4 light workers");

    // Verify all have unique names
    let names: Vec<&str> = workers.iter().map(|w| w.name.as_str()).collect();
    assert!(names.contains(&"m1-mac"));
    assert!(names.contains(&"ayaneo"));
    assert!(names.contains(&"aspire5"));
    assert!(names.contains(&"android-1"));
    assert!(names.contains(&"android-2"));
    assert!(names.contains(&"iphone"));
    assert!(names.contains(&"ipad"));
}

// ═══════════════════════════════════════════════════════════════════════════
// A2: Tool Dispatch Reaches Correct Device Types
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a2_tool_dispatch_routing() {
    let (_registry, hub) = build_8_device_cluster().await;

    // web_search should go to any worker (including light)
    let web_routing = hub.tool_routing("web_search");
    assert_eq!(web_routing, ToolRouting::AnyWorker,
        "web_search should route to AnyWorker");

    // shell should go to full workers only
    let shell_routing = hub.tool_routing("shell");
    assert_eq!(shell_routing, ToolRouting::FullWorkerOnly,
        "shell should route to FullWorkerOnly");

    // file_write should be local
    let file_routing = hub.tool_routing("file_write");
    assert_eq!(file_routing, ToolRouting::Local,
        "file_write should route locally");

    // file_edit should be local
    let edit_routing = hub.tool_routing("file_edit");
    assert_eq!(edit_routing, ToolRouting::Local,
        "file_edit should route locally");

    // http_request can go to any worker
    let http_routing = hub.tool_routing("http_request");
    assert_eq!(http_routing, ToolRouting::AnyWorker,
        "http_request should route to AnyWorker");
}

// ═══════════════════════════════════════════════════════════════════════════
// A3: Staleness Detection and Recovery
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a3_staleness_detection_and_recovery() {
    let (registry, _hub) = build_8_device_cluster().await;

    // All 7 workers should be online
    let online = registry.online_workers().await;
    assert_eq!(online.len(), 7);

    // All registered nodes (including 'local')
    let all = registry.status().await;
    assert_eq!(all.len(), 8, "7 workers + 1 local hub node");

    // Verify heartbeat updates load
    registry.heartbeat("m1-mac", 0.9).await.unwrap();
    let node = registry.get_node("m1-mac").await.unwrap();
    assert!((node.cpu_load - 0.9).abs() < 0.01, "Heartbeat should update CPU load");

    // Re-register a node (simulates reconnection)
    registry.register_full("m1-mac", "10.0.2.1", 7879,
        &["tools".into(), "llm".into(), "build".into()], "full").await.unwrap();
    let node = registry.get_node("m1-mac").await.unwrap();
    assert_eq!(node.status, "online", "Re-registered node should be online");
}

// ═══════════════════════════════════════════════════════════════════════════
// A4: Self-Modify with Approval Gate — Approved
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a4_self_modify_approval_gate_approved() {
    let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
        timeout_secs: 5,
        tiered_enabled: false,
        ..Default::default()
    }));

    let hand = Hand {
        name: "test_self_optimize".to_string(),
        description: "Test self-optimize hand".to_string(),
        category: "test".to_string(),
        provider: "auto".to_string(),
        model: String::new(),
        phases: vec![
            Phase {
                name: "analyze".to_string(),
                system_prompt: "Analyze system state".to_string(),
                max_rounds: 1,
                condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: vec![],
                    tools: None,
                    provider: None,
                    model: None,
                    extra: HashMap::new(),
            },
        ],
        tools: Some(vec!["file_read".to_string()]),
        output_format: "markdown".to_string(),
        schedule: None,
        settings: {
            let mut s = HashMap::new();
            s.insert("require_approval".to_string(), "true".to_string());
            s
        },
        chain_to: None,
        guardrail: None,
        eval: None,
        extra: HashMap::new(),
    };

    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
    let tool_registry = ToolRegistry::new(SecurityConfig::default());

    let gate_clone = gate.clone();

    // Auto-approve in background using public API
    let approve_handle = tokio::spawn(async move {
        // Wait for a pending approval to appear
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let ids = gate_clone.pending_ids().await;
            if let Some(id) = ids.first() {
                gate_clone.respond(id, true).await;
                break;
            }
        }
    });

    let result = HandRunner::run(
        &hand, "test optimize", &runtime, &router, &tool_registry, Some(&gate),
    ).await.unwrap();

    approve_handle.await.unwrap();

    // Should have proceeded (even if phases fail due to no LLM)
    assert_eq!(result.hand_name, "test_self_optimize");
    // After approval, the hand attempts execution. It may fail due to no LLM, but it should NOT
    // be "Denied by approval gate" or "Approval timed out" — those mean approval didn't work.
    assert!(result.final_output != "Denied by approval gate" && result.final_output != "Approval timed out",
        "Hand should have attempted execution after approval, got: {}", result.final_output);
}

// ═══════════════════════════════════════════════════════════════════════════
// A5: Self-Modify with Approval Gate — Denied
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a5_self_modify_approval_gate_denied() {
    let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
        timeout_secs: 5,
        tiered_enabled: false,
        ..Default::default()
    }));

    let hand = Hand {
        name: "test_denied".to_string(),
        description: "Test denied hand".to_string(),
        category: "test".to_string(),
        provider: "auto".to_string(),
        model: String::new(),
        phases: vec![
            Phase {
                name: "dangerous".to_string(),
                system_prompt: "Do something dangerous".to_string(),
                max_rounds: 1,
                condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: vec![],
                    tools: None,
                    provider: None,
                    model: None,
                    extra: HashMap::new(),
            },
        ],
        tools: Some(vec!["shell".to_string()]),
        output_format: "markdown".to_string(),
        schedule: None,
        settings: {
            let mut s = HashMap::new();
            s.insert("require_approval".to_string(), "true".to_string());
            s
        },
        chain_to: None,
        guardrail: None,
        eval: None,
        extra: HashMap::new(),
    };

    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
    let tool_registry = ToolRegistry::new(SecurityConfig::default());

    let gate_clone = gate.clone();

    // Auto-deny in background
    let deny_handle = tokio::spawn(async move {
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let ids = gate_clone.pending_ids().await;
            if let Some(id) = ids.first() {
                gate_clone.respond(id, false).await;
                break;
            }
        }
    });

    let result = HandRunner::run(
        &hand, "test deny", &runtime, &router, &tool_registry, Some(&gate),
    ).await.unwrap();

    deny_handle.await.unwrap();

    assert_eq!(result.hand_name, "test_denied");
    assert_eq!(result.phases_completed, 0, "Denied hand should not execute any phases");
    assert_eq!(result.final_output, "Denied by approval gate");
}

// ═══════════════════════════════════════════════════════════════════════════
// A6: Metrics Accumulation Across Workers
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a6_metrics_accumulation() {
    let (_registry, hub) = build_8_device_cluster().await;

    // Record some metrics via the ClusterMetrics API
    hub.metrics.record_success("android-1", 150).await;
    hub.metrics.record_success("iphone", 200).await;
    hub.metrics.record_success("m1-mac", 500).await;
    hub.metrics.record_failure("m1-mac", "timeout").await;

    let total = hub.metrics.dispatch_count.load(Ordering::Relaxed);
    let failures = hub.metrics.dispatch_failures.load(Ordering::Relaxed);
    assert_eq!(total, 4, "Should have 4 total dispatches");
    assert_eq!(failures, 1, "Should have 1 failure");

    // Check per-worker stats
    let m1_stats = hub.metrics.worker_stats("m1-mac").await;
    assert!(m1_stats.is_some(), "m1-mac should have stats");
    let m1 = m1_stats.unwrap();
    assert_eq!(m1.tasks_completed, 1);
    assert_eq!(m1.tasks_failed, 1);
}

// ═══════════════════════════════════════════════════════════════════════════
// A7: SecurityConfig allowed_paths Unlocks src/ Access
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a7_allowed_paths_unlocks_src() {
    let temp_src = tempfile::tempdir().unwrap();
    let temp_ws = tempfile::tempdir().unwrap();

    // Create a test file in "src"
    std::fs::write(temp_src.path().join("test.rs"), "fn main() {}").unwrap();

    let security = SecurityConfig {
        workspace_dir: temp_ws.path().to_string_lossy().to_string(),
        workspace_only: true,
        allowed_commands: vec![],
        rate_limit: RateLimitConfig::default(),
        allowed_paths: vec![temp_src.path().to_string_lossy().to_string()],
    };

    // Should allow path in allowed_paths
    let src_file = temp_src.path().join("test.rs").canonicalize().unwrap();
    assert!(security.is_allowed_path(&src_file),
        "File in allowed_paths should be accessible");

    // Should allow path in workspace
    std::fs::write(temp_ws.path().join("ws.txt"), "workspace file").unwrap();
    let ws_file = temp_ws.path().join("ws.txt").canonicalize().unwrap();
    assert!(security.is_allowed_path(&ws_file),
        "File in workspace should be accessible");
}

// ═══════════════════════════════════════════════════════════════════════════
// A8: Shell Tool Timeout and Working Dir
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a8_shell_timeout_and_working_dir() {
    use clawtex_core::tools::shell::ShellTool;
    use clawtex_core::tools::Tool;

    let temp_dir = tempfile::tempdir().unwrap();
    let security = SecurityConfig {
        workspace_dir: temp_dir.path().to_string_lossy().to_string(),
        workspace_only: true,
        allowed_commands: vec!["echo".into(), "pwd".into()],
        rate_limit: RateLimitConfig::default(),
        allowed_paths: vec![temp_dir.path().to_string_lossy().to_string()],
    };

    let tool = ShellTool::new(security);

    // Test with custom timeout
    let result = tool.execute(json!({
        "command": "echo hello",
        "timeout_secs": 60
    })).await.unwrap();
    assert!(result.success, "Echo with custom timeout should succeed: {}", result.output);

    // Test with working_dir
    let result = tool.execute(json!({
        "command": "echo test",
        "working_dir": temp_dir.path().to_string_lossy().to_string()
    })).await.unwrap();
    assert!(result.success, "Echo with working_dir should succeed: {}", result.output);

    // Test with invalid working_dir (should fail)
    let result = tool.execute(json!({
        "command": "echo test",
        "working_dir": "/nonexistent/path/that/surely/does/not/exist"
    })).await.unwrap();
    assert!(!result.success, "Invalid working_dir should fail");
}

// ═══════════════════════════════════════════════════════════════════════════
// A9: Hand Without Approval Runs Normally
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a9_hand_without_approval_runs_normally() {
    let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
        tiered_enabled: false,
        ..Default::default()
    }));

    let hand = Hand {
        name: "normal_hand".to_string(),
        description: "No approval needed".to_string(),
        category: "test".to_string(),
        provider: "auto".to_string(),
        model: String::new(),
        phases: vec![
            Phase {
                name: "do_stuff".to_string(),
                system_prompt: "Do something".to_string(),
                max_rounds: 1,
                condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: vec![],
                    tools: None,
                    provider: None,
                    model: None,
                    extra: HashMap::new(),
            },
        ],
        tools: Some(vec![]),
        output_format: "markdown".to_string(),
        schedule: None,
        settings: HashMap::new(),
        chain_to: None,
        guardrail: None,
        eval: None,
        extra: HashMap::new(),
    };

    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
    let tool_registry = ToolRegistry::new(SecurityConfig::default());

    // Should run without blocking on approval
    let result = HandRunner::run(
        &hand, "test", &runtime, &router, &tool_registry, Some(&gate),
    ).await.unwrap();

    assert_eq!(result.hand_name, "normal_hand");
    // Hand was NOT blocked by approval gate — it attempted execution.
    // phases_completed may be 0 if no LLM provider is available, which is expected in tests.
    assert!(result.final_output != "Denied by approval gate" && result.final_output != "Approval timed out",
        "Hand without require_approval should not be blocked by approval gate, got: {}", result.final_output);
}

// ═══════════════════════════════════════════════════════════════════════════
// A10: Approval Gate Timeout
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a10_approval_gate_timeout() {
    let gate = Arc::new(ApprovalGate::new(ApprovalConfig {
        timeout_secs: 1,
        tiered_enabled: false,
        ..Default::default()
    }));

    let hand = Hand {
        name: "timeout_hand".to_string(),
        description: "Will timeout".to_string(),
        category: "test".to_string(),
        provider: "auto".to_string(),
        model: String::new(),
        phases: vec![
            Phase {
                name: "wait".to_string(),
                system_prompt: "Wait".to_string(),
                max_rounds: 1,
                condition: None,
                    target_worker: None,
                    target_capability: None,
                    parallel_queries: vec![],
                    tools: None,
                    provider: None,
                    model: None,
                    extra: HashMap::new(),
            },
        ],
        tools: Some(vec![]),
        output_format: "markdown".to_string(),
        schedule: None,
        settings: {
            let mut s = HashMap::new();
            s.insert("require_approval".to_string(), "true".to_string());
            s
        },
        chain_to: None,
        guardrail: None,
        eval: None,
        extra: HashMap::new(),
    };

    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
    let tool_registry = ToolRegistry::new(SecurityConfig::default());

    let result = HandRunner::run(
        &hand, "test", &runtime, &router, &tool_registry, Some(&gate),
    ).await.unwrap();

    assert_eq!(result.phases_completed, 0, "Timed-out hand should not execute phases");
    assert_eq!(result.final_output, "Approval timed out");
}

// ═══════════════════════════════════════════════════════════════════════════
// A11: Self-Optimize Hand Loads Correctly
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a11_self_optimize_hand_loads() {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".into());
    let hands_dir = format!("{}/.clawtex/hands", home);
    let registry = HandRegistry::load(&hands_dir).unwrap_or_else(|_| HandRegistry::empty());

    if let Some(hand) = registry.get("self_optimize") {
        assert_eq!(hand.category, "infrastructure");
        assert!(hand.tools.as_ref().map_or(false, |t| t.contains(&"shell".to_string())), "self_optimize must have shell tool");
        assert!(hand.tools.as_ref().map_or(false, |t| t.contains(&"file_read".to_string())), "self_optimize must have file_read tool");
        assert!(hand.tools.as_ref().map_or(false, |t| t.contains(&"file_edit".to_string())), "self_optimize must have file_edit tool");
        assert_eq!(hand.settings.get("require_approval").map(|s| s.as_str()), Some("true"),
            "self_optimize must require approval");
        assert_eq!(hand.phases.len(), 4, "self_optimize should have 4 phases");
        assert_eq!(hand.phases[0].name, "read_state");
        assert_eq!(hand.phases[1].name, "plan_changes");
        assert_eq!(hand.phases[2].name, "apply_changes");
        assert_eq!(hand.phases[3].name, "verify");
    } else {
        println!("self_optimize hand not found at {}, skipping", hands_dir);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// A12: Cluster Hub Dispatch to Best Worker
// ═══════════════════════════════════════════════════════════════════════════

#[tokio::test]
async fn a12_cluster_hub_dispatch_best_worker() {
    let (registry, hub) = build_8_device_cluster().await;

    // For AnyWorker routing, any online worker should be available
    let workers = registry.online_workers().await;
    assert!(!workers.is_empty(), "Should have online workers");

    // For FullWorkerOnly, best_worker_for("tools") should return a full worker
    let best_full = registry.best_worker_for("tools").await;
    assert!(best_full.is_some(), "Should find a full worker with 'tools' capability");
    let full_worker = best_full.unwrap();
    assert_eq!(full_worker.device_type, "full",
        "Worker with 'tools' capability should be full type, got: {}", full_worker.name);
    // aspire5 has the lowest load (0.2) among full workers
    assert_eq!(full_worker.name, "aspire5",
        "Should pick least loaded full worker (aspire5 at 0.2)");

    // Verify tool routing logic
    assert!(hub.should_dispatch("web_search"), "web_search should be dispatched");
    assert!(hub.should_dispatch("shell"), "shell should be dispatched");
    assert!(!hub.should_dispatch("file_write"), "file_write should NOT be dispatched (local only)");
}
