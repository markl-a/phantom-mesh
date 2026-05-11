# Phantom Mesh Quickstart

> Five minutes from clone to first agent run. For the full operator
> walkthrough see [GETTING-STARTED.md](GETTING-STARTED.md); for fast
> verification of every feature added today see
> [VERIFY-CHEATSHEET.md](VERIFY-CHEATSHEET.md).

## 1. Install

```bash
# Build from source (Rust 1.75+)
git clone https://github.com/your-org/phantom-mesh.git
cd phantom-mesh
cargo install --path core --bin phantom

# Verify
phantom --version    # → 0.1.0
which phantom        # → ~/.cargo/bin/phantom
```

## 2. Configure

Set at least one provider API key in `~/.phantom-mesh/env`:

```bash
mkdir -p ~/.phantom-mesh
cat >> ~/.phantom-mesh/env <<'EOF'
GROQ_API_KEY=gsk_...          # free tier, fast — recommended for first run
GEMINI_API_KEY=AIza...        # optional
ANTHROPIC_API_KEY=sk-ant-...  # optional
EOF
```

Or run the onboarding wizard which writes `agents.toml` for you:

```bash
phantom onboarding             # opens browser
# or
phantom                        # terminal wizard auto-runs on first launch
```

## 3. First run — pick an interface

### Standalone REPL (Claude Code-style)

```bash
phantom
> use shell to run "ls" and summarize
```

Streaming output, inline tool calls, markdown rendering, Tab completion,
multi-line input via trailing `\`, Ctrl-C cancels in-flight stream.

### Full-screen TUI (ratatui)

```bash
phantom tui
```

Persistent multi-line input box, scrollable transcript, status bar.
Same slash commands as the REPL.

### Web dashboard

```bash
phantom serve                  # default :7878
open http://localhost:7878
```

xterm.js terminal pane, **Cmd+K** command palette, Info tab with
Todo / Sessions / Cost / Tools sub-panels, live peer-ping dots.

### One-shot

```bash
phantom "find all TODO comments in core/src and group by file"
```

### Subagent for Claude Code or Codex CLI

See [INTEGRATIONS.md](INTEGRATIONS.md). Short version:

```bash
# Claude Code: edit ~/.claude.json (see CLAUDE-CODE-SETUP.md)
# Codex CLI 0.39+:
codex mcp add phantom $(which phantom) mcp
```

Both expose 45 tools.

### Self-iteration

```bash
phantom evolve "fix the warning in core/src/cost.rs" --max-rounds 3 --agent coder
```

Reads files, edits code, retries until done. See
[SELF-EVOLVE.md](SELF-EVOLVE.md) for a worked example ($0 cost on Groq free
tier).

## 4. Useful slash commands inside the REPL/TUI

| Command | Effect |
|---|---|
| `/help` | full list (24 commands) |
| `/agents`, `/agent <name>` | list / switch active agent |
| `/tools` | categorized list of available tools |
| `/sessions`, `/resume <prefix>` | session management |
| `/todo` | dump `~/.phantom-mesh/todos.json` |
| `/plan` | toggle plan-mode gating (denies tools until you say `go`) |
| `/show`, `/show <n>` | list / expand captured tool calls |
| `/density compact\|full` | tool result preview length |
| `/theme <name>` | color scheme |
| `/perm ask\|allow\|deny\|list\|reset` | per-tool permission gate |
| `/cost` | session cost so far |

## 5. Common settings

```bash
PHANTOM_PERM=ask phantom         # launch with permission-prompt mode on
PHANTOM_DENSITY=compact phantom  # compact tool results
PHANTOM_MD=0 phantom             # disable markdown highlight
NO_COLOR=1 phantom               # disable all ANSI colors
```

## 6. Cluster

Add Tailscale peer URLs to `agents.toml` `[cluster]` and share a secret:

```toml
[cluster]
peers = ["http://100.x.x.2:7878", "http://100.x.x.3:7878"]
cluster_secret = "openssl rand -hex 32"
```

`phantom coordinator` does zero-config peer discovery via mDNS.

See the main [README.md](../README.md) for the full mesh story and the
[DEPLOYMENT.md](DEPLOYMENT.md) walkthrough for multi-node bring-up.
