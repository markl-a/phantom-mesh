use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// A cluster node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterNode {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub status: String,            // "online" | "offline" | "unknown"
    pub models: Vec<String>,       // Ollama models available
    pub last_seen: String,
    pub capabilities: Vec<String>, // ["llm", "tools", "web_only"]
    pub device_type: String,       // "full" | "light"
    pub cpu_load: f32,             // 0.0-1.0, reported by worker
}

/// Cluster node registry backed by SQLite
pub struct ClusterRegistry {
    pub conn: Mutex<Connection>,
}

impl ClusterRegistry {
    pub async fn new(db_path: &str) -> Result<Self> {
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS cluster_nodes (
                name         TEXT PRIMARY KEY,
                host         TEXT NOT NULL,
                port         INTEGER NOT NULL DEFAULT 7878,
                status       TEXT NOT NULL DEFAULT 'unknown',
                models       TEXT NOT NULL DEFAULT '[]',
                last_seen    TEXT NOT NULL,
                capabilities TEXT NOT NULL DEFAULT '[\"tools\"]',
                device_type  TEXT NOT NULL DEFAULT 'full',
                cpu_load     REAL NOT NULL DEFAULT 0.0
            );",
        )?;
        // Schema migration: add new columns to existing tables
        // ALTER TABLE ADD COLUMN is idempotent-safe (fails silently if column exists)
        let _ = conn.execute_batch(
            "ALTER TABLE cluster_nodes ADD COLUMN capabilities TEXT NOT NULL DEFAULT '[\"tools\"]';"
        );
        let _ = conn.execute_batch(
            "ALTER TABLE cluster_nodes ADD COLUMN device_type TEXT NOT NULL DEFAULT 'full';"
        );
        let _ = conn.execute_batch(
            "ALTER TABLE cluster_nodes ADD COLUMN cpu_load REAL NOT NULL DEFAULT 0.0;"
        );
        // Register localhost by default
        let now = Utc::now().to_rfc3339();
        conn.execute(
            "INSERT OR IGNORE INTO cluster_nodes (name, host, port, status, models, last_seen, capabilities, device_type, cpu_load)
             VALUES ('local', '127.0.0.1', 7878, 'online', '[]', ?1, '[\"llm\",\"tools\"]', 'full', 0.0)",
            params![now],
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Get status of all nodes
    pub async fn status(&self) -> Vec<ClusterNode> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name, host, port, status, models, last_seen, capabilities, device_type, cpu_load FROM cluster_nodes")
            .unwrap();
        stmt.query_map([], |row| {
            let models_json: String = row.get(4)?;
            let models: Vec<String> =
                serde_json::from_str(&models_json).unwrap_or_default();
            let caps_json: String = row.get(6)?;
            let capabilities: Vec<String> =
                serde_json::from_str(&caps_json).unwrap_or_default();
            Ok(ClusterNode {
                name: row.get(0)?,
                host: row.get(1)?,
                port: row.get(2)?,
                status: row.get(3)?,
                models,
                last_seen: row.get(5)?,
                capabilities,
                device_type: row.get(7)?,
                cpu_load: row.get(8)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    /// Register or update a node
    pub async fn register(&self, name: &str, host: &str, port: u16) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cluster_nodes (name, host, port, status, models, last_seen, capabilities, device_type, cpu_load)
             VALUES (?1, ?2, ?3, 'online', '[]', ?4, '[\"tools\"]', 'full', 0.0)
             ON CONFLICT(name) DO UPDATE SET host=?2, port=?3, status='online', last_seen=?4",
            params![name, host, port, now],
        )?;
        Ok(())
    }

    /// Register a node with full metadata (capabilities, device_type)
    pub async fn register_full(
        &self,
        name: &str,
        host: &str,
        port: u16,
        capabilities: &[String],
        device_type: &str,
    ) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let caps_json = serde_json::to_string(capabilities)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO cluster_nodes (name, host, port, status, models, last_seen, capabilities, device_type, cpu_load)
             VALUES (?1, ?2, ?3, 'online', '[]', ?4, ?5, ?6, 0.0)
             ON CONFLICT(name) DO UPDATE SET host=?2, port=?3, status='online', last_seen=?4, capabilities=?5, device_type=?6",
            params![name, host, port, now, caps_json, device_type],
        )?;
        Ok(())
    }

    /// Update heartbeat: refresh last_seen and cpu_load for a node
    pub async fn heartbeat(&self, name: &str, cpu_load: f32) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let updated = conn.execute(
            "UPDATE cluster_nodes SET last_seen = ?1, cpu_load = ?2, status = 'online' WHERE name = ?3",
            params![now, cpu_load, name],
        )?;
        if updated == 0 {
            anyhow::bail!("Unknown node: {}", name);
        }
        Ok(())
    }

