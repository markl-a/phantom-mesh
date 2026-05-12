# phantom on macOS — Install Guide

Pairs with `INSTALL-ANDROID.md` and `INSTALL-IOS.md`. Mac is the
recommended **coordinator** for a phantom mesh — it ships native
launchd auto-start, APFS snapshot rollback, MLX local LLM, and the
deepest doctor diagnostics.

---

## TL;DR — 90 seconds

```bash
# 1. One-shot install (no sudo needed; ~/.cargo/bin goes on PATH automatically)
git clone https://github.com/markl-a/phantom-mesh && \
  cd phantom-mesh/core && cargo install --path .

# 2. First-time setup — wizard writes agents.toml, runs doctor, prints next steps
phantom onboarding

# 3. Auto-start at every login (launchd LaunchAgent)
phantom service install

# 4. (Optional) hourly self-improvement loop
phantom autoevolve schedule install

# 5. Verify — expect 11 sections, all ✓ or ⚠ (⚠ is fine for opt-in features)
phantom doctor
```

After step 4, phantom serve runs every login, the agent fixes failing
tests every hour, and you have one-line diagnostics any time you
suspect something is off.

---

## Prereqs

- macOS 13 (Ventura) or newer; macOS 26 (Sequoia/Tahoe) recommended
- Apple Silicon strongly preferred (M1 or newer) — required for the
  optional MLX on-device LLM
- Rust toolchain (`rustup`) for building from source
- A valid API key from at least one provider (Anthropic / OpenAI /
  Groq / Gemini) — phantom is BYOK; we never ship our own key

Optional but useful:

- Tailscale account (for the cluster across devices)
- Xcode command-line tools (`xcode-select --install`) — unlocks the
  `xcode_simctl` tool
- `pip install mlx-lm` — unlocks the on-device LLM (`phantom mlx`)

---

## What gets installed where

| Path | Purpose |
|---|---|
| `~/.cargo/bin/phantom` | symlink to `core/target/release/phantom` (cargo install) |
| `~/Library/Application Support/phantom-mesh/bin/phantom` | TCC-safe copy used by launchd (created on `phantom service install`) |
| `~/Library/Application Support/phantom-mesh/{dist,scripts}/` | Mirrored repo `dist/` + `scripts/` for `/dist/*` and `/scripts/*` HTTP routes |
| `~/Library/LaunchAgents/ai.phantommesh.serve.plist` | LaunchAgent for `phantom serve` |
| `~/Library/LaunchAgents/ai.phantommesh.autoevolve.plist` | LaunchAgent for hourly `autoevolve --once` |
| `~/Library/Logs/phantom-serve.log` | LaunchAgent serve stdout/stderr |
| `~/Library/Logs/phantom-autoevolve.log` | LaunchAgent autoevolve stdout/stderr |
| `~/.phantom-mesh/agents.toml` | provider keys, agent definitions, cluster |
| `~/.phantom-mesh/env` | shell-sourcable secrets (optional, BYOK loader) |
| `~/.phantom-mesh/autoevolve.log` | JSONL log of every autoevolve iteration |
| `~/.phantom-mesh/costs.json` | lifetime LLM spend persisted across runs |
| `~/.phantom-mesh/conversations/<id>.jsonl` | per-session persistent transcripts |

Everything except the binary lives under `$HOME` — easy to backup,
easy to wipe.

---

## Detailed install

### 1. Build the binary

```bash
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo install --path .
phantom --version
# → phantom 0.1.0 (<git-hash>+, macos-aarch64, built YYYY-MM-DD)
```

`cargo install --path .` puts `phantom` on your PATH at
`~/.cargo/bin/phantom`. The build is ~2 minutes on M1 the first time.

### 2. First-time wizard

```bash
phantom onboarding
```

Interactive — asks for your Groq / Gemini / Anthropic / OpenAI keys,
writes `~/.phantom-mesh/agents.toml`, then runs `phantom doctor` to
confirm. Ends with a 3-step "next steps" block telling you exactly
what to type.

