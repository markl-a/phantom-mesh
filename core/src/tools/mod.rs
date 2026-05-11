pub mod ask_user;
pub mod bash_bg;
pub mod trait_def;
pub use trait_def::{Tool, ToolContext, BuiltinTool, McpToolWrapper, live_tools};
pub mod cluster;
pub mod subagent;
pub mod diag;
pub mod diagnostic;
pub mod diff_view;
pub mod fetch;
pub mod file;
pub mod http_client;
pub mod fs;
pub mod git;
pub mod ls;
pub mod memory;
pub mod multi_edit;
pub mod patch;
pub mod search;
pub mod shell;
pub mod todo;
pub mod web;
pub mod web_fetch;
#[cfg(target_os = "macos")]
pub mod spotlight;
#[cfg(target_os = "macos")]
pub mod xcode;

use serde_json::Value;
use crate::config::ToolsConfig;

/// Largest prefix of `s` that fits within `max_bytes` and ends on a UTF-8
/// char boundary. Safe for slicing arbitrary user/external input.
pub fn floor_char_boundary(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Smallest suffix of `s` whose start sits on a UTF-8 char boundary at or
/// after byte offset `start`.
fn ceil_char_boundary(s: &str, start: usize) -> &str {
    let mut i = start.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
}

pub fn truncate(s: String, max_chars: usize) -> String {
    if s.len() <= max_chars { return s; }
    let half = max_chars / 2;
    format!(
        "{}\n\n[... {} chars truncated ...]\n\n{}",
        floor_char_boundary(&s, half),
        s.len() - max_chars,
        ceil_char_boundary(&s, s.len() - half),
    )
}

/// Returns all registered tool names. Useful for /tools and help output.
pub fn all_tool_names() -> Vec<&'static str> {
    // mut is unused on non-macOS targets because the conditional
    // v.push(...) block below is gated out. Allow the warning instead
    // of duplicating the whole vec! literal under cfg arms.
    #[allow(unused_mut)]
    let mut v: Vec<&'static str> = vec![
        // file (sandbox-safe — iOS can read/write within app container)
        "file_read",
        "file_write",
        "file_edit",
        // search
        "content_search",
        "glob_search",
        // web
        "web_search",
        // memory
        "memory_store",
        "memory_recall",
        "memory_list",
        "memory_delete",
        "memory_search",
        // ls
        "ls",
        "stat",
        // patch
        "apply_patch",
        // todos (in-agent TODO list)
        "todo_add",
        "todo_update",
        "todo_list",
        "todo_clear",
        // multi-edit
        "multi_file_edit",
        // diff
        "diff_files",
        "diff_strings",
        // http
        "http_get",
        "http_post",
        // web fetch (HTML→text)
        "web_fetch",
        // interactive — pause agent and ask the human
        "ask_user",
        // subagent orchestration — spawn another configured agent
        "task",
        "subagent",
        "parallel_tasks",
        // cluster awareness — read-only "who's reachable / who's online"
        // so the agent can pick a `node:` target for task/parallel_tasks.
        "cluster_status",
        "cluster_sessions",
        "cluster_peers",
        // self-introspection — read phantom's own diagnostic state
        "diag_read",
    ];

    // Tools that require subprocess spawn / native toolchain — iOS sandbox
    // forbids fork/exec, so we drop them from the registry on iOS. They
    // remain registered on macOS/Windows/Linux/Android workers, and the
    // mesh's required_caps filter routes shell/git/cargo tasks to those
    // peers instead. (v1.5 G8 step c)
    #[cfg(not(target_os = "ios"))]
    {
        // core shell
        v.push("shell");
        // git (existing + new) — relies on git binary
        v.push("git_status");
        v.push("git_diff");
        v.push("git_log");
        v.push("git_commit");
        v.push("git_branch_list");
        v.push("git_checkout");
        v.push("git_show");
        v.push("git_blame");
        v.push("git_add");
        v.push("git_stash_list");
        // diagnostics — needs cargo / tsc / runner
        v.push("cargo_check");
        v.push("cargo_test");
        v.push("tsc_check");
        v.push("run_tests");
        // background bash
        v.push("bash_run_background");
        v.push("bash_output");
        v.push("bash_kill");
    }

    // macOS-only tools — vec! doesn't honor #[cfg(...)] on inline element
    // attributes the way a match arm does, so push them after construction.
    #[cfg(target_os = "macos")]
    {
        v.push("spotlight_search");
        v.push("xcode_simctl");
    }

    v
}

