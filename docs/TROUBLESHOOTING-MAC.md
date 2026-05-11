# phantom on macOS — Troubleshooting

If something is wrong, **start with `phantom doctor`** — it surfaces
binary provenance, config location, provider keys, healthz, launchd
state, Tailscale, tool count, tmutil, Spotlight, and Xcode CLT in one
screen.

```bash
phantom doctor
```

Anything below is "I ran doctor and it told me X — what now?"

---

## healthz unreachable on :7878

**doctor row**: `⚠ healthz: unreachable on :7878`

1. Check the daemon: `phantom service status`
2. If `registered : no`, install it: `phantom service install`
3. If `registered : yes` but `healthz : unreachable`, the launchd
   process is alive but not listening. This is almost always one of
   the two TCC traps below.

### Trap #1 — binary under ~/Documents (TCC blocks dyld)

**Symptom**: `phantom service status` shows `registered: yes, pid: N`,
but `lsof -nP -iTCP:7878` shows no LISTEN, and the log shows the
banner up to "Registering Service ..." then nothing more.

**Cause**: macOS 26 (Sequoia / Tahoe) TCC blocks launchd-spawned
processes from loading binaries that live under `~/Documents`,
`~/Downloads`, `~/Desktop`. The dyld dynamic linker stalls in
`__open` before `main()` is reached. `launchctl print` still reports
`state = running` because the process did spawn — just hangs.

**Fix**: `phantom service install` since 65338ab copies the binary
into `~/Library/Application Support/phantom-mesh/bin/phantom` (TCC-
unrestricted) and points the plist there. Re-run `install` to pick
up that path:

```bash
phantom service uninstall
phantom service install
```

### Trap #2 — cwd under ~/Documents (TCC blocks getcwd)

**Symptom**: same banner-and-stop pattern as Trap #1, but `lsof
-p <pid>` shows `cwd /Users/.../Documents/...`. `sample <pid>` shows
the process stalled in `std::env::current_dir` from inside
`find_config()`.

**Cause**: same TCC subsystem also blocks `getcwd()` on protected
paths.

**Fix**: same `phantom service install` since 65338ab also overrides
the plist `WorkingDirectory` to `~/Library/Application Support/
phantom-mesh` whenever install was run from inside `~/Documents`,
`~/Downloads`, or `~/Desktop`.

### Trap #3 — port already bound

**Symptom**: launchd-spawned daemon doesn't listen, but a manual
`phantom serve` run from a terminal does.

**Cause**: another phantom serve is already on :7878 (often a
left-over from a prior interactive run). `axum::serve` does not
panic on bind failure — it logs and continues, leaving the listener
unattached.

**Fix**:
```bash
pkill -f "phantom serve"
launchctl kickstart -k gui/$(id -u)/ai.phantommesh.serve
```

---

## launchd doesn't auto-start after reboot

**doctor row**: `⚠ launchd: not installed`

```bash
phantom service install
```

If install succeeds but post-reboot it still doesn't come up:

1. `tail -50 ~/Library/Logs/phantom-serve.log` — look for the banner
2. If no banner, the binary itself is failing to load — see Trap #1
3. If the banner appears but stops, see Trap #2
4. If the binary path no longer exists (e.g. you `cargo clean`'d the
   target/release/), reinstall the service: `phantom service install`
   refreshes the copied binary.

---

## ROG / Android worker unreachable

**Doctor on the ROG** (in Termux):

```bash
~/.phantom-mesh/bin/phantom doctor
```

Common issues:

1. **Tailscale not connected on the phone** — open Tailscale app on
   the device, log in to the same tailnet
2. **Termux process killed in background** — Android battery
   optimization kills phantom serve; in Settings → Apps → Termux,
   set battery to "Unrestricted"
3. **Coordinator URL wrong in agents.toml** — the install script
   defaults to `http://100.87.93.58:7878` (the original Mac
   coordinator). If your Mac TS IP changed, edit
   `~/.phantom-mesh/agents.toml` and restart.