    /// Mark nodes as offline if they haven't been seen in `timeout_secs` seconds
    pub async fn mark_offline_stale(&self, timeout_secs: i64) {
        let conn = self.conn.lock().unwrap();
        let cutoff = (Utc::now() - chrono::Duration::seconds(timeout_secs)).to_rfc3339();
        let _ = conn.execute(
            "UPDATE cluster_nodes SET status = 'offline' WHERE last_seen < ?1 AND status = 'online' AND name != 'local'",
            params![cutoff],
        );
    }

    /// Get all online worker nodes (excludes 'local' hub)
    pub async fn online_workers(&self) -> Vec<ClusterNode> {
        self.status().await
            .into_iter()
            .filter(|n| n.status == "online" && n.name != "local")
            .collect()
    }

    /// Find the best (least loaded) online worker with a given capability
    pub async fn best_worker_for(&self, capability: &str) -> Option<ClusterNode> {
        let workers = self.online_workers().await;
        workers
            .into_iter()
            .filter(|n| n.capabilities.iter().any(|c| c == capability))
            .min_by(|a, b| a.cpu_load.partial_cmp(&b.cpu_load).unwrap_or(std::cmp::Ordering::Equal))
    }

    /// Get a specific node by name
    pub async fn get_node(&self, name: &str) -> Option<ClusterNode> {
        self.status().await
            .into_iter()
            .find(|n| n.name == name)
    }

