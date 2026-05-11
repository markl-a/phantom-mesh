# Android (Termux CLI binary) — Smoke Checklist

End-to-end validation procedure for the `phantom-aarch64-linux-android` CLI
binary running as a cluster worker inside Termux on a real Android device.

**Coverage:** install · CLI subcommands · daemon · HTTP/RPC matrix · embedded
web frontend · MCP stdio · HMAC enforcement · real LLM dispatch · cluster
mesh integration · ratatui TUI · autoevolve · persistence (Termux:Boot) ·
stress · failure modes · cleanup.

**Time budget:** first run ~60–90 min, regression run ~15 min.

> Companion doc: [INSTALL-ANDROID.md](INSTALL-ANDROID.md) covers install only.
> This doc assumes you've completed §B of that.

---

## Phase 0 · Prerequisites (10 min)

```
[ ] Tailscale on phone — VPN icon in status bar, 100.x.y.z assigned
[ ] Termux from F-Droid (NOT Play Store)
[ ] Termux:Boot from F-Droid (optional; needed for Phase 12)
[ ] Mac coordinator at 100.87.93.58:7878 with phantom serve running
[ ] One usable Groq API key (gsk_…) — Phase 8 needs it
[ ] Out-of-band shell to phone — pick one:
      - adb-tcp:  adb connect 100.84.223.59:38913
      - SSH:      ssh -p 8022 u0_a187@100.84.223.59
                  (requires `pkg install openssh && sshd` in Termux)
```

From phone Termux, both must succeed:
```bash
ping -c 1 100.87.93.58
curl -sS http://100.87.93.58:7878/healthz   # → ok
```

**Pass if:** every checkbox ticked, both curls succeed.

---

## Phase 1 · Install via Termux (5 min)

In Termux on the phone:

```bash
COORD=http://100.87.93.58:7878
GROQ_KEY=gsk_yourkeyhere
curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
```

What the script does:
- `pkg install` curl/wget/git/termux-tools
- pulls the latest `phantom-aarch64-linux-android` from `<COORD>/dist/`
- writes `~/.phantom-mesh/agents.toml` with the cluster_secret + Groq key
- starts `phantom serve --port 7879` in background
- prints the 3-way menu (TUI / browser / cluster worker)

**Pass if:**
```bash
which phantom                  # $PREFIX/bin/phantom
file $(which phantom)          # ELF 64-bit ARM aarch64
ls -lh ~/.phantom-mesh/        # bin/, data/, agents.toml
curl -sS http://127.0.0.1:7879/healthz   # ok
```

---

## Phase 2 · CLI sanity (5 min)

```bash
phantom --version              # → phantom 0.4.0 (..., android-aarch64, …)
phantom -V                     # same
phantom doctor                 # 9 sections — most ✓ or ⚠

phantom autoevolve log         # "no runs yet" on first boot — fine
phantom evolve goals list      # tries to load EVOLVE-GOALS.md; "not found" is fine
```

Expected platform-gated failures (correct behaviour):

```bash
phantom service status         # ✗ not yet implemented on this platform
phantom snapshot list          # ✗ macOS-only (uses tmutil)
```

**Avoid these — known CLI bugs that hang the shell:**
```bash
# phantom serve --help     ← actually starts the daemon
# phantom mcp --help       ← also broken; spawns the stdio server
```

**Pass if:** doctor 8/9 ✓-or-⚠ (no ✗), the two platform-gated commands exit 1
with the expected message.

---

## Phase 3 · Daemon (serve) verification

`termux-setup.sh` already started serve. Verify it's healthy:

```bash
PID=$(pgrep -f "phantom serve" | head -1)
echo "PID=$PID"
ss -ltn | grep 7879            # LISTEN 0.0.0.0:7879
grep -E 'VmRSS|Threads' /proc/$PID/status

tail -20 ~/.phantom-mesh/data/phantom-serve.log
```

**Pass if:** PID exists · port listening · banner in log · RSS < 50 MB · 0
error/panic/fatal lines.

---

## Phase 4 · HTTP / RPC endpoint matrix (10 min)

```bash
PORT=7879
for path in /healthz /rpc/ping /rpc/peers /api/sessions /api/cost /api/todos /api/nodes / /m /static/app.css /static/xterm.css; do
  CODE=$(curl -sSo /dev/null -w '%{http_code}' http://127.0.0.1:$PORT$path)
  printf '  %-30s  %s\n' "$path" "$CODE"
done
```