To re-bootstrap from the coordinator:

```bash
COORD=http://<NEW-COORD-IP>:7878 \
  curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
```

---

## phantom MCP unreachable from Claude Code / Codex

**doctor doesn't expose this directly**, but if `mcp__phantom__*`
tools fail in Claude Code or `codex mcp list` doesn't show phantom:

1. `cat ~/.claude.json | grep -A5 phantom` — must show stdio + path
2. `cat ~/.codex/config.toml | grep -A2 phantom` — must show
   `[mcp_servers.phantom]`
3. Re-register if missing:
   ```bash
   claude mcp add phantom $(which phantom) mcp
   codex mcp add phantom -- $(which phantom) mcp
   ```
4. Restart the Claude Code session — MCP servers are spawned at
   session start and don't hot-reload when the binary changes
5. Run `./scripts/validate-mcp.sh` to confirm the binary itself is
   healthy

---

## Provider keys not loaded

**doctor row**: `⚠ Anthropic / OpenAI / Groq / Gemini: not in env`

1. Confirm `~/.phantom-mesh/env` exists and has `KEY=value` lines
2. Load it for the current shell:
   ```bash
   set -a; source ~/.phantom-mesh/env; set +a
   ```
3. Make it auto-load: append to `~/.zshrc` /
   `~/.bashrc`:
   ```bash
   [ -f ~/.phantom-mesh/env ] && set -a && source ~/.phantom-mesh/env && set +a
   ```
4. For the launchd-spawned daemon, the keys must be in the plist's
   `EnvironmentVariables` block. The default install only injects
   `PATH` and `HOME` — secrets are not included on purpose; configure
   them via `~/.phantom-mesh/agents.toml` `[providers.*]` blocks
   instead.

---

## Spotlight `spotlight_search` returns nothing

**doctor row**: `⚠ Spotlight: not indexing /Users/.../phantom-mesh`

```bash
sudo mdutil -i on /Users/marklight/Documents/workspace/hailmary/phantom-mesh
sudo mdutil -E /Users/marklight/Documents/workspace/hailmary/phantom-mesh
```

Wait ~30 s for the index rebuild, then re-run `spotlight_search`.

`spotlight_search` falls back gracefully — if no result, the tool
prints a hint pointing at `mdutil`.

---

## Xcode tools (`xcode_simctl`) report missing

**doctor row**: `⚠ Xcode CLT: missing`

```bash
xcode-select --install
```

Full Xcode is not required for `xcode_simctl` — only the
command-line tools providing `xcrun simctl`. Once installed, run
`phantom doctor` again.

---

## Subagent / parallel_tasks return "runtime not initialised"

**Cause**: you are running an old phantom binary (pre-48bb842) where
the `phantom mcp` stdio path forgot to call `subagent::init_global()`.

**Fix**: rebuild and reinstall:

```bash
cd /path/to/phantom-mesh/core
cargo build --release --bin phantom
phantom service uninstall
phantom service install
```

Then restart any Claude Code / Codex session that has the MCP server
already spawned — they don't pick up new binaries on the fly.

---

## subagent budget exceeded ridiculously

**Symptom**: you set `max_cost_usd: 0.10` and the task spent $0.98.

**Cause**: pre-72b34f7 the budget was checked **post-hoc**, after
the agent loop returned. Decorative.

**Fix**: rebuild from a tree that includes 72b34f7 — the budget is
now polled per round and breaks out at the next round boundary.
Expected overrun is ~10% (the LLM completes one more round before
the check lands), not 10×.

---

## Re-validate after any of the above

```bash
phantom doctor
./scripts/validate-mcp.sh
```

Both should be all green. If something is still off, the relevant
log is `~/Library/Logs/phantom-serve.log` for the daemon and `tail
-f` while you reproduce the issue.
