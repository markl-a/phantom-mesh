use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use pm_types::{Workspace, WorkspaceId};
use rusqlite::{params, Connection};
use tokio::sync::Mutex;

/// SQLite-backed registry of known workspaces. Thread-safe via async Mutex wrap.
#[derive(Clone)]
pub struct WorkspaceRegistry {
    conn: Arc<Mutex<Connection>>,
    db_path: PathBuf,
}

impl WorkspaceRegistry {
    /// Open (or create) the SQLite registry at `~/.phantom-mesh/phantom.db`.
    pub fn open_default() -> Result<Self> {
        let dir = crate::cli_config::phantom_data_dir()?;
        std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        let db_path = dir.join("phantom.db");
        Self::open_at(db_path)
    }

    pub fn open_at(db_path: PathBuf) -> Result<Self> {
        let conn = Connection::open(&db_path)
            .with_context(|| format!("open sqlite at {}", db_path.display()))?;
        Self::init_schema(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            db_path,
        })
    }

    fn init_schema(conn: &Connection) -> Result<()> {
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS workspaces (
                 id             TEXT PRIMARY KEY,
                 name           TEXT,
                 root           TEXT NOT NULL UNIQUE,
                 created_at     INTEGER NOT NULL,
                 last_used_at   INTEGER NOT NULL,
                 project_type   TEXT,
                 tags           TEXT NOT NULL
             );",
        )?;
        Ok(())
    }

    /// Look up by id. Returns None if the id is unknown.
    pub async fn get(&self, id: &WorkspaceId) -> Result<Option<Workspace>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, root, created_at, last_used_at, project_type, tags \
             FROM workspaces WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id.as_str()])?;
        if let Some(row) = rows.next()? {
            Ok(Some(row_to_workspace(row)?))
        } else {
            Ok(None)
        }
    }

    /// Return all workspaces ordered by last_used_at DESC.
    pub async fn list(&self) -> Result<Vec<Workspace>> {
        let conn = self.conn.lock().await;
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, root, created_at, last_used_at, project_type, tags \
             FROM workspaces ORDER BY last_used_at DESC",
        )?;
        let rows = stmt
            .query_map([], row_to_workspace)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Insert a new workspace record; returns the existing row if one is already present.
    pub async fn upsert(&self, ws: Workspace) -> Result<Workspace> {
        let conn = self.conn.lock().await;
        let existing: Option<String> = conn
            .query_row(
                "SELECT id FROM workspaces WHERE id = ?1",
                params![ws.id.as_str()],
                |r| r.get::<_, String>(0),
            )
            .ok();

        if existing.is_some() {
            conn.execute(
                "UPDATE workspaces SET last_used_at = ?1 WHERE id = ?2",
                params![ws.last_used_at, ws.id.as_str()],
            )?;
        } else {
            conn.execute(
                "INSERT INTO workspaces (id, name, root, created_at, last_used_at, project_type, tags) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    ws.id.as_str(),
                    ws.name,
                    ws.root.to_string_lossy(),
                    ws.created_at,
                    ws.last_used_at,
                    ws.project_type,
                    serde_json::to_string(&ws.tags).unwrap_or_else(|_| "[]".into()),
                ],
            )?;
        }
        Ok(ws)
    }

    /// Update display name.
    pub async fn rename(&self, id: &WorkspaceId, name: Option<String>) -> Result<()> {
        let conn = self.conn.lock().await;
        conn.execute(
            "UPDATE workspaces SET name = ?1 WHERE id = ?2",
            params![name, id.as_str()],
        )?;
        Ok(())
    }

    /// Add a tag; idempotent.
    pub async fn add_tag(&self, id: &WorkspaceId, tag: &str) -> Result<()> {
        let conn = self.conn.lock().await;
        let current: String = conn.query_row(
            "SELECT tags FROM workspaces WHERE id = ?1",
            params![id.as_str()],
            |r| r.get(0),
        )?;
        let mut tags: Vec<String> = serde_json::from_str(&current).unwrap_or_default();
        if !tags.iter().any(|t| t == tag) {
            tags.push(tag.to_string());
        }
        conn.execute(
            "UPDATE workspaces SET tags = ?1 WHERE id = ?2",
            params![
                serde_json::to_string(&tags).unwrap_or_else(|_| "[]".into()),
                id.as_str()
            ],
        )?;
        Ok(())
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

fn row_to_workspace(row: &rusqlite::Row<'_>) -> rusqlite::Result<Workspace> {
    let id: String = row.get(0)?;
    let name: Option<String> = row.get(1)?;
    let root: String = row.get(2)?;
    let created_at: i64 = row.get(3)?;
    let last_used_at: i64 = row.get(4)?;
    let project_type: Option<String> = row.get(5)?;
    let tags_str: String = row.get(6)?;
    let tags: Vec<String> = serde_json::from_str(&tags_str).unwrap_or_default();
    Ok(Workspace {
        id: WorkspaceId(id),
        name,
        root: PathBuf::from(root),
        created_at,
        last_used_at,
        project_type,
        tags,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample(id: &str, root: &Path) -> Workspace {
        Workspace {
            id: WorkspaceId(id.into()),
            name: None,
            root: root.to_path_buf(),
            created_at: 1_000,
            last_used_at: 1_000,
            project_type: None,
            tags: vec![],
        }
    }

    #[tokio::test]
    async fn upsert_and_list() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("t.db");
        let reg = WorkspaceRegistry::open_at(db).unwrap();

        let ws = sample("abc", dir.path());
        reg.upsert(ws.clone()).await.unwrap();

        let got = reg.get(&ws.id).await.unwrap().unwrap();
        assert_eq!(got.id, ws.id);

        let all = reg.list().await.unwrap();
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn rename_and_tag() {
        let dir = tempdir().unwrap();
        let db = dir.path().join("t.db");
        let reg = WorkspaceRegistry::open_at(db).unwrap();

        let ws = sample("abc", dir.path());
        reg.upsert(ws.clone()).await.unwrap();

        reg.rename(&ws.id, Some("nice".into())).await.unwrap();
        reg.add_tag(&ws.id, "work").await.unwrap();
        reg.add_tag(&ws.id, "work").await.unwrap(); // idempotent

        let got = reg.get(&ws.id).await.unwrap().unwrap();
        assert_eq!(got.name.as_deref(), Some("nice"));
        assert_eq!(got.tags, vec!["work"]);
    }
}
