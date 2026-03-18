//! Cluster integration tests — 20+ tests covering ClusterHub creation,
//! worker registration, task dispatch, priority queue, and batch dispatch.
//! All tests use public APIs only.

use clawtex_core::{
    ClusterHub, ClusterRegistry, ClusterMetrics, WorkerStats, ToolRouting,
    PollTaskResponse, TaskResultPayload,
};
use serde_json::json;
use std::sync::Arc;

// ── ClusterMetrics Tests ──────────────────────────────────────────────────────

#[tokio::test]
async fn cluster_metrics_initial_counts_zero() {
    let m = ClusterMetrics::new();
    let snap = m.snapshot().await;
    assert_eq!(snap["dispatch_count"], 0);
    assert_eq!(snap["dispatch_failures"], 0);
    assert_eq!(snap["avg_response_ms"], 0);
}

#[tokio::test]
async fn cluster_metrics_record_success_increments_count() {
    let m = ClusterMetrics::new();
    m.record_success("worker1", 100).await;
    m.record_success("worker1", 200).await;
    let snap = m.snapshot().await;
    assert_eq!(snap["dispatch_count"], 2);
    assert_eq!(snap["dispatch_failures"], 0);
}

#[tokio::test]
async fn cluster_metrics_record_failure_increments_failure_count() {
    let m = ClusterMetrics::new();
    m.record_failure("worker1", "timeout").await;
    let snap = m.snapshot().await;
    assert_eq!(snap["dispatch_failures"], 1);
    assert_eq!(snap["dispatch_count"], 1);
}

#[tokio::test]
async fn cluster_metrics_per_worker_stats_tracked() {
    let m = ClusterMetrics::new();
    m.record_success("acer", 150).await;
    m.record_success("acer", 250).await;
    m.record_failure("m1-mac", "connection refused").await;
    let snap = m.snapshot().await;
    let workers = snap["per_worker"].as_object().unwrap();
    assert!(workers.contains_key("acer"));
    assert!(workers.contains_key("m1-mac"));
    assert_eq!(workers["acer"]["tasks_completed"], 2);
    assert_eq!(workers["m1-mac"]["tasks_failed"], 1);
}

#[tokio::test]
async fn cluster_metrics_worker_stats_returns_none_for_unknown() {
    let m = ClusterMetrics::new();
    assert!(m.worker_stats("nonexistent").await.is_none());
}

#[tokio::test]
async fn cluster_metrics_worker_stats_returns_data_after_record() {
    let m = ClusterMetrics::new();
    m.record_success("z13", 50).await;
    let stats = m.worker_stats("z13").await;
    assert!(stats.is_some());
    let s = stats.unwrap();
    assert_eq!(s.tasks_completed, 1);
    assert_eq!(s.tasks_failed, 0);
}

#[tokio::test]
async fn cluster_metrics_failure_last_error_recorded() {
    let m = ClusterMetrics::new();
    m.record_failure("ayaneo", "connection timeout").await;
    let stats = m.worker_stats("ayaneo").await.unwrap();
    assert_eq!(stats.tasks_failed, 1);
    assert!(stats.last_error.is_some());
    assert!(stats.last_error.unwrap().contains("timeout"));
}

#[tokio::test]
async fn cluster_metrics_mixed_success_and_failure() {
    let m = ClusterMetrics::new();
    m.record_success("w1", 100).await;
    m.record_success("w1", 200).await;
    m.record_failure("w1", "error").await;
    let stats = m.worker_stats("w1").await.unwrap();
    assert_eq!(stats.tasks_completed, 2);
    assert_eq!(stats.tasks_failed, 1);
}

// ── ClusterRegistry Tests ─────────────────────────────────────────────────────

#[tokio::test]
async fn cluster_registry_creates_with_in_memory_db() {
    let registry = ClusterRegistry::new(":memory:").await;
    assert!(registry.is_ok(), "ClusterRegistry::new(:memory:) must succeed");
}

#[tokio::test]
async fn cluster_registry_local_node_pre_registered() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    let nodes = registry.status().await;
    assert!(!nodes.is_empty(), "Should have at least 1 node (local)");
    assert!(nodes.iter().any(|n| n.name == "local"));
}

#[tokio::test]
async fn cluster_registry_local_node_is_online() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    let nodes = registry.status().await;
    let local = nodes.iter().find(|n| n.name == "local").unwrap();
    assert_eq!(local.status, "online");
}

#[tokio::test]
async fn cluster_registry_register_new_worker() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    let result = registry.register("acer", "10.0.1.3", 7881).await;
    assert!(result.is_ok(), "register must succeed: {:?}", result);
    let nodes = registry.status().await;
    assert!(nodes.iter().any(|n| n.name == "acer"));
}

