//! The conductor loop: ties queue + scheduler + executor + gate + landing together.
use crate::fleet::executor::Executor;
use crate::fleet::gate::{review, Reviewer};
use crate::fleet::landing::{land, GitOps, Landed};
use crate::fleet::queue::FleetQueue;
use crate::fleet::types::{BacklogTask, TaskState, Tier, Verdict};
use crate::fleet::Limits;
use anyhow::Result;
use std::path::Path;

/// Borrowed dependencies for one task's processing (keeps the loop testable).
pub struct Deps<'a> {
    pub queue: &'a FleetQueue,
    pub executor: &'a dyn Executor,
    pub reviewer: &'a dyn Reviewer,
    pub git: &'a dyn GitOps,
    pub limits: Limits,
    pub worker: &'a str,
}

const PRIMARY: [&str; 2] = ["codex", "claude"];
const BACKUPS: [&str; 2] = ["opencode", "agy"];

/// Drive one already-claimed task through execute -> gate -> (bounded CHANGES) -> land.
pub async fn process_one(dep: &Deps<'_>, task: &BacklogTask, repo_root: &Path) -> Result<()> {
    // Operate strictly inside the task's own repo checkout — never the process cwd.
    // (Full per-task worktree isolation is a documented live-path follow-up; this at least
    // guarantees the RIGHT repo, so a satellite task can never touch the main checkout.)
    let wt = repo_root;
    loop {
        if !dep
            .queue
            .set_state(&task.task_id, dep.worker, TaskState::Executing)
            .await?
        {
            return Ok(()); // lost ownership (lease reaped + reclaimed elsewhere)
        }
        let outcome = dep.executor.run(task, wt).await?;
        if !outcome.build_ok {
            let round = match dep
                .queue
                .bump_changes_round(&task.task_id, dep.worker)
                .await?
            {
                Some(r) => r,
                None => return Ok(()),
            };
            if round >= dep.limits.max_changes_rounds {
                dep.queue
                    .park(&task.task_id, dep.worker, "build failed (max rounds)")
                    .await?;
                return Ok(());
            }
            continue;
        }

        if !dep
            .queue
            .set_state(&task.task_id, dep.worker, TaskState::Gating)
            .await?
        {
            return Ok(());
        }
        let verdict = review(
            dep.reviewer,
            &outcome.diff,
            &task.component,
            &PRIMARY,
            &BACKUPS,
        )
        .await;
        match verdict {
            Verdict::Lgtm => {
                if !dep
                    .queue
                    .set_state(&task.task_id, dep.worker, TaskState::Landing)
                    .await?
                {
                    return Ok(());
                }
                let tier = Tier::for_repo(&task.repo);
                let landed = land(dep.git, wt, task, tier)?;
                let terminal = match landed {
                    Landed::Pushed => TaskState::Landed,
                    Landed::Staged => TaskState::Staged,
                };
                let _ = dep
                    .queue
                    .complete(&task.task_id, dep.worker, terminal)
                    .await?;
                return Ok(());
            }
            Verdict::Changes(_) => {
                let round = match dep
                    .queue
                    .bump_changes_round(&task.task_id, dep.worker)
                    .await?
                {
                    Some(r) => r,
                    None => return Ok(()),
                };
                if round >= dep.limits.max_changes_rounds {
                    let _ = dep
                        .queue
                        .park(&task.task_id, dep.worker, "max CHANGES rounds")
                        .await?;
                    return Ok(());
                }
                // loop: re-execute for another correction round
            }
            Verdict::Inconclusive => {
                let _ = dep
                    .queue
                    .park(
                        &task.task_id,
                        dep.worker,
                        "gate inconclusive (reviewer flake)",
                    )
                    .await?;
                return Ok(());
            }
        }
    }
}

/// Provides an isolated working directory per task and removes it afterward, so the conductor
/// never mutates the operator's actual repo checkout and concurrent tasks don't collide.
pub trait Workspaces: Send + Sync {
    /// Create an isolated workspace for `task_id` off the repo at `repo_root`; returns its path.
    fn create(&self, repo_root: &Path, task_id: &str) -> Result<std::path::PathBuf>;
    /// Remove the workspace for this task. Best-effort + idempotent — runs on success AND error.
    fn cleanup(&self, repo_root: &Path, workspace: &Path, task_id: &str);
}

