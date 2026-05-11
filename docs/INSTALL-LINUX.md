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

```bash
phantom doctor                  # 9 sections; all ✓ in a healthy install
phantom doctor --json | jq      # JSON form; .status = "ok" / "warn" / "fail"
phantom selftest                # 22+ feature checks
./scripts/test-mac.sh           # actually works on Linux too (despite the name)
```

Open the dashboard:
```bash
xdg-open http://127.0.0.1:7878/projects
```

Should show 6 project tiles + cluster status bar + recent activity.
Each [Run Demo] streams output live via SSE.

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

| Symptom | Fix |
|---|---|
| `cargo install` fails on `openssl-sys` | `apt install libssl-dev pkg-config` and retry |
| `cargo install` fails on `link.exe` not found | You're on WSL accidentally — use `cargo build --target x86_64-unknown-linux-gnu` |
| `phantom: command not found` after install | `source ~/.cargo/env` or add `~/.cargo/bin` to PATH |
| `systemctl --user start phantom-serve` says "Failed to connect to bus" | You're in an ssh session without lingering — `sudo loginctl enable-linger $USER` |
| Port 7878 already in use | Set `[core] port = 7879` in agents.toml |
| `phantom doctor` shows tailscale not connected | `sudo tailscale up` (and check the Tailscale admin UI for an auth nag) |
| `phantom autoevolve --once` crashes with no API key | Edit `agents.toml`, set `api_key_env = "ANTHROPIC_API_KEY"` (or whichever provider), export the env var, retry |

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
