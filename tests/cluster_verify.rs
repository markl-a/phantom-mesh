//! Cluster Verification Tests — 10 tests proving the cluster infrastructure works.
//! Run with: cargo test --test cluster_verify

use clawtex_core::*;
use clawtex_core::cluster::ClusterRegistry;
use clawtex_core::cluster_hub::{ClusterHub, ClusterMetrics, ToolRouting};
use clawtex_core::cluster_worker::ClusterConfig;
use serde_json::json;
use std::sync::Arc;

// ── V1: Worker Registration Flow ────────────────────────────────────────────

#[tokio::test]
async fn v1_worker_registration_flow() {
    // Verify: workers register with hub, appear in registry with correct metadata
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let _hub = ClusterHub::new(registry.clone());

    // Register a full worker (M1 Mac)
    registry.register_full(
        "m1-mac", "10.0.2.1", 7879,
        &["tools".into(), "llm".into()],
        "full",
    ).await.unwrap();

    // Register a light worker (Android)
    registry.register_full(
        "android-1", "100.0.0.10", 7880,
        &["web_search".into(), "http_request".into(), "email_send".into()],
        "light",
    ).await.unwrap();

    // Register another full worker (Ayaneo)
    registry.register_full(
        "ayaneo", "100.0.0.20", 7879,
        &["tools".into()],
        "full",
    ).await.unwrap();

    let workers = registry.online_workers().await;
    assert_eq!(workers.len(), 3, "Should have 3 online workers (not counting 'local')");

    // Verify metadata
    let m1 = workers.iter().find(|w| w.name == "m1-mac").unwrap();
    assert_eq!(m1.device_type, "full");
    assert!(m1.capabilities.contains(&"llm".to_string()));

    let android = workers.iter().find(|w| w.name == "android-1").unwrap();
    assert_eq!(android.device_type, "light");
    assert_eq!(android.capabilities.len(), 3);

    println!("[V1 PASS] 3 workers registered: m1-mac(full), android-1(light), ayaneo(full)");
}

// ── V2: Heartbeat and CPU Load Tracking ──────────────────────────────────────

#[tokio::test]
async fn v2_heartbeat_and_load_tracking() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());

    registry.register_full("w1", "10.0.0.1", 7879, &["tools".into()], "full").await.unwrap();
    registry.register_full("w2", "10.0.0.2", 7880, &["tools".into()], "full").await.unwrap();

    // Simulate heartbeats with different loads
    registry.heartbeat("w1", 0.2).await.unwrap();
    registry.heartbeat("w2", 0.8).await.unwrap();

    let w1 = registry.get_node("w1").await.unwrap();
    let w2 = registry.get_node("w2").await.unwrap();
    assert!((w1.cpu_load - 0.2).abs() < 0.01);
    assert!((w2.cpu_load - 0.8).abs() < 0.01);

    // Update load
    registry.heartbeat("w1", 0.9).await.unwrap();
    let w1 = registry.get_node("w1").await.unwrap();
    assert!((w1.cpu_load - 0.9).abs() < 0.01);

    // Unknown node should fail
    assert!(registry.heartbeat("nonexistent", 0.5).await.is_err());

    println!("[V2 PASS] Heartbeat tracks CPU load: w1=0.9, w2=0.8");
}

// ── V3: Staleness Detection (Offline Marking) ───────────────────────────────

#[tokio::test]
async fn v3_staleness_detection() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());

    registry.register("fresh", "10.0.0.1", 7879).await.unwrap();
    registry.register("stale", "10.0.0.2", 7880).await.unwrap();

    // Make "stale" node old
    {
        let conn = registry.conn.lock().unwrap();
        let old = (chrono::Utc::now() - chrono::Duration::seconds(200)).to_rfc3339();
        conn.execute(
            "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'stale'",
            rusqlite::params![old],
        ).unwrap();
    }

    // Mark stale after 90s
    registry.mark_offline_stale(90).await;

    let fresh = registry.get_node("fresh").await.unwrap();
    let stale = registry.get_node("stale").await.unwrap();
    assert_eq!(fresh.status, "online");
    assert_eq!(stale.status, "offline");

    // "local" should never go offline
    let local = registry.get_node("local").await.unwrap();
    assert_eq!(local.status, "online");

    println!("[V3 PASS] Stale node marked offline, fresh + local remain online");
}

// ── V4: Tool Routing Logic ──────────────────────────────────────────────────

