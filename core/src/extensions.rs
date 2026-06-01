//! User-side extension loader (CONTRIBUTOR-FUNNEL §4 + SPEC-FREEZE-V1.1 §4.1-b).
//!
//! Folder convention:
//!
//!   ~/.phantom-mesh/extensions/
//!   ├── prompts/  *.md      — user prompt overrides per agent
//!   ├── skills/   *.json    — composite multi-step skill definitions
//!   └── hooks/    *.sh      — pre-tool / post-agent shell hooks
//!
//! v0.1.0 ships READ-ONLY discovery + ensure-dir-exists. Auto-merge of
//! extension content into agent runtime is v0.2 (CO-EVO Phase 1+2).
//! Today's value:
//!
//!   1. `phantom keys init` + boot of `phantom serve` create the dir
//!      tree so users have a documented place to drop customisations.
//!   2. `extensions::summarize()` lists what's there (used by
//!      `/diag` + `phantom doctor`).
//!   3. `phantom upgrade` (v0.2) preserves this dir across binary swap.

use anyhow::Result;
use std::fs;
use std::path::PathBuf;

/// Path to `~/.phantom-mesh/extensions/`.
pub fn extensions_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".phantom-mesh")
        .join("extensions")
}

/// One sub-directory per extension type; returned in load-order
/// preference (prompts override skills override hooks if a name
/// collides — though collisions are unusual).
pub fn subdirs() -> [PathBuf; 3] {
    let root = extensions_dir();
    [
        root.join("prompts"),
        root.join("skills"),
        root.join("hooks"),
    ]
}

/// Idempotently create the extension dir tree.
/// Called at daemon startup + by `phantom keys init` so the layout
/// exists before users start dropping customisations.
pub fn ensure_layout() -> Result<()> {
    for dir in subdirs() {
        fs::create_dir_all(&dir)?;
    }
    Ok(())
}

/// Snapshot of what's currently in the extensions tree, for `phantom
/// doctor` and similar diagnostics. Counts files per subdir.
#[derive(Debug, Clone, Default)]
pub struct Summary {
    pub root: PathBuf,
    pub prompts: usize,
    pub skills: usize,
    pub hooks: usize,
}

impl Summary {
    pub fn total(&self) -> usize {
        self.prompts + self.skills + self.hooks
    }
}

pub fn summarize() -> Summary {
    let mut s = Summary {
        root: extensions_dir(),
        ..Default::default()
    };
    let dirs = subdirs();
    for (i, d) in dirs.iter().enumerate() {
        if let Ok(entries) = fs::read_dir(d) {
            let count = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                .count();
            match i {
                0 => s.prompts = count,
                1 => s.skills = count,
                _ => s.hooks = count,
            }
        }
    }
    s
}

/// List the files in a given category. Used by future Tier 1 loaders
/// (v0.2 — agent runtime reads prompt overrides from
/// extensions/prompts/<agent_name>.md).
pub fn list_files(category: ExtCategory) -> Vec<PathBuf> {
    let dir = match category {
        ExtCategory::Prompts => extensions_dir().join("prompts"),
        ExtCategory::Skills => extensions_dir().join("skills"),
        ExtCategory::Hooks => extensions_dir().join("hooks"),
    };
    fs::read_dir(&dir)
        .ok()
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_file())
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy)]
pub enum ExtCategory {
    Prompts,
    Skills,
    Hooks,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// Test-only helper: counts files in a category given an explicit
    /// root, sidestepping `dirs::home_dir()` so the test doesn't
    /// mutate process-wide $HOME (which races with sibling tests
    /// that resolve checkpoint paths through dirs).
    fn summarize_at(root: &std::path::Path) -> Summary {
        let mut s = Summary {
            root: root.to_path_buf(),
            ..Default::default()
        };
        for (i, sub) in ["prompts", "skills", "hooks"].iter().enumerate() {
            if let Ok(entries) = fs::read_dir(root.join(sub)) {
                let count = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
                    .count();
                match i {
                    0 => s.prompts = count,
                    1 => s.skills = count,
                    _ => s.hooks = count,
                }
            }
        }
        s
    }

    #[test]
    fn layout_helpers_count_files_correctly() {
        // Build a minimal extensions/ tree under a tempdir and verify
        // both the dir layout (mirror of ensure_layout) and the count
        // logic match what `summarize` would produce in production.
        let tmp = tempdir().unwrap();
        let exts = tmp.path().join("extensions");
        for sub in ["prompts", "skills", "hooks"] {
            fs::create_dir_all(exts.join(sub)).unwrap();
        }
        assert!(exts.join("prompts").is_dir());

        let s = summarize_at(&exts);
        assert_eq!(s.prompts, 0);
        assert_eq!(s.skills, 0);
        assert_eq!(s.hooks, 0);
        assert_eq!(s.total(), 0);

        fs::write(exts.join("prompts").join("coder-vim.md"), "# my style\n").unwrap();
        fs::write(exts.join("skills").join("rebase.json"), "{}").unwrap();
        fs::write(exts.join("skills").join("deploy.json"), "{}").unwrap();
        let s = summarize_at(&exts);
        assert_eq!(s.prompts, 1);
        assert_eq!(s.skills, 2);
        assert_eq!(s.hooks, 0);
        assert_eq!(s.total(), 3);
    }
}
