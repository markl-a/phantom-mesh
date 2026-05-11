use std::path::Path;

use anyhow::Result;
use pm_types::{Workspace, WorkspaceId};

use super::registry::WorkspaceRegistry;

/// Resolves and (if needed) creates workspaces from cwd paths.
#[derive(Clone)]
pub struct WorkspaceResolver {
    registry: WorkspaceRegistry,
}

impl WorkspaceResolver {
    pub fn new(registry: WorkspaceRegistry) -> Self {
        Self { registry }
    }

    /// Resolve a workspace for the given cwd. If unknown, create it silently.
    pub async fn resolve_or_create(&self, cwd: &Path) -> Result<Workspace> {
        let id = WorkspaceId::from_cwd(cwd)?;
        if let Some(mut existing) = self.registry.get(&id).await? {
            existing.last_used_at = now_millis();
            self.registry.upsert(existing.clone()).await?;
            return Ok(existing);
        }
        let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let now = now_millis();
        let ws = Workspace {
            id,
            name: None,
            root: canonical,
            created_at: now,
            last_used_at: now,
            project_type: None,
            tags: vec![],
        };
        tracing::info!(
            workspace_id = %ws.id,
            root = %ws.root.display(),
            "new workspace created"
        );
        self.registry.upsert(ws.clone()).await?;
        Ok(ws)
    }

    /// Resolve without creating. Returns None if unknown.
    pub async fn get_by_id(&self, id: &WorkspaceId) -> Result<Option<Workspace>> {
        self.registry.get(id).await
    }

    /// List all workspaces (LRU by last_used_at).
    pub async fn list(&self) -> Result<Vec<Workspace>> {
        self.registry.list().await
    }

    pub fn registry(&self) -> &WorkspaceRegistry {
        &self.registry
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn resolve_creates_then_reuses() {
        let dir = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let registry = WorkspaceRegistry::open_at(db_dir.path().join("t.db")).unwrap();
        let resolver = WorkspaceResolver::new(registry);

        let ws1 = resolver.resolve_or_create(dir.path()).await.unwrap();
        let ws2 = resolver.resolve_or_create(dir.path()).await.unwrap();
        assert_eq!(ws1.id, ws2.id);
        // second resolve should bump last_used_at (may be same millis though)
        assert!(ws2.last_used_at >= ws1.last_used_at);

        let all = resolver.list().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn resolve_different_paths_are_distinct() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let registry = WorkspaceRegistry::open_at(db_dir.path().join("t.db")).unwrap();
        let resolver = WorkspaceResolver::new(registry);

        let a = resolver.resolve_or_create(dir_a.path()).await.unwrap();
        let b = resolver.resolve_or_create(dir_b.path()).await.unwrap();
        assert_ne!(a.id, b.id);
    }
}
