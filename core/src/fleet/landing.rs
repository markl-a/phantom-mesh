//! Tiered landing: satellites auto-push on LGTM; main stages a `fleet/<slug>`
//! review branch and pushes it to origin for review (main itself is never auto-pushed).
use crate::fleet::types::{BacklogTask, Tier};
use anyhow::Result;
use std::path::Path;
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
pub enum Landed {
    Pushed,
    Staged,
}

pub trait GitOps: Send + Sync {
    fn commit_push(&self, repo_root: &Path, msg: &str) -> Result<()>;
    fn stage_branch(&self, repo_root: &Path, branch: &str) -> Result<()>;
}

/// Apply the tiered landing policy. The diff is assumed already present in the worktree.
///
/// - `Tier::Satellite`: commits and pushes directly to `main`.
/// - `Tier::Main`: STAGES a `fleet/<slug>` review branch and PUSHES that branch to
///   origin for review. Main itself is never auto-pushed — the operator merges the
///   review branch. Returns `Landed::Staged` (the work is durable + remotely reviewable).
pub fn land(git: &dyn GitOps, repo_root: &Path, task: &BacklogTask, tier: Tier) -> Result<Landed> {
    match tier {
        Tier::Satellite => {
            let msg = format!(
                "feat({}): {}\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>",
                task.slug, task.component
            );
            git.commit_push(repo_root, &msg)?;
            Ok(Landed::Pushed)
        }
        Tier::Main => {
            git.stage_branch(repo_root, &format!("fleet/{}", task.slug))?;
            Ok(Landed::Staged)
        }
    }
}

/// Real git operations.
pub struct RealGit;
impl GitOps for RealGit {
    fn commit_push(&self, repo_root: &Path, msg: &str) -> Result<()> {
        run(repo_root, &["add", "-A"])?;
        run(repo_root, &["commit", "-m", msg])?;
        run(repo_root, &["push", "origin", "HEAD:main"])?;
        Ok(())
    }
    fn stage_branch(&self, repo_root: &Path, branch: &str) -> Result<()> {
        run(repo_root, &["checkout", "-b", branch])?;
        run(repo_root, &["add", "-A"])?;
        run(
            repo_root,
            &["commit", "-m", "fleet: staged for operator review"],
        )?;
        // Push the REVIEW branch to origin (operator merges it to main — main itself
        // is never auto-pushed). No --force: a re-run of an un-merged spec whose
        // origin/<branch> already exists will push-fail non-FF and the conductor parks
        // the task; that's acceptable — the operator merges/deletes the prior branch first.
        run(repo_root, &["push", "origin", branch])?;
        Ok(())
    }
}

fn run(repo_root: &Path, args: &[&str]) -> Result<()> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo_root)
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::types::{BacklogTask, Tier};
    use std::sync::Mutex;

    #[derive(Default)]
    struct SpyGit {
        calls: Mutex<Vec<String>>,
    }
    impl GitOps for SpyGit {
        fn commit_push(&self, repo_root: &std::path::Path, msg: &str) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("push:{}:{}", repo_root.display(), msg));
            Ok(())
        }
        fn stage_branch(&self, repo_root: &std::path::Path, branch: &str) -> anyhow::Result<()> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("stage:{}:{}", repo_root.display(), branch));
            Ok(())
        }
    }

    fn task(repo: &str) -> BacklogTask {
        BacklogTask {
            task_id: "id".into(),
            repo: repo.into(),
            slug: "x".into(),
            component: "c".into(),
            acceptance: "a".into(),
            caps: vec![],
            max_files: 3,
        }
    }

    #[test]
    fn satellite_pushes() {
        let git = SpyGit::default();
        let outcome = land(
            &git,
            std::path::Path::new("/r"),
            &task("spectyn-quant"),
            Tier::Satellite,
        )
        .unwrap();
        assert_eq!(outcome, Landed::Pushed);
        assert!(git.calls.lock().unwrap()[0].starts_with("push:"));
    }

    #[test]
    fn main_stages_and_pushes_review_branch() {
        // Main tier stages a `fleet/<slug>` review branch. RealGit pushes that branch
        // to origin (NOT to `main`); the SpyGit trait records the single stage_branch
        // call with the review-branch name. Main itself is never auto-pushed.
        let git = SpyGit::default();
        let outcome = land(
            &git,
            std::path::Path::new("/r"),
            &task("spectyn-mesh"),
            Tier::Main,
        )
        .unwrap();
        assert_eq!(outcome, Landed::Staged);
        let calls = git.calls.lock().unwrap();
        // Records the stage call for the task slug's review branch.
        assert!(calls[0].starts_with("stage:"));
        assert!(
            calls[0].ends_with(":fleet/x"),
            "expected the review branch fleet/<slug>, got {}",
            calls[0]
        );
        // Main tier must never push to `main` directly.
        assert!(
            !calls.iter().any(|c| c.contains("HEAD:main")),
            "Main tier must not push to main directly: {calls:?}"
        );
    }
}
