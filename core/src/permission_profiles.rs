//! Named permission *profiles* — the "Execution Permission" layer of the
//! 4-layer onboarding model (identity / provider / project-trust / permission).
//!
//! A profile is a one-word preset that expands into the existing
//! [`crate::permission`] deny/ask/allow `Tool(specifier)` DSL — it adds NO new
//! enforcement mechanism, it just authors rule lists that compile to a
//! [`permission::Engine`] via [`Engine::from_lists`]. Users pick a profile in
//! `agents.toml` (`[permissions] profile = "workspace-write"`); explicit
//! deny/ask/allow rules, if present, take precedence over the profile (see the
//! engine builder in `bin/spectyn.rs`).
//!
//! The four profiles, least → most capable:
//!   • **observe**         — read-only. Inspect files/git/memory; every write,
//!                           exec and network call is DENIED. The safe default
//!                           for "let it look at my machine".
//!   • **suggest**         — read freely; every write/exec/network call ASKS
//!                           first. The agent proposes, you confirm.
//!   • **workspace-write** — autonomous local dev: read + write files + run
//!                           build/test loops without prompting; raw shell,
//!                           computer-control and network still ASK. (WHICH
//!                           directory it may write in is Project Trust's job,
//!                           Phase 2b — this profile only says "may write".)
//!   • **developer-full**  — allow everything (the legacy unrestricted default,
//!                           now named). Bash redirect/chain hardening in the
//!                           engine still downgrades risky `shell` calls to Ask.
//!
//! Design note: within a profile every tool lands in exactly ONE bucket
//! (allow XOR ask XOR deny), so there is never a same-tool precedence clash —
//! the profiles compile to engines whose behaviour is obvious from the lists.
//!
//! 中文: 權限 profile = 一個字的預設,展開成既有 deny/ask/allow DSL(不新增強制機制)。
//! observe 唯讀、suggest 動作前都問、workspace-write 可自動讀寫+跑 build/test(原始
//! shell/網路仍問)、developer-full 全開。每個工具在一個 profile 內只落一個桶,無優先序衝突。

use crate::permission::Engine;

// ── Tool taxonomy ───────────────────────────────────────────────────────────
// Profiles are authored from these categories so a new tool only has to be
// classified once. A test asserts the categories are disjoint and cover the
// canonical `config::VALID_TOOLS` list, so coverage can't silently rot.

/// Read-only / inspection tools — no side effects, no egress. Safe in every
/// profile. `ask_user` lives here: it only prompts the human, it acts on nothing.
pub const READ_TOOLS: &[&str] = &[
    "file_read", "ls", "stat", "content_search", "glob_search", "spotlight_search",
    "diff_files", "diff_strings", "diag_read", "bash_output",
    "git_status", "git_diff", "git_log", "git_show", "git_blame",
    "git_branch_list", "git_stash_list",
    "memory_recall", "memory_list", "memory_search",
    "todo_list", "todoist_list_tasks",
    "cluster_status", "cluster_sessions", "cluster_peers",
    "ask_user",
];

/// Local state mutation — writes files, repo, memory, todos, or generated media.
pub const WRITE_TOOLS: &[&str] = &[
    "file_write", "file_edit", "multi_file_edit", "apply_patch",
    "memory_store", "memory_delete",
    "git_add", "git_commit", "git_checkout",
    "todo_add", "todo_update", "todo_clear",
    "todoist_add_task", "todoist_complete_task",
    "image_generate", "video_generate", "music_generate",
];

/// Build / test / orchestration — scoped, autonomous-dev exec. Allowed in
/// `workspace-write` (this is how the inner dev loop runs) but not in the
/// read-only profiles.
pub const BUILD_TOOLS: &[&str] = &[
    "cargo_check", "cargo_test", "tsc_check", "run_tests", "dev_verify",
    "task", "parallel_tasks", "subagent",
];

/// Arbitrary code execution / computer control — the highest-blast-radius
/// tools. ASK even in `workspace-write`; DENY in the read-only profiles.
pub const RAW_EXEC_TOOLS: &[&str] = &[
    "shell", "bash_run_background", "bash_kill",
    "xcode_simctl", "screen_capture", "mouse_click", "keystroke",
];

/// Network egress.
pub const NETWORK_TOOLS: &[&str] = &["web_fetch", "web_search", "http_get", "http_post"];

/// Is this a read-only / inspection tool (no side effects, no egress)? Used by
/// Project Trust to decide what an untrusted directory may still run.
pub fn is_read_tool(tool: &str) -> bool {
    READ_TOOLS.contains(&tool)
}

/// Every non-read category, in one slice (write + build + raw-exec + network) —
/// the set the read-only profiles deny or ask.
fn non_read() -> Vec<&'static str> {
    let mut v = Vec::new();
    v.extend_from_slice(WRITE_TOOLS);
    v.extend_from_slice(BUILD_TOOLS);
    v.extend_from_slice(RAW_EXEC_TOOLS);
    v.extend_from_slice(NETWORK_TOOLS);
    v
}

