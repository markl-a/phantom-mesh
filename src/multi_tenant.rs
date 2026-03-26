//! Multi-Tenant Isolation System — manages tenants with isolated workspaces,
//! API key authentication, and tier-based access control.
//! Persisted to SQLite (~/.phantom-mesh/tenants.db).

use anyhow::{bail, Result};
use chrono::Utc;
use rand::Rng;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tracing::{debug, info, warn};

use crate::service_tier::ServiceTier;

// ── Tenant Struct ────────────────────────────────────────────────────────────

/// A tenant in the multi-tenant system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: String,
    pub name: String,
    pub api_key: String,
    pub tier: ServiceTier,
    pub created_at: String,
    pub active: bool,
    pub workspace_path: String,
    pub settings: serde_json::Value,
}

// ── API Key Generation ───────────────────────────────────────────────────────

/// Generate a new API key in the format "ctx_" + 32-char random hex
fn generate_api_key() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
    format!("ctx_{}", hex::encode(bytes))
}

// ── Tenant Manager ──────────────────────────────────────────────────────────

/// Manages multi-tenant isolation with SQLite persistence.
/// Each tenant gets an isolated workspace directory, unique API key, and tier assignment.
pub struct TenantManager {
    conn: Mutex<Connection>,
    base_dir: String,
}

impl TenantManager {
    /// Create a new TenantManager backed by the given SQLite path.
    /// `base_dir` is the parent directory for tenant workspaces (e.g. ~/.phantom-mesh/tenants).
    pub fn new(db_path: &str, base_dir: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tenants (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                api_key TEXT NOT NULL UNIQUE,
                tier TEXT NOT NULL DEFAULT 'lite',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                active INTEGER NOT NULL DEFAULT 1,
                workspace_path TEXT NOT NULL,
                settings_json TEXT NOT NULL DEFAULT '{}'
            );
            CREATE INDEX IF NOT EXISTS idx_tenants_api_key ON tenants(api_key);
            CREATE INDEX IF NOT EXISTS idx_tenants_active ON tenants(active);"
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
            base_dir: base_dir.to_string(),
        })
    }

    /// Create a new tenant with the given name and tier.
    /// Auto-generates a unique ID, API key, and workspace directory.
    pub fn create_tenant(&self, name: &str, tier: ServiceTier) -> Result<Tenant> {
        let id = uuid::Uuid::new_v4().to_string();
        let api_key = generate_api_key();
        let workspace_path = format!("{}/{}/workspace", self.base_dir, id);
        let created_at = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();

        // Create workspace directory
        if let Err(e) = std::fs::create_dir_all(&workspace_path) {
            warn!("Failed to create tenant workspace '{}': {}", workspace_path, e);
            // Continue anyway — directory can be created later
        }

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO tenants (id, name, api_key, tier, created_at, active, workspace_path, settings_json)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, '{}')",
            params![id, name, api_key, tier.to_string(), created_at, workspace_path],
        )?;

        info!("Created tenant '{}' (id={}, tier={})", name, id, tier);

        Ok(Tenant {
            id,
            name: name.to_string(),
            api_key,
            tier,
            created_at,
            active: true,
            workspace_path,
            settings: serde_json::json!({}),
        })
    }

    /// Get a tenant by ID
    pub fn get_tenant(&self, id: &str) -> Option<Tenant> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, api_key, tier, created_at, active, workspace_path, settings_json
             FROM tenants WHERE id = ?1",
            params![id],
            |row| Self::row_to_tenant(row),
        )
        .ok()
    }

    /// Get a tenant by API key
    pub fn get_by_api_key(&self, key: &str) -> Option<Tenant> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, api_key, tier, created_at, active, workspace_path, settings_json
             FROM tenants WHERE api_key = ?1",
            params![key],
            |row| Self::row_to_tenant(row),
        )
        .ok()
    }

    /// List all tenants (active and inactive)
    pub fn list_tenants(&self) -> Vec<Tenant> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT id, name, api_key, tier, created_at, active, workspace_path, settings_json
                 FROM tenants ORDER BY created_at DESC",
            )
            .unwrap();
        stmt.query_map([], |row| Self::row_to_tenant(row))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect()
    }

    /// Update the tier for a tenant
    pub fn update_tier(&self, id: &str, tier: ServiceTier) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tenants SET tier = ?1 WHERE id = ?2",
            params![tier.to_string(), id],
        )?;
        if affected == 0 {
            bail!("Tenant '{}' not found", id);
        }
        info!("Updated tier for tenant '{}' to '{}'", id, tier);
        Ok(())
    }

    /// Deactivate a tenant (soft delete)
    pub fn deactivate_tenant(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tenants SET active = 0 WHERE id = ?1",
            params![id],
        )?;
        if affected == 0 {
            bail!("Tenant '{}' not found", id);
        }
        info!("Deactivated tenant '{}'", id);
        Ok(())
    }

    /// Validate an API key and return the tenant if active.
    /// Returns None if the key is invalid or the tenant is deactivated.
    pub fn validate_api_key(&self, key: &str) -> Option<Tenant> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT id, name, api_key, tier, created_at, active, workspace_path, settings_json
             FROM tenants WHERE api_key = ?1 AND active = 1",
            params![key],
            |row| Self::row_to_tenant(row),
        )
        .ok()
    }

    /// Update tenant settings (JSON merge)
    pub fn update_settings(&self, id: &str, settings: serde_json::Value) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let settings_str = serde_json::to_string(&settings)?;
        let affected = conn.execute(
            "UPDATE tenants SET settings_json = ?1 WHERE id = ?2",
            params![settings_str, id],
        )?;
        if affected == 0 {
            bail!("Tenant '{}' not found", id);
        }
        debug!("Updated settings for tenant '{}'", id);
        Ok(())
    }

    /// Reactivate a previously deactivated tenant
    pub fn reactivate_tenant(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let affected = conn.execute(
            "UPDATE tenants SET active = 1 WHERE id = ?1",
            params![id],
        )?;
        if affected == 0 {
            bail!("Tenant '{}' not found", id);
        }
        info!("Reactivated tenant '{}'", id);
        Ok(())
    }

    /// Count active tenants
    pub fn active_count(&self) -> u32 {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM tenants WHERE active = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
    }

    /// Helper: convert a rusqlite Row to a Tenant
    fn row_to_tenant(row: &rusqlite::Row) -> rusqlite::Result<Tenant> {
        let tier_str: String = row.get(3)?;
        let active_int: i32 = row.get(5)?;
        let settings_str: String = row.get(7)?;
        Ok(Tenant {
            id: row.get(0)?,
            name: row.get(1)?,
            api_key: row.get(2)?,
            tier: ServiceTier::from_str_loose(&tier_str).unwrap_or(ServiceTier::Lite),
            created_at: row.get(4)?,
            active: active_int != 0,
            workspace_path: row.get(6)?,
            settings: serde_json::from_str(&settings_str).unwrap_or(serde_json::json!({})),
        })
    }
}

