//! Ingest: scan a repo's `backlog/*.toml` (skipping `backlog/done/`) into BacklogTasks.
use crate::fleet::types::BacklogTask;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
struct BacklogFile {
    spec: BacklogSpec,
}

#[derive(Debug, Deserialize)]
struct BacklogSpec {
    #[serde(default)]
    component: String,
    #[serde(default)]
    acceptance: String,
    #[serde(default)]
    caps: Vec<String>,
    #[serde(default)]
    max_files: u32,
}

/// Stable, repo-scoped id: FNV-1a 64-bit hex of "repo\0slug".
/// FNV-1a is fixed/toolchain-independent (unlike DefaultHasher), so this PRIMARY KEY
/// stays stable across Rust upgrades and won't re-id existing tasks.
pub fn task_id(repo: &str, slug: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in repo.bytes().chain(std::iter::once(0u8)).chain(slug.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Scan `<repo_root>/backlog/*.toml` (top level only; `done/` is a subdir, skipped).
pub fn scan_repo(repo: &str, repo_root: &Path) -> Result<Vec<BacklogTask>> {
    let bl = repo_root.join("backlog");
    let mut out = Vec::new();
    if !bl.is_dir() {
        return Ok(out);
    }
    for entry in std::fs::read_dir(&bl).with_context(|| format!("read_dir {}", bl.display()))? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("toml") {
            continue; // skips done/ (a dir) and non-toml
        }
        let slug = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let text = std::fs::read_to_string(&path)?;
        let parsed: BacklogFile =
            toml::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
        out.push(BacklogTask {
            task_id: task_id(repo, &slug),
            repo: repo.to_string(),
            slug,
            component: parsed.spec.component,
            acceptance: parsed.spec.acceptance,
            caps: parsed.spec.caps,
            max_files: parsed.spec.max_files,
        });
    }
    out.sort_by(|a, b| a.slug.cmp(&b.slug)); // deterministic order
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_id_is_stable_and_repo_scoped() {
        let a = task_id("spectyn-quant", "add-sma");
        assert_eq!(a, task_id("spectyn-quant", "add-sma")); // stable
        assert_ne!(a, task_id("spectyn-finance", "add-sma")); // repo-scoped
    }

    #[test]
    fn task_id_golden_is_toolchain_stable() {
        // Locks the FNV-1a output so an accidental algorithm change is caught.
        assert_eq!(task_id("spectyn-quant", "add-sma"), "e3594fe796141f4d");
    }

    #[test]
    fn scan_repo_parses_toml_specs() {
        let dir = tempfile::tempdir().unwrap();
        let bl = dir.path().join("backlog");
        std::fs::create_dir_all(&bl).unwrap();
        std::fs::write(
            bl.join("add-sma.toml"),
            "[spec]\ncomponent = \"add SMA indicator\"\nacceptance = \"sma() returns mean\"\ncaps = [\"quant\"]\nmax_files = 3\n",
        )
        .unwrap();
        std::fs::write(bl.join("README.md"), "ignore me").unwrap();

        let tasks = scan_repo("spectyn-quant", dir.path()).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].slug, "add-sma");
        assert_eq!(tasks[0].component, "add SMA indicator");
        assert_eq!(tasks[0].caps, vec!["quant".to_string()]);
        assert_eq!(tasks[0].max_files, 3);
    }

    #[test]
    fn scan_repo_skips_done_dir() {
        let dir = tempfile::tempdir().unwrap();
        let done = dir.path().join("backlog").join("done");
        std::fs::create_dir_all(&done).unwrap();
        std::fs::write(done.join("finished.toml"), "[spec]\ncomponent = \"x\"\n").unwrap();
        let tasks = scan_repo("spectyn-quant", dir.path()).unwrap();
        assert!(tasks.is_empty(), "done/ items must not be ingested");
    }
}