/// Real workspaces backed by `git worktree` (one fresh branch + dir per task).
pub struct GitWorktrees;

impl Workspaces for GitWorktrees {
    fn create(&self, repo_root: &Path, task_id: &str) -> Result<std::path::PathBuf> {
        let branch = format!("fleet/wt-{task_id}");
        // Place the worktree OUTSIDE the operator's checkout (under spectyn's data dir) so the
        // live working tree never gains untracked nested-worktree noise.
        let base = crate::cli_config::spectyn_data_dir()?.join("fleet-worktrees");
        std::fs::create_dir_all(&base).ok();
        let wt = base.join(task_id);
        // Idempotently clear any stale leftover (worktree dir AND/OR branch) before recreating.
        crate::fleet::executor::clear_stale_worktree(repo_root, &wt, &branch);
        crate::fleet::executor::worktree_add(repo_root, &wt, &branch)?;
        Ok(wt)
    }
    fn cleanup(&self, repo_root: &Path, workspace: &Path, task_id: &str) {
        let branch = format!("fleet/wt-{task_id}");
        crate::fleet::executor::clear_stale_worktree(repo_root, workspace, &branch);
    }
}

/// Claim-and-process loop for a single worker. Ingests are done by the caller; this repeatedly
/// reaps expired leases, snapshots, picks next, claims (CAS), creates an isolated workspace,
/// processes, cleans up, until no claimable work remains or `max_tasks` is reached. On a
/// processing error the task is parked (never stranded) and its workspace is still cleaned up.
#[allow(clippy::too_many_arguments)]
pub async fn run_forever(
    queue: &FleetQueue,
    executor: &dyn Executor,
    reviewer: &dyn Reviewer,
    git: &dyn GitOps,
    workspaces: &dyn Workspaces,
    repos: &std::collections::HashMap<String, std::path::PathBuf>,
    limits: Limits,
    worker: &str,
    lease_secs: i64,
    max_tasks: usize,
) -> Result<usize> {
    let mut done = 0;
    while done < max_tasks {
        queue.reap_expired().await?;
        let snap = queue.active_snapshot().await?;
        let Some(next_id) = crate::fleet::scheduler::pick_next(&snap, &limits) else {
            break;
        };
        if !queue.claim(&next_id, worker, lease_secs).await? {
            continue; // lost the race
        }
        let row = snap.iter().find(|r| r.task_id == next_id).cloned();
        if let Some(row) = row {
            // Resolve the task's configured checkout. Refuse to run in an arbitrary cwd:
            // an unconfigured repo is parked (never executed/landed in the wrong directory).
            let Some(repo_root) = repos.get(&row.repo).cloned() else {
                queue
                    .park(
                        &row.task_id,
                        worker,
                        &format!("repo '{}' not configured in fleet.toml", row.repo),
                    )
                    .await?;
                done += 1;
                continue;
            };
            // Fetch the FULL persisted task (the snapshot only carries id/repo/slug); without
            // this the executor would receive empty component/acceptance and tell the CLI to
            // implement nothing.
            let Some(task) = queue.get_task(&row.task_id).await? else {
                queue
                    .park(&row.task_id, worker, "task vanished from queue")
                    .await?;
                done += 1;
                continue;
            };
            // Create an isolated workspace for this task — never run in the operator's live
            // checkout. If creation fails, park rather than fall back to the real checkout.
            let workspace = match workspaces.create(&repo_root, &task.task_id) {
                Ok(w) => w,
                Err(e) => {
                    queue
                        .park(
                            &task.task_id,
                            worker,
                            &format!("workspace create failed: {e}"),
                        )
                        .await?;
                    done += 1;
                    continue;
                }
            };
            let dep = Deps {
                queue,
                executor,
                reviewer,
                git,
                limits: limits.clone(),
                worker,
            };
            let result = process_one(&dep, &task, &workspace).await;
            // Always clean up the workspace — on success AND on error — so a failed run never
            // strands a worktree/branch.
            workspaces.cleanup(&repo_root, &workspace, &task.task_id);
            if let Err(e) = result {
                queue
                    .park(&task.task_id, worker, &format!("processing error: {e}"))
                    .await?;
            }
        }
        done += 1;
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::executor::MockExecutor;
    use crate::fleet::landing::GitOps;
    use crate::fleet::queue::FleetQueue;
    use crate::fleet::types::{BacklogTask, ExecOutcome, TaskState};
    use crate::fleet::Limits;
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[derive(Default)]
    struct NoopGit;
    impl GitOps for NoopGit {
        fn commit_push(&self, _: &std::path::Path, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
        fn stage_branch(&self, _: &std::path::Path, _: &str) -> anyhow::Result<()> {
            Ok(())
        }
    }

    /// Records create/cleanup calls and hands back `repo_root` as the workspace (the mock
    /// executor ignores the path), so tests assert the create+cleanup CONTRACT hermetically.
    #[derive(Default)]
    struct MockWorkspaces {
        created: std::sync::Mutex<usize>,
        cleaned: std::sync::Mutex<usize>,
    }
    impl Workspaces for MockWorkspaces {
        fn create(&self, repo_root: &std::path::Path, _id: &str) -> anyhow::Result<PathBuf> {
            *self.created.lock().unwrap() += 1;
            Ok(repo_root.to_path_buf())
        }
        fn cleanup(&self, _repo_root: &std::path::Path, _workspace: &std::path::Path, _id: &str) {
            *self.cleaned.lock().unwrap() += 1;
        }
    }

    fn task(repo: &str, slug: &str) -> BacklogTask {
        BacklogTask {
            task_id: crate::fleet::backlog::task_id(repo, slug),
            repo: repo.into(),
            slug: slug.into(),
            component: "c".into(),
            acceptance: "a".into(),
            caps: vec![],
            max_files: 3,
        }
    }

    #[tokio::test]
    async fn satellite_task_runs_to_landed() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        q.claim(&task("spectyn-quant", "x").task_id, "w1", 60)
            .await
            .unwrap();
        let ex = MockExecutor::new(ExecOutcome {
            diff: "d".into(),
            build_ok: true,
            logs: "".into(),
        });
        let mut scripted = HashMap::new();
        scripted.insert("codex".into(), "VERDICT: LGTM".into());
        scripted.insert("claude".into(), "VERDICT: LGTM".into());
        let r = crate::fleet::gate::MockReviewer::new(scripted);
        let dep = Deps {
            queue: &q,
            executor: &ex,
            reviewer: &r,
            git: &NoopGit,
            limits: Limits::default(),
            worker: "w1",
        };
        process_one(&dep, &task("spectyn-quant", "x"), std::path::Path::new("."))
            .await
            .unwrap();
        assert_eq!(q.list(TaskState::Landed).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn changes_loop_parks_after_max_rounds() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "y")).await.unwrap();
        q.claim(&task("spectyn-quant", "y").task_id, "w1", 60)
            .await
            .unwrap();
        let ex = MockExecutor::new(ExecOutcome {
            diff: "d".into(),
            build_ok: true,
            logs: "".into(),
        });
        let mut scripted = HashMap::new();
        scripted.insert("codex".into(), "VERDICT: CHANGES: never happy".into());
        scripted.insert("claude".into(), "VERDICT: LGTM".into());
        let r = crate::fleet::gate::MockReviewer::new(scripted);
        let limits = Limits {
            max_changes_rounds: 2,
            ..Limits::default()
        };
        let dep = Deps {
            queue: &q,
            executor: &ex,
            reviewer: &r,
            git: &NoopGit,
            limits,
            worker: "w1",
        };
        process_one(&dep, &task("spectyn-quant", "y"), std::path::Path::new("."))
            .await
            .unwrap();
        assert_eq!(q.list(TaskState::Parked).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn run_forever_parks_task_with_unconfigured_repo_instead_of_running_in_cwd() {
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        let ex = MockExecutor::new(ExecOutcome {
            diff: "d".into(),
            build_ok: true,
            logs: "".into(),
        });
        let mut scripted = HashMap::new();
        scripted.insert("codex".into(), "VERDICT: LGTM".into());
        scripted.insert("claude".into(), "VERDICT: LGTM".into());
        let r = crate::fleet::gate::MockReviewer::new(scripted);
        let repos: HashMap<String, PathBuf> = HashMap::new(); // spectyn-quant NOT configured
        let done = run_forever(
            &q,
            &ex,
            &r,
            &NoopGit,
            &MockWorkspaces::default(),
            &repos,
            Limits::default(),
            "w1",
            60,
            5,
        )
        .await
        .unwrap();
        assert_eq!(done, 1, "the one pending task was processed once");
        assert_eq!(
            q.list(TaskState::Parked).await.unwrap().len(),
            1,
            "unconfigured-repo task must be parked"
        );
        assert_eq!(
            q.list(TaskState::Landed).await.unwrap().len(),
            0,
            "must NOT land a task via an arbitrary cwd"
        );
    }

    /// Records the `BacklogTask` it is handed so the test can assert full-field propagation.
    struct SpyExecutor {
        seen: std::sync::Mutex<Vec<BacklogTask>>,
        out: ExecOutcome,
    }
    #[async_trait::async_trait]
    impl crate::fleet::executor::Executor for SpyExecutor {
        async fn run(&self, t: &BacklogTask, _wt: &std::path::Path) -> anyhow::Result<ExecOutcome> {
            self.seen.lock().unwrap().push(t.clone());
            Ok(self.out.clone())
        }
    }

    #[tokio::test]
    async fn run_forever_passes_full_persisted_task_fields_to_executor() {
        let q = FleetQueue::open_in_memory().unwrap();
        let t = BacklogTask {
            task_id: crate::fleet::backlog::task_id("spectyn-quant", "x"),
            repo: "spectyn-quant".into(),
            slug: "x".into(),
            component: "REAL COMPONENT".into(),
            acceptance: "REAL ACCEPTANCE".into(),
            caps: vec!["quant".into()],
            max_files: 5,
        };
        q.upsert(&t).await.unwrap();
        let ex = SpyExecutor {
            seen: std::sync::Mutex::new(Vec::new()),
            out: ExecOutcome {
                diff: "d".into(),
                build_ok: true,
                logs: "".into(),
            },
        };
        let mut scripted = HashMap::new();
        scripted.insert("codex".into(), "VERDICT: LGTM".into());
        scripted.insert("claude".into(), "VERDICT: LGTM".into());
        let r = crate::fleet::gate::MockReviewer::new(scripted);
        let mut repos: HashMap<String, PathBuf> = HashMap::new();
        repos.insert("spectyn-quant".into(), PathBuf::from("."));
        let ws = MockWorkspaces::default();
        run_forever(
            &q,
            &ex,
            &r,
            &NoopGit,
            &ws,
            &repos,
            Limits::default(),
            "w1",
            60,
            5,
        )
        .await
        .unwrap();
        let seen = ex.seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "the task was executed once");
        assert_eq!(
            seen[0].component, "REAL COMPONENT",
            "executor receives the real component, not empty"
        );
        assert_eq!(seen[0].acceptance, "REAL ACCEPTANCE");
        assert_eq!(seen[0].max_files, 5);
        // The task ran in an isolated workspace that was created and then cleaned up.
        assert_eq!(*ws.created.lock().unwrap(), 1);
        assert_eq!(*ws.cleaned.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn run_forever_cleans_up_workspace_even_when_processing_errors() {
        // An executor that always errors, to exercise the error path.
        struct ErrExec;
        #[async_trait::async_trait]
        impl crate::fleet::executor::Executor for ErrExec {
            async fn run(
                &self,
                _t: &BacklogTask,
                _wt: &std::path::Path,
            ) -> anyhow::Result<ExecOutcome> {
                anyhow::bail!("boom")
            }
        }
        let q = FleetQueue::open_in_memory().unwrap();
        q.upsert(&task("spectyn-quant", "x")).await.unwrap();
        let mut scripted = HashMap::new();
        scripted.insert("codex".into(), "VERDICT: LGTM".into());
        scripted.insert("claude".into(), "VERDICT: LGTM".into());
        let r = crate::fleet::gate::MockReviewer::new(scripted);
        let mut repos: HashMap<String, PathBuf> = HashMap::new();
        repos.insert("spectyn-quant".into(), PathBuf::from("."));
        let ws = MockWorkspaces::default();
        run_forever(
            &q,
            &ErrExec,
            &r,
            &NoopGit,
            &ws,
            &repos,
            Limits::default(),
            "w1",
            60,
            5,
        )
        .await
        .unwrap();
        // Workspace cleaned up despite the error, and the task is parked (never stranded).
        assert_eq!(*ws.created.lock().unwrap(), 1);
        assert_eq!(
            *ws.cleaned.lock().unwrap(),
            1,
            "workspace must be cleaned up even when process_one errors"
        );
        assert_eq!(q.list(TaskState::Parked).await.unwrap().len(), 1);
    }
}
