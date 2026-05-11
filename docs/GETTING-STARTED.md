# phantom-mesh — Getting Started

Get a working agent and an optional second-machine mesh in under five minutes.

---

## 1. Install

### Option A — One-line install (recommended for a 2nd Mac)

If you already have a phantom-mesh coordinator running on another machine
(call it "Mac 1"), the coordinator serves a bootstrap script and a binary
over HTTP. From Mac 2, run:

```bash
curl -fsSL http://<coordinator-tailscale-ip>:7878/scripts/install-mac.sh \
  | COORD=http://<coordinator-tailscale-ip>:7878 bash
```

This pulls:
- the `phantom` binary into `~/.cargo/bin/`
- a cluster-bootstrap `~/.phantom-mesh/agents.toml` (cluster_secret + peers,
  **no API keys**)
- a launchd entry so `phantom serve` starts on login

It will *not* touch your provider keys — set those interactively after, in
the REPL (`/keys add groq`, etc.).

Requirements: Apple Silicon (`arm64`), `curl`, and (recommended) Tailscale
joining the same tailnet as the coordinator.

### Option B — From source

```bash
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo build --release --bin phantom
cp target/release/phantom ~/.cargo/bin/phantom
```

Make sure `~/.cargo/bin` is on your `PATH`. First run will guide you through
interactive provider setup.

### Verify

```bash
phantom --version          # prints the build commit + date
phantom doctor             # checks config, binary, daemon, tailnet, peers
```

`phantom doctor` is the fastest "is anything wrong" check — it walks each
subsystem and prints OK / warn / fail with hints.

---

## 2. First 5 minutes

```bash
phantom                    # launches the TUI
```

You'll see a prompt with gold-on-black input. Type a message and hit Enter
— the response streams back inline.

Then try:

```
/help                      # list every slash command
/keys add groq             # paste a Groq API key; it's auto-tested
/model fast                # switch to the fastest available model
What's the current git status of my home dir?
```

That last prompt should trigger the `git_status` tool and show real output.
You're done — the rest of this doc is reference.

---

## 3. Power-user commands

All available inside the TUI and the `phantom --repl` (which has a few
extras like interactive pickers).

| Command | What it does |
|---|---|
| `/model` | Show current model + provider defaults |
| `/model fast` / `smart` / `cheap` | Switch to fastest / smartest / cheapest model in your config |
| `/model fetch <provider>` | Pull live model list from a provider (`groq`, `openrouter`, `gemini`, `anthropic`) |
| `/model pick` | Interactive numbered picker (REPL only) |
| `/keys list` | Show which providers have a key set |
| `/keys add <provider>` | Paste a key; auto-tested before save |
| `/keys test <provider>` | Smoke-test a stored key |
| `/keys remove <provider>` | Delete a stored key |
| `/copy` | Copy last assistant response to clipboard |
| `/copy all` | Copy full session |
| `/copy turn` | Copy last user+assistant turn |
| `/export [path]` | Save the session as Markdown |
| `/compact` | LLM-summarize older turns, keep the last 6 verbatim |
| `/sessions` | List saved sessions |
| `/resume <prefix>` | Switch to a session by ID prefix (or no arg = most recent) |
| `/fork` | Branch the current session into a new one (REPL) |
| `/plan` | Toggle plan mode — agent previews its plan before any tool call |
| `/agent [name]` | Show or switch the active agent |
| `/agents` | List configured agents |
| `/tools` | List tools enabled for the active agent |
| `/perm ask\|allow\|deny` | Permission mode for tool calls |
| `/cost` | Session + total $ spent, request count |
| `/density compact\|full` | One-line vs multi-line tool output |
| `/theme <name>` | `dark`, `light`, `claude`, `codex`, `gemini`, `mono` |
| `/init` | Generate a project `PHANTOM.md` in cwd |
| `/clear` | Clear transcript + evict session history |
| `/exit` | Ctrl-C also works |

REPL-only commands that need blocking input: `/login`, `/logout`, `/add`,
`/undo`, `/keys add`, `/model pick`. Run `phantom --repl` if you want
those in line-mode instead of the TUI.

---

## 4. Mesh — connecting another machine

The whole point of phantom-mesh is dispatching subagents across your
machines. Setup:

### Prerequisite: Tailscale on both machines

```bash
brew install tailscale
sudo tailscale up
tailscale ip -4              # note this IP — the coordinator URL
```

### On the new machine

Run the one-line installer from §1A, pointing `COORD` at the existing
machine's tailscale IP. The script writes `~/.phantom-mesh/agents.toml`
with the cluster_secret and peer list.

### Verify the mesh

```bash
phantom peer list            # online/offline + active tasks per peer
phantom peer discover        # mDNS + Tailscale scan, no config needed
phantom peer ping http://<peer-ip>:7878
```

### Dispatch work to a peer

```bash
# Send a one-shot job to whichever peer scores best:
phantom peer assign --agent master "summarise the README.md in 5 bullets"

# Async — get a job ID back, poll later:
phantom peer send-async --agent master "long task..."
phantom peer poll http://<peer-ip>:7878 <job-id>
```

Inside the TUI, the active agent can spawn cross-mesh subagents
automatically when its `parallel_tasks` budget allows — see `/tasks`.

---

## 5. Use phantom as a Claude Code subagent

`phantom mcp` speaks MCP over stdio, exposing every tool (shell, file_*,
git_*, web_*, memory_*, subagent, parallel_tasks, etc.) to a parent
Claude Code session.

In `~/.claude.json`:

```json
{
  "mcpServers": {
    "phantom": {
      "command": "/Users/<you>/.cargo/bin/phantom",
      "args": ["mcp"]
    }
  }
}
```

Then in Claude Code:

> Use the phantom MCP server to run `cargo test` in `~/projects/foo`,
> and if it fails, open the failing test file and explain the failure.

Claude Code will call `mcp__phantom__shell` to run the tests and
`mcp__phantom__file_read` to open the file. You get phantom's tool
sandboxing + cluster routing without leaving your existing editor.

---

## 6. Troubleshooting

**Build fails.** Make sure you have a current Rust toolchain:
```bash
rustup update stable
```

**`phantom serve` not starting on login.** Check launchd:
```bash
phantom doctor
launchctl list | grep phantommesh
```
If the unit is missing, re-register it:
```bash
phantom service install
```

**Mesh peer offline.** Verify the network path before blaming phantom:
```bash
tailscale status                       # peer up on the tailnet?
phantom peer ping http://<peer-ip>:7878
curl -fsS http://<peer-ip>:7878/healthz
```
If `healthz` answers but `peer ping` fails, the cluster_secret on the two
ends doesn't match — re-run the install script on the offline node.

**"Model not supported" on a free-tier provider.** Free providers
(opencode router, Groq, Gemini) drop and rotate models often. Refresh:
```
/model fetch groq
/model fetch openrouter
/model pick
```

**Config not found.** phantom looks (in order) at:
1. `$PHANTOM_MESH_CONFIG` if set
2. `~/.phantom-mesh/agents.toml`
3. `~/Library/Application Support/ai.phantommesh.app/agents.toml` (macOS)
4. `~/.config/phantom-mesh/agents.toml` (Linux)

`phantom doctor` prints which path was actually loaded.

---

## Next docs

| Goal | Read |
|---|---|
| Multi-node Tailscale topology | [TAILSCALE-SETUP.md](TAILSCALE-SETUP.md) |
| 24/7 cloud node | [DEPLOY.md](DEPLOY.md) / [DEPLOYMENT.md](DEPLOYMENT.md) |
| Architecture overview | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Per-role config templates | [`configs/`](../configs/) |