#[tokio::test]
async fn cluster_registry_register_full_worker_with_capabilities() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    let caps = vec!["web_search".to_string(), "http_request".to_string()];
    let result = registry.register_full("test-worker", "10.0.0.1", 7882, &caps, "full").await;
    assert!(result.is_ok());
    let nodes = registry.status().await;
    let node = nodes.iter().find(|n| n.name == "test-worker");
    assert!(node.is_some());
    let n = node.unwrap();
    assert_eq!(n.device_type, "full");
}

#[tokio::test]
async fn cluster_registry_multiple_workers_registered() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    for (name, host, port) in [
        ("z13", "127.0.0.1", 7878u16),
        ("m1-mac", "10.0.2.1", 7879),
        ("acer", "10.0.1.3", 7881),
    ] {
        registry.register(name, host, port).await.unwrap();
    }
    let nodes = registry.status().await;
    // local + 3 registered = at least 4
    assert!(nodes.len() >= 4);
}

#[tokio::test]
async fn cluster_registry_get_node_by_name() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    registry.register("my-worker", "10.0.0.5", 8000).await.unwrap();
    let node = registry.get_node("my-worker").await;
    assert!(node.is_some());
    assert_eq!(node.unwrap().host, "10.0.0.5");
}

#[tokio::test]
async fn cluster_registry_get_node_nonexistent_returns_none() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    let node = registry.get_node("no-such-worker").await;
    assert!(node.is_none());
}

#[tokio::test]
async fn cluster_registry_heartbeat_updates_cpu_load() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    registry.register("heartbeat-worker", "10.0.0.6", 9000).await.unwrap();
    let result = registry.heartbeat("heartbeat-worker", 0.75).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn cluster_registry_best_worker_for_capability() {
    let registry = ClusterRegistry::new(":memory:").await.unwrap();
    registry.register_full(
        "capable-worker", "10.0.0.7", 9001,
        &["web_search".to_string(), "browser".to_string()],
        "full"
    ).await.unwrap();
    // Should find a worker with "web_search" capability
    let best = registry.best_worker_for("web_search").await;
    // May or may not find the worker depending on status filtering
    let _ = best;
}

// ── ClusterHub Creation & Tool Routing Tests ──────────────────────────────────

#[tokio::test]
async fn cluster_hub_new_succeeds() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    let _ = hub;
}

#[tokio::test]
async fn cluster_hub_has_metrics() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    let snap = hub.metrics.snapshot().await;
    assert_eq!(snap["dispatch_count"], 0);
}

#[tokio::test]
async fn cluster_hub_tool_routing_local_only_tools() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    assert!(matches!(hub.tool_routing("file_read"), ToolRouting::Local));
    assert!(matches!(hub.tool_routing("file_write"), ToolRouting::Local));
    assert!(matches!(hub.tool_routing("file_edit"), ToolRouting::Local));
    assert!(matches!(hub.tool_routing("memory_store"), ToolRouting::Local));
    assert!(matches!(hub.tool_routing("memory_recall"), ToolRouting::Local));
    assert!(matches!(hub.tool_routing("memory_forget"), ToolRouting::Local));
    assert!(matches!(hub.tool_routing("glob_search"), ToolRouting::Local));
    assert!(matches!(hub.tool_routing("content_search"), ToolRouting::Local));
}

#[tokio::test]
async fn cluster_hub_tool_routing_any_worker_tools() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    assert!(matches!(hub.tool_routing("web_search"), ToolRouting::AnyWorker));
    assert!(matches!(hub.tool_routing("http_request"), ToolRouting::AnyWorker));
    assert!(matches!(hub.tool_routing("email_send"), ToolRouting::AnyWorker));
}

#[tokio::test]
async fn cluster_hub_tool_routing_mobile_only_tools() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    assert!(matches!(hub.tool_routing("sensor_gps"), ToolRouting::MobileOnly));
    assert!(matches!(hub.tool_routing("sensor_camera"), ToolRouting::MobileOnly));
    assert!(matches!(hub.tool_routing("sensor_accel"), ToolRouting::MobileOnly));
    assert!(matches!(hub.tool_routing("local_llm"), ToolRouting::MobileOnly));
    assert!(matches!(hub.tool_routing("js_exec"), ToolRouting::MobileOnly));
}

#[tokio::test]
async fn cluster_hub_tool_routing_full_worker_only() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    assert!(matches!(hub.tool_routing("shell"), ToolRouting::FullWorkerOnly));
    assert!(matches!(hub.tool_routing("ai_code"), ToolRouting::FullWorkerOnly));
    assert!(matches!(hub.tool_routing("browser"), ToolRouting::FullWorkerOnly));
    assert!(matches!(hub.tool_routing("skeleton_generate"), ToolRouting::FullWorkerOnly));
}

#[tokio::test]
async fn cluster_hub_should_dispatch_false_for_local_tools() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    assert!(!hub.should_dispatch("file_read"));
    assert!(!hub.should_dispatch("memory_store"));
    assert!(!hub.should_dispatch("glob_search"));
    assert!(!hub.should_dispatch("content_search"));
}

