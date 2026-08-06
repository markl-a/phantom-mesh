//! Pure breadth-first scheduling policy over a queue snapshot.
use crate::fleet::queue::QueueRow;
use crate::fleet::types::{TaskState, Tier};
use crate::fleet::Limits;
use std::collections::HashMap;

fn is_active(s: TaskState) -> bool {
    matches!(
        s,
        TaskState::Claimed | TaskState::Executing | TaskState::Gating | TaskState::Landing
    )
}

fn cap_for(repo: &str, limits: &Limits) -> usize {
    match Tier::for_repo(repo) {
        Tier::Main => limits.main_cap,
        Tier::Satellite => limits.per_repo_cap,
    }
}

/// Choose the next claimable task: among repos below their active cap, pick the
/// pending task whose repo has the fewest active tasks (ties broken by repo then slug).
pub fn pick_next(snapshot: &[QueueRow], limits: &Limits) -> Option<String> {
    let mut active: HashMap<&str, usize> = HashMap::new();
    for r in snapshot {
        if is_active(r.state) {
            *active.entry(r.repo.as_str()).or_default() += 1;
        }
    }
    snapshot
        .iter()
        .filter(|r| r.state == TaskState::Pending)
        .filter(|r| active.get(r.repo.as_str()).copied().unwrap_or(0) < cap_for(&r.repo, limits))
        .min_by(|a, b| {
            let aa = active.get(a.repo.as_str()).copied().unwrap_or(0);
            let bb = active.get(b.repo.as_str()).copied().unwrap_or(0);
            aa.cmp(&bb)
                .then(a.repo.cmp(&b.repo))
                .then(a.slug.cmp(&b.slug))
        })
        .map(|r| r.task_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fleet::queue::QueueRow;
    use crate::fleet::types::TaskState;
    use crate::fleet::Limits;

    fn row(repo: &str, slug: &str, state: TaskState) -> QueueRow {
        QueueRow {
            task_id: format!("{repo}/{slug}"),
            repo: repo.into(),
            slug: slug.into(),
            state,
        }
    }

    #[test]
    fn picks_pending_from_repo_with_fewest_active() {
        let snap = vec![
            row("a", "1", TaskState::Executing), // a has 1 active
            row("a", "2", TaskState::Pending),
            row("b", "9", TaskState::Pending), // b has 0 active
        ];
        let pick = pick_next(&snap, &Limits::default());
        assert_eq!(pick.as_deref(), Some("b/9"), "fewest-active repo (b) wins");
    }

    #[test]
    fn respects_per_repo_cap() {
        let limits = Limits {
            per_repo_cap: 1,
            ..Limits::default()
        };
        let snap = vec![
            row("a", "1", TaskState::Executing), // a already at cap 1
            row("a", "2", TaskState::Pending),
        ];
        assert_eq!(
            pick_next(&snap, &limits),
            None,
            "a is at cap, nothing else claimable"
        );
    }

    #[test]
    fn main_repo_uses_main_cap_of_one() {
        let limits = Limits {
            per_repo_cap: 5,
            main_cap: 1,
            ..Limits::default()
        };
        let snap = vec![
            row("spectyn-mesh", "1", TaskState::Executing), // main at its cap 1
            row("spectyn-mesh", "2", TaskState::Pending),
        ];
        assert_eq!(pick_next(&snap, &limits), None, "main capped at 1 active");
    }
}