You can re-run `phantom onboarding` any time to revisit / overwrite.

### 3. Auto-start

```bash
phantom service install
```

This is the macOS-26-aware version: copies the binary into
`~/Library/Application Support/phantom-mesh/bin/phantom` (TCC-safe
because `~/Documents` is blocked from launchd-spawned processes) and
loads `ai.phantommesh.serve.plist` via `launchctl bootstrap`. The
service starts immediately and re-launches at every user login,
KeepAlive on non-zero exit (10 s throttle).

```bash
phantom service status        # registered/pid/healthz
phantom service uninstall     # bootout + remove plist
```

### 4. Hourly self-improvement (optional)

```bash
phantom autoevolve schedule install
```

Installs a second LaunchAgent that runs `phantom autoevolve --once`
every hour. When `cargo check` is red, it spawns `phantom evolve` to
fix it; when fixes land green, it `git commit`s them. Past iterations
get fed back into the LLM's prompt as a "what worked / what didn't"
hint.

```bash
phantom autoevolve schedule status
phantom autoevolve log --n 10        # last 10 JSONL entries, pretty
phantom autoevolve schedule uninstall
```

### 5. (Optional) on-device LLM

```bash
pip3 install mlx-lm                  # one-time
phantom mlx pull                     # default Llama 3.1 8B 4-bit (~5 GB)
phantom mlx serve                    # foreground at :8080
```

Then add to `~/.phantom-mesh/agents.toml`:

```toml
[providers.mlx-local]
type          = "openai"
base_url      = "http://127.0.0.1:8080/v1"
api_key       = "mlx"
default_model = "mlx-community/Llama-3.1-8B-Instruct-4bit"

[agent.local]
provider = "mlx-local"
model    = "mlx-community/Llama-3.1-8B-Instruct-4bit"
```

After this, `phantom autoevolve --once --agent local` runs the entire
self-improvement loop **fully on-device, fully offline, zero per-token
cost**. See `docs/MLX-PROVIDER.md` for the perf table by model size.

### 6. (Optional) cluster

If you have other Macs / Windows / Linux / Termux nodes:

1. Make sure they're all in the same Tailscale tailnet.
2. On each peer, install phantom (the right binary lives at
   `http://<this-mac-ts-ip>:7878/dist/phantom-<target>`).
3. On this Mac, edit `~/.phantom-mesh/agents.toml`:
   ```toml
   [cluster]
   node_name      = "mac-coordinator"
   cluster_secret = "<shared-secret-string>"
   peers = [
     "http://<peer-1-ts-ip>:7878",
     "http://<peer-2-ts-ip>:7879",
   ]
   ```
4. Restart the service (`launchctl kickstart -k
   gui/$UID/ai.phantommesh.serve`) and run `phantom doctor` — the
   network row should show your peers.

Cross-mesh dispatch then works: `mcp__phantom__subagent({node:
"100.84.223.59:7879", agent: "master", prompt: "..."})`.

---

## Verify

### 1. Quick health check

```bash
phantom doctor
```

`phantom doctor` runs 11 colour-coded sections. In a healthy install
every line is `✓` green or `⚠` yellow. `⚠` is expected for features
you haven't opted into (MLX server, Spotlight indexing, unused API
keys). Red `✗` lines mean something needs attention.

**Expected output on a well-configured Mac** (your keys and node names
will differ):

