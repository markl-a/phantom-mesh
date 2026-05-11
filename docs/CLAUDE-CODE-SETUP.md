# Claude Code → phantom-mesh MCP Setup (company Mac)

Wire your `phantom` binary into Claude Code as an MCP server. After this, Claude Code can call all 45 phantom tools (shell, file_edit, content_search, web_fetch, http_client, cargo_test, git_*, ask_user, bash_run_background, agent runners, etc.) as native tools.

## 1. Pre-req checklist

```bash
# 1. Install the phantom binary at a stable path (build/copy or symlink). Confirm:
which phantom    # expect: /usr/local/bin/phantom

# 2. Export your provider keys in your shell rc (~/.zshrc):
export GROQ_API_KEY=... GEMINI_API_KEY=... OPENCODE_API_KEY=... ANTHROPIC_API_KEY=...

# 3. Smoke-test the MCP transport (Ctrl-C to exit; should print "phantom MCP server started"):
phantom mcp
```

## 2. JSON snippet for `~/.claude.json`

Open `~/.claude.json` and add a `phantom` entry under `mcpServers`. If `mcpServers` does not exist yet, create it at the top level.

### Option A — inherit env vars from your shell (recommended)

Cleanest. Claude Code launches `phantom` as a child process and the keys you exported in `~/.zshrc` flow through automatically. The `env` object stays empty so secrets never touch the JSON.

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/usr/local/bin/phantom",
      "args": ["mcp"],
      "env": {}
    }
  }
}
```

### Option B — explicit env in JSON

Use this if Claude Code is launched from a GUI context (Spotlight, dock) that does not inherit your interactive shell env. Replace the placeholders. Treat `~/.claude.json` as a secret file (`chmod 600 ~/.claude.json`).

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/usr/local/bin/phantom",
      "args": ["mcp"],
      "env": {
        "GROQ_API_KEY": "<YOUR_GROQ_KEY>",
        "GEMINI_API_KEY": "<YOUR_GEMINI_KEY>",
        "OPENCODE_API_KEY": "<YOUR_OPENCODE_KEY>",
        "ANTHROPIC_API_KEY": "<YOUR_ANTHROPIC_KEY>"
      }
    }
  }
}
```

Notes:
- `args` must be `["mcp"]` — that selects the stdio MCP subcommand (spec 2024-11-05).
- The path must be absolute. `~` and `$HOME` are not expanded by the MCP launcher.
- At least one of `GROQ_API_KEY` / `GEMINI_API_KEY` / `OPENCODE_API_KEY` / `ANTHROPIC_API_KEY` must be set or the agent tools refuse to start (shell/file/git tools work without them).

## 3. Verify

```bash
# 1. Fully restart Claude Code (quit, not just close window).
# 2. Inside Claude Code, list registered MCP servers:
/mcp
# Expected:  phantom   connected   45 tools
```

Then trigger a tool call from a Claude Code chat:

> Use the phantom `shell` tool to run `date` and show me the output.

You should see Claude invoke `mcp__phantom__shell` (or similar) and return the current date. Try one more:

> Use phantom's `content_search` tool to find "TODO" in /tmp.

## 4. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `/mcp` shows `phantom: failed to start` | `command` path wrong. Run `which phantom`, paste the absolute path verbatim into the JSON. Don't use `~` or `$HOME`. |
| `permission denied` in Claude Code logs | `chmod +x /usr/local/bin/phantom`. If installed via `cp` from a downloaded zip, also clear the quarantine bit: `xattr -d com.apple.quarantine /usr/local/bin/phantom`. |
| Server starts but agent tools error with `no providers configured` | Env vars not picked up. Switch to Option B (explicit env in JSON), or launch Claude Code from a terminal: `open -a "Claude Code"` from a shell that has the keys exported. |
| `EADDRINUSE` / port conflict in stderr | A stray `phantom serve` is still running on 7878. `pkill -f "phantom serve"` and reload `/mcp`. The `mcp` subcommand itself uses stdio and binds no ports. |
| `tools/list` returns 0 tools | You're on an old build. Rebuild from `phase1-r1-foundations` or later (`cargo build --release -p phantom-mesh --bin phantom`) and recopy the binary. |
| Claude Code hangs on first tool call | Check stderr by running `phantom mcp` manually in a terminal and pasting the JSON-RPC `initialize` request. If it hangs there too, the binary is broken; rebuild. If it works manually but not from Claude Code, the env is stripped — use Option B. |
| Tools work but can't reach LAN peers (Z13, etc.) | Tailscale not up on the company Mac, or `agents.toml` peer URLs unreachable from corp network. Test with `tailscale ping <peer>` first. |
