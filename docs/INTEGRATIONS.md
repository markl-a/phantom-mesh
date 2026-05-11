# Integrations

Phantom-Mesh is designed to be used in four modes:

1. **Standalone REPL / TUI** — `phantom` (line REPL) or `phantom tui` (full-screen ratatui)
2. **Subagent for Claude Code** — via MCP stdio (`phantom mcp`)
3. **Subagent for Codex CLI** — via MCP stdio (also `phantom mcp`)
4. **WebSocket / web dashboard** — `phantom serve`

This guide covers wiring 2, 3, and 4.

## What's new in v0.1.0-alpha (2026-04-27)

- Tool count is now **45** (added `web_fetch`, `bash_run_background`,
  `bash_output`, `bash_kill`, `ask_user`).
- REPL gained markdown rendering, Tab completion, Ctrl-C cancel, plan-mode
  gating, `@image.png` multimodal attach, and slash commands `/show /perm
  /density /theme /agent /agents /todo /plan /resume`.
- New `phantom tui` ratatui full-screen interface.
- Web dashboard adds Cmd+K palette, Tools/Sessions/Cost panels in the
  Info tab, xterm.js terminal, and live peer-ping dots.
- `phantom evolve` self-iteration verified end-to-end (see
  [SELF-EVOLVE.md](SELF-EVOLVE.md)).

---

## 1. Claude Code (recommended)

Claude Code consumes phantom as an MCP server. After setup, all 45 phantom
tools become callable from inside any Claude Code session. See
[CLAUDE-CODE-SETUP.md](CLAUDE-CODE-SETUP.md) for the full guide; the short
version follows.

### Setup

Edit `~/.claude.json` (or use `claude mcp add` if your Claude Code build supports it):

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/usr/local/bin/phantom",
      "args": ["mcp"],
      "env": {
        "GROQ_API_KEY": "gsk_...",
        "GEMINI_API_KEY": "AIza..."
      }
    }
  }
}
```

Replace `/usr/local/bin/phantom` with `which phantom` on your machine.

### Verify

Restart Claude Code, then run `/mcp` inside any session. You should see `phantom` listed with 45 tools (shell, file_*, content_search, web_fetch, web_search, hardware, scaffold, mesh, mcp, ask_user, bash_run_background, etc.).

### Use

Phantom tools are invoked by name. Examples Claude Code can run for you:

```
"Use phantom's hardware tool to check this Mac's specs"
"Have phantom run a parallel grep across the home cluster for TODO"
"Use phantom mesh delegate to run cargo test on the Z13 worker"
```

Phantom keeps its own session state in `~/.phantom-mesh/conversations/`, independent of Claude Code's history.

### Mesh use cases (the unique value)

If your phantom node is part of a mesh (`agents.toml` `[cluster]` block configured with peers), you can ask Claude Code to delegate heavy work to other nodes while you stay productive on your laptop:

- "Have phantom dispatch the test suite to a worker that has more cores"
- "Use phantom mesh swarm to ask all peers about their CPU/RAM, summarize"
- "Delegate this dataset analysis to the Z13 (it has the GPU)"

---

## 2. Codex CLI (0.39+)

Codex now speaks MCP stdio natively. One command wires phantom in:

```bash
codex mcp add phantom $(which phantom) mcp
```

That writes `~/.codex/config.toml` with a `[mcp_servers.phantom]` block.
Verify:

```bash
grep -A2 "mcp_servers.phantom" ~/.codex/config.toml
# command = "/Users/.../bin/phantom"
# args = ["mcp"]
```

Then inside `codex` use `/mcp` to confirm phantom shows 45 tools, and call
them like any other tool: `Use phantom shell to run pwd`.

### WebSocket fallback (older Codex / custom clients)

Phantom also exposes a Codex-compatible WebSocket JSON-RPC endpoint:

```bash
phantom serve --bind 127.0.0.1:7878
# WebSocket endpoint: ws://localhost:7878/ws
```

If `agents.toml` includes a `[cluster] cluster_secret`, that secret is used
as a bearer token on incoming WS connections; pass it from your client.

---

## 3. Standalone REPL (no client needed)

```bash
# Interactive REPL, Claude Code style
phantom