Expected — all `200`:

| Path | What |
|---|---|
| `/healthz` | health probe (`ok`) |
| `/rpc/ping` | node identity JSON |
| `/rpc/peers` | peer list JSON |
| `/api/sessions` | session list (likely `[]`) |
| `/api/cost` | cost summary |
| `/api/todos` | todos |
| `/api/nodes` | live peer ping |
| `/` | desktop web frontend (HTML) |
| `/m` | mobile chat UI (HTML) |
| `/static/app.css` | embedded stylesheet |
| `/static/xterm.css` | xterm.js stylesheet |

Expected `404` (coordinator-only — workers don't register them):
`/dist/<file>`, `/scripts/<file>`, `/api/onboarding/{token,config}`,
`/api/health`, `/api/peers`, `/api/tools`.

**Pass if:** the 11 endpoints above all return 200; the 404s come back as 404.

---

## Phase 5 · Web frontend in browser (5 min)

Open Chrome (or any browser) on the phone, navigate to:

```
http://127.0.0.1:7879/
```

**Expect:**
- Title bar shows `phantom · mesh`
- Cream / dark theme header
- xterm.js terminal panel renders (dark)
- Info tab visible with sub-tabs Sessions / Cost / Todo / Tools
- Browser console has no red errors

```
http://127.0.0.1:7879/m
```

**Expect:** mobile chat UI with bottom navigation, input box at bottom.

**Pass if:** both pages render, no blank screens, no console errors.

> Tip: Chrome → ⋮ → "Add to Home screen" gives you a PWA-style icon.

---

## Phase 6 · MCP stdio JSON-RPC (10 min)

In Termux:

```bash
echo '{"jsonrpc":"2.0","method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}},"id":1}' \
  | phantom mcp 2>/dev/null
```

**Expect:**
```json
{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{"listChanged":false}},"protocolVersion":"2024-11-05","serverInfo":{"name":"phantom-mesh","version":"0.4.0"}}}
```

```bash
echo '{"jsonrpc":"2.0","method":"tools/list","id":2}' \
  | phantom mcp 2>/dev/null | head -c 1000
```

**Expect:** tools array starting with shell / file_read / file_write / web_fetch / …

**Pass if:** both return valid JSON-RPC responses with the right shape.

**Advanced:** wire it into Claude Code on Mac/Win:

```jsonc
// ~/.claude.json
"mcpServers": {
  "phantom-android": {
    "command": "ssh",
    "args": ["-p", "8022", "u0_a187@100.84.223.59", "phantom mcp"]
  }
}
```

After restarting Claude Code, `mcp__phantom-android__*` tools should appear
in ToolSearch.

---

## Phase 7 · HMAC enforcement (5 min)

```bash
SECRET=$(grep cluster_secret ~/.phantom-mesh/agents.toml | sed 's/.*"\(.*\)"/\1/')
BODY='{"agent":"master","prompt":"reply: ok"}'
GOOD=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
PORT=7879

# bad → 401
curl -sS -w 'HTTP %{http_code}\n' -X POST http://127.0.0.1:$PORT/rpc/task/assign \
  -H "X-Cluster-Auth: $(printf '0%.0s' {1..64})" \
  -H 'Content-Type: application/json' -d "$BODY"

# good → {job_id}
curl -sS -X POST http://127.0.0.1:$PORT/rpc/task/assign \
  -H "X-Cluster-Auth: $GOOD" -H 'Content-Type: application/json' -d "$BODY"
```

**Pass if:** bad → `HTTP 401`; good → `{"job_id":"…"}`.

> When `cluster_secret` is **not** configured (no agents.toml), the daemon
> runs in dev / no-auth mode and accepts any request. Always configure
> `[cluster].cluster_secret` for non-localhost deployments.

---

## Phase 8 · Real LLM dispatch (5 min)

Confirm `~/.phantom-mesh/agents.toml` has `[providers.groq].api_key = "gsk_…"`
(real key, not the placeholder). If still placeholder:

```bash
nano ~/.phantom-mesh/agents.toml   # set api_key
pkill phantom
nohup phantom serve > ~/.phantom-mesh/data/phantom-serve.log 2>&1 &
sleep 4
```

Reuse `SECRET` and `GOOD` from Phase 7:

```bash
RESP=$(curl -sS -X POST http://127.0.0.1:7879/rpc/task/assign \
  -H "X-Cluster-Auth: $GOOD" -H 'Content-Type: application/json' -d "$BODY")
JOB=$(echo "$RESP" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')
sleep 8
curl -sS http://127.0.0.1:7879/rpc/task/status/$JOB
```

**Expect:**
```json
{"status":"done","output":"<llama text>","error":null,"job_id":"…"}
```

**Pass if:** `status=done`, `output` non-empty, `error=null`.

> Llama 3.3 70B's tool-use formatter sometimes returns 400 to Groq. Cleanest
> single-shot prompt: `reply with at most 8 words`. To eliminate tool-use
> entirely set `[agent.master].tools = []`.

---

## Phase 9 · Cluster mesh integration (10 min)

On the phone:

```bash
# Add Mac as peer
nano ~/.phantom-mesh/agents.toml
# under [cluster] add:
#   peers = ["http://100.87.93.58:7878"]
pkill phantom
nohup phantom serve > ~/.phantom-mesh/data/phantom-serve.log 2>&1 &
sleep 4

curl -sS http://127.0.0.1:7879/rpc/peers      # Mac should appear
PHONE_IP=$(tailscale ip -4 | head -1)
echo "phone TS IP: $PHONE_IP"
```

From the **Mac coordinator** terminal:

```bash
PHONE_IP=<value from above>
curl -sS http://$PHONE_IP:7879/healthz
curl -sS http://$PHONE_IP:7879/rpc/ping

# Mac → phone HMAC dispatch
SECRET="phantom-cluster-2026"
BODY='{"agent":"master","prompt":"reply: hi from mac"}'
AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
RESP=$(curl -sS -X POST http://$PHONE_IP:7879/rpc/task/assign \
  -H "X-Cluster-Auth: $AUTH" -H 'Content-Type: application/json' -d "$BODY")
JOB=$(echo "$RESP" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')
sleep 8
curl -sS http://$PHONE_IP:7879/rpc/task/status/$JOB
```

**Pass if:**
- phone's `/rpc/peers` lists Mac
- Mac's `curl http://$PHONE_IP:7879/healthz` returns `ok`
- HMAC dispatch from Mac → phone returns `status: done`
- Mac's `/api/nodes` shows phone in the online list

---

## Phase 10 · TUI (interactive, 10 min)

```bash
phantom            # default = ratatui TUI
```

Test:
- `↑` / `↓` — input history
- Tab — slash command / `@file` autocomplete
- `/help` — list 22 slash commands
- `/agents` — agents from agents.toml
- `/tools` — 49 tools
- `/density compact`
- Enter a normal prompt (`hi`) — see streaming token-by-token reply
- Ctrl-C to exit

**Pass if:** every keybinding responds, prompt streams real LLM output, Ctrl-C
exits cleanly.

> Adb-shell-rendered TUI has minor wrapping / colour glitches but works. Prefer
> a proper Termux session over adb shell for TUI testing.

---

## Phase 11 · Autoevolve (10 min)

> Autoevolve assumes the cwd has a Cargo.toml. On a fresh Termux without a
> cloned phantom-mesh repo it will fail gracefully. Either git-clone first or
> accept the no-target outcome.

```bash
cd ~
phantom autoevolve --once
phantom autoevolve log --n 3
```

**Expect:** one entry; status `no-target` or `cargo-missing` if no Cargo.toml.

```bash
phantom autoevolve schedule install
phantom autoevolve schedule status
```

**Expect:** `not yet implemented on this platform` (Android has no
LaunchAgent / systemd) — correct behaviour.

**Pass if:** autoevolve runs once without panic; schedule install fails with
the expected platform message.

---

## Phase 12 · Termux:Boot persistence (15 min, includes phone reboot)

After installing Termux:Boot from F-Droid:

```bash
mkdir -p ~/.termux/boot
cat > ~/.termux/boot/phantom-serve <<'EOF'
#!/data/data/com.termux/files/usr/bin/sh
~/.phantom-mesh/bin/phantom serve >> ~/.phantom-mesh/data/phantom-serve.log 2>&1 &
EOF
chmod +x ~/.termux/boot/phantom-serve
```

**Reboot the phone**. After it wakes (give it 30 s):

```bash
# in Termux
pgrep phantom
curl -sS http://127.0.0.1:7879/healthz
```

**Pass if:** phantom is already running and healthz returns `ok`.

---

## Phase 13 · Stress / longevity (30 min)

```bash
# 100 sequential healthz hits
START=$(date +%s%N)
OK=0
for i in $(seq 1 100); do
  R=$(curl -sS http://127.0.0.1:7879/healthz)
  [ "$R" = "ok" ] && OK=$((OK+1))
done
END=$(date +%s%N)
echo "$OK/100 OK, $(( (END-START)/1000000 ))ms total"

# 10 concurrent dispatches (needs Groq key)
SECRET="phantom-cluster-2026"
for i in $(seq 1 10); do
  BODY="{\"agent\":\"master\",\"prompt\":\"echo $i\"}"
  AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
  curl -sS -X POST http://127.0.0.1:7879/rpc/task/assign \
    -H "X-Cluster-Auth: $AUTH" -H 'Content-Type: application/json' -d "$BODY" &
done
wait
sleep 10
curl -sS http://127.0.0.1:7879/api/sessions | head -c 400

# memory after 1 h
PID=$(pgrep -f "phantom serve")
grep VmRSS /proc/$PID/status
ls /proc/$PID/fd | wc -l
```

**Pass if:** 100/100 OK, ≥ 8/10 concurrent dispatches reach `done` (Groq may
rate-limit on 1 GB E2.1.Micro-tier accounts), VmRSS growth after 1 h idle <
5 MB, fd count < 50.

---

## Phase 14 · Failure modes (20 min)

| # | Inject | Procedure | Expected |
|---|---|---|---|
| 1 | Bad provider key | edit toml, `api_key = "gsk_invalid"`, restart, dispatch | status=error, error contains 401 |
| 2 | Network drop | `tailscale down`, dispatch | status=error, error contains timeout / connection refused |
| 3 | OOM edge | 5 concurrent prompts on 1 GB phone | ≥ 3 done, no panic |
| 4 | `kill -9` daemon | `pkill -9 phantom`, inspect log | log clean, socket released, no panic line |
| 5 | Port collision | start a 2nd serve before 1st dies | 2nd exits 1 with `Address already in use` |
| 6 | Broken agents.toml | `cluster_secret = ` (no value), restart | exit 1 with toml parse error before bind |
| 7 | Disk full (Termux home) | `dd if=/dev/zero of=~/big bs=1M count=$(df ~ \| awk 'NR==2{print $4}')` | dispatch error logged, no panic |
| 8 | Tailscale IP change | `tailscale down; tailscale up` | restart daemon → still reachable from Mac on new IP |

**Pass if:** every case fails *safely* — error message present, no panic, no
zombie process, daemon survives or exits cleanly.

---

## Phase 15 · Cleanup (5 min)

```bash
pkill phantom
sleep 2
pgrep phantom || echo "all stopped"

# Optional full uninstall
rm -rf ~/.phantom-mesh
rm -f ~/.termux/boot/phantom-serve

ls ~/.phantom-mesh 2>&1            # No such file or directory
which phantom 2>&1                 # not found
```

**Pass if:** phantom and its config directory are gone from the device.

---

## Result matrix

Track Pass / Fail / Skipped per phase:

```
Phase  Title                              Result      Notes
─────  ────────────────────────────────  ──────────  ────────────────────
0      Prerequisites                      [ ]
1      Termux install                     [ ]
2      CLI sanity                         [ ]
3      Daemon serve                       [ ]
4      HTTP/RPC matrix                    [ ]
5      Web frontend (Chrome)              [ ]
6      MCP stdio                          [ ]
7      HMAC enforcement                   [ ]
8      Real LLM dispatch                  [ ]
9      Cluster mesh integration           [ ]
10     TUI                                [ ]
11     Autoevolve                         [ ]
12     Termux:Boot persistence            [ ]
13     Stress / longevity                 [ ]
14     Failure modes (8 cases)            [ ]
15     Cleanup                            [ ]
```

---

## Known caveats (as of 2026-05-01)

- `phantom serve --help` and `phantom mcp --help` start the daemon instead of
  printing usage. Avoid in foreground scripts.
- `phantom serve --port <N>` is silently ignored; daemon uses the value from
  `~/.phantom-mesh/agents.toml` `[core].port` (default 7878). Termux setup
  picks 7879 via the toml.
- `/api/health`, `/api/peers`, `/api/tools` return 404 on workers. Use the
  `/rpc/*` equivalents.
- `/dist/<…>`, `/scripts/<…>`, `/api/onboarding/*` are coordinator-only.
- `VmPeak` shown in `/proc/<pid>/status` looks alarming (~12 GB virtual) — it's
  tokio reserving worker stacks. `VmRSS` is the actual physical footprint
  (~10 MB idle, < 50 MB busy).
