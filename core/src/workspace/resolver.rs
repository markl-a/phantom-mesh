use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use pm_types::{Workspace, WorkspaceId};

use super::registry::WorkspaceRegistry;
use crate::clock::{Clock, SystemClock};

/// Resolves and (if needed) creates workspaces from cwd paths.
#[derive(Clone)]
pub struct WorkspaceResolver {
    registry: WorkspaceRegistry,
    /// Time source for `created_at` / `last_used_at`. Production uses the real
    /// wall clock; tests inject a `MockClock` so LRU-by-`last_used_at` ordering
    /// is deterministic (no wall-clock dependence / same-millisecond flake).
    clock: Arc<dyn Clock>,
}

impl WorkspaceResolver {
    /// Resolver backed by the real wall clock (production default).
    pub fn new(registry: WorkspaceRegistry) -> Self {
        Self::with_clock(registry, Arc::new(SystemClock))
    }

    /// Resolver with an injected clock — tests pass a `MockClock` to pin/advance
    /// "now" and assert on `created_at` / `last_used_at` deterministically.
    pub fn with_clock(registry: WorkspaceRegistry, clock: Arc<dyn Clock>) -> Self {
        Self { registry, clock }
    }

    /// Resolve a workspace for the given cwd. If unknown, create it silently.
    pub async fn resolve_or_create(&self, cwd: &Path) -> Result<Workspace> {
        let id = WorkspaceId::from_cwd(cwd)?;
        if let Some(mut existing) = self.registry.get(&id).await? {
            existing.last_used_at = self.now_millis();
            self.registry.upsert(existing.clone()).await?;
            return Ok(existing);
        }
        let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
        let now = self.now_millis();
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

    /// Epoch milliseconds from the injected clock, as the `i64` the `Workspace`
    /// timestamps use. `SystemClock` reproduces the previous free-function
    /// behavior exactly (`SystemTime::now() - UNIX_EPOCH`, saturating to 0).
    fn now_millis(&self) -> i64 {
        self.clock.now_ms() as i64
    }
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
    async fn last_used_at_strictly_advances_with_the_clock() {
        // With an injected clock we can prove the LRU bump is monotonic without a
        // wall-clock dependency — closing the "may be same millis though" gap the
        // sibling test has to tolerate.
        use crate::clock::MockClock;
        let dir = tempdir().unwrap();
        let db_dir = tempdir().unwrap();
        let registry = WorkspaceRegistry::open_at(db_dir.path().join("t.db")).unwrap();
        let clock = Arc::new(MockClock::new(1_000_000));
        let resolver = WorkspaceResolver::with_clock(registry, clock.clone());

        let ws1 = resolver.resolve_or_create(dir.path()).await.unwrap();
        assert_eq!(ws1.created_at, 1_000_000);
        assert_eq!(ws1.last_used_at, 1_000_000);

        clock.advance_ms(5_000); // 5s later, deterministically
        let ws2 = resolver.resolve_or_create(dir.path()).await.unwrap();
        assert_eq!(ws1.id, ws2.id, "same cwd → same workspace");
        assert_eq!(
            ws2.last_used_at, 1_005_000,
            "re-resolve stamps last_used_at from the (advanced) clock"
        );
        assert!(
            ws2.last_used_at > ws1.last_used_at,
            "LRU timestamp strictly advances"
        );
        assert_eq!(
            ws2.created_at, 1_000_000,
            "created_at is preserved, not re-stamped"
        );
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
