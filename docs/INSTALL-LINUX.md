# Installing phantom-mesh on Linux

Tested on **Ubuntu 22.04 / 24.04** + **Debian 12 (bookworm)**. Other
glibc-based distros (Fedora, Arch, openSUSE) should work — the binary
is statically-ish linked but you'll need `glibc 2.31+` and OpenSSL.

For Alpine / musl-based distros, you'd want a separate static-musl
build — not yet in `dist/`; cross-compile with
`cargo build --release --target x86_64-unknown-linux-musl`.

---

## TL;DR — 60 seconds

```bash
# As a normal user (sudo only for systemd install)
sudo apt-get update && sudo apt-get install -y curl ca-certificates git build-essential pkg-config libssl-dev

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"

git clone https://github.com/markl-a/phantom-mesh && cd phantom-mesh/core
cargo install --path . --locked

phantom onboarding              # interactive ~90s wizard
phantom serve &                  # or set up systemd unit (below)
phantom doctor                   # verify all green
```

Then open `http://127.0.0.1:7878/projects` in a browser — you should
see 6 tiles with [Run Demo] buttons.

---

## Prereqs

| Component | Why | How to get |
|---|---|---|
| Rust toolchain ≥ 1.80 | Build phantom from source | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| `git`                  | Clone the repo                      | `apt install git` |
| OpenSSL dev headers    | Some Rust deps build against system OpenSSL | `apt install libssl-dev pkg-config` |
| `build-essential`      | gcc + make for native deps          | `apt install build-essential` |
| Tailscale (optional)   | Cross-machine cluster + mobile access | https://tailscale.com/download/linux |

**Total fresh-install time on a 4-core 8 GB VPS:** ~6 min (mostly
cargo build).

---

## Detailed install

### 1. Build the binary

```bash
cd ~/Documents
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh/core
cargo install --path . --locked
phantom --version
```

`cargo install` puts the binary at `~/.cargo/bin/phantom`. If that's
not on your `$PATH`, the rustup installer added a line to `~/.profile`
that adds it — `source ~/.profile` or reopen the shell.

### 2. First-time onboarding

```bash
phantom onboarding
```

90-second interactive wizard. Sets up:
- `~/.phantom-mesh/agents.toml` with one provider (you pick which —
  Anthropic / OpenAI / Groq / OpenRouter / OpenCode / etc.)
- Default agent set (master, coder, reviewer, researcher)
- Cluster secret for HMAC peer auth

Skip the cluster section if you're solo; you can edit `agents.toml`
later to add peers.

### 3. Run as a systemd user unit (so it survives logout)

phantom's `service install` writes a systemd `--user` unit:

```bash
phantom service install
systemctl --user status phantom-serve     # should be active (running)
```

The unit lives at `~/.config/systemd/user/phantom-serve.service` and
runs `phantom serve` with auto-restart on failure. To start on boot
without an active login session (headless server):

```bash
sudo loginctl enable-linger $USER
```

### 4. (Optional) hourly autoevolve

```bash
phantom autoevolve schedule install --interval 3600
systemctl --user status phantom-autoevolve.timer
```

The autoevolve timer runs `phantom autoevolve --once` every hour:
checks cargo for red, dispatches a fix agent if so, commits when green.

Log at `~/.local/state/phantom-mesh/autoevolve.log` (or
`~/.phantom-mesh/autoevolve.log` depending on XDG state).

### 5. (Optional) cluster

Edit `~/.phantom-mesh/agents.toml`:

```toml
[cluster]
node_name      = "linux-1"
cluster_secret = "<same secret across nodes>"
peers = [
  "http://100.87.93.58:7878",      # mac (over Tailscale)
  "http://100.87.70.65:7879",      # windows-z13
]
```

`tailscale up` first if you haven't already. Then verify reachability:

```bash
curl http://<peer-tailscale-ip>:7878/healthz
```

---

## Verify

### 1. Quick health check

```bash
phantom doctor
```

`phantom doctor` runs 11 colour-coded sections on Linux
(binary, config, permissions, provider keys, phantom serve,
systemd, network, autoevolve, identity, diagnostics, tools).
Every line should be `✓` green or `⚠` yellow. `⚠` is expected for
features you haven't opted into (unused provider keys, autoevolve
not yet run). Red `✗` lines need fixing.

**Expected output on a healthy Linux install:**

```
phantom doctor 0.4.0

binary
  ✓ version: phantom 0.4.0 (093b1af4c8+, linux-x86_64, built 2026-05-11)
  ✓ path: /home/you/.cargo/bin/phantom

config
  ✓ agents.toml: /home/you/.phantom-mesh/agents.toml
  ✓ ~/.phantom-mesh: exists

permissions
  ⚠ [permissions]: no rules → allow all (legacy default).
                    See docs/PERMISSIONS.md for the Tool(specifier) DSL.

provider keys
  ⚠ Anthropic: not in env or agents.toml
  ✓ Groq: env (gsk_L1…)

phantom serve
  ✓ healthz: 200 OK on http://127.0.0.1:7878/healthz
  ✓ systemd: phantom-serve.service active

network
  ✓ Tailscale: connected (100.x.x.x  your-host  …  linux  -)

autoevolve
  ⚠ history: no runs yet — `phantom autoevolve --once`
  ⚠ schedule: not scheduled — `phantom autoevolve schedule install`

identity
  ✓ identity: local-only (broker not deployed yet — login becomes available
              once phantommesh.io/healthz returns 200)

diagnostics
  ✓ crash logs: 0 (no panics recorded)
  ✓ events log: /home/you/.phantom-mesh/events.jsonl (0 bytes)

tools
  ✓ tools: 54 total (52 built-in + 2 cluster RPC)

done.
```

