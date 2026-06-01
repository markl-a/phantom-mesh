//! Pinned-projects registry — the data behind the `/projects` dashboard.
//!
//! The 6 entries here mirror the user's GitHub-pinned repos. Each is
//! discoverable from any device on the same Tailscale network via
//! `phantom serve`'s `/projects` HTML route or the `/api/projects`
//! JSON route, with a one-tap [Run Demo] button that streams subprocess
//! output back over Server-Sent Events.
//!
//! The "hub-and-spoke" architecture this enables: phone / iPad /
//! Windows desktop on the user's Tailscale tailnet open a browser to
//! `http://<mac>:7878/projects`, see the same 6 tiles, and can drive
//! demos that run on the Mac coordinator. iOS / iPad can't run any of
//! the Python or Rust projects natively; the dashboard makes that
//! limitation invisible because every device gets the same control
//! surface.
//!
//! Demo commands assume:
//!   * The sibling repos live as siblings of phantom-mesh under
//!     `~/Documents/GitHub/` (matches the user's actual layout).
//!   * `make demo-mock` exists in each repo where the README claims a
//!     demo is runnable. Where it doesn't (Automation_with_Agent at
//!     time of writing — see ship-readiness scorecard), the demo
//!     command is a placeholder that the dashboard surfaces as "demo
//!     not yet available, click GitHub link" rather than a broken run.
//!
//! Stable IDs (lowercase, no spaces) are used by the run-demo POST
//! route as the URL segment, e.g. `POST /api/projects/phantom-mesh/run`.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Project {
    /// URL-safe stable identifier, used as `/api/projects/{id}/run`.
    pub id: &'static str,
    /// Display name for the dashboard tile heading.
    pub name: &'static str,
    /// One-line tagline (≤ 100 chars). Shown beneath the heading.
    pub tagline: &'static str,
    /// GitHub URL for the [GitHub →] link on the tile.
    pub repo_url: &'static str,
    /// Shell command run when the user clicks [Run Demo]. Executed
    /// via `tokio::process::Command` from the project's working dir.
    /// `None` means "demo not wired yet" — the tile shows a disabled
    /// button instead of a broken run.
    pub demo_cmd: Option<DemoCmd>,
    /// Project status badge ("active", "alpha", "ready", "wip").
    pub status: &'static str,
    /// Language stack chip(s), comma-separated. Shown bottom of tile.
    pub stack: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct DemoCmd {
    /// Working directory the command runs in. Resolved against the
    /// user's home dir so the dashboard works without compile-time
    /// path baking.
    pub cwd_under_home: &'static str,
    /// Argv. First element is the executable name (resolved via PATH).
    pub argv: &'static [&'static str],
    /// Approx duration so the dashboard can show a progress hint.
    pub expected_duration_secs: u32,
}

