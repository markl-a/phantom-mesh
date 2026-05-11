# Phantom Mesh — Tool Reference

This document is the authoritative reference for every tool available to agents running inside the phantom-mesh core. It is written so that another AI agent can understand exactly what each tool does, what arguments it accepts, and how to chain tools together effectively.

---

## Overview Table

| Tool | Module | Description |
|---|---|---|
| `shell` | shell | Execute a shell command (with safety filtering) and return stdout/stderr + exit code |
| `file_read` | file | Read file contents, optionally sliced by line range |
| `file_write` | file | Write (or create) a file with arbitrary content |
| `file_edit` | file | Replace an exact string occurrence inside a file |
| `content_search` | search | Search file contents using ripgrep (regex/literal, with context lines) |
| `glob_search` | search | Find files by name glob pattern |
| `web_search` | web | Search the web via Brave API or DuckDuckGo fallback |
| `memory_store` | memory | Persist a key-value pair to disk |
| `memory_recall` | memory | Retrieve a value by key from disk |
| `memory_list` | memory | List all stored memory entries (optionally filtered by namespace) |
| `memory_delete` | memory | Delete a single memory entry by key |
| `memory_search` | memory | Search memory values by substring |
| `git_status` | git | Show the short git working-tree status |
| `git_diff` | git | Show git diff stat (staged or unstaged, optionally for one file) |
| `git_log` | git | Show recent git commits (oneline format) |
| `git_commit` | git | Create a git commit with a message |
| `git_add` | git | Stage one or more files for the next commit |
| `git_push` | git | Push commits to a remote (requires PHANTOM_AUTO_APPROVE=1) |
| `git_reset` | git | Reset the working tree (soft/hard) |
| `git_blame` | git | Show per-line authorship of a file |
| `git_show` | git | Show details of a specific commit |
| `git_branch_list` | git | List local (or all) branches with verbose info |
| `git_checkout` | git | Switch branch or create a new branch |
| `git_stash_list` | git | List all stashes in the repository |
| `fetch` | fetch | Fetch a URL and return cleaned text (HTML stripped, JSON pretty-printed) |
| `http_get` | http_client | Raw HTTP GET request; returns status + body |
| `http_post` | http_client | Raw HTTP POST request with JSON or text body |
| `ls` | ls | List directory contents (short or long format, optional tree view) |
| `ls_stat` | ls | Stat a single file/directory (size, permissions, timestamps, line count) |
| `list_files` | fs | Recursively list files under a directory, with optional name filter |
| `list_dir` | fs | List one directory level with sizes |
| `create_dir` | fs | Create a directory (and parents) |
| `rename_file` | fs | Rename/move a file (requires PHANTOM_AUTO_APPROVE=1) |
| `delete_file` | fs | Delete a single file (max 10 MB safety guard) |
| `patch` | patch | Apply a unified diff patch to one or more files |
| `multi_file_edit` | multi_edit | Apply multiple exact-string replacements atomically across files |
| `diff_files` | diff_view | Generate a unified diff between two files |
| `diff_strings` | diff_view | Generate a unified diff between two strings |
| `cargo_check` | diagnostic | Run `cargo check` and summarise errors/warnings |
| `cargo_test` | diagnostic | Run `cargo test` and summarise results |
| `tsc_check` | diagnostic | Run TypeScript compiler type-check (no emit) |
| `run_tests` | diagnostic | Run any test command and return output |
| `task_add` | task | Add a task to the in-session task list |
| `task_update` | task | Update the status of a task |
| `task_list` | task | List tasks (optionally filtered by status) |
| `task_clear` | task | Remove tasks (all, or done-only) |
| `shell_bg` | shell | Spawn a long-running background job (returns immediately with PID) |
| `shell_bg_check` | shell | Check status of background jobs |

---

## Tool Reference

---

### `shell`

Execute a shell command and return stdout, stderr, and exit code. Supports compound commands (`&&`, `||`, `;`). Certain destructive patterns require `PHANTOM_AUTO_APPROVE=1`.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `command` | string | yes | — | Shell command. Supports `&&`, `||`, `;` compound operators (max 10 parts). `$(...)` and backtick substitution are blocked. |
| `timeout_secs` | integer | no | 30 | Max execution time in seconds (capped at 300). |
| `cwd` | string | no | current dir | Working directory; must exist. |
| `env` | object | no | `{}` | Extra environment variables merged with the current environment. |
| `stdin` | string | no | — | Text piped into the command's stdin. |