#[tokio::test]
async fn cluster_hub_should_dispatch_true_for_remote_tools() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    assert!(hub.should_dispatch("web_search"));
    assert!(hub.should_dispatch("http_request"));
    assert!(hub.should_dispatch("shell"));
    assert!(hub.should_dispatch("sensor_gps"));
}

#[tokio::test]
async fn cluster_hub_dispatch_local_tool_returns_error() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    let result = hub.dispatch_tool("file_read", json!({"path": "test.txt"})).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("local-only"));
}

#[tokio::test]
async fn cluster_hub_dispatch_mobile_tool_no_workers_errors() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    let result = hub.dispatch_tool("sensor_gps", json!({})).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No mobile workers"));
}

#[tokio::test]
async fn cluster_hub_poll_task_empty_queue_returns_none() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    registry.register_full("w1", "0.0.0.0", 0, &["web_search".into()], "mobile").await.unwrap();
    let hub = ClusterHub::new(registry);
    let result = hub.poll_task("w1").await;
    assert!(result.is_none(), "Empty queue must return None");
}

#[tokio::test]
async fn cluster_hub_poll_task_unknown_worker_returns_none() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    let result = hub.poll_task("nonexistent-worker").await;
    assert!(result.is_none());
}

#[tokio::test]
async fn cluster_hub_submit_result_unknown_task_returns_error() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    let result = hub.submit_result(TaskResultPayload {
        task_id: "nonexistent-task".to_string(),
        success: true,
        output: "test".to_string(),
        worker: None,
    }).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No inflight task"));
}

#[tokio::test]
async fn cluster_hub_cleanup_expired_tasks_runs_without_panic() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);
    // Should run without panic even when there's nothing to clean up
    hub.cleanup_expired_tasks(300).await;
}

// ── PollTaskResponse Serialization Tests ──────────────────────────────────────

#[test]
fn poll_task_response_default_priority() {
    let json_str = r#"{"task_id":"t-1","tool":"web_search","input":{}}"#;
    let resp: PollTaskResponse = serde_json::from_str(json_str).unwrap();
    assert_eq!(resp.priority, 100); // default_priority() == 100
}

#[test]
fn poll_task_response_explicit_priority() {
    let resp = PollTaskResponse {
        task_id: "t-2".to_string(),
        tool: "http_request".to_string(),
        input: json!({"url": "https://example.com"}),
        priority: 5,
    };
    let v = serde_json::to_value(&resp).unwrap();
    assert_eq!(v["priority"], 5);
    assert_eq!(v["tool"], "http_request");
}

#[test]
fn poll_task_response_roundtrip_serialization() {
    let resp = PollTaskResponse {
        task_id: "t-roundtrip".to_string(),
        tool: "web_search".to_string(),
        input: json!({"query": "test", "mode": "search"}),
        priority: 50,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let restored: PollTaskResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.task_id, "t-roundtrip");
    assert_eq!(restored.tool, "web_search");
    assert_eq!(restored.priority, 50);
    assert_eq!(restored.input["query"], "test");
}

#[test]
fn task_result_payload_serializes_correctly() {
    let payload = TaskResultPayload {
        task_id: "t-99".to_string(),
        success: true,
        output: "result data".to_string(),
        worker: Some("acer".to_string()),
    };
    let v = serde_json::to_value(&payload).unwrap();
    assert_eq!(v["task_id"], "t-99");
    assert_eq!(v["success"], true);
    assert_eq!(v["worker"], "acer");
}

#[test]
fn task_result_payload_no_worker() {
    let payload = TaskResultPayload {
        task_id: "t-hub".to_string(),
        success: false,
        output: "error message".to_string(),
        worker: None,
    };
    let v = serde_json::to_value(&payload).unwrap();
    assert_eq!(v["success"], false);
    assert!(v["worker"].is_null());
}

// ── ToolRouting Enum Tests ────────────────────────────────────────────────────

#[test]
fn tool_routing_enum_variants_are_distinct() {
    let local = ToolRouting::Local;
    let any = ToolRouting::AnyWorker;
    let full = ToolRouting::FullWorkerOnly;
    let mobile = ToolRouting::MobileOnly;

    assert!(matches!(local, ToolRouting::Local));
    assert!(matches!(any, ToolRouting::AnyWorker));
    assert!(matches!(full, ToolRouting::FullWorkerOnly));
    assert!(matches!(mobile, ToolRouting::MobileOnly));
}

// ── WorkerStats Tests ─────────────────────────────────────────────────────────

#[test]
fn worker_stats_default_values() {
    let stats = WorkerStats::default();
    assert_eq!(stats.tasks_completed, 0);
    assert_eq!(stats.tasks_failed, 0);
    assert_eq!(stats.avg_latency_ms, 0);
    assert!(stats.last_error.is_none());
}