```
phantom doctor 0.4.0

binary
  ✓ version: phantom 0.4.0 (093b1af4c8+, macos-aarch64, built 2026-05-11)
  ✓ path: /Users/you/.cargo/bin/phantom

config
  ✓ agents.toml: /Users/you/.phantom-mesh/agents.toml
  ✓ ~/.phantom-mesh: exists

permissions
  ⚠ [permissions]: no rules → allow all (legacy default).
                    See docs/PERMISSIONS.md for the Tool(specifier) DSL.

provider keys
  ⚠ Anthropic: not in env or agents.toml
  ✓ Groq: env (gsk_L1…)
  ✓ Gemini: agents.toml
  ⚠ DeepSeek: not in env or agents.toml

phantom serve
  ✓ healthz: 200 OK on http://127.0.0.1:7878/healthz
  ✓ launchd: registered (pid 61585)

network
  ✓ Tailscale: connected (100.x.x.x  your-host  userid:…  macOS  -)

MLX local LLM
  ✓ mlx_lm: importable (`pip install mlx-lm` available)
  ⚠ server: not reachable — `phantom mlx serve`

autoevolve
  ✓ history: last run @ 2026-05-12 07:10 → green (140 total)
  ✓ schedule: registered (LaunchAgent)

identity
  ✓ logged in: you@example.com (Your Name)  via Google  device xxxxxxxx

diagnostics
  ⚠ crash logs: 7 recorded — latest: …/crash-xxxxxxx.log
               › read with: phantom debug last
  ✓ events log: …/events.jsonl (513196 bytes)

tools
  ✓ tools: 54 total (52 built-in + 2 cluster RPC)

macOS integrations
  ✓ APFS snapshots: tmutil reachable (0 snapshots — `phantom snapshot create`)
  ⚠ Spotlight: not indexing /Users/you/repos/phantom-mesh
  ✓ Xcode CLT: installed (xcode_simctl tool ready)

done.
```

The **⚠ lines to watch for on first install** (normal, not errors):
- `Anthropic: not in env` — you didn't choose Anthropic during onboarding;
  add `ANTHROPIC_API_KEY` to env or `agents.toml` if you want it
- `MLX server: not reachable` — expected unless you ran `phantom mlx serve`
- `Spotlight: not indexing …` — expected unless you add a Spotlight path
  in `agents.toml [core].spotlight_paths`
- `crash logs: N recorded` — may appear after a bad agent run; use
  `phantom debug last` to inspect

**Red ✗ lines that need fixing:**
- `agents.toml: not found` → run `phantom onboarding`
- `launchd: not installed` → run `phantom service install`
- `healthz: unreachable` → run `phantom serve` or `phantom service install`
- `Tailscale: not in PATH or not connected` → `tailscale up`
- `systemd: no unit installed` → run `phantom service install`

For machine-readable output (CI / monitoring / scripted checks):

```bash
phantom doctor --json | jq '.status'       # → "ok" / "warn" / "fail"
phantom doctor --json | jq '.serve'        # port, running, status code
phantom doctor --json | jq '.autoevolve'   # queue + last run timestamp
```

### 2. Run the test sweeps

```bash
./scripts/test-mac.sh        # 51 fast checks, ~30 s
phantom selftest             # 22+ feature checks (TUI, MCP, doctor, dashboard…)
phantom selftest --p0-only   # critical checks only, ~5 s
```

`test-mac.sh` expects PASS 51 / FAIL 0 / SKIP ≤ 1.
`phantom selftest` expects 22+ pass / 0 fail.

### 3. Open the dashboard

```bash
phantom serve &              # if not already running via launchd
open http://127.0.0.1:7878/projects
```

You should see:
- 6 pinned-project tiles with [Run Demo] buttons
- A cluster status bar (single node when running solo, more pills
  when peers are reachable via Tailscale)
- A "Recent activity" strip showing autoevolve runs

Tap any [Run Demo] — output streams live via Server-Sent Events.

### 4. (Optional) wire into Claude Code

`phantom-mesh` exposes its 50+ tools as an MCP server. To use them
from Claude Code:

```bash
claude mcp add phantom $(which phantom) mcp
```

Or, if you're working *inside* the phantom-mesh repo, the project-
local `.mcp.json` auto-registers — just trust the prompt on first
Claude Code session start. After that, every tool surfaces as
`mcp__phantom__file_read`, `mcp__phantom__shell`, etc.