// ── Middleware Helper ────────────────────────────────────────────────────────

/// Extract tenant API key from request headers.
/// Checks X-API-Key header first, then Authorization: Bearer <key>.
pub fn extract_tenant_key(headers: &axum::http::HeaderMap) -> Option<String> {
    // Try X-API-Key header first
    if let Some(key) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        return Some(key.to_string());
    }
    // Fall back to Authorization: Bearer <key>
    if let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) {
        if let Some(token) = auth.strip_prefix("Bearer ") {
            if token.starts_with("ctx_") {
                return Some(token.to_string());
            }
        }
    }
    None
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_db() -> (String, String) {
        let id = uuid::Uuid::new_v4().to_string();
        let dir = std::env::temp_dir().join(format!("phantom_mesh_tenant_test_{}", &id[..8]));
        let _ = std::fs::create_dir_all(&dir);
        let db_path = dir.join("tenants.db");
        let base_dir = dir.join("tenants");
        let _ = std::fs::create_dir_all(&base_dir);
        (
            db_path.to_string_lossy().to_string(),
            base_dir.to_string_lossy().to_string(),
        )
    }

    fn cleanup(db_path: &str, base_dir: &str) {
        let _ = std::fs::remove_file(db_path);
        let _ = std::fs::remove_dir_all(base_dir);
        // Also clean parent temp dir
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    #[test]
    fn test_create_tenant() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("Acme Corp", ServiceTier::Pro).unwrap();
        assert_eq!(tenant.name, "Acme Corp");
        assert_eq!(tenant.tier, ServiceTier::Pro);
        assert!(tenant.active);
        assert!(tenant.api_key.starts_with("ctx_"));
        assert_eq!(tenant.api_key.len(), 4 + 32); // "ctx_" + 32 hex chars
        assert!(tenant.workspace_path.contains(&tenant.id));
        cleanup(&db, &base);
    }

    #[test]
    fn test_get_tenant_by_id() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("TestCo", ServiceTier::Lite).unwrap();
        let fetched = mgr.get_tenant(&tenant.id).unwrap();
        assert_eq!(fetched.id, tenant.id);
        assert_eq!(fetched.name, "TestCo");
        assert_eq!(fetched.tier, ServiceTier::Lite);
        cleanup(&db, &base);
    }

    #[test]
    fn test_get_tenant_not_found() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        assert!(mgr.get_tenant("nonexistent-id").is_none());
        cleanup(&db, &base);
    }

    #[test]
    fn test_get_by_api_key() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("KeyTest", ServiceTier::Team).unwrap();
        let fetched = mgr.get_by_api_key(&tenant.api_key).unwrap();
        assert_eq!(fetched.id, tenant.id);
        assert_eq!(fetched.tier, ServiceTier::Team);
        cleanup(&db, &base);
    }

    #[test]
    fn test_get_by_api_key_invalid() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        assert!(mgr.get_by_api_key("ctx_invalid_key_12345678901234567").is_none());
        cleanup(&db, &base);
    }

    #[test]
    fn test_list_tenants() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        mgr.create_tenant("Alpha", ServiceTier::Lite).unwrap();
        mgr.create_tenant("Beta", ServiceTier::Pro).unwrap();
        mgr.create_tenant("Gamma", ServiceTier::Team).unwrap();
        let tenants = mgr.list_tenants();
        assert_eq!(tenants.len(), 3);
        let names: Vec<&str> = tenants.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"Alpha"));
        assert!(names.contains(&"Beta"));
        assert!(names.contains(&"Gamma"));
        cleanup(&db, &base);
    }

    #[test]
    fn test_update_tier() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("TierTest", ServiceTier::Lite).unwrap();
        assert_eq!(mgr.get_tenant(&tenant.id).unwrap().tier, ServiceTier::Lite);
        mgr.update_tier(&tenant.id, ServiceTier::Pro).unwrap();
        assert_eq!(mgr.get_tenant(&tenant.id).unwrap().tier, ServiceTier::Pro);
        mgr.update_tier(&tenant.id, ServiceTier::Team).unwrap();
        assert_eq!(mgr.get_tenant(&tenant.id).unwrap().tier, ServiceTier::Team);
        cleanup(&db, &base);
    }

    #[test]
    fn test_update_tier_not_found() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let result = mgr.update_tier("no-such-id", ServiceTier::Pro);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
        cleanup(&db, &base);
    }

    #[test]
    fn test_deactivate_tenant() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("Deactivate", ServiceTier::Lite).unwrap();
        assert!(mgr.get_tenant(&tenant.id).unwrap().active);
        mgr.deactivate_tenant(&tenant.id).unwrap();
        let deactivated = mgr.get_tenant(&tenant.id).unwrap();
        assert!(!deactivated.active);
        cleanup(&db, &base);
    }

    #[test]
    fn test_deactivate_not_found() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let result = mgr.deactivate_tenant("no-such-id");
        assert!(result.is_err());
        cleanup(&db, &base);
    }

    #[test]
    fn test_validate_api_key_active() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("Active", ServiceTier::Pro).unwrap();
        let validated = mgr.validate_api_key(&tenant.api_key);
        assert!(validated.is_some());
        assert_eq!(validated.unwrap().id, tenant.id);
        cleanup(&db, &base);
    }

    #[test]
    fn test_validate_api_key_deactivated() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("WillDeactivate", ServiceTier::Lite).unwrap();
        mgr.deactivate_tenant(&tenant.id).unwrap();
        // Deactivated tenant's key should not validate
        let validated = mgr.validate_api_key(&tenant.api_key);
        assert!(validated.is_none());
        cleanup(&db, &base);
    }

    #[test]
    fn test_validate_api_key_invalid() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        assert!(mgr.validate_api_key("ctx_not_a_real_key_00000000000000").is_none());
        cleanup(&db, &base);
    }

    #[test]
    fn test_api_key_format() {
        let key = generate_api_key();
        assert!(key.starts_with("ctx_"));
        assert_eq!(key.len(), 36); // "ctx_" (4) + 32 hex chars
        // Verify hex portion is valid hex
        let hex_part = &key[4..];
        assert!(hex::decode(hex_part).is_ok());
    }

    #[test]
    fn test_api_key_uniqueness() {
        let key1 = generate_api_key();
        let key2 = generate_api_key();
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_workspace_path_contains_tenant_id() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("PathTest", ServiceTier::Lite).unwrap();
        assert!(tenant.workspace_path.contains(&tenant.id));
        assert!(tenant.workspace_path.ends_with("/workspace"));
        cleanup(&db, &base);
    }

    #[test]
    fn test_reactivate_tenant() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("Reactivate", ServiceTier::Lite).unwrap();
        mgr.deactivate_tenant(&tenant.id).unwrap();
        assert!(!mgr.get_tenant(&tenant.id).unwrap().active);
        mgr.reactivate_tenant(&tenant.id).unwrap();
        assert!(mgr.get_tenant(&tenant.id).unwrap().active);
        // Validate API key should work again
        assert!(mgr.validate_api_key(&tenant.api_key).is_some());
        cleanup(&db, &base);
    }

    #[test]
    fn test_active_count() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        assert_eq!(mgr.active_count(), 0);
        let t1 = mgr.create_tenant("One", ServiceTier::Lite).unwrap();
        mgr.create_tenant("Two", ServiceTier::Pro).unwrap();
        assert_eq!(mgr.active_count(), 2);
        mgr.deactivate_tenant(&t1.id).unwrap();
        assert_eq!(mgr.active_count(), 1);
        cleanup(&db, &base);
    }

    #[test]
    fn test_update_settings() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("Settings", ServiceTier::Pro).unwrap();
        let new_settings = serde_json::json!({
            "max_agents": 5,
            "custom_models": ["gpt-4o", "claude-sonnet-4"],
            "notifications": true
        });
        mgr.update_settings(&tenant.id, new_settings.clone()).unwrap();
        let fetched = mgr.get_tenant(&tenant.id).unwrap();
        assert_eq!(fetched.settings["max_agents"], 5);
        assert_eq!(fetched.settings["notifications"], true);
        cleanup(&db, &base);
    }

    #[test]
    fn test_update_settings_not_found() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let result = mgr.update_settings("no-id", serde_json::json!({}));
        assert!(result.is_err());
        cleanup(&db, &base);
    }

    #[test]
    fn test_extract_tenant_key_xapikey() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-api-key", "ctx_abc123def456".parse().unwrap());
        let key = extract_tenant_key(&headers);
        assert_eq!(key, Some("ctx_abc123def456".to_string()));
    }

    #[test]
    fn test_extract_tenant_key_bearer() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer ctx_abc123def456".parse().unwrap());
        let key = extract_tenant_key(&headers);
        assert_eq!(key, Some("ctx_abc123def456".to_string()));
    }

    #[test]
    fn test_extract_tenant_key_bearer_non_ctx() {
        let mut headers = axum::http::HeaderMap::new();
        // Non-ctx_ Bearer token should NOT be extracted (hub auth token, not tenant key)
        headers.insert("authorization", "Bearer phantom_mesh-hub-2026".parse().unwrap());
        let key = extract_tenant_key(&headers);
        assert!(key.is_none());
    }

    #[test]
    fn test_extract_tenant_key_xapikey_priority() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-api-key", "ctx_from_header".parse().unwrap());
        headers.insert("authorization", "Bearer ctx_from_bearer".parse().unwrap());
        // X-API-Key should take priority
        let key = extract_tenant_key(&headers);
        assert_eq!(key, Some("ctx_from_header".to_string()));
    }

    #[test]
    fn test_extract_tenant_key_none() {
        let headers = axum::http::HeaderMap::new();
        assert!(extract_tenant_key(&headers).is_none());
    }

    #[test]
    fn test_multiple_tenants_unique_keys() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let t1 = mgr.create_tenant("T1", ServiceTier::Lite).unwrap();
        let t2 = mgr.create_tenant("T2", ServiceTier::Lite).unwrap();
        let t3 = mgr.create_tenant("T3", ServiceTier::Lite).unwrap();
        // All keys should be unique
        assert_ne!(t1.api_key, t2.api_key);
        assert_ne!(t2.api_key, t3.api_key);
        assert_ne!(t1.api_key, t3.api_key);
        // Each key should resolve to the correct tenant
        assert_eq!(mgr.get_by_api_key(&t1.api_key).unwrap().id, t1.id);
        assert_eq!(mgr.get_by_api_key(&t2.api_key).unwrap().id, t2.id);
        assert_eq!(mgr.get_by_api_key(&t3.api_key).unwrap().id, t3.id);
        cleanup(&db, &base);
    }

    #[test]
    fn test_tenant_default_settings() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenant = mgr.create_tenant("Defaults", ServiceTier::Lite).unwrap();
        assert_eq!(tenant.settings, serde_json::json!({}));
        cleanup(&db, &base);
    }

    #[test]
    fn test_list_tenants_empty() {
        let (db, base) = temp_db();
        let mgr = TenantManager::new(&db, &base).unwrap();
        let tenants = mgr.list_tenants();
        assert!(tenants.is_empty());
        cleanup(&db, &base);
    }

    #[test]
    fn test_tenant_serialization() {
        let tenant = Tenant {
            id: "test-id".to_string(),
            name: "Test".to_string(),
            api_key: "ctx_0000000000000000000000000000abcd".to_string(),
            tier: ServiceTier::Pro,
            created_at: "2026-03-17T12:00:00Z".to_string(),
            active: true,
            workspace_path: "/tmp/tenants/test-id/workspace".to_string(),
            settings: serde_json::json!({"key": "value"}),
        };
        let json = serde_json::to_string(&tenant).unwrap();
        let back: Tenant = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "test-id");
        assert_eq!(back.name, "Test");
        assert_eq!(back.tier, ServiceTier::Pro);
        assert!(back.active);
        assert_eq!(back.settings["key"], "value");
    }
}