# One-shot prompt
phantom "find all TODO comments in src/"

# Resume the last session
phantom -c

# Switch agent (default: master)
phantom --agent reviewer "review this PR diff: ..."
```

REPL features:

- **Streaming output** with inline tool calls (`● shell(cargo test)` ... `✓ ok`)
- **Markdown rendering** — bullets, numbered lists, blockquotes, links, code spans
- **Multi-line input** — end a line with `\` to continue on the next
- **Tab completion** — `/cmd` and `@path/to/...` expand on Tab
- **Ctrl-C** cancels the in-flight LLM stream (REPL stays alive)
- **Plan mode** — `/plan` toggles ON; agent must output a plan first, you say `go` to execute
- **Slash commands** — `/help` for the full list (24 commands)
- **`@path/to/file`** — inline file contents into a prompt
- **`@image.png`** — attach PNG/JPG as multimodal `image_url` (works on OpenAI, Gemini, Anthropic)
- **Per-tool permissions** — `/perm ask|allow|deny|list|reset`
- **Session continuation** — `phantom -c` or `/resume <prefix>`
- **Cost tracking** — `[↑ $0.0023  ∑ $0.0145  3.2s]` after each turn

For a full-screen alternative, `phantom tui` opens a ratatui interface
(persistent input box, scrollable transcript, status bar). For autonomous
self-iteration, see [SELF-EVOLVE.md](SELF-EVOLVE.md).

---

## 4. Provider configuration

Set API keys in `~/.phantom-mesh/env`:

```bash
GROQ_API_KEY=gsk_xxx
GEMINI_API_KEY=AIzaSy_xxx
ANTHROPIC_API_KEY=sk-ant-xxx   # optional
```

Or hard-code them per node in `~/.phantom-mesh/agents.toml`:

```toml
[[providers]]
name = "groq"
base_url = "https://api.groq.com/openai/v1"
api_key = "gsk_xxx"
default_model = "llama-3.3-70b-versatile"
primary = true
```

Free tiers that work today:
- **Groq** — fast Llama 3.3 70B, generous free quota
- **Gemini** — long context, free tier daily quota (small)

---

## 5. Quick reference

| Mode | Command | Endpoint | Use with |
|---|---|---|---|
| MCP stdio | `phantom mcp` | stdin/stdout | Claude Code, Codex CLI 0.39+, Cursor, any MCP client |
| WebSocket | `phantom serve` | `ws://host:7878/ws` | older Codex, custom clients |
| Web dashboard | `phantom serve` | `http://host:7878` | browser (Cmd+K palette, xterm.js, Info panels) |
| REPL | `phantom` | terminal | direct human use |
| TUI | `phantom tui` | terminal | full-screen ratatui interface |
| One-shot | `phantom "..."` | terminal | scripts, cron, automation |
| Self-iterate | `phantom evolve "..."` | terminal | autonomous edit loop on current repo |

## Attaching images

The REPL's `@<path>` syntax now treats image files (`.png`, `.jpg`, `.jpeg`,
`.gif`, `.webp`) specially: instead of inlining the bytes as text, the file is
base64-encoded and attached as a multimodal `image_url` content part on the
outgoing chat message. OpenAI, Gemini's OpenAI-compat endpoint, and the native
Anthropic Messages API are all handled transparently — Anthropic requests are
rewritten to the equivalent `image` / `source.base64` shape. Non-image `@`
expansions retain their previous behaviour (the file is read as text and
wrapped in a `<file path="…">…</file>` block).

Example:

```bash
phantom "describe @/path/to/screenshot.png"
```

Inside the interactive REPL the same syntax works; you can mix multiple
images and free-form text in a single prompt. Files are read at send time, so
each prompt sees the current contents on disk.