Smoke-test the MCP wire format:
```bash
./scripts/test-mcp-tools.sh   # 13 tool/call e2e checks
```

---

## Updating

```bash
phantom self-update                       # pulls /dist/<target> from coord
phantom self-update --source <URL>        # explicit source
phantom self-update --dry-run             # show what would happen
```

After self-update finishes, the launchd service is automatically
restarted with `launchctl kickstart -k`. If something is wrong:

```bash
mv ~/Library/Application\ Support/phantom-mesh/bin/phantom.bak \
   ~/Library/Application\ Support/phantom-mesh/bin/phantom
launchctl kickstart -k gui/$UID/ai.phantommesh.serve
```

---

## Uninstall

```bash
phantom autoevolve schedule uninstall
phantom service uninstall
rm ~/.cargo/bin/phantom
rm -rf ~/.phantom-mesh
rm -rf ~/Library/Application\ Support/phantom-mesh
rm    ~/Library/Logs/phantom-{serve,autoevolve}.log
```

That's clean. No system-level state, no kexts, no daemons.

---

## Troubleshooting

See `docs/TROUBLESHOOTING-MAC.md` for the full footgun catalogue
(every issue we hit while building this gets one section).

### `phantom doctor` quick triage

Run `phantom doctor` and look for the failure in this order:

| `phantom doctor` line | Cause | Fix |
|---|---|---|
| `✗ agents.toml: not found` | onboarding not run | `phantom onboarding` |
| `✗ healthz: unreachable` | serve not running | `phantom serve &` or `phantom service install` |
| `✗ launchd: not installed` | service not set up | `phantom service install` |
| `⚠ MLX server: not reachable` | not started | `phantom mlx serve` (first run ~5 min download) |
| `⚠ Spotlight: not indexing …` | path not in config | Add paths to `agents.toml [core].spotlight_paths` |
| `⚠ Tailscale: not in PATH or not connected` | not logged in | `tailscale up` |
| `⚠ autoevolve/history: no runs yet` | first run never done | `phantom autoevolve --once` |
| `⚠ autoevolve/schedule: not scheduled` | schedule not installed | `phantom autoevolve schedule install` |
| `⚠ crash logs: N recorded` | a recent agent run crashed | `phantom debug last` to read the latest |
| `⚠ identity: local-only` | expected (broker not deployed) | nothing to fix — this is normal |
| `✗ [permissions]: parse error` | syntax in `agents.toml [permissions]` block | check the DSL in docs/PERMISSIONS.md |

The single most common macOS-26-specific issue is the TCC trap, fixed
in commit 65338ab — but if you upgraded across that boundary, run:

```bash
phantom service uninstall && phantom service install
```

For full automated triage of the 51 environment checks:

```bash
./scripts/test-mac.sh    # tells you exactly which check is failing
```

---

## Performance baseline (M1 16 GB, macOS 26.3)

| Operation | Time |
|---|---|
| `phantom --version` | < 30 ms |
| `phantom doctor` | ~ 800 ms |
| `phantom mcp` cold start (stdio handshake + tools/list) | ~ 200 ms |
| `mcp__phantom__subagent({...})` round-trip via Groq Llama 3.3 70B | 2-5 s |
| `mcp__phantom__subagent` cross-mesh to peer | + 200-1000 ms over local |
| `phantom mlx serve` cold load 8B-4bit → first token | ~ 150 s |
| `phantom mlx serve` warm 8B-4bit → 50 tokens | ~ 5 s |
| `phantom autoevolve --once` (green tree, no work) | ~ 110 s (cargo check rebuild) |
| `phantom autoevolve --once` (red tree → fix → commit) | 30-180 s depending on agent |
| `cargo build --release --bin phantom` (cold) | ~ 2 min |

Hot paths land well under a second; the slow ones are LLM-bound
(network round-trip dominates).