#[test]
fn v4_tool_routing_logic() {
    let registry = Arc::new(futures_util::FutureExt::now_or_never(
        ClusterRegistry::new(":memory:")
    ).unwrap().unwrap());
    let hub = ClusterHub::new(registry);

    // Local-only tools (filesystem, memory)
    let local_tools = ["file_write", "file_edit", "file_read", "memory_store",
                       "memory_recall", "memory_forget", "glob_search", "content_search"];
    for tool in &local_tools {
        assert_eq!(hub.tool_routing(tool), ToolRouting::Local,
            "{} should be Local", tool);
        assert!(!hub.should_dispatch(tool), "{} should not dispatch", tool);
    }

    // Network tools (any worker including light)
    let network_tools = ["web_search", "http_request", "email_send"];
    for tool in &network_tools {
        assert_eq!(hub.tool_routing(tool), ToolRouting::AnyWorker,
            "{} should be AnyWorker", tool);
        assert!(hub.should_dispatch(tool), "{} should dispatch", tool);
    }

    // Compute tools (full workers only)
    let compute_tools = ["shell", "ai_code", "browser", "computer_use",
                         "skeleton_generate", "delegate", "twitter", "blog_publish"];
    for tool in &compute_tools {
        assert_eq!(hub.tool_routing(tool), ToolRouting::FullWorkerOnly,
            "{} should be FullWorkerOnly", tool);
        assert!(hub.should_dispatch(tool), "{} should dispatch", tool);
    }

    println!("[V4 PASS] {} local, {} network, {} compute tools correctly routed",
        local_tools.len(), network_tools.len(), compute_tools.len());
}

// ── V5: Best Worker Selection (Load Balancing) ──────────────────────────────

#[tokio::test]
async fn v5_load_balancing_best_worker() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());

    // 3 full workers with different loads
    registry.register_full("busy", "10.0.0.1", 7879, &["tools".into()], "full").await.unwrap();
    registry.register_full("medium", "10.0.0.2", 7880, &["tools".into()], "full").await.unwrap();
    registry.register_full("idle", "10.0.0.3", 7881, &["tools".into()], "full").await.unwrap();

    registry.heartbeat("busy", 0.95).await.unwrap();
    registry.heartbeat("medium", 0.50).await.unwrap();
    registry.heartbeat("idle", 0.05).await.unwrap();

    // Should pick "idle" (lowest load)
    let best = registry.best_worker_for("tools").await.unwrap();
    assert_eq!(best.name, "idle");

    // After idle becomes busy
    registry.heartbeat("idle", 0.99).await.unwrap();
    let best = registry.best_worker_for("tools").await.unwrap();
    assert_eq!(best.name, "medium");

    // No worker with "llm" capability
    assert!(registry.best_worker_for("llm").await.is_none());

    println!("[V5 PASS] Load balancing picks least loaded worker, handles missing capabilities");
}

// ── V6: Metrics Accumulation ────────────────────────────────────────────────

#[tokio::test]
async fn v6_metrics_accumulation() {
    let metrics = ClusterMetrics::new();

    // Simulate 5 successful dispatches
    metrics.record_success("w1", 100).await;
    metrics.record_success("w1", 200).await;
    metrics.record_success("w2", 50).await;
    metrics.record_failure("w2", "connection refused").await;
    metrics.record_success("w1", 300).await;

    let snap = metrics.snapshot().await;
    assert_eq!(snap["dispatch_count"], 5);
    assert_eq!(snap["dispatch_failures"], 1);

    // Per-worker stats
    let w1 = metrics.worker_stats("w1").await.unwrap();
    assert_eq!(w1.tasks_completed, 3);
    assert_eq!(w1.tasks_failed, 0);
    assert_eq!(w1.avg_latency_ms, 200); // (100+200+300)/3

    let w2 = metrics.worker_stats("w2").await.unwrap();
    assert_eq!(w2.tasks_completed, 1);
    assert_eq!(w2.tasks_failed, 1);
    assert_eq!(w2.last_error, Some("connection refused".to_string()));

    // Non-existent worker
    assert!(metrics.worker_stats("w3").await.is_none());

    println!("[V6 PASS] Metrics: 5 dispatches, 1 failure, avg latency tracked per worker");
}

// ── V7: Dispatch Routing Without Workers ────────────────────────────────────

#[tokio::test]
async fn v7_dispatch_errors_without_workers() {
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());
    let hub = ClusterHub::new(registry);

    // Network tool with no workers → error
    let r = hub.dispatch_tool("web_search", json!({"query": "test"})).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("No online workers"));

    // Compute tool with no full workers → error
    let r = hub.dispatch_tool("shell", json!({"command": "echo hi"})).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("No full workers"));

    // Local-only tool → error (should never dispatch)
    let r = hub.dispatch_tool("file_write", json!({})).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().to_string().contains("local-only"));

    println!("[V7 PASS] All dispatch errors correctly reported when no workers available");
}

// ── V8: Multi-Device Cluster Topology ───────────────────────────────────────