The **⚠ lines to watch for on first install** (normal, not errors):
- `Anthropic: not in env` — you didn't choose it during onboarding;
  add `ANTHROPIC_API_KEY` to env or `agents.toml` if needed
- `autoevolve/history: no runs yet` — expected before first run;
  fix with `phantom autoevolve --once`
- `autoevolve/schedule: not scheduled` — normal if you skipped that step
- `identity: local-only (broker not deployed)` — expected; the broker at
  phantommesh.io isn't live yet, so login is not yet available

**Red ✗ lines that need fixing:**
- `agents.toml: not found` → run `phantom onboarding`
- `healthz: unreachable` → run `phantom serve` or `systemctl --user start phantom-serve`
- `systemd: no unit installed` → run `phantom service install`
- `Tailscale: not in PATH or not connected` → `sudo tailscale up`

For machine-readable output:

```bash
phantom doctor --json | jq '.status'       # "ok" / "warn" / "fail"
phantom doctor --json | jq '.serve'         # port, running, status
phantom doctor --json | jq '.autoevolve'   # queue + last run
```

### 2. Open the dashboard

```bash
xdg-open http://127.0.0.1:7878/projects
```

Should show 6 project tiles + cluster status bar + recent activity.
Each [Run Demo] streams output live via SSE.

### 3. Feature sweep

```bash
phantom selftest                # 22+ feature checks
phantom selftest --p0-only       # critical checks only, ~3 s
./scripts/test-mac.sh           # works on Linux too (51 checks)
./scripts/test-mcp-tools.sh     # 13 MCP tool/call e2e checks
```

---

## MCP integration with Claude Code

```bash
claude mcp add phantom $(which phantom) mcp
```

After this, Claude Code's tool palette gains `mcp__phantom__*` tools
(file_read, shell, content_search, git_*, task, subagent, …).

Smoke-test:
```bash
./scripts/test-mcp-tools.sh    # 13 checks; expect all pass
```

---

## Updating

```bash
cd ~/Documents/phantom-mesh
git pull
cd core && cargo install --path . --locked
systemctl --user restart phantom-serve.service
```

---

## Uninstall

```bash
phantom autoevolve schedule uninstall
phantom service uninstall
rm -rf ~/.phantom-mesh ~/.local/state/phantom-mesh
cargo uninstall phantom-mesh
# Optional: rm -rf ~/Documents/phantom-mesh   # clone itself
```

---

## Troubleshooting

### `phantom doctor` quick triage

Run `phantom doctor` and look for the failure in this order:

| `phantom doctor` line | Cause | Fix |
|---|---|---|
| `✗ agents.toml: not found` | onboarding not run | `phantom onboarding` |
| `✗ healthz: unreachable` | serve not running | `phantom serve &` or `systemctl --user start phantom-serve` |
| `⚠ autoevolve/history: no runs yet` | first run never done | `phantom autoevolve --once` |
| `⚠ autoevolve/schedule: not scheduled` | schedule not installed | `phantom autoevolve schedule install` |
| `⚠ Tailscale: not in PATH` | Tailscale not installed | `curl -fsSL https://tailscale.com/install.sh \| sh` |
| `⚠ Tailscale: not connected` | not logged in | `sudo tailscale up` |
| `⚠ crash logs: N recorded` | a recent agent run crashed | `phantom debug last` to read the latest |
| `⚠ identity: local-only` | expected (broker not deployed) | nothing to fix — this is normal |
| `⚠ events.jsonl: 0 bytes` | expected on first run | nothing to fix — this is normal |
| `✗ [permissions]: parse error` | syntax in `agents.toml [permissions]` block | check the DSL in docs/PERMISSIONS.md |

### Other shell-level failures

| Symptom | Fix |
|---|---|
| `cargo install` fails on `openssl-sys` | `apt install libssl-dev pkg-config` and retry |
| `cargo install` fails on `link.exe` not found | You're on WSL — use `cargo build --target x86_64-unknown-linux-gnu` instead |
| `phantom: command not found` after install | `source ~/.cargo/env` or add `~/.cargo/bin` to PATH |
| `systemctl --user start phantom-serve` says "Failed to connect to bus" | Not lingered — `sudo loginctl enable-linger $USER` then retry |
| `phantom autoevolve --once` crashes with no API key | In `agents.toml`, set `api_key_env = "GROQ_API_KEY"` (or your provider), export the env var, retry |
| Port 7878 already in use | Set `[core] port = 7879` in agents.toml |
| `phantom doctor` output is garbled (ANSI codes) | Pipe through `cat -v` or `less -R`; terminal may not support colour |

---

## Performance baseline (Ubuntu 24.04, AMD EPYC 4 vCPU, 8 GB RAM)

| Operation | Time |
|---|---|
| Fresh `cargo install --path .` (first build) | ~5-6 min |
| Incremental rebuild after 1-file edit | 4-8 s |
| `phantom doctor` cold | ~1 s |
| `phantom selftest --p0-only` | ~3 s |
| `phantom autoevolve --once` (green path) | < 5 s |
| `phantom mcp` startup → tools/list response | < 200 ms |
| HTTP `/api/projects` cold | < 50 ms |
