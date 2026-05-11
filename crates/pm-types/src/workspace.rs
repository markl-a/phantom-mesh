use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Workspace identifier — 16-char hex string derived from `fnv1a_64(canonical_cwd)`.
/// Acts as the scope for all phantom-mesh data (tasks, memory, sessions, config).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct WorkspaceId(pub String);

impl WorkspaceId {
    /// Derive a workspace id from a directory. Canonicalizes so symlinks / `..`
    /// resolve consistently, then hashes the resulting path bytes with FNV-1a 64.
    pub fn from_cwd(cwd: &Path) -> std::io::Result<Self> {
        let canonical = cwd.canonicalize()?;
        let bytes = canonical.as_os_str().as_encoded_bytes();
        let mut hasher = fnv::FnvHasher::default();
        std::hash::Hasher::write(&mut hasher, bytes);
        let hash = std::hash::Hasher::finish(&hasher);
        Ok(Self(format!("{:016x}", hash)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkspaceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Full workspace record. Persisted in SQLite; in-memory copies share this shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: Option<String>,
    pub root: PathBuf,
    pub created_at: i64,
    pub last_used_at: i64,
    pub project_type: Option<String>,
    pub tags: Vec<String>,
}

impl Workspace {
    /// Default display name: explicit `name`, else the cwd's basename, else the id.
    pub fn display_name(&self) -> String {
        if let Some(name) = &self.name {
            return name.clone();
        }
        self.root
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.id.0.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_id_is_deterministic() {
        let tmp = std::env::temp_dir();
        let a = WorkspaceId::from_cwd(&tmp).unwrap();
        let b = WorkspaceId::from_cwd(&tmp).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.0.len(), 16);
    }

    #[test]
    fn workspace_id_differs_across_paths() {
        let tmp1 = std::env::temp_dir();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        if tmp1 == home {
            return; // skip weird env
        }
        let a = WorkspaceId::from_cwd(&tmp1).unwrap();
        let b = WorkspaceId::from_cwd(&home).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn display_name_falls_back_to_basename() {
        let ws = Workspace {
            id: WorkspaceId("abc".into()),
            name: None,
            root: PathBuf::from("/foo/bar"),
            created_at: 0,
            last_used_at: 0,
            project_type: None,
            tags: vec![],
        };
        assert_eq!(ws.display_name(), "bar");
    }
}
