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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_default_local_node() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        let nodes = registry.status().await;
        assert!(!nodes.is_empty());
        assert_eq!(nodes[0].name, "local");
        assert_eq!(nodes[0].status, "online");
        assert!(nodes[0].capabilities.contains(&"llm".to_string()));
        assert!(nodes[0].capabilities.contains(&"tools".to_string()));
        assert_eq!(nodes[0].device_type, "full");
        assert_eq!(nodes[0].cpu_load, 0.0);
    }

    #[tokio::test]
    async fn test_register_node() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("m1", "100.87.93.58", 7878).await.unwrap();
        let nodes = registry.status().await;
        let m1 = nodes.iter().find(|n| n.name == "m1").unwrap();
        assert_eq!(m1.host, "100.87.93.58");
        assert_eq!(m1.status, "online");
    }

    #[tokio::test]
    async fn test_register_idempotent() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("z13", "localhost", 7878).await.unwrap();
        registry.register("z13", "127.0.0.1", 7878).await.unwrap();
        let nodes = registry.status().await;
        let z13_count = nodes.iter().filter(|n| n.name == "z13").count();
        assert_eq!(z13_count, 1); // no duplicates
    }

    #[tokio::test]
    async fn test_register_full() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register_full(
            "android1", "100.0.0.5", 7880,
            &["web_search".to_string(), "http_request".to_string(), "email_send".to_string()],
            "light",
        ).await.unwrap();
        let node = registry.get_node("android1").await.unwrap();
        assert_eq!(node.device_type, "light");
        assert_eq!(node.capabilities.len(), 3);
        assert!(node.capabilities.contains(&"web_search".to_string()));
    }

    #[tokio::test]
    async fn test_heartbeat() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("m1", "100.0.0.2", 7879).await.unwrap();
        registry.heartbeat("m1", 0.75).await.unwrap();
        let node = registry.get_node("m1").await.unwrap();
        assert_eq!(node.cpu_load, 0.75);
        assert_eq!(node.status, "online");
    }

    #[tokio::test]
    async fn test_heartbeat_unknown_node() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        let result = registry.heartbeat("nonexistent", 0.5).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mark_offline_stale() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("stale_node", "100.0.0.9", 7879).await.unwrap();
        // Force an old last_seen
        {
            let conn = registry.conn.lock().unwrap();
            let old_time = (Utc::now() - chrono::Duration::seconds(120)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'stale_node'",
                params![old_time],
            ).unwrap();
        }
        registry.mark_offline_stale(60).await;
        let node = registry.get_node("stale_node").await.unwrap();
        assert_eq!(node.status, "offline");
    }

    #[tokio::test]
    async fn test_online_workers() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("w1", "100.0.0.2", 7879).await.unwrap();
        registry.register("w2", "100.0.0.3", 7880).await.unwrap();
        let workers = registry.online_workers().await;
        assert_eq!(workers.len(), 2); // excludes 'local'
        assert!(workers.iter().all(|w| w.name != "local"));
    }

    #[tokio::test]
    async fn test_best_worker_for() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register_full(
            "w1", "100.0.0.2", 7879,
            &["tools".to_string()],
            "full",
        ).await.unwrap();
        registry.register_full(
            "w2", "100.0.0.3", 7880,
            &["tools".to_string()],
            "full",
        ).await.unwrap();
        registry.heartbeat("w1", 0.8).await.unwrap();
        registry.heartbeat("w2", 0.2).await.unwrap();
        let best = registry.best_worker_for("tools").await.unwrap();
        assert_eq!(best.name, "w2"); // lower CPU load
    }

    #[tokio::test]
    async fn test_best_worker_for_no_match() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register_full(
            "w1", "100.0.0.2", 7879,
            &["tools".to_string()],
            "full",
        ).await.unwrap();
        let best = registry.best_worker_for("llm").await;
        assert!(best.is_none()); // w1 doesn't have "llm" capability
    }

    // ===== Additional tests =====

    #[tokio::test]
    async fn test_get_node_local() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        let node = registry.get_node("local").await;
        assert!(node.is_some());
        let n = node.unwrap();
        assert_eq!(n.name, "local");
        assert_eq!(n.host, "127.0.0.1");
        assert_eq!(n.port, 7878);
    }

    #[tokio::test]
    async fn test_get_node_nonexistent() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        let node = registry.get_node("does-not-exist").await;
        assert!(node.is_none());
    }

    #[tokio::test]
    async fn test_register_updates_host_on_conflict() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("w1", "10.0.0.1", 7879).await.unwrap();
        registry.register("w1", "10.0.0.99", 8080).await.unwrap();
        let node = registry.get_node("w1").await.unwrap();
        assert_eq!(node.host, "10.0.0.99");
        assert_eq!(node.port, 8080);
    }

    #[tokio::test]
    async fn test_online_workers_excludes_offline() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("w1", "10.0.0.2", 7879).await.unwrap();
        registry.register("w2", "10.0.0.3", 7880).await.unwrap();
        // Force w2 to be stale
        {
            let conn = registry.conn.lock().unwrap();
            let old = (Utc::now() - chrono::Duration::seconds(300)).to_rfc3339();
            conn.execute(
                "UPDATE cluster_nodes SET last_seen = ?1 WHERE name = 'w2'",
                params![old],
            ).unwrap();
        }
        registry.mark_offline_stale(60).await;
        let workers = registry.online_workers().await;
        assert_eq!(workers.len(), 1);
        assert_eq!(workers[0].name, "w1");
    }

    #[tokio::test]
    async fn test_status_returns_all_nodes() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register("a", "10.0.0.1", 7879).await.unwrap();
        registry.register("b", "10.0.0.2", 7880).await.unwrap();
        registry.register("c", "10.0.0.3", 7881).await.unwrap();
        let all = registry.status().await;
        // local + a + b + c = 4
        assert_eq!(all.len(), 4);
    }

    #[tokio::test]
    async fn test_register_full_updates_capabilities() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register_full(
            "w1", "10.0.0.2", 7879,
            &["tools".to_string()],
            "full",
        ).await.unwrap();
        // Re-register with different capabilities
        registry.register_full(
            "w1", "10.0.0.2", 7879,
            &["tools".to_string(), "llm".to_string(), "web_search".to_string()],
            "light",
        ).await.unwrap();
        let node = registry.get_node("w1").await.unwrap();
        assert_eq!(node.capabilities.len(), 3);
        assert_eq!(node.device_type, "light");
    }

    #[tokio::test]
    async fn test_best_worker_for_with_multiple_capabilities() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        registry.register_full(
            "w1", "10.0.0.2", 7879,
            &["tools".to_string(), "llm".to_string()],
            "full",
        ).await.unwrap();
        registry.register_full(
            "w2", "10.0.0.3", 7880,
            &["tools".to_string()],
            "light",
        ).await.unwrap();
        registry.heartbeat("w1", 0.5).await.unwrap();
        registry.heartbeat("w2", 0.3).await.unwrap();
        // llm capability only on w1
        let best = registry.best_worker_for("llm").await.unwrap();
        assert_eq!(best.name, "w1");
    }

    #[tokio::test]
    async fn test_mark_offline_stale_preserves_local() {
        let registry = ClusterRegistry::new(":memory:").await.unwrap();
        // Even with 0 timeout, local should remain online
        registry.mark_offline_stale(0).await;
        let local = registry.get_node("local").await.unwrap();
        assert_eq!(local.status, "online");
    }
}