pub async fn execute(name: &str, args: &Value, config: &ToolsConfig) -> String {
    // External MCP servers (configured via `[[mcp_servers]]` in agents.toml)
    // expose their tools under a `<server>_<tool>` namespace. If the tool name
    // matches a registered prefix, route the call there before falling through
    // to the built-in match below.
    if let Some(reg) = crate::mcp_client::global() {
        if let Some(out) = reg.dispatch(name, args).await {
            return out;
        }
    }
    match name {
        // ── core ─────────────────────────────────────────────────────────────
        #[cfg(not(target_os = "ios"))]
        "shell"            => shell::run(args).await,
        "file_read"        => file::read(args).await,
        "file_write"       => file::write(args).await,
        "file_edit"        => file::edit(args).await,
        // ── search ───────────────────────────────────────────────────────────
        "content_search"   => search::content(args).await,
        "glob_search"      => search::glob(args).await,
        // ── web ──────────────────────────────────────────────────────────────
        "web_search"       => web::search(args, config).await,
        // ── memory ───────────────────────────────────────────────────────────
        "memory_store"     => memory::store(args).await,
        "memory_recall"    => memory::recall(args).await,
        "memory_list"      => memory::list(args).await,
        "memory_delete"    => memory::delete(args).await,
        "memory_search"    => memory::search(args).await,
        // ── git (existing) — needs `git` binary, no-op on iOS sandbox ────────
        #[cfg(not(target_os = "ios"))]
        "git_status"       => git::status(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_diff"         => git::diff(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_log"          => git::log(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_commit"       => git::commit(args).await,
        // ── git (new) ────────────────────────────────────────────────────────
        #[cfg(not(target_os = "ios"))]
        "git_branch_list"  => git::git_branch_list(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_checkout"     => git::git_checkout(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_show"         => git::git_show(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_blame"        => git::git_blame(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_add"          => git::git_add(args).await,
        #[cfg(not(target_os = "ios"))]
        "git_stash_list"   => git::git_stash_list(args).await,
        // ── ls ───────────────────────────────────────────────────────────────
        "ls"               => ls::list(args).await,
        "stat"             => ls::stat(args).await,
        // ── patch ────────────────────────────────────────────────────────────
        "apply_patch"      => patch::apply(args).await,
        // ── diagnostics — toolchain not in iOS sandbox ──────────────────────
        #[cfg(not(target_os = "ios"))]
        "cargo_check"      => diagnostic::cargo_check(args).await,
        #[cfg(not(target_os = "ios"))]
        "cargo_test"       => diagnostic::cargo_test(args).await,
        #[cfg(not(target_os = "ios"))]
        "tsc_check"        => diagnostic::tsc_check(args).await,
        #[cfg(not(target_os = "ios"))]
        "run_tests"        => diagnostic::run_tests(args).await,
        // ── todos (in-agent TODO list) ───────────────────────────────────────
        "todo_add"         => todo::add(args).await,
        "todo_update"      => todo::update(args).await,
        "todo_list"        => todo::list(args).await,
        "todo_clear"       => todo::clear(args).await,
        // ── multi-edit ───────────────────────────────────────────────────────
        "multi_file_edit"  => multi_edit::execute(args).await,
        // ── diff ─────────────────────────────────────────────────────────────
        "diff_files"       => diff_view::diff_files(args).await,
        "diff_strings"     => diff_view::diff_strings(args).await,
        // ── http ─────────────────────────────────────────────────────────────
        "http_get"            => http_client::get(args).await,
        "http_post"           => http_client::post(args).await,
        // ── web fetch (HTML→text) ────────────────────────────────────────────
        "web_fetch"           => web_fetch::fetch(args).await,
        // ── background bash — fork/exec forbidden in iOS sandbox ────────────
        #[cfg(not(target_os = "ios"))]
        "bash_run_background" => bash_bg::run_background(args).await,
        #[cfg(not(target_os = "ios"))]
        "bash_output"         => bash_bg::output(args).await,
        #[cfg(not(target_os = "ios"))]
        "bash_kill"           => bash_bg::kill(args).await,
        // ── interactive ──────────────────────────────────────────────────────
        "ask_user"            => ask_user::ask(args).await,
        // ── subagent orchestration ───────────────────────────────────────────
        "task" | "subagent"   => subagent::spawn(args).await,
        "parallel_tasks"      => subagent::parallel(args).await,
        // ── cluster awareness ────────────────────────────────────────────────
        "cluster_status"      => cluster::status(args).await,
        "cluster_sessions"    => cluster::sessions(args).await,
        "cluster_peers"       => cluster::peers(args).await,

        // self-introspection — let the agent read its own diag state
        "diag_read"           => diag::read(args).await,
        // ── macOS-only: Spotlight + Xcode ────────────────────────────────────
        #[cfg(target_os = "macos")]
        "spotlight_search"    => spotlight::search(args).await,
        #[cfg(target_os = "macos")]
        "xcode_simctl"        => xcode::simctl(args).await,
        // ── unknown ──────────────────────────────────────────────────────────
        other              => format!("Unknown tool: {}", other),
    }
}

pub fn schema(name: &str) -> Option<Value> {
    match name {
        // ── shell ─────────────────────────────────────────────────────────────
        "shell" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "shell",
                "description": "Execute a shell command and return stdout/stderr with exit code. \
                    Supports &&, ||, ; compound commands, custom working directory, extra env vars, and stdin.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute. Supports &&, ||, and ; operators."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Timeout in seconds (default 30, max 300)"
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Working directory for the command. Must exist."
                        },
                        "env": {
                            "type": "object",
                            "additionalProperties": {"type": "string"},
                            "description": "Additional environment variables merged into the current environment."
                        },
                        "stdin": {
                            "type": "string",
                            "description": "Text to pipe into stdin of the command."
                        }
                    },
                    "required": ["command"]
                }
            }
        })),

        // ── file_read ─────────────────────────────────────────────────────────
        "file_read" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "file_read",
                "description": "Read the contents of a file. Supports pagination via offset/limit. \
                    Returns binary detection info for non-text files.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Absolute or relative file path to read."
                        },
                        "offset": {
                            "type": "integer",
                            "description": "1-based line number to start reading from (inclusive)."
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Maximum number of lines to return."
                        },
                        "show_line_numbers": {
                            "type": "boolean",
                            "description": "If true, prefix each line with its 1-based line number."
                        }
                    },
                    "required": ["path"]
                }
            }
        })),

        // ── file_write ────────────────────────────────────────────────────────
        "file_write" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "file_write",
                "description": "Write content to a file, creating it and any missing parent directories.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path to write to."
                        },
                        "content": {
                            "type": "string",
                            "description": "Text content to write."
                        },
                        "create_dirs": {
                            "type": "boolean",
                            "description": "Create missing parent directories (default true)."
                        }
                    },
                    "required": ["path", "content"]
                }
            }
        })),

        // ── file_edit ─────────────────────────────────────────────────────────
        "file_edit" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "file_edit",
                "description": "Replace an exact string in a file. old_string must match exactly once unless replace_all is true. \
                    Optionally scope the search to a line range.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "File path to edit."
                        },
                        "old_string": {
                            "type": "string",
                            "description": "Exact string to find and replace."
                        },
                        "new_string": {
                            "type": "string",
                            "description": "Replacement text (may be empty to delete)."
                        },
                        "replace_all": {
                            "type": "boolean",
                            "description": "If true, replace all occurrences instead of requiring exactly one match."
                        },
                        "line_range": {
                            "type": "object",
                            "description": "Restrict search to a range of lines (1-based, inclusive).",
                            "properties": {
                                "start": {"type": "integer"},
                                "end": {"type": "integer"}
                            }
                        }
                    },
                    "required": ["path", "old_string", "new_string"]
                }
            }
        })),

        // ── content_search ────────────────────────────────────────────────────
        "content_search" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "content_search",
                "description": "Search file contents using ripgrep (falls back to grep). Returns matching lines with file paths.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Regex or literal search pattern."
                        },
                        "path": {
                            "type": "string",
                            "description": "Directory or file to search (default: .)."
                        },
                        "context_lines": {
                            "type": "integer",
                            "description": "Lines of context before/after each match (default 2)."
                        },
                        "file_type": {
                            "type": "string",
                            "description": "Filter by file type extension without dot, e.g. 'rs', 'ts', 'py'."
                        },
                        "case_sensitive": {
                            "type": "boolean",
                            "description": "If true, search is case-sensitive (default false)."
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of matches to return (default 50)."
                        }
                    },
                    "required": ["pattern"]
                }
            }
        })),

        // ── glob_search ───────────────────────────────────────────────────────
        "glob_search" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "glob_search",
                "description": "Find files matching a glob pattern. Uses ripgrep --files, falls back to find.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "pattern": {
                            "type": "string",
                            "description": "Glob pattern, e.g. '**/*.rs' or 'src/**/*.tsx'."
                        },
                        "path": {
                            "type": "string",
                            "description": "Base directory to search from (default: .)."
                        },
                        "exclude": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Glob patterns to exclude, e.g. ['target/**', '*.lock']."
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Maximum number of files to return (default 200)."
                        }
                    },
                    "required": ["pattern"]
                }
            }
        })),

        // ── web_search ────────────────────────────────────────────────────────
        "web_search" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the web for information.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Search query string."}
                    },
                    "required": ["query"]
                }
            }
        })),

        // ── memory_store ──────────────────────────────────────────────────────
        "memory_store" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "memory_store",
                "description": "Store a key-value pair in persistent memory. Optionally namespace the key.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "key": {"type": "string", "description": "Memory key."},
                        "value": {"type": "string", "description": "Value to store."},
                        "namespace": {
                            "type": "string",
                            "description": "Optional namespace prefix, e.g. 'project' stores as 'project/key'."
                        }
                    },
                    "required": ["key", "value"]
                }
            }
        })),

        // ── memory_recall ─────────────────────────────────────────────────────
        "memory_recall" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "memory_recall",
                "description": "Recall a value from persistent memory by key.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "key": {"type": "string", "description": "Memory key to look up."},
                        "namespace": {
                            "type": "string",
                            "description": "Optional namespace prefix matching the one used when storing."
                        }
                    },
                    "required": ["key"]
                }
            }
        })),

        // ── memory_list ───────────────────────────────────────────────────────
        "memory_list" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "memory_list",
                "description": "List all stored memory keys and truncated values, optionally filtered by namespace.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "namespace": {
                            "type": "string",
                            "description": "Show only keys under this namespace prefix."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── memory_delete ─────────────────────────────────────────────────────
        "memory_delete" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "memory_delete",
                "description": "Delete a key from persistent memory.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "key": {"type": "string", "description": "Memory key to delete."},
                        "namespace": {
                            "type": "string",
                            "description": "Optional namespace prefix matching the one used when storing."
                        }
                    },
                    "required": ["key"]
                }
            }
        })),

        // ── memory_search ─────────────────────────────────────────────────────
        "memory_search" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "memory_search",
                "description": "Search memory entries whose values contain the query string (case-insensitive).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {"type": "string", "description": "Substring to search for in stored values."},
                        "namespace": {
                            "type": "string",
                            "description": "Restrict search to this namespace prefix."
                        }
                    },
                    "required": ["query"]
                }
            }
        })),

        // ── git_status ────────────────────────────────────────────────────────
        "git_status" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_status",
                "description": "Show git working tree status (git status --short).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Git repo path (default: .)."}
                    },
                    "required": []
                }
            }
        })),

        // ── git_diff ──────────────────────────────────────────────────────────
        "git_diff" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_diff",
                "description": "Show git diff stat for the working tree or a specific file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Git repo path (default: .)."},
                        "cached": {"type": "boolean", "description": "If true, diff staged changes (--cached)."},
                        "file": {"type": "string", "description": "Restrict diff to this file path."}
                    },
                    "required": []
                }
            }
        })),

        // ── git_log ───────────────────────────────────────────────────────────
        "git_log" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_log",
                "description": "Show recent git commits in oneline format.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Git repo path (default: .)."},
                        "n": {"type": "integer", "description": "Number of commits to show (default 10)."}
                    },
                    "required": []
                }
            }
        })),

        // ── git_commit ────────────────────────────────────────────────────────
        "git_commit" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_commit",
                "description": "Create a git commit with the given message. Stage files with git_add first.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "message": {"type": "string", "description": "Commit message (max 1000 chars)."},
                        "path": {"type": "string", "description": "Git repo path (default: .)."}
                    },
                    "required": ["message"]
                }
            }
        })),

        // ── git_branch_list ───────────────────────────────────────────────────
        "git_branch_list" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_branch_list",
                "description": "List git branches with their latest commit summary.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Git repo path (default: .)."},
                        "remote": {
                            "type": "boolean",
                            "description": "If true, show remote-tracking branches as well (git branch -av)."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── git_checkout ──────────────────────────────────────────────────────
        "git_checkout" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_checkout",
                "description": "Switch to a branch (or create it with create:true).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "branch": {"type": "string", "description": "Branch name to check out."},
                        "path": {"type": "string", "description": "Git repo path (default: .)."},
                        "create": {
                            "type": "boolean",
                            "description": "If true, create the branch before switching (git checkout -b)."
                        }
                    },
                    "required": ["branch"]
                }
            }
        })),

        // ── git_show ──────────────────────────────────────────────────────────
        "git_show" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_show",
                "description": "Show a commit's details and diff.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "ref_": {
                            "type": "string",
                            "description": "Commit ref to show (default HEAD). Use 'ref_' as the key."
                        },
                        "path": {"type": "string", "description": "Git repo path (default: .)."},
                        "stat_only": {
                            "type": "boolean",
                            "description": "If true, show only --stat output without the full diff."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── git_blame ─────────────────────────────────────────────────────────
        "git_blame" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_blame",
                "description": "Show who last modified each line of a file (git blame). Output truncated to 100 lines.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Path to the file to blame (required)."
                        },
                        "repo": {
                            "type": "string",
                            "description": "Git repo root directory (default: .)."
                        }
                    },
                    "required": ["path"]
                }
            }
        })),

        // ── git_add ───────────────────────────────────────────────────────────
        "git_add" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_add",
                "description": "Stage one or more files for the next commit (git add).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "files": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "List of file paths to stage."
                        },
                        "path": {"type": "string", "description": "Git repo path (default: .)."}
                    },
                    "required": ["files"]
                }
            }
        })),

        // ── git_stash_list ────────────────────────────────────────────────────
        "git_stash_list" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "git_stash_list",
                "description": "List all stashes in the repository.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Git repo path (default: .)."}
                    },
                    "required": []
                }
            }
        })),

        // ── ls ────────────────────────────────────────────────────────────────
        "ls" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "ls",
                "description": "List directory contents. Supports long format and tree view.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Directory to list (default: .)."},
                        "long": {
                            "type": "boolean",
                            "description": "If true, show permissions, size, and modification date."
                        },
                        "tree": {
                            "type": "boolean",
                            "description": "If true, render directory as a tree (max depth 3)."
                        },
                        "hidden": {
                            "type": "boolean",
                            "description": "If true, include hidden files and directories (starting with '.')."
                        },
                        "max_entries": {
                            "type": "integer",
                            "description": "Maximum number of entries to return (default 200)."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── stat ──────────────────────────────────────────────────────────────
        "stat" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "stat",
                "description": "Show detailed metadata about a file or directory: type, size, permissions, timestamps, and line count.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "Path to the file or directory."}
                    },
                    "required": ["path"]
                }
            }
        })),

        // ── apply_patch ───────────────────────────────────────────────────────
        "apply_patch" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "apply_patch",
                "description": "Apply a unified diff patch to one or more files. Validates context before writing.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "patch": {
                            "type": "string",
                            "description": "Unified diff text (output of git diff or diff -u)."
                        },
                        "base_dir": {
                            "type": "string",
                            "description": "Base directory for resolving relative paths in the patch (default: cwd)."
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "If true, validate hunks without writing any files."
                        }
                    },
                    "required": ["patch"]
                }
            }
        })),

        // ── cargo_check ───────────────────────────────────────────────────────
        "cargo_check" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "cargo_check",
                "description": "Run cargo check to validate Rust code for compile errors and warnings.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory containing Cargo.toml (default: .)."
                        },
                        "package": {
                            "type": "string",
                            "description": "Specific package to check in a workspace (--package flag)."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── cargo_test ────────────────────────────────────────────────────────
        "cargo_test" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "cargo_test",
                "description": "Run cargo tests, optionally filtering by test name.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Directory containing Cargo.toml (default: .)."
                        },
                        "filter": {
                            "type": "string",
                            "description": "Test name filter (passed directly to cargo test)."
                        },
                        "package": {
                            "type": "string",
                            "description": "Specific package to test in a workspace."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── tsc_check ─────────────────────────────────────────────────────────
        "tsc_check" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "tsc_check",
                "description": "Run TypeScript type-checking via tsc --noEmit (or npx tsc if tsc not in PATH).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Project directory containing tsconfig.json (default: .)."
                        },
                        "config": {
                            "type": "string",
                            "description": "Path to a specific tsconfig file (--project flag)."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── run_tests ─────────────────────────────────────────────────────────
        "run_tests" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "run_tests",
                "description": "Run an arbitrary test command (e.g. 'pytest', 'jest', 'go test ./...') and return results.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The test command to run, e.g. 'pytest tests/' or 'jest --ci'."
                        },
                        "path": {
                            "type": "string",
                            "description": "Working directory for the command (default: .)."
                        }
                    },
                    "required": ["command"]
                }
            }
        })),

        // ── todo_add ──────────────────────────────────────────────────────────
        "todo_add" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "todo_add",
                "description": "Add a new item to the agent's in-session TODO list with status 'todo'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "description": {
                            "type": "string",
                            "description": "Human-readable TODO description."
                        },
                        "session": {
                            "type": "string",
                            "description": "TODO list session name (default: 'default'). Allows separate lists per project."
                        }
                    },
                    "required": ["description"]
                }
            }
        })),

        // ── todo_update ───────────────────────────────────────────────────────
        "todo_update" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "todo_update",
                "description": "Update the status of a TODO by its ID.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "id": {
                            "type": "integer",
                            "description": "Task ID to update."
                        },
                        "status": {
                            "type": "string",
                            "enum": ["todo", "in_progress", "done"],
                            "description": "New status for the task."
                        },
                        "session": {
                            "type": "string",
                            "description": "Task list session name (default: 'default')."
                        }
                    },
                    "required": ["id", "status"]
                }
            }
        })),

        // ── todo_list ─────────────────────────────────────────────────────────
        "todo_list" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "todo_list",
                "description": "List TODOs in the agent's in-session list, optionally filtered by status.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "status_filter": {
                            "type": "string",
                            "enum": ["todo", "in_progress", "done"],
                            "description": "Show only tasks with this status. Omit to show all."
                        },
                        "session": {
                            "type": "string",
                            "description": "Task list session name (default: 'default')."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── todo_clear ────────────────────────────────────────────────────────
        "todo_clear" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "todo_clear",
                "description": "Remove TODOs from the agent's in-session list. By default removes all; set done_only to remove only completed items.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "done_only": {
                            "type": "boolean",
                            "description": "If true, remove only tasks with status 'done'."
                        },
                        "session": {
                            "type": "string",
                            "description": "Task list session name (default: 'default')."
                        }
                    },
                    "required": []
                }
            }
        })),

        // ── multi_file_edit ───────────────────────────────────────────────────
        "multi_file_edit" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "multi_file_edit",
                "description": "Apply multiple exact-string replacements across one or more files atomically. \
                    All edits are validated before any file is written.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "edits": {
                            "type": "array",
                            "description": "List of edit operations to apply.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "path": {"type": "string", "description": "File path to edit."},
                                    "old_string": {
                                        "type": "string",
                                        "description": "Exact string to find (must appear exactly once in the file)."
                                    },
                                    "new_string": {
                                        "type": "string",
                                        "description": "Replacement text."
                                    }
                                },
                                "required": ["path", "old_string", "new_string"]
                            }
                        },
                        "dry_run": {
                            "type": "boolean",
                            "description": "If true, validate edits without writing any changes."
                        }
                    },
                    "required": ["edits"]
                }
            }
        })),

        // ── diff_files ────────────────────────────────────────────────────────
        "diff_files" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "diff_files",
                "description": "Compute a unified diff between two files using Myers algorithm.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path_a": {"type": "string", "description": "Path to the 'before' file."},
                        "path_b": {"type": "string", "description": "Path to the 'after' file."},
                        "context_lines": {
                            "type": "integer",
                            "description": "Lines of context around each change (default 3)."
                        }
                    },
                    "required": ["path_a", "path_b"]
                }
            }
        })),

        // ── diff_strings ──────────────────────────────────────────────────────
        "diff_strings" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "diff_strings",
                "description": "Compute a unified diff between two strings using Myers algorithm.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "a": {"type": "string", "description": "The 'before' text."},
                        "b": {"type": "string", "description": "The 'after' text."},
                        "label_a": {
                            "type": "string",
                            "description": "Label for the 'before' side in the diff header (default 'a')."
                        },
                        "label_b": {
                            "type": "string",
                            "description": "Label for the 'after' side in the diff header (default 'b')."
                        },
                        "context_lines": {
                            "type": "integer",
                            "description": "Lines of context around each change (default 3)."
                        }
                    },
                    "required": ["a", "b"]
                }
            }
        })),

        // ── http_get ──────────────────────────────────────────────────────────
        "http_get" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "http_get",
                "description": "Perform an HTTP GET request and return the response body.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "Full URL to request."},
                        "headers": {
                            "type": "object",
                            "additionalProperties": {"type": "string"},
                            "description": "Optional HTTP headers to include in the request."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Request timeout in seconds (default 30)."
                        }
                    },
                    "required": ["url"]
                }
            }
        })),

        // ── http_post ─────────────────────────────────────────────────────────
        "http_post" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "http_post",
                "description": "Perform an HTTP POST request. Send JSON via 'body' or plain text via 'body_text'.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {"type": "string", "description": "Full URL to POST to."},
                        "body": {
                            "description": "JSON body to send (sets Content-Type: application/json)."
                        },
                        "body_text": {
                            "type": "string",
                            "description": "Plain text body to send (sets Content-Type: text/plain)."
                        },
                        "headers": {
                            "type": "object",
                            "additionalProperties": {"type": "string"},
                            "description": "Optional HTTP headers to include in the request."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Request timeout in seconds (default 30)."
                        }
                    },
                    "required": ["url"]
                }
            }
        })),

        // ── web_fetch ─────────────────────────────────────────────────────────
        "web_fetch" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a URL and return the body converted to readable plain text. \
                    Strips HTML tags/scripts/styles, decodes common entities, collapses whitespace. \
                    Unlike http_get (which returns raw response), this returns just the readable text — \
                    suitable for feeding web pages to an LLM.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "Full URL to fetch (http or https)."
                        },
                        "max_chars": {
                            "type": "integer",
                            "description": "Maximum characters to return (default 50000, capped at 200000). \
                                Output is truncated with a note appended if longer."
                        }
                    },
                    "required": ["url"]
                }
            }
        })),

        // ── bash_run_background ───────────────────────────────────────────────
        "bash_run_background" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "bash_run_background",
                "description": "Spawn a shell command in the background and return an opaque handle. \
                    Use bash_output to poll its accumulated stdout/stderr and bash_kill to terminate it. \
                    The command is run via /bin/sh -c (Unix) or cmd /C (Windows).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "Shell command to execute (passed to /bin/sh -c or cmd /C)."
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Working directory for the command. Must exist."
                        },
                        "timeout_secs": {
                            "type": "integer",
                            "description": "Maximum runtime in seconds before the job is auto-killed (default 600)."
                        }
                    },
                    "required": ["command"]
                }
            }
        })),

        // ── bash_output ───────────────────────────────────────────────────────
        "bash_output" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "bash_output",
                "description": "Read accumulated stdout+stderr from a background job started by bash_run_background. \
                    Returns JSON with output (utf8, lossy-decoded), status (running/exited/killed), \
                    exit_code (or null), and total_bytes captured so far. Non-blocking.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "handle": {
                            "type": "string",
                            "description": "Handle returned by bash_run_background."
                        },
                        "since_byte": {
                            "type": "integer",
                            "description": "Byte offset to start reading from (default 0 — return all accumulated output). \
                                Pass the previous total_bytes value to get only new output."
                        }
                    },
                    "required": ["handle"]
                }
            }
        })),

        // ── bash_kill ─────────────────────────────────────────────────────────
        "bash_kill" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "bash_kill",
                "description": "Terminate a background job started by bash_run_background. \
                    Returns {killed: true} on success, or {killed: false, reason: \"...\"} \
                    if the handle is unknown or the job already exited.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "handle": {
                            "type": "string",
                            "description": "Handle returned by bash_run_background."
                        }
                    },
                    "required": ["handle"]
                }
            }
        })),

        "ask_user" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "ask_user",
                "description": "Pause the agent and ask the human a free-form question. \
                    Returns the user's typed answer as a string. Use this when you need \
                    a decision, missing information, or a clarification that you cannot \
                    infer from context. If running headless (no TTY), this returns the \
                    `default` value if set, or a message indicating no user is available.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "question": {
                            "type": "string",
                            "description": "The question to display to the user."
                        },
                        "default": {
                            "type": "string",
                            "description": "Optional fallback answer used when the user just \
                                presses Enter, or when no TTY is available."
                        }
                    },
                    "required": ["question"]
                }
            }
        })),

        "task" | "subagent" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": name,
                "description": "Spawn another configured agent as a subagent to handle a focused \
                    task. The subagent runs in an ISOLATED context (no shared history) and its \
                    final output is returned as the tool result. \n\n\
                    Use this when the user's request fits a specific role better than the \
                    current agent. Examples:\n\
                    - 'have the reviewer check this PR' → task({agent:'reviewer', prompt:'review the diff in HEAD'})\n\
                    - 'research the best Rust async runtime' → task({agent:'researcher', prompt:'compare tokio vs async-std vs smol with citations'})\n\
                    - 'fix the failing test' → task({agent:'coder', prompt:'run cargo test, identify the failure, fix it'})\n\n\
                    The result is returned as plain text starting with a [subagent: <name>] header.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent": {
                            "type": "string",
                            "description": "Name of the agent to spawn. Must match a [agent.<name>] block in agents.toml. Common values: master, coder, reviewer, researcher. Accepts `subagent_type` as alias for Claude Code Agent-tool compatibility."
                        },
                        "subagent_type": {
                            "type": "string",
                            "description": "Alias for `agent` — matches Claude Code's Agent tool field name. Either `agent` OR `subagent_type` is required."
                        },
                        "description": {
                            "type": "string",
                            "description": "Short label for the task (3-5 words). Compatibility shim with Claude Code's Agent tool — accepted but currently unused by phantom (logged for /tasks UI in future)."
                        },
                        "prompt": {
                            "type": "string",
                            "description": "The task description for the subagent. Be specific and self-contained — the subagent has NO access to the current conversation."
                        },
                        "format": {
                            "type": "string",
                            "enum": ["wrapped", "raw", "json"],
                            "description": "Output format. `wrapped` (default) prepends '[subagent: <agent> · <rounds> rounds · $<cost> · <secs>s]' header for human readability. `raw` returns just the agent's output text — byte-for-byte parity with Claude Code Agent's return shape, ideal when chaining results. `json` returns a structured envelope {agent, rounds, cost_usd, elapsed_secs, output, status} for programmatic consumers."
                        },
                        "max_rounds": {
                            "type": "integer",
                            "description": "Optional cap on the subagent's tool-call rounds. Default = the agent's configured limit."
                        },
                        "max_secs": {
                            "type": "integer",
                            "description": "Optional wall-clock timeout for the subagent in seconds."
                        },
                        "max_cost_usd": {
                            "type": "number",
                            "description": "Optional cost ceiling — if the subagent's tracked cost exceeds this value, the result is wrapped in a budget-exceeded notice."
                        },
                        "node": {
                            "type": "string",
                            "description": "Optional mesh peer URL or substring (e.g. 'yoyogood', '100.87.70.65', or full 'http://100.87.70.65:7879'). When set, the subagent runs on that remote peer's phantom serve and only the result string is returned. Useful to offload heavy work to a specific cluster machine."
                        },
                        "auto_snapshot": {
                            "type": "boolean",
                            "description": "macOS-only safety net: take an APFS local snapshot via `tmutil` BEFORE the subagent runs. The snapshot id is prepended to the result so a misbehaving subagent can be rolled back with `phantom snapshot rollback <id>`. ~1s overhead, no sudo required, no-op on non-mac. Default: false."
                        }
                    },
                    "anyOf": [
                        {"required": ["agent", "prompt"]},
                        {"required": ["subagent_type", "prompt"]}
                    ]
                }
            }
        })),

        "parallel_tasks" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "parallel_tasks",
                "description": "Spawn MULTIPLE subagents concurrently and join their outputs. \
                    Use this when several distinct sub-questions can be answered independently — \
                    you'll get all answers back roughly in the time of the slowest single one. \
                    Each task is {agent, prompt, node?}; budgets (max_rounds, max_secs, \
                    max_cost_usd) apply to every spawned subagent uniformly. Per-task `node` \
                    routes that ONE task to a cluster peer (full URL, host:port substring, or \
                    unique prefix like 'mac1'/'yoyogood'); when omitted the task runs locally.\n\n\
                    Example local: parallel_tasks({tasks: [\
                    {agent:'researcher', prompt:'how does tokio executor pin tasks?'}, \
                    {agent:'coder', prompt:'show a tokio::spawn example'}]})\n\n\
                    Example fan-out across two Macs: parallel_tasks({tasks: [\
                    {agent:'coder', prompt:'review repo A', node:'mac1'}, \
                    {agent:'coder', prompt:'review repo B', node:'mac2'}]})",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "tasks": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "agent":  {"type": "string"},
                                    "prompt": {"type": "string"},
                                    "node":   {"type": "string", "description": "Optional cluster peer to dispatch this task to. Forms accepted: full URL ('http://100.x.x.x:7878'), host:port substring ('100.x.x.x:7878'), or any unique prefix ('mac1' / 'yoyogood'). Empty = run locally."}
                                },
                                "required": ["agent", "prompt"]
                            },
                            "description": "Array of {agent, prompt, node?} subtasks to run concurrently."
                        },
                        "max_rounds":   {"type": "integer"},
                        "max_secs":     {"type": "integer"},
                        "max_cost_usd": {"type": "number"},
                        "format": {
                            "type": "string",
                            "enum": ["wrapped", "raw", "json"],
                            "description": "`wrapped` (default): single string with [parallel_tasks · N] header + per-subagent ── #i agent ── blocks. `raw`: outputs joined with double newlines (no labels). `json`: array of {label, agent, status, rounds, cost_usd, elapsed_secs, output} — closest to how Claude Code returns N independent Agent tool results."
                        }
                    },
                    "required": ["tasks"]
                }
            }
        })),

        // ── Cluster awareness ─────────────────────────────────────────────
        // These three are READ-ONLY discovery tools. To actually run work
        // on a remote peer, use `task` or `parallel_tasks` with a `node:`
        // parameter — those already handle dispatch via /rpc/message.
        "cluster_status" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "cluster_status",
                "description": "Ping every configured cluster peer in parallel and return alive/dead + RTT. \
                    Call this BEFORE deciding which peer to delegate work to via task({node:'X'}) — \
                    a dead peer would just time out the dispatch. Free / fast (1 round-trip per peer).",
                "parameters": {"type": "object", "properties": {}}
            }
        })),
        "cluster_sessions" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "cluster_sessions",
                "description": "List live phantom TUI sessions across the user's mesh (any session that \
                    heartbeated to the broker within the last 60s). Shows machine, agent, cwd, alive \
                    duration, last-seen-ago. Useful when the user asks 'where am I working right now' \
                    or you want to avoid dispatching to a machine someone else is actively using.",
                "parameters": {"type": "object", "properties": {}}
            }
        })),
        "cluster_peers" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "cluster_peers",
                "description": "Return the static peer registry — names, URLs, capability tags, and \
                    which one is THIS machine — as JSON. No network call (reads ~/.phantom-mesh/peers.json). \
                    Use this to enumerate dispatch targets BY NAME so you can pass them as `node:` to \
                    task / parallel_tasks. Capabilities (e.g. ['rust','gpu']) are user-tagged hints \
                    about what the peer is good at.",
                "parameters": {"type": "object", "properties": {}}
            }
        })),

        // ── Self-introspection — read phantom's own diagnostic state ──────
        "diag_read" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "diag_read",
                "description": "Read phantom's own diagnostic state — recent events, crash logs, or a one-paragraph summary. Use this when the user asks 'what just went wrong', when investigating an evolve/autoevolve failure, or when self-debugging. The agent should always call this FIRST when a panic/crash is mentioned, before guessing root causes.\n\nKinds:\n  summary      — counts + last crash path + top event kinds (start here)\n  events       — last N events from the in-memory ring (default 30)\n  crashes      — list of recent crash log files (default 5)\n  last_crash   — full content of the newest crash log (capped at 8000 chars)",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "kind":  {"type": "string", "enum": ["summary","events","crashes","last_crash"], "description": "What to read. Default 'summary'."},
                        "limit": {"type": "integer", "description": "For events/crashes — how many to return."}
                    }
                }
            }
        })),

        // ── macOS-only: Spotlight ──────────────────────────────────────────
        #[cfg(target_os = "macos")]
        "spotlight_search" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "spotlight_search",
                "description": "macOS Spotlight (`mdfind`) wrapper. Uses the system content index for sub-100ms searches across the whole filesystem. Pass a substring (matched against display names, case-insensitive) OR a raw Spotlight expression (anything containing '=' or 'kMDItem'). Optional scope, time-window, and result cap. Much faster than glob_search for system-wide queries.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Substring to match against display names, OR a raw Spotlight expression (e.g. `kMDItemContentType == \"public.swift-source\"`)."
                        },
                        "scope": {
                            "type": "string",
                            "description": "Optional directory to limit the search to (e.g. /Users/me/projects)."
                        },
                        "changed_within_hours": {
                            "type": "integer",
                            "description": "Restrict to files whose content changed within the last N hours."
                        },
                        "max_results": {
                            "type": "integer",
                            "description": "Max paths returned (default 50, hard cap 500)."
                        }
                    },
                    "required": ["query"]
                }
            }
        })),

        // ── macOS-only: Xcode simctl ───────────────────────────────────────
        #[cfg(target_os = "macos")]
        "xcode_simctl" => Some(serde_json::json!({
            "type": "function",
            "function": {
                "name": "xcode_simctl",
                "description": "Drive the iOS Simulator via `xcrun simctl`. Common actions: list (runtimes/devices/devicetypes), boot, shutdown, shutdown_all, erase, erase_all, install, uninstall, launch, terminate, openurl, screenshot. Requires Xcode command-line tools.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "description": "simctl verb. One of: list, boot, shutdown, shutdown_all, erase, erase_all, install, uninstall, launch, terminate, openurl, screenshot."
                        },
                        "device": {
                            "type": "string",
                            "description": "Target device UUID or name. Defaults to 'booted'."
                        },
                        "args": {
                            "type": "array",
                            "items": {"type": "string"},
                            "description": "Extra positional args forwarded to simctl after the action verb."
                        },
                        "path": {
                            "type": "string",
                            "description": "For action=screenshot: output PNG path. Auto-generated under /tmp if omitted."
                        }
                    },
                    "required": ["action"]
                }
            }
        })),

        _ => None,
    }
}