#[tokio::test]
async fn v8_multi_device_cluster_topology() {
    // Simulate the full 8-device cluster from the plan
    let registry = Arc::new(ClusterRegistry::new(":memory:").await.unwrap());

    let devices = vec![
        // Hub is "local" (auto-registered)
        ("m1-mac", "10.0.2.1", 7879, vec!["tools", "llm"], "full"),
        ("ayaneo", "100.0.0.20", 7879, vec!["tools"], "full"),
        ("aspire5", "100.0.0.30", 7879, vec!["tools"], "full"),
        ("android-1", "100.0.0.40", 7880, vec!["web_search", "http_request", "email_send"], "light"),
        ("android-2", "100.0.0.41", 7880, vec!["web_search", "http_request", "email_send"], "light"),
        ("iphone", "100.0.0.50", 7880, vec!["web_search", "http_request"], "light"),
        ("ipad", "100.0.0.51", 7880, vec!["web_search", "http_request"], "light"),
    ];

    for (name, host, port, caps, dtype) in &devices {
        let caps: Vec<String> = caps.iter().map(|s| s.to_string()).collect();
        registry.register_full(name, host, *port, &caps, dtype).await.unwrap();
    }

    let all = registry.status().await;
    assert_eq!(all.len(), 8, "7 workers + 1 local hub = 8 total");

    let workers = registry.online_workers().await;
    assert_eq!(workers.len(), 7);

    let full_count = workers.iter().filter(|w| w.device_type == "full").count();
    let light_count = workers.iter().filter(|w| w.device_type == "light").count();
    assert_eq!(full_count, 3, "M1, Ayaneo, Aspire5");
    assert_eq!(light_count, 4, "Android x2, iPhone, iPad");

    // Workers with "tools" capability
    let tool_workers: Vec<_> = workers.iter()
        .filter(|w| w.capabilities.contains(&"tools".to_string()))
        .collect();
    assert_eq!(tool_workers.len(), 3);

    // Workers with web_search
    let web_workers: Vec<_> = workers.iter()
        .filter(|w| w.capabilities.contains(&"web_search".to_string()))
        .collect();
    assert_eq!(web_workers.len(), 4);

    println!("[V8 PASS] 8-device cluster: 1 hub + 3 full + 4 light workers");
}

// ── V9: ClusterConfig TOML Parsing ──────────────────────────────────────────

#[test]
fn v9_cluster_config_parsing() {
    // Hub config
    let hub_toml = r#"
role = "hub"
"#;
    let config: ClusterConfig = toml::from_str(hub_toml).unwrap();
    assert_eq!(config.role, "hub");
    assert!(config.hub_url.is_none());

    // Worker config
    let worker_toml = r#"
role = "worker"
hub_url = "http://10.0.2.1:7878"
node_name = "m1-mac"
"#;
    let config: ClusterConfig = toml::from_str(worker_toml).unwrap();
    assert_eq!(config.role, "worker");
    assert_eq!(config.hub_url.unwrap(), "http://10.0.2.1:7878");
    assert_eq!(config.node_name.unwrap(), "m1-mac");

    // Empty config defaults to hub
    let config: ClusterConfig = toml::from_str("").unwrap();
    assert_eq!(config.role, "hub");

    println!("[V9 PASS] ClusterConfig parses hub, worker, and default modes");
}

// ── V10: Full Cluster Wiring Smoke Test ─────────────────────────────────────

#[tokio::test]
async fn v10_full_cluster_wiring_smoke_test() {
    // Verify: all cluster components can be initialized and wired together
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_string_lossy().to_string();

    // 1. ClusterRegistry
    let registry = Arc::new(ClusterRegistry::new(&db_path).await.unwrap());
    assert!(!registry.status().await.is_empty(), "local node should exist");

    // 2. ClusterHub
    let hub = Arc::new(ClusterHub::new(registry.clone()));

    // 3. Register workers
    registry.register_full("w1", "10.0.0.1", 7879, &["tools".into()], "full").await.unwrap();
    registry.register_full("w2", "10.0.0.2", 7880, &["web_search".into()], "light").await.unwrap();

    // 4. Verify routing
    assert!(hub.should_dispatch("web_search"));
    assert!(!hub.should_dispatch("file_write"));
    assert!(hub.should_dispatch("shell"));

    // 5. AgentRuntime with cluster hub
    let mut runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    runtime.set_cluster_hub(hub.clone());

    // 6. Metrics
    hub.metrics.record_success("w1", 100).await;
    let snap = hub.metrics.snapshot().await;
    assert_eq!(snap["dispatch_count"], 1);

    // 7. Workers list
    let workers = registry.online_workers().await;
    assert_eq!(workers.len(), 2);

    // 8. Best worker for tools
    let best = registry.best_worker_for("tools").await;
    assert!(best.is_some());

    // 9. Staleness (doesn't crash)
    registry.mark_offline_stale(90).await;

    // 10. Health check returns empty (no real workers)
    let health = hub.broadcast_health_check().await;
    assert_eq!(health.len(), 2); // 2 workers checked (both will fail since not running)

    println!("[V10 PASS] Full cluster stack wired: registry → hub → metrics → runtime → dispatch");
}
