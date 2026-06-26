//! Quantified life goals — the life-accountability capability ② keystone.
//!
//! A [`Goal`] is a *quantified* intention: "focus 180 minutes a day",
//! "exercise 3 times a week". Storing the target up front is what lets the
//! daily-review later compute *deviation* ("you wanted 180 min of focus, you
//! logged 95") instead of vague vibes. This module owns only the **definition +
//! storage** half; the deviation computation lives in the daily-review and is
//! intentionally out of scope here.
//!
//! Storage is an append-only JSON-lines ledger at `~/.phantom-mesh/goals.jsonl`,
//! mirroring the `partner-signals.jsonl` convention in [`crate::partner`]: no DB
//! to provision, survives a crash mid-write (the worst case is one truncated
//! trailing line, which the reader skips), and is trivially greppable. We
//! deliberately do NOT touch the rpc/event wire enum — defining a goal is a
//! local-first write, not a mesh event, so it stays out of the schema-migration
//! zone.
//!
//! "Newest-wins per tag": redefining a goal appends a new line rather than
//! rewriting the file, so the ledger keeps the full history (useful for "when
//! did I raise my focus target?") while [`list_goals_in`] collapses to the
//! latest definition per `tag`.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A quantified goal. `window` is the cadence over which `target` applies:
/// `"daily"` or `"weekly"`. `unit` is free-form (`"minutes"`, `"times"`,
/// `"km"`, …) so the same struct serves focus-minutes, gym-sessions, etc.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Goal {
    pub tag: String,
    pub target: f64,
    pub unit: String,
    pub window: String,
}

/// Default goals-ledger file name under the `.phantom-mesh` home dir.
const GOALS_FILE: &str = "goals.jsonl";

/// Path of the goals JSONL ledger: `~/.phantom-mesh/goals.jsonl`. Resolved from
/// `$HOME` (via `dirs::home_dir`) so a temp `$HOME` would isolate it; tests use
/// the path-injectable [`define_goal_in`] / [`list_goals_in`] cores directly and
/// never touch the real home dir. Mirrors `partner::signals_path`.
pub fn goals_jsonl_path() -> PathBuf {
    crate::cli_config::phantom_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(GOALS_FILE)
}

/// Append one `Goal` to the JSONL ledger inside `dir` (creating `dir` and any
/// parents). One goal per line, serialized as a JSON object. Append-only:
/// redefining a tag adds a line, it does not rewrite the file.
pub fn define_goal_in(dir: &Path, g: &Goal) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(GOALS_FILE);
    let line = serde_json::to_string(g)?;
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

/// Read the goals ledger inside `dir` and return the *effective* goals:
/// newest-wins per `tag` (a later append of the same tag supersedes the earlier
/// one). Returns an empty Vec if the ledger does not exist yet. Malformed /
/// truncated lines are skipped so one bad trailing write never poisons the read.
///
/// Order is deterministic: goals come back sorted by `tag` so list output and
/// tests are stable regardless of HashMap iteration order.
pub fn list_goals_in(dir: &Path) -> std::io::Result<Vec<Goal>> {
    let path = dir.join(GOALS_FILE);
    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    // Last definition per tag wins: overwrite as we scan top-to-bottom, since the
    // ledger is append-only so later lines are newer.
    let mut latest: HashMap<String, Goal> = HashMap::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(g) = serde_json::from_str::<Goal>(trimmed) {
            latest.insert(g.tag.clone(), g);
        }
    }
    let mut goals: Vec<Goal> = latest.into_values().collect();
    goals.sort_by(|a, b| a.tag.cmp(&b.tag));
    Ok(goals)
}

/// Append a goal to the real `~/.phantom-mesh/goals.jsonl` ledger. Thin wrapper
/// over [`define_goal_in`] using the home-derived dir.
pub fn define_goal(g: &Goal) -> std::io::Result<()> {
    let path = goals_jsonl_path();
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    define_goal_in(&dir, g)
}

/// List the effective goals from the real `~/.phantom-mesh/goals.jsonl` ledger.
/// Thin wrapper over [`list_goals_in`] using the home-derived dir.
pub fn list_goals() -> std::io::Result<Vec<Goal>> {
    let path = goals_jsonl_path();
    let dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    list_goals_in(&dir)
}

/// List the effective goals for a given `home` directory — the ledger lives at
/// `<home>/.phantom-mesh/goals.jsonl`. This is the path-injectable entry point
/// the daily-review uses: production passes the real home, tests pass a tempdir,
/// so the deviation computation reads a real `goals.jsonl` off real files in
/// both cases (no mock). Resolves to the same dir `list_goals` would for the
/// real `$HOME` and delegates to [`list_goals_in`].
pub fn list_goals_for_home(home: &Path) -> std::io::Result<Vec<Goal>> {
    let dir = crate::cli_config::phantom_dir_under(home);
    list_goals_in(&dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real-file round-trip (NO mock): define a goal into a tempdir, reload via
    /// `list_goals_in`, and assert every field survives the JSONL trip.
    #[test]
    fn goal_round_trips_through_jsonl_ledger() {
        let dir = std::env::temp_dir().join(format!(
            "phantom-goals-roundtrip-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        let g = Goal {
            tag: "focus".to_string(),
            target: 180.0,
            unit: "minutes".to_string(),
            window: "daily".to_string(),
        };
        define_goal_in(&dir, &g).expect("define_goal_in writes the ledger");

        let loaded = list_goals_in(&dir).expect("list_goals_in reads it back");
        assert_eq!(loaded.len(), 1, "exactly one goal in the ledger");
        let got = &loaded[0];
        assert_eq!(got.tag, "focus");
        assert_eq!(got.target, 180.0);
        assert_eq!(got.unit, "minutes");
        assert_eq!(got.window, "daily");
        // Whole-struct equality pins that nothing silently mutated.
        assert_eq!(*got, g);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Newest-wins per tag: redefining `focus` (120 → 180) appends a second line,
    /// and `list_goals_in` must collapse to the latest target.
    #[test]
    fn newest_definition_wins_per_tag() {
        let dir = std::env::temp_dir().join(format!(
            "phantom-goals-newestwins-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);

        define_goal_in(
            &dir,
            &Goal {
                tag: "focus".to_string(),
                target: 120.0,
                unit: "minutes".to_string(),
                window: "daily".to_string(),
            },
        )
        .unwrap();
        define_goal_in(
            &dir,
            &Goal {
                tag: "focus".to_string(),
                target: 180.0,
                unit: "minutes".to_string(),
                window: "daily".to_string(),
            },
        )
        .unwrap();

        let loaded = list_goals_in(&dir).expect("read back");
        assert_eq!(loaded.len(), 1, "newest-wins collapses to one `focus` goal");
        assert_eq!(
            loaded[0].target, 180.0,
            "the later 180 supersedes the earlier 120"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