// ── Profiles ─────────────────────────────────────────────────────────────────

/// A named permission preset. See the module docs for the capability ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Observe,
    Suggest,
    WorkspaceWrite,
    DeveloperFull,
}

impl Profile {
    /// All profiles, least → most capable (for listing / help / tests).
    pub const ALL: [Profile; 4] = [
        Profile::Observe,
        Profile::Suggest,
        Profile::WorkspaceWrite,
        Profile::DeveloperFull,
    ];

    /// The canonical hyphenated slug used in `agents.toml` and the CLI.
    pub fn slug(self) -> &'static str {
        match self {
            Profile::Observe => "observe",
            Profile::Suggest => "suggest",
            Profile::WorkspaceWrite => "workspace-write",
            Profile::DeveloperFull => "developer-full",
        }
    }

    /// Parse a slug. Accepts the canonical hyphen form and the underscore
    /// variant (`developer_full`) for leniency; case-insensitive.
    pub fn from_slug(s: &str) -> Option<Profile> {
        match s.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "observe" => Some(Profile::Observe),
            "suggest" => Some(Profile::Suggest),
            "workspace-write" => Some(Profile::WorkspaceWrite),
            "developer-full" => Some(Profile::DeveloperFull),
            _ => None,
        }
    }

    /// One-line human description (shown by `doctor` / `permissions` listing).
    pub fn summary(self) -> &'static str {
        match self {
            Profile::Observe => "read-only — writes/exec/network denied",
            Profile::Suggest => "read freely; ask before any write/exec/network",
            Profile::WorkspaceWrite => {
                "read + write + build/test autonomously; ask for raw shell + network"
            }
            Profile::DeveloperFull => "allow everything (named legacy default)",
        }
    }

    /// The `(deny, ask, allow)` rule lists this profile expands to. Returns owned
    /// strings because the read-only profiles carry a priority prefix
    /// (`"100:file_read"`) so a high-priority read-allow beats a blanket deny.
    pub fn rule_lists(self) -> (Vec<String>, Vec<String>, Vec<String>) {
        let own = |xs: &[&str]| xs.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        match self {
            // Deny EVERYTHING, then high-priority allow the read set. The blanket
            // `*` deny is what makes observe truly read-only: an unknown or
            // MCP-namespaced tool (e.g. `Gmail_create_draft`) that is in no
            // category is DENIED, not left to the engine's default Ask. Priority
            // 100 on the read allows beats the priority-0 `*` deny (Engine sorts
            // by priority first), so reads still go through.
            Profile::Observe => {
                let allow = READ_TOOLS.iter().map(|t| format!("100:{t}")).collect();
                (vec!["*".to_string()], Vec::new(), allow)
            }
            // Read freely; everything not read-listed (write/exec/network AND any
            // unknown/MCP tool) falls to the engine's default Ask — the agent
            // proposes, the gate asks. No catch-all needed.
            Profile::Suggest => (Vec::new(), own(&non_read()), own(READ_TOOLS)),
            Profile::WorkspaceWrite => {
                let mut allow = own(READ_TOOLS);
                allow.extend(own(WRITE_TOOLS));
                allow.extend(own(BUILD_TOOLS));
                let mut ask = own(RAW_EXEC_TOOLS);
                ask.extend(own(NETWORK_TOOLS));
                (Vec::new(), ask, allow)
            }
            // Allow everything. The engine's bash redirect/chain hardening still
            // downgrades a risky `shell` Allow to Ask, so even "full" isn't a
            // blind exfil green-light.
            Profile::DeveloperFull => (Vec::new(), Vec::new(), vec!["*".to_string()]),
        }
    }

    /// Compile this profile to a permission [`Engine`]. Builtin profiles are
    /// always valid DSL (a test guarantees it), so this never fails in practice.
    pub fn engine(self) -> Engine {
        let (deny, ask, allow) = self.rule_lists();
        let d: Vec<&str> = deny.iter().map(String::as_str).collect();
        let a: Vec<&str> = ask.iter().map(String::as_str).collect();
        let al: Vec<&str> = allow.iter().map(String::as_str).collect();
        Engine::from_lists(&d, &a, &al).expect("builtin permission profiles must be valid DSL")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cats() -> [&'static [&'static str]; 5] {
        [READ_TOOLS, WRITE_TOOLS, BUILD_TOOLS, RAW_EXEC_TOOLS, NETWORK_TOOLS]
    }

    #[test]
    fn categories_are_disjoint() {
        let all: Vec<&str> = cats().iter().flat_map(|c| c.iter().copied()).collect();
        let mut seen = std::collections::HashSet::new();
        for t in &all {
            assert!(seen.insert(*t), "tool {t} is in more than one category");
        }
    }

    #[test]
    fn taxonomy_covers_the_real_tool_registry() {
        // Guards drift against the ACTUAL executable registry (tools::all_tool_names),
        // not the stale config::VALID_TOOLS subset — a new built-in tool must be
        // categorized or this fails. (MCP tools are namespaced + dynamic, so they
        // can't be enumerated here; observe's `*` catch-all denies the uncategorized,
        // which is what protects them.)
        let all: std::collections::HashSet<&str> =
            cats().iter().flat_map(|c| c.iter().copied()).collect();
        let missing: Vec<&str> = crate::tools::all_tool_names()
            .into_iter()
            .filter(|t| !all.contains(t))
            .collect();
        assert!(missing.is_empty(), "tools missing from every profile category: {missing:?}");
    }

    #[test]
    fn every_profile_compiles_to_an_engine() {
        // The `.expect()` in engine() must never fire — prove it for all four.
        for p in Profile::ALL {
            let e = p.engine();
            // developer-full is the only one whose rule set is "allow *".
            if p == Profile::DeveloperFull {
                assert!(!e.is_empty());
            } else {
                assert!(!e.rules().is_empty(), "{} produced no rules", p.slug());
            }
        }
    }

    #[test]
    fn slug_round_trips_and_is_lenient() {
        for p in Profile::ALL {
            assert_eq!(Profile::from_slug(p.slug()), Some(p));
        }
        assert_eq!(Profile::from_slug("developer_full"), Some(Profile::DeveloperFull));
        assert_eq!(Profile::from_slug("  WORKSPACE-WRITE "), Some(Profile::WorkspaceWrite));
        assert_eq!(Profile::from_slug("nonsense"), None);
    }

    #[test]
    fn observe_denies_writes_and_exec_allows_reads() {
        let e = Profile::Observe.engine();
        use crate::permission::Decision;
        assert!(matches!(e.evaluate("file_read", &json!({"path": "x"})), Decision::Allow));
        assert!(matches!(e.evaluate("file_write", &json!({"path": "x"})), Decision::Deny(_)));
        assert!(matches!(e.evaluate("shell", &json!({"cmd": "ls"})), Decision::Deny(_)));
        assert!(matches!(e.evaluate("web_fetch", &json!({"url": "x"})), Decision::Deny(_)));
    }

    #[test]
    fn observe_denies_unknown_and_mcp_namespaced_tools() {
        // The catch-all `*` deny makes observe truly read-only: a tool in NO
        // category (a future built-in, or an MCP `server_tool`) must be DENIED,
        // not fall through to the engine's default Ask.
        let e = Profile::Observe.engine();
        use crate::permission::Decision;
        assert!(matches!(e.evaluate("Gmail_create_draft", &json!({})), Decision::Deny(_)));
        assert!(matches!(e.evaluate("some_future_tool", &json!({})), Decision::Deny(_)));
        // but a read tool still works (priority beats the blanket deny)
        assert!(matches!(e.evaluate("git_status", &json!({})), Decision::Allow));
    }

    #[test]
    fn suggest_asks_for_writes_allows_reads() {
        let e = Profile::Suggest.engine();
        use crate::permission::Decision;
        assert!(matches!(e.evaluate("file_read", &json!({"path": "x"})), Decision::Allow));
        assert!(matches!(e.evaluate("file_write", &json!({"path": "x"})), Decision::Ask));
        assert!(matches!(e.evaluate("shell", &json!({"cmd": "ls"})), Decision::Ask));
    }

    #[test]
    fn workspace_write_allows_writes_and_build_asks_raw_shell() {
        let e = Profile::WorkspaceWrite.engine();
        use crate::permission::Decision;
        assert!(matches!(e.evaluate("file_write", &json!({"path": "x"})), Decision::Allow));
        assert!(matches!(e.evaluate("cargo_test", &json!({})), Decision::Allow));
        assert!(matches!(e.evaluate("shell", &json!({"cmd": "ls"})), Decision::Ask));
        assert!(matches!(e.evaluate("web_fetch", &json!({"url": "x"})), Decision::Ask));
    }

    #[test]
    fn developer_full_allows_everything_but_hardens_risky_shell() {
        let e = Profile::DeveloperFull.engine();
        use crate::permission::Decision;
        assert!(matches!(e.evaluate("file_write", &json!({"path": "x"})), Decision::Allow));
        assert!(matches!(e.evaluate("shell", &json!({"cmd": "ls"})), Decision::Allow));
        // redirect/chain still downgrades to Ask even under "full"
        assert!(matches!(
            e.evaluate("shell", &json!({"cmd": "cat secrets > /tmp/x"})),
            Decision::Ask
        ));
    }
}