    /// Remove a node from the registry. Returns Ok(true) if a node was removed,
    /// Ok(false) if the node did not exist.
    pub async fn deregister(&self, name: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM cluster_nodes WHERE name = ?1",
            params![name],
        )?;
        Ok(deleted > 0)
    }

    /// Return the total number of registered nodes (all statuses).
    pub async fn node_count(&self) -> usize {
        self.status().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ---------------------------------------------------------------
    //  Helper: create a fresh in-memory registry
    // ---------------------------------------------------------------
    async fn fresh_registry() -> ClusterRegistry {
        ClusterRegistry::new(":memory:").await.unwrap()
    }

    // ---------------------------------------------------------------
    //  1. Register a node, verify it appears in list
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_register_node_appears_in_list() {
        let reg = fresh_registry().await;
        reg.register("m1", "10.0.2.1", 7878).await.unwrap();
        let nodes = reg.status().await;
        let m1 = nodes.iter().find(|n| n.name == "m1").unwrap();
        assert_eq!(m1.host, "10.0.2.1");
        assert_eq!(m1.status, "online");
    }

    // ---------------------------------------------------------------
    //  2. Register duplicate node (same name) — should update
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_register_duplicate_updates() {
        let reg = fresh_registry().await;
        reg.register("z13", "localhost", 7878).await.unwrap();
        reg.register("z13", "127.0.0.1", 9999).await.unwrap();
        let nodes = reg.status().await;
        let z13_count = nodes.iter().filter(|n| n.name == "z13").count();
        assert_eq!(z13_count, 1, "no duplicate rows");
        let z13 = nodes.iter().find(|n| n.name == "z13").unwrap();
        assert_eq!(z13.host, "127.0.0.1", "host should be updated");
        assert_eq!(z13.port, 9999, "port should be updated");
    }

    // ---------------------------------------------------------------
    //  3. Deregister a node
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_deregister_node() {
        let reg = fresh_registry().await;
        reg.register("temp", "10.0.0.50", 7879).await.unwrap();
        assert!(reg.get_node("temp").await.is_some());
        let removed = reg.deregister("temp").await.unwrap();
        assert!(removed, "should return true when node existed");
        assert!(reg.get_node("temp").await.is_none(), "node should be gone");
    }

    // ---------------------------------------------------------------
    //  4. Deregister non-existent node (no panic)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_deregister_nonexistent_no_panic() {
        let reg = fresh_registry().await;
        let removed = reg.deregister("ghost").await.unwrap();
        assert!(!removed, "should return false for non-existent node");
    }

    // ---------------------------------------------------------------
    //  5. Heartbeat updates last_seen timestamp
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_heartbeat_updates_last_seen() {
        let reg = fresh_registry().await;
        reg.register("w1", "10.0.0.2", 7879).await.unwrap();
        let before = reg.get_node("w1").await.unwrap().last_seen.clone();
        // Small sleep so the timestamp actually advances
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        reg.heartbeat("w1", 0.4).await.unwrap();
        let after = reg.get_node("w1").await.unwrap().last_seen.clone();
        assert_ne!(before, after, "last_seen should have been refreshed");
        assert_eq!(reg.get_node("w1").await.unwrap().cpu_load, 0.4);
    }

    // ---------------------------------------------------------------
    //  6. Node expiry (last_seen older than timeout -> marked offline)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_node_expiry_marks_offline() {
        let reg = fresh_registry().await;
        reg.register("stale_node", "100.0.0.9", 7879).await.unwrap();
        // Force an old last_seen
        {
            let conn = reg.conn.lock().unwrap();
            let old_time = (Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'stale_node'",
                params![old_time],
            ).unwrap();
        }
        reg.mark_offline_stale(60).await;
        let node = reg.get_node("stale_node").await.unwrap();
        assert_eq!(node.status, "offline");
    }

    // ---------------------------------------------------------------
    //  7. Capacity tracking (cpu_load acts as capacity indicator)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_capacity_tracking_cpu_load() {
        let reg = fresh_registry().await;
        reg.register("w1", "10.0.0.2", 7879).await.unwrap();
        // Initially cpu_load should be 0.0
        assert_eq!(reg.get_node("w1").await.unwrap().cpu_load, 0.0);
        // Update load
        reg.heartbeat("w1", 0.95).await.unwrap();
        assert_eq!(reg.get_node("w1").await.unwrap().cpu_load, 0.95);
        // Update again — should reflect the latest
        reg.heartbeat("w1", 0.1).await.unwrap();
        assert_eq!(reg.get_node("w1").await.unwrap().cpu_load, 0.1);
    }

    // ---------------------------------------------------------------
    //  8. Multiple nodes registration
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_multiple_nodes_registration() {
        let reg = fresh_registry().await;
        for i in 0..10 {
            reg.register(&format!("node-{}", i), &format!("10.0.0.{}", i + 10), 7879 + i)
                .await
                .unwrap();
        }
        let all = reg.status().await;
        // 10 new nodes + 1 default 'local'
        assert_eq!(all.len(), 11);
        for i in 0..10 {
            let name = format!("node-{}", i);
            assert!(all.iter().any(|n| n.name == name), "missing {}", name);
        }
    }

    // ---------------------------------------------------------------
    //  9. Get node by name
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_get_node_by_name() {
        let reg = fresh_registry().await;
        reg.register("target", "192.168.1.42", 8080).await.unwrap();
        let node = reg.get_node("target").await;
        assert!(node.is_some());
        let n = node.unwrap();
        assert_eq!(n.name, "target");
        assert_eq!(n.host, "192.168.1.42");
        assert_eq!(n.port, 8080);
    }

    #[tokio::test]
    async fn test_get_node_nonexistent() {
        let reg = fresh_registry().await;
        assert!(reg.get_node("does-not-exist").await.is_none());
    }

    // ---------------------------------------------------------------
    //  10. List online nodes only
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_list_online_nodes_only() {
        let reg = fresh_registry().await;
        reg.register("online1", "10.0.0.2", 7879).await.unwrap();
        reg.register("online2", "10.0.0.3", 7880).await.unwrap();
        reg.register("will_expire", "10.0.0.4", 7881).await.unwrap();
        // Force will_expire to be stale
        {
            let conn = reg.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::seconds(300)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'will_expire'",
                params![old],
            ).unwrap();
        }
        reg.mark_offline_stale(60).await;
        let workers = reg.online_workers().await;
        assert_eq!(workers.len(), 2);
        assert!(workers.iter().all(|w| w.status == "online"));
        assert!(workers.iter().all(|w| w.name != "will_expire"));
    }

    // ---------------------------------------------------------------
    //  11. List all nodes (online + offline)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_list_all_nodes_includes_offline() {
        let reg = fresh_registry().await;
        reg.register("a", "10.0.0.1", 7879).await.unwrap();
        reg.register("b", "10.0.0.2", 7880).await.unwrap();
        // Force b offline
        {
            let conn = reg.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'b'",
                params![old],
            ).unwrap();
        }
        reg.mark_offline_stale(60).await;
        let all = reg.status().await;
        // local + a + b = 3
        assert_eq!(all.len(), 3);
        let b = all.iter().find(|n| n.name == "b").unwrap();
        assert_eq!(b.status, "offline");
        let a = all.iter().find(|n| n.name == "a").unwrap();
        assert_eq!(a.status, "online");
    }

    // ---------------------------------------------------------------
    //  12. Node capabilities field
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_node_capabilities_field() {
        let reg = fresh_registry().await;
        reg.register_full(
            "android1", "100.0.0.5", 7880,
            &["web_search".to_string(), "http_request".to_string(), "email_send".to_string()],
            "light",
        ).await.unwrap();
        let node = reg.get_node("android1").await.unwrap();
        assert_eq!(node.device_type, "light");
        assert_eq!(node.capabilities.len(), 3);
        assert!(node.capabilities.contains(&"web_search".to_string()));
        assert!(node.capabilities.contains(&"http_request".to_string()));
        assert!(node.capabilities.contains(&"email_send".to_string()));
    }

    #[tokio::test]
    async fn test_capabilities_update_on_reregister() {
        let reg = fresh_registry().await;
        reg.register_full(
            "w1", "10.0.0.2", 7879,
            &["tools".to_string()],
            "full",
        ).await.unwrap();
        reg.register_full(
            "w1", "10.0.0.2", 7879,
            &["tools".to_string(), "llm".to_string(), "web_search".to_string()],
            "light",
        ).await.unwrap();
        let node = reg.get_node("w1").await.unwrap();
        assert_eq!(node.capabilities.len(), 3);
        assert_eq!(node.device_type, "light");
    }

    #[tokio::test]
    async fn test_empty_capabilities() {
        let reg = fresh_registry().await;
        reg.register_full(
            "bare", "10.0.0.2", 7879,
            &[],
            "light",
        ).await.unwrap();
        let node = reg.get_node("bare").await.unwrap();
        assert!(node.capabilities.is_empty());
    }

    // ---------------------------------------------------------------
    //  13. Empty registry (only default 'local' node)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_empty_registry() {
        let reg = fresh_registry().await;
        let all = reg.status().await;
        assert_eq!(all.len(), 1, "only default local node");
        assert_eq!(all[0].name, "local");
    }

    #[tokio::test]
    async fn test_empty_registry_no_online_workers() {
        let reg = fresh_registry().await;
        let workers = reg.online_workers().await;
        assert!(workers.is_empty(), "no workers besides local hub");
    }

    // ---------------------------------------------------------------
    //  14. Node with custom port
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_node_with_custom_port() {
        let reg = fresh_registry().await;
        reg.register("custom-port", "192.168.1.100", 12345).await.unwrap();
        let node = reg.get_node("custom-port").await.unwrap();
        assert_eq!(node.port, 12345);
    }

    #[tokio::test]
    async fn test_node_port_update() {
        let reg = fresh_registry().await;
        reg.register("w1", "10.0.0.1", 7879).await.unwrap();
        assert_eq!(reg.get_node("w1").await.unwrap().port, 7879);
        reg.register("w1", "10.0.0.1", 9090).await.unwrap();
        assert_eq!(reg.get_node("w1").await.unwrap().port, 9090);
    }

    // ---------------------------------------------------------------
    //  15. Node status transitions (online -> offline -> online)
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_status_transition_online_offline_online() {
        let reg = fresh_registry().await;
        reg.register("bounce", "10.0.0.7", 7879).await.unwrap();
        assert_eq!(reg.get_node("bounce").await.unwrap().status, "online");

        // Force offline via stale timestamp
        {
            let conn = reg.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'bounce'",
                params![old],
            ).unwrap();
        }
        reg.mark_offline_stale(60).await;
        assert_eq!(reg.get_node("bounce").await.unwrap().status, "offline");

        // Heartbeat brings it back online
        reg.heartbeat("bounce", 0.1).await.unwrap();
        assert_eq!(reg.get_node("bounce").await.unwrap().status, "online");
    }

    #[tokio::test]
    async fn test_register_revives_offline_node() {
        let reg = fresh_registry().await;
        reg.register("revive", "10.0.0.8", 7879).await.unwrap();
        // Force offline
        {
            let conn = reg.conn.lock().unwrap();
            conn.execute(
                "UPDATE cluster_nodes SET status = 'offline' WHERE name = 'revive'",
                params![],
            ).unwrap();
        }
        assert_eq!(reg.get_node("revive").await.unwrap().status, "offline");
        // Re-register should set it online again
        reg.register("revive", "10.0.0.8", 7879).await.unwrap();
        assert_eq!(reg.get_node("revive").await.unwrap().status, "online");
    }

    // ---------------------------------------------------------------
    //  16. Serialization / deserialization of ClusterNode
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_cluster_node_serialization_roundtrip() {
        let node = ClusterNode {
            name: "test-ser".to_string(),
            host: "10.0.0.1".to_string(),
            port: 7878,
            status: "online".to_string(),
            models: vec!["llama3".to_string(), "mistral".to_string()],
            last_seen: Utc::now().to_rfc3339(),
            capabilities: vec!["llm".to_string(), "tools".to_string()],
            device_type: "full".to_string(),
            cpu_load: 0.42,
        };
        let json = serde_json::to_string(&node).unwrap();
        let deserialized: ClusterNode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "test-ser");
        assert_eq!(deserialized.host, "10.0.0.1");
        assert_eq!(deserialized.port, 7878);
        assert_eq!(deserialized.status, "online");
        assert_eq!(deserialized.models.len(), 2);
        assert_eq!(deserialized.capabilities.len(), 2);
        assert_eq!(deserialized.device_type, "full");
        assert!((deserialized.cpu_load - 0.42).abs() < f32::EPSILON);
    }

    #[tokio::test]
    async fn test_cluster_node_deserialize_from_json_literal() {
        let json = r#"{
            "name": "from-json",
            "host": "192.168.1.1",
            "port": 8080,
            "status": "offline",
            "models": [],
            "last_seen": "2026-01-01T00:00:00+00:00",
            "capabilities": ["web_only"],
            "device_type": "light",
            "cpu_load": 0.0
        }"#;
        let node: ClusterNode = serde_json::from_str(json).unwrap();
        assert_eq!(node.name, "from-json");
        assert_eq!(node.status, "offline");
        assert_eq!(node.capabilities, vec!["web_only"]);
    }

    #[tokio::test]
    async fn test_registry_status_nodes_are_serializable() {
        let reg = fresh_registry().await;
        reg.register("s1", "10.0.0.1", 7879).await.unwrap();
        let nodes = reg.status().await;
        // Serialize the entire Vec<ClusterNode> to JSON — should not panic
        let json = serde_json::to_string(&nodes).unwrap();
        let parsed: Vec<ClusterNode> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), nodes.len());
    }

    // ---------------------------------------------------------------
    //  17. Concurrent access patterns
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_concurrent_registers() {
        let reg = Arc::new(fresh_registry().await);
        let mut handles = Vec::new();
        for i in 0..20 {
            let r = Arc::clone(&reg);
            handles.push(tokio::spawn(async move {
                r.register(
                    &format!("concurrent-{}", i),
                    &format!("10.0.0.{}", i + 10),
                    7879 + i,
                ).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        let all = reg.status().await;
        // 20 concurrent nodes + 1 local
        assert_eq!(all.len(), 21);
    }

    #[tokio::test]
    async fn test_concurrent_heartbeats() {
        let reg = Arc::new(fresh_registry().await);
        // Pre-register nodes
        for i in 0..10 {
            reg.register(&format!("hb-{}", i), &format!("10.0.0.{}", i + 10), 7879)
                .await
                .unwrap();
        }
        // Send concurrent heartbeats
        let mut handles = Vec::new();
        for i in 0..10 {
            let r = Arc::clone(&reg);
            let load = i as f32 / 10.0;
            handles.push(tokio::spawn(async move {
                r.heartbeat(&format!("hb-{}", i), load).await.unwrap();
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // Verify all heartbeats landed
        for i in 0..10 {
            let node = reg.get_node(&format!("hb-{}", i)).await.unwrap();
            assert_eq!(node.status, "online");
            let expected_load = i as f32 / 10.0;
            assert!((node.cpu_load - expected_load).abs() < f32::EPSILON);
        }
    }

    #[tokio::test]
    async fn test_concurrent_reads_and_writes() {
        let reg = Arc::new(fresh_registry().await);
        reg.register("rw-target", "10.0.0.1", 7879).await.unwrap();
        let mut handles = Vec::new();
        // Spawn readers
        for _ in 0..10 {
            let r = Arc::clone(&reg);
            handles.push(tokio::spawn(async move {
                let _ = r.status().await;
                let _ = r.online_workers().await;
                let _ = r.get_node("rw-target").await;
            }));
        }
        // Spawn writers
        for i in 0..10 {
            let r = Arc::clone(&reg);
            handles.push(tokio::spawn(async move {
                let load = i as f32 / 10.0;
                let _ = r.heartbeat("rw-target", load).await;
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
        // Should not deadlock or panic; node should still exist
        assert!(reg.get_node("rw-target").await.is_some());
    }

    // ---------------------------------------------------------------
    //  18. Node count / stats
    // ---------------------------------------------------------------
    #[tokio::test]
    async fn test_node_count_initial() {
        let reg = fresh_registry().await;
        assert_eq!(reg.node_count().await, 1, "only default local");
    }

    #[tokio::test]
    async fn test_node_count_after_adds() {
        let reg = fresh_registry().await;
        reg.register("a", "10.0.0.1", 7879).await.unwrap();
        reg.register("b", "10.0.0.2", 7880).await.unwrap();
        assert_eq!(reg.node_count().await, 3); // local + a + b
    }

    #[tokio::test]
    async fn test_node_count_after_deregister() {
        let reg = fresh_registry().await;
        reg.register("tmp", "10.0.0.1", 7879).await.unwrap();
        assert_eq!(reg.node_count().await, 2);
        reg.deregister("tmp").await.unwrap();
        assert_eq!(reg.node_count().await, 1);
    }

    #[tokio::test]
    async fn test_stats_online_vs_total() {
        let reg = fresh_registry().await;
        reg.register("on1", "10.0.0.1", 7879).await.unwrap();
        reg.register("on2", "10.0.0.2", 7880).await.unwrap();
        reg.register("off1", "10.0.0.3", 7881).await.unwrap();
        // Force off1 offline
        {
            let conn = reg.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'off1'",
                params![old],
            ).unwrap();
        }
        reg.mark_offline_stale(60).await;
        let total = reg.node_count().await;
        let online_workers = reg.online_workers().await.len();
        assert_eq!(total, 4); // local + on1 + on2 + off1
        assert_eq!(online_workers, 2); // on1 + on2 (local excluded from workers)
    }

    // ---------------------------------------------------------------
    //  Extra: Original tests preserved for completeness
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_default_local_node() {
        let reg = fresh_registry().await;
        let nodes = reg.status().await;
        assert!(!nodes.is_empty());
        let local = nodes.iter().find(|n| n.name == "local").unwrap();
        assert_eq!(local.status, "online");
        assert!(local.capabilities.contains(&"llm".to_string()));
        assert!(local.capabilities.contains(&"tools".to_string()));
        assert_eq!(local.device_type, "full");
        assert_eq!(local.cpu_load, 0.0);
    }

    #[tokio::test]
    async fn test_heartbeat_basic() {
        let reg = fresh_registry().await;
        reg.register("m1", "100.0.0.2", 7879).await.unwrap();
        reg.heartbeat("m1", 0.75).await.unwrap();
        let node = reg.get_node("m1").await.unwrap();
        assert_eq!(node.cpu_load, 0.75);
        assert_eq!(node.status, "online");
    }

    #[tokio::test]
    async fn test_heartbeat_unknown_node() {
        let reg = fresh_registry().await;
        let result = reg.heartbeat("nonexistent", 0.5).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_online_workers_excludes_local() {
        let reg = fresh_registry().await;
        reg.register("w1", "100.0.0.2", 7879).await.unwrap();
        reg.register("w2", "100.0.0.3", 7880).await.unwrap();
        let workers = reg.online_workers().await;
        assert_eq!(workers.len(), 2);
        assert!(workers.iter().all(|w| w.name != "local"));
    }

    #[tokio::test]
    async fn test_best_worker_for_lowest_load() {
        let reg = fresh_registry().await;
        reg.register_full("w1", "100.0.0.2", 7879, &["tools".to_string()], "full").await.unwrap();
        reg.register_full("w2", "100.0.0.3", 7880, &["tools".to_string()], "full").await.unwrap();
        reg.heartbeat("w1", 0.8).await.unwrap();
        reg.heartbeat("w2", 0.2).await.unwrap();
        let best = reg.best_worker_for("tools").await.unwrap();
        assert_eq!(best.name, "w2");
    }

    #[tokio::test]
    async fn test_best_worker_for_no_match() {
        let reg = fresh_registry().await;
        reg.register_full("w1", "100.0.0.2", 7879, &["tools".to_string()], "full").await.unwrap();
        let best = reg.best_worker_for("llm").await;
        assert!(best.is_none());
    }

    #[tokio::test]
    async fn test_best_worker_for_with_multiple_capabilities() {
        let reg = fresh_registry().await;
        reg.register_full("w1", "10.0.0.2", 7879, &["tools".to_string(), "llm".to_string()], "full").await.unwrap();
        reg.register_full("w2", "10.0.0.3", 7880, &["tools".to_string()], "light").await.unwrap();
        reg.heartbeat("w1", 0.5).await.unwrap();
        reg.heartbeat("w2", 0.3).await.unwrap();
        let best = reg.best_worker_for("llm").await.unwrap();
        assert_eq!(best.name, "w1");
    }

    #[tokio::test]
    async fn test_get_node_local() {
        let reg = fresh_registry().await;
        let node = reg.get_node("local").await;
        assert!(node.is_some());
        let n = node.unwrap();
        assert_eq!(n.name, "local");
        assert_eq!(n.host, "127.0.0.1");
        assert_eq!(n.port, 7878);
    }

    #[tokio::test]
    async fn test_mark_offline_stale_preserves_local() {
        let reg = fresh_registry().await;
        reg.mark_offline_stale(0).await;
        let local = reg.get_node("local").await.unwrap();
        assert_eq!(local.status, "online");
    }

    // ---------------------------------------------------------------
    //  Bonus: edge-case tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn test_deregister_local_node() {
        let reg = fresh_registry().await;
        // Should be possible (no special protection in deregister)
        let removed = reg.deregister("local").await.unwrap();
        assert!(removed);
        assert!(reg.get_node("local").await.is_none());
        assert_eq!(reg.node_count().await, 0);
    }

    #[tokio::test]
    async fn test_models_field_default_empty() {
        let reg = fresh_registry().await;
        reg.register("w1", "10.0.0.1", 7879).await.unwrap();
        let node = reg.get_node("w1").await.unwrap();
        assert!(node.models.is_empty(), "default register sets models to []");
    }

    #[tokio::test]
    async fn test_cpu_load_boundary_values() {
        let reg = fresh_registry().await;
        reg.register("w1", "10.0.0.1", 7879).await.unwrap();

        reg.heartbeat("w1", 0.0).await.unwrap();
        assert_eq!(reg.get_node("w1").await.unwrap().cpu_load, 0.0);

        reg.heartbeat("w1", 1.0).await.unwrap();
        assert_eq!(reg.get_node("w1").await.unwrap().cpu_load, 1.0);
    }

    #[tokio::test]
    async fn test_best_worker_excludes_offline() {
        let reg = fresh_registry().await;
        reg.register_full("w1", "10.0.0.2", 7879, &["tools".to_string()], "full").await.unwrap();
        reg.register_full("w2", "10.0.0.3", 7880, &["tools".to_string()], "full").await.unwrap();
        reg.heartbeat("w1", 0.9).await.unwrap();
        reg.heartbeat("w2", 0.1).await.unwrap();
        // Force w2 (the lowest load) offline
        {
            let conn = reg.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::seconds(600)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'w2'",
                params![old],
            ).unwrap();
        }
        reg.mark_offline_stale(60).await;
        // best_worker_for uses online_workers which filters offline
        let best = reg.best_worker_for("tools").await.unwrap();
        assert_eq!(best.name, "w1", "offline w2 should be excluded even though it has lower load");
    }

    #[tokio::test]
    async fn test_deregister_then_reregister() {
        let reg = fresh_registry().await;
        reg.register("ephemeral", "10.0.0.1", 7879).await.unwrap();
        reg.deregister("ephemeral").await.unwrap();
        assert!(reg.get_node("ephemeral").await.is_none());
        // Re-register same name
        reg.register("ephemeral", "10.0.0.2", 8080).await.unwrap();
        let node = reg.get_node("ephemeral").await.unwrap();
        assert_eq!(node.host, "10.0.0.2");
        assert_eq!(node.port, 8080);
        assert_eq!(node.status, "online");
    }

    #[tokio::test]
    async fn test_mark_offline_stale_only_affects_online_nodes() {
        let reg = fresh_registry().await;
        reg.register("already_off", "10.0.0.1", 7879).await.unwrap();
        // Manually set to offline with a fresh timestamp
        {
            let conn = reg.conn.lock().unwrap();
            conn.execute(
                "UPDATE cluster_nodes SET status = 'offline' WHERE name = 'already_off'",
                params![],
            ).unwrap();
        }
        // mark_offline_stale should not re-process already offline nodes
        // (the SQL WHERE clause filters on status = 'online')
        reg.mark_offline_stale(0).await;
        let node = reg.get_node("already_off").await.unwrap();
        assert_eq!(node.status, "offline");
    }
}