**Security notes:**
- Hard-blocked patterns (always rejected): `rm -rf /`, `rm -rf ~`, `sudo rm`, `sudo dd`, `mkfs`, `dd if=/dev/zero of=/dev/`, `:(){:|:&};:`, `chmod -R 777 /`, `curl | sh`, etc.
- Patterns requiring `PHANTOM_AUTO_APPROVE=1`: `rm `, `sudo `, `kill `, `pkill `, `git reset --hard`, `git clean `, `chmod `, `chown `, `DROP TABLE`, `curl `, `wget `, `nc `, and `mv`/`cp` to absolute paths.
- `$(...)` subshell substitution and backtick expansion are always blocked.

**Output format:**
- stdout only: raw text + `[exit code: N]`
- both stdout and stderr: `STDOUT:\n<text>\nSTDERR:\n<text>\n[exit code: N]`
- stderr only: `STDERR:\n<text>\n[exit code: N]`
- Output is truncated at 20,000 characters.

**Example:**
```json
{"command": "cargo build --release", "cwd": "/workspace/myproject", "timeout_secs": 120}
```

**Example with env and stdin:**
```json
{"command": "cat", "stdin": "hello world", "env": {"DEBUG": "1"}}
```

**Returns:** Combined stdout/stderr with exit code appended.

---

### `shell_bg`

Spawn a long-running command in the background without waiting for it to finish. Returns immediately with the PID.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `command` | string | yes | — | Command to run in the background. |
| `label` | string | no | same as command | Human-readable label for tracking. |

**Example:**
```json
{"command": "sleep 600", "label": "keep-alive"}
```

**Returns:** `Job started: PID=12345 label='keep-alive'\nUse shell with command 'kill 12345' to stop...`

---

### `shell_bg_check`

Check status of background jobs tracked by `shell_bg`.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `pid` | integer | no | — | Specific PID to check. If omitted, lists all tracked jobs. |

**Example:**
```json
{"pid": 12345}
```

**Returns:** `PID 12345 (keep-alive): running` or `finished`.

---

### `file_read`

Read the contents of a file. Binary files are detected and reported without decoding. Output is truncated at 100,000 characters.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | yes | — | Absolute or relative file path. |
| `offset` | integer | no | 1 | Start line, 1-based. Use with `limit` for windowed reading. |
| `limit` | integer | no | all | Maximum number of lines to return. |
| `start_line` | integer | no | — | Legacy: 1-based start line (prefer `offset`). |
| `end_line` | integer | no | — | Legacy: 1-based end line inclusive (prefer `offset`+`limit`). |
| `show_line_numbers` | boolean | no | false | If true, prefix every line with its line number. |

When `offset` or `limit` is provided, the response includes a header like `[Lines 10-59 of 300]` and each line is prefixed with its line number.

**Example — read whole file:**
```json
{"path": "src/main.rs"}
```

**Example — read lines 100-149:**
```json
{"path": "src/main.rs", "offset": 100, "limit": 50}
```

**Example — read with line numbers:**
```json
{"path": "Cargo.toml", "show_line_numbers": true}
```

**Returns:** File text (possibly with line-number prefixes), or `[binary file, N bytes]`, or an error string.

---

### `file_write`

Write content to a file, creating missing parent directories by default.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | yes | — | Destination file path. |
| `content` | string | yes | `""` | Full content to write (overwrites existing file). |
| `create_dirs` | boolean | no | true | If `true`, create missing parent directories automatically. |

**Example:**
```json
{"path": "src/config.rs", "content": "pub const VERSION: &str = \"1.0\";\n"}
```

**Returns:** `Written N bytes to <path>` or an error string.

---

### `file_edit`

Replace an exact string in a file. The `old_string` must match exactly once (by default). Use `replace_all` to replace every occurrence. Use `line_range` to restrict the search scope.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | yes | — | File to edit. |
| `old_string` | string | yes | — | Exact text to find. Must match exactly once unless `replace_all` is true. |
| `new_string` | string | no | `""` | Replacement text. |
| `replace_all` | boolean | no | false | If true, replace every occurrence. |
| `line_range` | object | no | — | Scope the search: `{"start": N, "end": M}` (1-based, inclusive). |

**Errors:**
- `old_string not found` — text not present; response includes a 200-character preview of the search scope.
- `old_string appears N times` — when `replace_all` is false and multiple matches exist; response lists the line numbers.

**Example — single replacement:**
```json
{"path": "src/lib.rs", "old_string": "fn old_name(", "new_string": "fn new_name("}
```

**Example — replace all within a range:**
```json
{
  "path": "src/config.rs",
  "old_string": "TODO",
  "new_string": "DONE",
  "replace_all": true,
  "line_range": {"start": 10, "end": 50}
}
```

**Returns:** On success: `Edited <path> successfully.\n\nDiff:\n<mini-diff>` (or, for `replace_all`, `Edited <path> (N occurrence(s) replaced).`).

---

### `content_search`