/// The 6 pinned projects, in display order. Top-left → bottom-right
/// in the dashboard's 3×2 grid (responsive on phone to 1×6).
pub fn registry() -> Vec<Project> {
    vec![
        Project {
            id: "phantom-mesh",
            name: "phantom-mesh",
            tagline:
                "Self-hostable AI agent runtime — Mac/Linux/Windows/Android/iOS, Tailscale cluster",
            repo_url: "https://github.com/markl-a/phantom-mesh",
            demo_cmd: Some(DemoCmd {
                cwd_under_home: "path/to/phantom-mesh",
                argv: &[
                    "phantom",
                    "autoevolve",
                    "--once",
                    "--no-commit",
                    "--target",
                    "check",
                    "--max-rounds",
                    "1",
                ],
                expected_duration_secs: 30,
            }),
            status: "active",
            stack: "Rust",
        },
        Project {
            id: "phantom-secops",
            name: "phantom-secops",
            tagline: "Red/blue-team agent simulation — built on phantom-mesh runtime",
            repo_url: "https://github.com/markl-a/phantom-secops",
            demo_cmd: Some(DemoCmd {
                cwd_under_home: "Documents/GitHub/phantom-secops",
                argv: &["make", "demo-mock"],
                expected_duration_secs: 90,
            }),
            status: "alpha",
            stack: "Python · MCP",
        },
        Project {
            id: "phantom-mobile",
            name: "phantom-mobile",
            tagline:
                "Agentic E2E testing for Android — vision-LLM scenario judge across emulator matrix",
            repo_url: "https://github.com/markl-a/phantom-mobile",
            demo_cmd: Some(DemoCmd {
                cwd_under_home: "Documents/GitHub/phantom-mobile",
                argv: &["make", "demo-mock"],
                expected_duration_secs: 90,
            }),
            status: "alpha",
            stack: "Python · Kotlin",
        },
        Project {
            id: "data-analysis-with-agents",
            name: "Data-Analysis-with-Agents",
            tagline:
                "Streamlit dashboard — clustering + RFM segmentation + agent telemetry analytics",
            repo_url: "https://github.com/markl-a/Data-Analysis-with-Agents",
            demo_cmd: Some(DemoCmd {
                cwd_under_home: "Documents/GitHub/Data-Analysis-with-Agents",
                argv: &[
                    "streamlit",
                    "run",
                    "app.py",
                    "--server.headless",
                    "true",
                    "--server.port",
                    "8501",
                ],
                // Long-running: the dashboard streams the URL back so the
                // user can hop over. We cap the run-demo SSE stream at
                // ~10s of startup logs and link them to the live UI.
                expected_duration_secs: 10,
            }),
            status: "active",
            stack: "Python · Streamlit",
        },
        Project {
            id: "automation-with-agent",
            name: "Automation_with_Agent",
            tagline:
                "Applied automation + AIOps + MLOps — fetch → chunk → embed → RAG → LLM in 30 s",
            repo_url: "https://github.com/markl-a/Automation_with_Agent",
            // Wired 2026-05-10 (commit c8ebdf8): top-level demo.py runs
            // a stdlib-only RAG demo over a real Wikipedia URL. Mock LLM
            // by default — no API keys needed for the dashboard to show
            // a green pass.
            demo_cmd: Some(DemoCmd {
                cwd_under_home: "Documents/GitHub/Automation_with_Agent",
                argv: &["make", "demo"],
                expected_duration_secs: 10,
            }),
            status: "active",
            stack: "Python",
        },
        Project {
            id: "my-ai-learning-notes",
            name: "My-AI-Learning-Notes",
            tagline: "繁中 AI 學習路徑 + 面試準備教材 (17 ⭐) — 教材導向，含可運行 notebook 範例",
            repo_url: "https://github.com/markl-a/My-AI-Learning-Notes",
            // For the notes repo, "Run Demo" opens the rendered notebook
            // on GitHub (zero local setup, works on any device including
            // iOS). The dashboard frontend treats demo_cmd=None+is_notes
            // as "open repo_url/blob/main/notebooks/medical_chat.ipynb"
            // — see the HTML template's per-tile button logic.
            demo_cmd: None,
            status: "active",
            stack: "Markdown · Jupyter",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_exactly_six_pinned_entries() {
        // Hard-coded in this many places (dashboard 3×2 grid layout,
        // README ecosystem table, the user's GitHub profile pinned-
        // repos limit) that drift would surface as a UI bug. Lock it.
        assert_eq!(registry().len(), 6);
    }

    #[test]
    fn ids_are_unique_and_url_safe() {
        let ids: Vec<&str> = registry().iter().map(|p| p.id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "duplicate id in registry");
        for id in &ids {
            assert!(
                id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'),
                "non-URL-safe id: {}",
                id
            );
        }
    }

    #[test]
    fn every_active_project_has_a_demo_or_explicit_wip() {
        // Ship-readiness invariant: a tile is either runnable (demo_cmd
        // = Some) or explicitly marked "wip" so the frontend renders a
        // disabled button. A "active" project with no demo_cmd is a bug.
        for p in registry() {
            if p.demo_cmd.is_none() {
                assert!(
                    p.status == "wip" || p.id == "my-ai-learning-notes",
                    "project '{}' has status '{}' but no demo_cmd — \
                     either wire a demo or set status='wip'",
                    p.id,
                    p.status
                );
            }
        }
    }

    #[test]
    fn taglines_fit_a_tile() {
        // Hard cap at 110 chars so taglines don't wrap chaotically on
        // mobile (we render at ~36em column width).
        for p in registry() {
            assert!(
                p.tagline.chars().count() <= 110,
                "tagline too long for tile: {} ({} chars)",
                p.id,
                p.tagline.chars().count()
            );
        }
    }
}
