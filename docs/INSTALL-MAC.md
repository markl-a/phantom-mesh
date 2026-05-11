# phantom on macOS — Install Guide

Pairs with `INSTALL-ANDROID.md` and `INSTALL-IOS.md`. Mac is the
recommended **coordinator** for a phantom mesh — it ships native
launchd auto-start, APFS snapshot rollback, MLX local LLM, and the
deepest doctor diagnostics.

---

## TL;DR — 90 seconds

```bash
# 1. Build / install the binary (one of):
brew install markl-a/phantom-mesh/phantom              # (planned)
curl -fsSL https://phantom-mesh.dev/install.sh | sh    # (planned)
git clone https://github.com/markl-a/phantom-mesh && \
  cd phantom-mesh/core && cargo install --path .       # ✅ works today

# 2. First-time setup — wizard writes ~/.phantom-mesh/agents.toml + runs doctor
phantom onboarding

# 3. Auto-start at every login (launchd LaunchAgent)
phantom service install

# 4. (Optional) hourly self-improvement loop
phantom autoevolve schedule install

# 5. Verify
phantom doctor
./scripts/test-mac.sh        # 51 automated checks, takes ~30s
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

```bash
phantom doctor
```

Expected: 9 sections, all ✓ (or yellow ⚠ for opt-in features you
haven't enabled, like MLX or Spotlight indexing). 0 red ✗ in a
working install.

```bash
./scripts/test-mac.sh
```

51 automated checks, ~30 s. Expects PASS 51 / FAIL 0 / SKIP ≤ 1.

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
(every issue we hit while building this gets one section). The
single most common one is the macOS-26 TCC trap, fixed in 65338ab —
but if you upgraded across that boundary, run `phantom service
uninstall && phantom service install` once.

For quick triage, the script `./scripts/test-mac.sh` will tell you
which of 51 checks is failing.

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