Search file contents using ripgrep (falls back to grep if rg is not installed). Returns matching lines with file paths and context.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `pattern` | string | yes | — | Regex or literal search pattern (max 500 characters). |
| `path` | string | no | `.` | Directory or file to search. |
| `context_lines` | integer | no | 2 | Lines of context shown before and after each match. |
| `file_type` | string | no | — | Filter by file type without dot, e.g. `"rs"`, `"ts"`, `"py"`. Uses ripgrep's `-t` flag. |
| `case_sensitive` | boolean | no | false | If true, perform a case-sensitive search. |
| `max_results` | integer | no | 50 | Maximum match lines to return. |

**Security:** Path argument is validated — `..` traversal and shell-injection characters (`;`, `|`, `&`, `$`, `` ` ``, `>`, `<`) are rejected.

**Example — find all uses of a function:**
```json
{"pattern": "fn send_message", "path": "core/src", "file_type": "rs"}
```

**Example — case-sensitive search:**
```json
{"pattern": "TODO", "path": ".", "case_sensitive": true, "context_lines": 0}
```

**Returns:** ripgrep output (file:line:content lines separated by `--` hunk delimiters) or `No matches found`.

---

### `glob_search`

Find files matching a glob pattern. Uses ripgrep `--files` for speed (automatically excludes `.git/`, `node_modules/`, `target/`); falls back to `find`.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `pattern` | string | yes | — | Glob pattern, e.g. `"**/*.rs"`, `"src/**/*.ts"`, `"*.toml"` (max 200 characters). |
| `path` | string | no | `.` | Base directory to search in. |
| `exclude` | array of strings | no | `[]` | Additional glob patterns to exclude, e.g. `["tests/**", "*.lock"]`. |
| `max_results` | integer | no | 200 | Maximum number of files to return. |

**Example — find all Rust source files:**
```json
{"pattern": "**/*.rs", "path": "core/src"}
```

**Example — find TypeScript files, excluding tests:**
```json
{"pattern": "**/*.ts", "path": "app/src", "exclude": ["**/*.test.ts"]}
```

**Returns:** Sorted list of matching file paths, one per line, with a truncation notice if the limit is hit.

---

### `web_search`

Search the web. Uses the Brave Search API when `brave_search_api_key` is configured in `agents.toml`; otherwise falls back to the DuckDuckGo Instant Answer API and then DuckDuckGo HTML search.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | string | yes | — | Search query. |
| `num_results` | integer | no | 5 | Number of results to return (max 10). |

**Example:**
```json
{"query": "Rust async tokio tutorial 2024", "num_results": 5}
```

**Returns:** Numbered list: `[N] Title\n    URL: ...\n    Snippet: ...` or `No results for: <query>`.

---

### `memory_store`

Persist a key-value string pair to `~/.phantom-mesh/memory.json`. Supports namespaced keys.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `key` | string | yes | — | Key name. |
| `value` | string | yes | — | Value to store. |
| `namespace` | string | no | — | Prefix; the stored key becomes `{namespace}/{key}`. |

**Example:**
```json
{"key": "project_root", "value": "/workspace/myproject", "namespace": "agent_a"}
```

**Returns:** `Stored: agent_a/project_root = /workspace/myproject`

---

### `memory_recall`

Retrieve a stored value by key.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `key` | string | yes | — | Key to look up. |
| `namespace` | string | no | — | Namespace prefix (must match what was used in `memory_store`). |

**Example:**
```json
{"key": "project_root", "namespace": "agent_a"}
```

**Returns:** The stored value string, or `No memory found for key: <key>`.

---

### `memory_list`

List all stored keys and truncated values (values cut at 50 characters). Optionally filter by namespace.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `namespace` | string | no | — | If provided, only list keys whose full name starts with `{namespace}/`. |

**Example — list all:**
```json
{}
```

**Example — list one namespace:**
```json
{"namespace": "agent_a"}
```

**Returns:** Sorted lines of `key: value…` or `No memory entries stored.`

---

### `memory_delete`

Delete a single memory entry by key.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `key` | string | yes | — | Key to delete. |
| `namespace` | string | no | — | Namespace prefix. |

**Example:**
```json
{"key": "project_root", "namespace": "agent_a"}
```

**Returns:** `deleted` or `key not found`.

---

### `memory_search`

Search memory values by substring match (case-insensitive). Optionally restrict to a namespace.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `query` | string | yes | — | Substring to search for in values. |
| `namespace` | string | no | — | Restrict search to this namespace. |

**Example:**
```json
{"query": "workspace", "namespace": "agent_a"}
```

**Returns:** Sorted lines of `key: value` for matching entries, or `No memory entries matching '<query>'.`

---

### `git_status`

Show the git working-tree status in short format (`git status --short`).

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Git repository path. |

**Example:**
```json
{"path": "/workspace/myproject"}
```

**Returns:** Short-format status lines (e.g. `M src/main.rs`, ` M Cargo.lock`) or `Working tree clean`.

---

### `git_diff`

Show a git diff stat (number of insertions/deletions per file). Can target staged changes or a specific file.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Repository directory. |
| `cached` | boolean | no | false | If true, show staged (cached) diff. |
| `file` | string | no | — | Restrict diff to this file. |

**Example — unstaged changes:**
```json
{"path": "."}
```

**Example — staged diff for one file:**
```json
{"cached": true, "file": "src/agent.rs"}
```

**Returns:** `git diff --stat` output.

---

### `git_log`

Show recent commits in oneline format (`git log --oneline -N`).

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Repository directory. |
| `n` | integer | no | 10 | Number of commits to show. |

**Example:**
```json
{"n": 5}
```

**Returns:** Commit hashes and messages, one per line.

---

### `git_commit`

Create a git commit. You must stage files first (use `git_add` or `shell` with `git add`).

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `message` | string | yes | — | Commit message (max 1000 characters; `$(...)` and backticks are blocked). |
| `path` | string | no | `.` | Repository directory. |

**Example:**
```json
{"message": "fix: correct off-by-one in line range calculation"}
```

**Returns:** Combined stdout/stderr from `git commit -m`.

---

### `git_add`

Stage one or more files for the next commit (`git add -- <files>`).

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `files` | array of strings | yes | — | List of file paths to stage. Must be non-empty. |
| `path` | string | no | `.` | Repository directory. |

**Example:**
```json
{"files": ["src/agent.rs", "src/session.rs"], "path": "/workspace/myproject"}
```

**Returns:** `Staged N file(s): file1, file2` or an error string.

---

### `git_push`

Push commits to a remote repository. **Requires `PHANTOM_AUTO_APPROVE=1`.**

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Repository directory. |
| `remote` | string | no | `origin` | Remote name. |
| `branch` | string | no | `HEAD` | Branch ref to push. |

**Example:**
```json
{"remote": "origin", "branch": "main"}
```

**Returns:** Combined stdout/stderr from `git push`, or `APPROVAL_REQUIRED: ...` if not auto-approved.

---

### `git_reset`

Reset the working tree. Hard reset **requires `PHANTOM_AUTO_APPROVE=1`**.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `mode` | string | no | `soft` | Reset mode: `"soft"`, `"mixed"`, or `"hard"`. |
| `path` | string | no | `.` | Repository directory. |

**Example:**
```json
{"mode": "soft"}
```

**Returns:** `Reset complete.` or an error/approval-required string.

---

### `git_blame`

Show who last modified each line of a file. Output is truncated to 100 lines.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | yes | — | The FILE to blame (not the repo dir). |
| `repo` | string | no | `.` | Repository directory. |

**Example:**
```json
{"path": "core/src/agent.rs", "repo": "/workspace/myproject"}
```

**Returns:** `git blame` output (up to 100 lines) with `... (output truncated to 100 lines)` notice if longer.

---

### `git_show`

Show details of a commit: stat and optionally the full diff.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `ref_` | string | no | `HEAD` | Commit ref (hash, tag, branch, etc.). |
| `stat_only` | boolean | no | false | If true, show only `--stat` output (no full diff). |
| `path` | string | no | `.` | Repository directory. |

**Example — show last commit with diff:**
```json
{"ref_": "HEAD"}
```

**Example — show specific commit stat only:**
```json
{"ref_": "abc1234", "stat_only": true}
```

**Returns:** `git show` output.

---

### `git_branch_list`

List branches with verbose info (last commit hash and message).

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Repository directory. |
| `remote` | boolean | no | false | If true, include remote-tracking branches (`git branch -av`). |

**Example:**
```json
{"remote": true}
```

**Returns:** Branch list output or `No branches found.`

---

### `git_checkout`

Switch to an existing branch or create a new one.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `branch` | string | yes | — | Target branch name. |
| `create` | boolean | no | false | If true, create the branch (`git checkout -b`). |
| `path` | string | no | `.` | Repository directory. |

**Example — switch branch:**
```json
{"branch": "feature/new-tool"}
```

**Example — create and switch:**
```json
{"branch": "feature/new-tool", "create": true}
```

**Returns:** Combined stdout/stderr from `git checkout`.

---

### `git_stash_list`

List all stashes in the repository.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Repository directory. |

**Example:**
```json
{}
```

**Returns:** `git stash list` output or `No stashes.`

---

### `fetch`

Fetch a URL and return its content as readable text. For HTML, strips script/style/nav/footer/header tags, removes comments, converts headings and block elements to newlines, and decodes HTML entities. JSON responses are pretty-printed. Only `http://` and `https://` are accepted; private/loopback IPs are blocked.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `url` | string | yes | — | Must start with `http://` or `https://`. Max length 2000 characters. |
| `timeout_secs` | integer | no | 15 | Request timeout in seconds. |
| `max_length` | integer | no | 8000 | Maximum characters to return (hard cap 50,000). Excess is replaced with `[... truncated]`. |
| `raw` | boolean | no | false | If true, return raw HTML/text without stripping. |
| `selector` | string | no | — | HTML tag name hint (e.g. `"article"`, `"main"`) to narrow extraction to the first matching element. |

**Supported content types:** `text/html`, `text/plain`, `application/json`. Other content types return an error.

**Blocked:** `localhost`, `::1`, `127.x.x.x`, `10.x.x.x`, `172.16-31.x.x`, `192.168.x.x`, `169.254.x.x`.

**Example — fetch and clean a doc page:**
```json
{"url": "https://docs.rs/tokio/latest/tokio/", "selector": "main", "max_length": 10000}
```

**Example — fetch raw JSON API:**
```json
{"url": "https://api.github.com/repos/rust-lang/rust/releases/latest", "max_length": 5000}
```

**Returns:** `Title: ...\nURL: ...\n---\n<cleaned content>` for HTML, or pretty JSON, or an error string.

---

### `http_get`

Raw HTTP GET request. No HTML cleaning — returns status code, Content-Type header, and raw body. Body is truncated at 8000 characters.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `url` | string | yes | — | Target URL. |
| `timeout_secs` | integer | no | 30 | Request timeout. |
| `headers` | object | no | `{}` | Extra HTTP headers as key-value pairs. |

**Example:**
```json
{
  "url": "https://api.example.com/status",
  "headers": {"Authorization": "Bearer <token>", "Accept": "application/json"}
}
```

**Returns:** `HTTP 200 OK\nContent-Type: application/json\n---\n<body>` or `ERROR: HTTP 404 Not Found\nURL: ...`

---

### `http_post`

Raw HTTP POST request. Sends either a JSON body or plain-text body.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `url` | string | yes | — | Target URL. |
| `body` | any JSON value | no | — | JSON body (sets `Content-Type: application/json` automatically). |
| `body_text` | string | no | — | Plain-text body (sets `Content-Type: text/plain`). If both `body` and `body_text` are absent, an empty body is sent. |
| `timeout_secs` | integer | no | 30 | Request timeout. |
| `headers` | object | no | `{}` | Extra HTTP headers. |

**Example — JSON body:**
```json
{
  "url": "https://api.example.com/create",
  "body": {"name": "agent-x", "role": "worker"},
  "headers": {"Authorization": "Bearer <token>"}
}
```

**Example — plain-text body:**
```json
{"url": "https://webhook.example.com/ping", "body_text": "hello"}
```

**Returns:** Same format as `http_get`.

---

### `ls`

List directory contents. Sorts directories first (alphabetically), then files (alphabetically). Supports long format and tree view.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Directory to list. |
| `long` | boolean | no | false | If true, show permissions, size, and modification date (like `ls -l`). |
| `tree` | boolean | no | false | If true, render a tree view up to 3 levels deep. |
| `hidden` | boolean | no | false | If true, include hidden files and directories (names starting with `.`). |
| `max_entries` | integer | no | 200 | Maximum entries to show; a truncation notice is appended if exceeded. |

**Example — simple list:**
```json
{"path": "core/src"}
```

**Example — long format:**
```json
{"path": "core/src", "long": true}
```

**Example — tree view:**
```json
{"path": "core", "tree": true}
```

**Returns:** Entry names (directories suffixed with `/`). Long format columns: `permissions  size  date  name`. Tree format uses Unicode box-drawing characters.

---

### `ls_stat`

Get detailed metadata for a single file or directory.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | yes | — | File or directory path. |

**Example:**
```json
{"path": "core/src/agent.rs"}
```

**Returns:**
```
Path:     /abs/path/to/file.rs
Type:     file
Size:     4096 bytes (4.0 KB)
Modified: 2026-04-24 10:30:00 UTC
Created:  2026-01-01 00:00:00 UTC
Perms:    644
Lines:    120
```
(Line count is only shown for text files under 1 MB.)

---

### `list_files`

Recursively list all files under a directory (up to 15 levels deep, max 500 results). Automatically skips `node_modules`, `.git`, `target`, `.next`, `dist`, `__pycache__`, `.cache`.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Base directory. |
| `pattern` | string | no | `""` | Simple name filter. Supports `prefix*`, `*suffix`, `*middle*`, or exact match. |

**Example — all files:**
```json
{"path": "core/src"}
```

**Example — only Rust files:**
```json
{"path": "core/src", "pattern": "*.rs"}
```

**Returns:** One file path per line, or `No files found`.

---

### `list_dir`

List a single directory level with entry name and size/type annotation.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Directory to list. Path traversal (`..`) is blocked. |

**Example:**
```json
{"path": "core/src/tools"}
```

**Returns:** Lines like `agent.rs (8192 bytes)`, `tools/ (dir)`, sorted alphabetically. Truncated at 10,000 characters.

---

### `create_dir`

Create a directory and all necessary parent directories (`mkdir -p` equivalent).

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | yes | — | Directory path to create. Path traversal (`..`) is blocked. |

**Example:**
```json
{"path": "core/src/tools/new_module"}
```

**Returns:** `Created directory: <abs-path>` or an error string.

---

### `rename_file`

Rename or move a file. **Requires `PHANTOM_AUTO_APPROVE=1`.**

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `src` | string | yes | — | Source file path. |
| `dst` | string | yes | — | Destination path. |

Path traversal (`..`) in either argument is blocked.

**Example:**
```json
{"src": "core/src/old_name.rs", "dst": "core/src/new_name.rs"}
```

**Returns:** `Renamed: <src> -> <dst>` or `APPROVAL_REQUIRED: ...`

---

### `delete_file`

Delete a single file. Refuses to delete directories or files larger than 10 MB.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | yes | — | File to delete. Path traversal (`..`) is blocked. |

**Example:**
```json
{"path": "core/src/old_module.rs"}
```

**Returns:** `Deleted: <path>` or an error string.

---

### `patch`

Apply a unified diff patch (e.g. from `git diff` or `diff_files`) to one or more files. Validates hunk context before writing.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `patch` | string | yes | — | Full unified diff text (can include multiple `--- / +++` file sections and multiple `@@` hunks). |
| `base_dir` | string | no | current dir | Base directory for resolving relative file paths in the diff. |
| `dry_run` | boolean | no | false | If true, describe what would change without modifying any files. |

**Patch format:** Standard unified diff as produced by `git diff`, `diff -u`, or `diff_files`. Supports `+++ b/path` and `+++ path` prefixes.

**Example:**
```json
{
  "patch": "--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,3 @@\n fn main() {\n-    println!(\"hello\");\n+    println!(\"world\");\n }\n",
  "base_dir": "/workspace/myproject"
}
```

**Returns:** `Applied N hunk(s) to M file(s). Modified: file1, file2\n\nPatched <path> (N hunks)` or per-file error details. On failure, hunk context mismatch details are reported.

---

### `multi_file_edit`

Apply multiple exact-string replacements across multiple files in a single atomic operation. All replacements are validated first; if any validation fails, no files are touched.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `edits` | array of objects | yes | — | List of edit specs. Each object must have `path` (string), `old_string` (string), and `new_string` (string). |
| `dry_run` | boolean | no | false | If true, report what would be changed without modifying files. |

Each `old_string` must match **exactly once** in its file — zero or multiple matches cause the entire batch to fail with no writes.

**Example:**
```json
{
  "edits": [
    {"path": "src/agent.rs", "old_string": "version = \"1.0\"", "new_string": "version = \"2.0\""},
    {"path": "Cargo.toml", "old_string": "version = \"1.0\"", "new_string": "version = \"2.0\""}
  ]
}
```

**Returns:** `Applied N edit(s):\n  <path>: replaced '<old>' → '<new>'\n  ...` or `Validation failed — no changes were made:\nERROR: ...`

---

### `diff_files`

Generate a unified diff between two files using the Myers diff algorithm.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path_a` | string | yes | — | First file path (shown as `a/path_a`). |
| `path_b` | string | yes | — | Second file path (shown as `b/path_b`). |
| `context_lines` | integer | no | 3 | Lines of context around each changed hunk. |

**Example:**
```json
{"path_a": "src/agent.rs.bak", "path_b": "src/agent.rs", "context_lines": 5}
```

**Returns:** Unified diff text (`--- a/...\n+++ b/...\n@@ ... @@\n...`) or `Files are identical`. Truncated at 5000 characters.

---

### `diff_strings`

Generate a unified diff between two strings (without reading files).

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `a` | string | yes | — | First string. |
| `b` | string | yes | — | Second string. |
| `label_a` | string | no | `"a"` | Label for the first string in the diff header. |
| `label_b` | string | no | `"b"` | Label for the second string in the diff header. |
| `context_lines` | integer | no | 3 | Lines of context around each changed hunk. |

**Example:**
```json
{
  "a": "fn foo() {}\n",
  "b": "fn bar() {}\n",
  "label_a": "before",
  "label_b": "after"
}
```

**Returns:** Same format as `diff_files`.

---

### `cargo_check`

Run `cargo check --message-format=short` and summarise the result. Falls back to plain `cargo check` if the flag is unsupported. Timeout: 120 seconds.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Directory containing `Cargo.toml` (manifest dir). |
| `package` | string | no | — | Specific package name to check (passes `--package <name>`). |

**Example:**
```json
{"path": "core"}
```

**Returns:** On success: `✓ cargo check passed (N warnings)`. On failure: `cargo check failed:\n<error lines>` (truncated at 5000 characters).

---

### `cargo_test`

Run `cargo test` with `--nocapture` and summarise pass/fail counts. Timeout: 120 seconds.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Manifest directory. |
| `filter` | string | no | — | Test name filter (substring match, passed as positional arg to cargo test). |
| `package` | string | no | — | Specific package to test. |

**Example — run all tests:**
```json
{"path": "core"}
```

**Example — run specific test:**
```json
{"path": "core", "filter": "test_parse_compound"}
```

**Returns:** `N passed, M failed` summary. On failure, lists failed test names and output (truncated at 3000 characters).

---

### `tsc_check`

Run the TypeScript compiler in type-check mode (`--noEmit`). Tries `tsc` first, then falls back to `npx tsc`. Timeout: 120 seconds.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `path` | string | no | `.` | Project directory (where `tsconfig.json` lives). |
| `config` | string | no | — | Explicit tsconfig path (passed as `--project <config>`). |

**Example:**
```json
{"path": "app"}
```

**Returns:** `✓ TypeScript check passed` or `TypeScript errors:\n<error lines>` (truncated at 5000 characters).

---

### `run_tests`

Run any arbitrary test command (e.g. `pytest`, `jest`, `go test ./...`) and return the output. Timeout: 120 seconds.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `command` | string | yes | — | Full test command, e.g. `"pytest tests/ -v"`. |
| `path` | string | no | `.` | Working directory for the command. |

**Example:**
```json
{"command": "pytest tests/ -x --tb=short", "path": "/workspace/myproject"}
```

**Returns:** `Tests completed successfully.\n<output>` or `Tests finished with failures.\n<output>` (truncated at 5000 characters).

---

### `task_add`

Add a new task to the session task list with status `todo`.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `description` | string | yes | — | Human-readable task description. |
| `session` | string | no | `"default"` | Session name (tasks are stored per-session in `~/.phantom-mesh/tasks/<session>.json`). |

**Example:**
```json
{"description": "Refactor authentication module", "session": "sprint-42"}
```

**Returns:** `Added task #N: <description>`

---

### `task_update`

Update the status of a task by ID.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `id` | integer | yes | — | Task ID (as returned by `task_add` or `task_list`). |
| `status` | string | yes | — | New status: `"todo"`, `"in_progress"`, or `"done"`. |
| `session` | string | no | `"default"` | Session name. |

**Example:**
```json
{"id": 3, "status": "in_progress", "session": "sprint-42"}
```

**Returns:** `Task #3 marked as in_progress` or `Error: task #3 not found`.

---

### `task_list`

List tasks for a session, optionally filtered by status.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `session` | string | no | `"default"` | Session name. |
| `status_filter` | string | no | — | Filter to `"todo"`, `"in_progress"`, or `"done"`. |

**Example — list all tasks:**
```json
{"session": "sprint-42"}
```

**Example — list only incomplete tasks:**
```json
{"session": "sprint-42", "status_filter": "todo"}
```

**Returns:** `Tasks (N total, M done):\n  #1 [todo] description\n  #2 [done] description\n...` or `No tasks found.`

---

### `task_clear`

Remove tasks from the session list.

| Param | Type | Required | Default | Description |
|---|---|---|---|---|
| `session` | string | no | `"default"` | Session name. |
| `done_only` | boolean | no | false | If true, only remove tasks with status `"done"`; otherwise remove all tasks. |

**Example — clear completed tasks:**
```json
{"session": "sprint-42", "done_only": true}
```

**Returns:** `Cleared N task(s)`

---

## Best Practices

### Choosing the right read tool

| Goal | Preferred tool |
|---|---|
| Read a specific file | `file_read` |
| Find files by name | `glob_search` |
| Find files by content | `content_search` |
| Browse a directory tree | `ls` with `tree: true` |
| List all files recursively | `list_files` |
| Check file metadata | `ls_stat` |

### Editing files safely

1. Always read a file with `file_read` before editing it to understand the current content and select a unique `old_string`.
2. Prefer `file_edit` over `file_write` for surgical changes — it preserves everything outside the replaced region and produces a diff for verification.
3. Use `multi_file_edit` when you need to apply coordinated changes across multiple files atomically (e.g. renaming a function used in many files).
4. Use `patch` when you have a complete unified diff (e.g. from a code review or a prior `diff_files` call).
5. Use `replace_all: true` in `file_edit` only when you are certain every occurrence should change. Otherwise provide more surrounding context to make `old_string` unique.

### Using `line_range` to scope edits

When a string appears multiple times in a file, narrow the search with `line_range`:
```json
{
  "path": "src/config.rs",
  "old_string": "let timeout = 30;",
  "new_string": "let timeout = 60;",
  "line_range": {"start": 45, "end": 55}
}
```

### Git workflow

The typical commit cycle:
1. Make changes with `file_edit` / `file_write` / `multi_file_edit`.
2. Verify with `cargo_check` or `tsc_check`.
3. Stage with `git_add`.
4. Commit with `git_commit`.

### Memory namespacing

Use namespaces to avoid key collisions between agents or sessions:
```json
{"key": "task_queue", "value": "[1,2,3]", "namespace": "agent_coordinator"}
```

Recall with the same namespace:
```json
{"key": "task_queue", "namespace": "agent_coordinator"}
```

### Shell safety

- Prefer dedicated tools (`git_add`, `file_write`, etc.) over `shell` when available — they have built-in validation and produce structured output.
- When you must use `shell` for a destructive command, set `PHANTOM_AUTO_APPROVE=1` in the environment, or the command will return `APPROVAL_REQUIRED`.
- Use `cwd` instead of `cd && ...` chains — the working directory is reset between shell calls anyway.
- Use `env` to pass secrets rather than embedding them in the command string.

### Fetching web content

- Use `fetch` for human-readable pages (documentation, blog posts) — it strips navigation chrome and returns clean text.
- Use `http_get` / `http_post` for structured API calls where you need raw JSON or specific headers.
- If `fetch` truncates, increase `max_length` (up to 50,000) or narrow with `selector`.
- Private/loopback IPs are blocked in `fetch`; use `http_get` for internal services (it has no IP filtering).

---

## Tool Chaining Examples

### Example 1: Find a function, read context, edit it

```
1. content_search: {"pattern": "fn authenticate", "path": "core/src", "file_type": "rs"}
   → core/src/auth.rs:42: pub fn authenticate(

2. file_read: {"path": "core/src/auth.rs", "offset": 38, "limit": 25}
   → read lines 38–62 to understand the function signature and body

3. file_edit: {
     "path": "core/src/auth.rs",
     "old_string": "pub fn authenticate(token: &str) -> bool {",
     "new_string": "pub fn authenticate(token: &str, timeout_ms: u64) -> bool {"
   }

4. cargo_check: {"path": "core"}
   → verify the edit compiles
```

### Example 2: Apply a patch from a diff, then commit

```
1. diff_files: {"path_a": "src/config.rs.orig", "path_b": "src/config.rs"}
   → produces unified diff

2. patch: {"patch": "<diff text>", "base_dir": "/workspace"}
   → applies the diff

3. git_add: {"files": ["src/config.rs"]}

4. git_commit: {"message": "chore: update config defaults"}
```

### Example 3: Research → fetch docs → implement

```
1. web_search: {"query": "tokio spawn_blocking documentation"}
   → [1] spawn_blocking in tokio::task — URL: https://docs.rs/tokio/...

2. fetch: {"url": "https://docs.rs/tokio/latest/tokio/task/fn.spawn_blocking.html", "selector": "main"}
   → cleaned documentation text

3. file_read: {"path": "core/src/agent.rs", "offset": 1, "limit": 30}
   → check existing imports

4. file_edit: { ... add spawn_blocking usage ... }

5. cargo_check: {"path": "core"}
```

### Example 4: Multi-file rename with validation

```
1. content_search: {"pattern": "JobStore", "path": "core/src"}
   → lists every file that uses JobStore

2. multi_file_edit: {
     "edits": [
       {"path": "core/src/session.rs", "old_string": "JobStore", "new_string": "TaskStore"},
       {"path": "core/src/lib.rs",     "old_string": "JobStore", "new_string": "TaskStore"},
       {"path": "core/src/agent.rs",   "old_string": "JobStore", "new_string": "TaskStore"}
     ]
   }
   → atomic: all succeed or none are touched

3. cargo_check: {"path": "core"}
```

### Example 5: Track long work as tasks

```
1. task_add: {"description": "Implement streaming tool", "session": "sprint-5"}
   → Added task #1

2. task_update: {"id": 1, "status": "in_progress", "session": "sprint-5"}

3. ... do implementation work ...

4. cargo_test: {"path": "core", "filter": "streaming"}
   → 3 passed, 0 failed

5. task_update: {"id": 1, "status": "done", "session": "sprint-5"}

6. task_list: {"session": "sprint-5"}
   → Tasks (1 total, 1 done): #1 [done] Implement streaming tool
```

### Example 6: Background job with status check

```
1. shell_bg: {"command": "cargo build --release", "label": "release-build"}
   → Job started: PID=45678 label='release-build'

2. ... do other work ...

3. shell_bg_check: {"pid": 45678}
   → PID 45678 (release-build): finished

4. shell: {"command": "ls -lh target/release/phantom-agent"}
   → -rwxr-xr-x 1 user group 12M Apr 24 10:30 target/release/phantom-agent
```
