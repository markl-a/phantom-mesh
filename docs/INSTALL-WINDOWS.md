# Installing phantom-mesh on Windows 11

Tested on **Windows 11 23H2 + 24H2**, native (not WSL). PowerShell 7
recommended over the bundled 5.1 because some scripts use `?.`
operator and structured-error parsing.

For WSL2: install via the [Linux guide](INSTALL-LINUX.md) — phantom
runs natively in WSL, you just lose the `phantom service install` →
Scheduled Task path (WSL doesn't have Windows Task Scheduler access).

---

## TL;DR — 90 seconds (native Win)

```powershell
# In an elevated PowerShell (Run as Administrator) for the install step,
# then drop back to normal user shell for everyday use.

# 1. Install dependencies via winget
winget install --silent Git.Git Rustlang.Rustup tailscale.tailscale

# 2. Refresh PATH (or close + reopen PowerShell)
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Program Files\Git\cmd;$env:PATH"

# 3. Build phantom
cd $env:USERPROFILE\Documents
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh\core
cargo install --path . --locked
phantom --version

# 4. First-time wizard
phantom onboarding

# 5. Start serve (in a separate PowerShell so it stays running)
Start-Process phantom -ArgumentList "serve" -WindowStyle Hidden

# 6. Verify
phantom doctor
Start-Process "http://127.0.0.1:7878/projects"
```

---

## Shortcut: cross-compiled binary (skip the cargo build)

If you have a Mac/Linux machine with the source, you can
**cross-compile** the Windows binary there (faster than building on
Windows for the first time):

On Mac:
```bash
brew install mingw-w64
rustup target add x86_64-pc-windows-gnu
cd ~/Documents/phantom-mesh/core
cargo build --release --target x86_64-pc-windows-gnu --bin phantom
# produces target/x86_64-pc-windows-gnu/release/phantom.exe (~33 MB)
```

Then transfer the `.exe` to Windows (SCP / SMB / `python -m http.server`)
and place at `$env:USERPROFILE\AppData\Local\Programs\phantom-mesh\phantom.exe`.

The `scripts/setup-z13.ps1` script in this repo automates this whole
flow — see comments at the top.

---

## Prereqs

| Component | Why | How |
|---|---|---|
| Rust toolchain ≥ 1.80 | Build phantom | `winget install Rustlang.Rustup` |
| `git`                 | Clone the repo | `winget install Git.Git` |
| Tailscale (optional)  | Cross-machine cluster + mobile access | `winget install tailscale.tailscale` |
| PowerShell 7 (recommended) | Some scripts use modern syntax | `winget install Microsoft.PowerShell` |

---

## Detailed install

### 1. Open PowerShell (elevated for installs, normal for usage)

```powershell
# Start menu → search "PowerShell" → "Run as Administrator"
$PSVersionTable.PSVersion   # confirm 7.x
```

### 2. winget installs

```powershell
winget install --silent Git.Git
winget install --silent Rustlang.Rustup
winget install --silent tailscale.tailscale
winget install --silent --id Microsoft.PowerShell

# Close + reopen PowerShell so PATH is fresh
```

### 3. Build phantom

```powershell
cd $env:USERPROFILE\Documents
git clone https://github.com/markl-a/phantom-mesh
cd phantom-mesh\core
cargo install --path . --locked
phantom --version
```

`cargo install` puts `phantom.exe` at
`$env:USERPROFILE\.cargo\bin\phantom.exe`. The Rustup installer adds
this to PATH; reopen PowerShell if not.

### 4. First-time onboarding

```powershell
phantom onboarding
```

90 s interactive wizard. Writes `$env:USERPROFILE\.phantom-mesh\agents.toml`.

### 5. Start phantom serve

phantom doesn't yet have a native Windows Service installer (planned).
For now, two options:

**Option A: Background process (manual restart on reboot)**
```powershell
Start-Process phantom -ArgumentList "serve" -WindowStyle Hidden
```

**Option B: Scheduled Task at logon** (auto-start)
```powershell
$action  = New-ScheduledTaskAction -Execute "phantom" -Argument "serve"
$trigger = New-ScheduledTaskTrigger -AtLogon
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
Register-ScheduledTask -TaskName "phantom-serve" -Action $action -Trigger $trigger -Settings $settings
Start-ScheduledTask -TaskName "phantom-serve"
```

To stop:
```powershell
Get-Process phantom | Stop-Process
Unregister-ScheduledTask -TaskName "phantom-serve" -Confirm:$false
```

### 6. (Optional) hourly autoevolve

```powershell
phantom autoevolve schedule install --interval 3600
```

If the message says "macOS + Windows only" then it's set; if it errors,
fall back to a manual Scheduled Task:
```powershell
$action  = New-ScheduledTaskAction -Execute "phantom" -Argument "autoevolve --once"
$trigger = New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Hours 1)
Register-ScheduledTask -TaskName "phantom-autoevolve" -Action $action -Trigger $trigger
```

### 7. (Optional) cluster

Edit `$env:USERPROFILE\.phantom-mesh\agents.toml`:

```toml
[cluster]
node_name      = "win-1"
cluster_secret = "<same secret across nodes>"
peers = [
  "http://100.87.93.58:7878",      # mac (over Tailscale)
  "http://100.107.205.98:7878",    # other windows
]
```

Then `tailscale up` and verify:
```powershell
Invoke-RestMethod http://100.87.93.58:7878/healthz
```

---

## Verify

### 1. Quick health check

```powershell
phantom doctor
```

`phantom doctor` runs 11 colour-coded sections on Windows
(binary, config, permissions, provider keys, phantom serve,
Scheduled Task, network, autoevolve, identity, diagnostics, tools).
Every line should be `✓` green or `⚠` yellow. `⚠` is expected for
features you haven't opted into (unused provider keys, autoevolve not
yet run). Red `✗` lines need fixing.

**Expected output on a healthy Windows install:**

```
phantom doctor 0.4.0

binary
  ✓ version: phantom 0.4.0 (093b1af4c8+, windows-x86_64, built 2026-05-11)
  ✓ path: C:\Users\you\.cargo\bin\phantom.exe

config
  ✓ agents.toml: C:\Users\you\.phantom-mesh\agents.toml
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
  ✓ Scheduled Task: registered (last run ?)

network
  ⚠ Tailscale: not in PATH or not connected — `tailscale up`

autoevolve
  ⚠ history: no runs yet — `phantom autoevolve --once`
  ⚠ schedule: not scheduled — `phantom autoevolve schedule install`

identity
  ✓ identity: local-only (broker not deployed yet — login becomes available
              once phantommesh.io/healthz returns 200)

diagnostics
  ✓ crash logs: 0 (no panics recorded)
  ✓ events log: C:\Users\you\.phantom-mesh\events.jsonl (0 bytes)

tools
  ✓ tools: 54 total (52 built-in + 2 cluster RPC)

done.
```

The **⚠ lines to watch for on first install** (normal, not errors):
- `Anthropic: not in env` — you didn't choose it during onboarding;
  add `ANTHROPIC_API_KEY` to env or `agents.toml` if needed
- `Tailscale: not in PATH or not connected` — run `tailscale up`
  from an elevated PowerShell
- `autoevolve/history: no runs yet` — expected before first run;
  fix with `phantom autoevolve --once`
- `autoevolve/schedule: not scheduled` — normal if you skipped that step
- `identity: local-only (broker not deployed)` — expected; broker
  at phantommesh.io isn't live yet

**Red ✗ lines that need fixing:**
- `agents.toml: not found` → run `phantom onboarding`
- `healthz: unreachable` → start phantom serve or check port 7878
- `Scheduled Task: not installed` → run the Scheduled Task commands
  in the install section above
- `Tailscale: not in PATH or not connected` → `tailscale up`

For machine-readable output:

```powershell
phantom doctor --json | ConvertFrom-Json | Select-Object status
phantom doctor --json | ConvertFrom-Json | Select-Object -ExpandProperty serve
phantom doctor --json | ConvertFrom-Json | Select-Object -ExpandProperty autoevolve
```

### 2. Open the dashboard

```powershell
Start-Process "http://127.0.0.1:7878/projects"
```

Should show 6 project tiles + cluster status + recent activity.
Each [Run Demo] streams output live via SSE.

### 3. Feature sweep

```powershell
phantom selftest                # 22+ feature checks
phantom selftest --p0-only      # critical checks only, ~4 s
```

---

## MCP integration with Claude Code

```powershell
claude mcp add phantom (Get-Command phantom).Source mcp
```

After this, Claude Code's tool palette gains `mcp__phantom__*` tools.

---

## Updating

```powershell
cd $env:USERPROFILE\Documents\phantom-mesh
git pull
cd core
cargo install --path . --locked
Stop-ScheduledTask -TaskName "phantom-serve"
Start-ScheduledTask -TaskName "phantom-serve"
```

---

## Uninstall

```powershell
Unregister-ScheduledTask -TaskName "phantom-autoevolve" -Confirm:$false -ErrorAction SilentlyContinue
Unregister-ScheduledTask -TaskName "phantom-serve"      -Confirm:$false -ErrorAction SilentlyContinue
Get-Process phantom -ErrorAction SilentlyContinue | Stop-Process -Force
cargo uninstall phantom-mesh
Remove-Item -Recurse -Force $env:USERPROFILE\.phantom-mesh
Remove-Item -Recurse -Force $env:USERPROFILE\AppData\Local\Programs\phantom-mesh
```

---

## Troubleshooting

### `phantom doctor` quick triage

Run `phantom doctor` and look for the failure in this order:

| `phantom doctor` line | Cause | Fix |
|---|---|---|
| `✗ agents.toml: not found` | onboarding not run | `phantom onboarding` |
| `✗ healthz: unreachable` | serve not running | `Start-Process phantom -ArgumentList "serve" -WindowStyle Hidden` |
| `⚠ autoevolve/history: no runs yet` | first run never done | `phantom autoevolve --once` |
| `⚠ autoevolve/schedule: not scheduled` | schedule not installed | `phantom autoevolve schedule install` |
| `⚠ Tailscale: not in PATH` | Tailscale not installed | `winget install tailscale.tailscale` |
| `⚠ Tailscale: not connected` | not logged in | `tailscale up` from elevated PowerShell |
| `⚠ crash logs: N recorded` | a recent agent run crashed | `phantom debug last` to read the latest |
| `⚠ identity: local-only` | expected (broker not deployed) | nothing to fix — this is normal |
| `⚠ events.jsonl: 0 bytes` | expected on first run | nothing to fix — this is normal |
| `✗ [permissions]: parse error` | syntax in `agents.toml [permissions]` block | check the DSL in docs/PERMISSIONS.md |

### Other PowerShell-level failures

| Symptom | Fix |
|---|---|
| `cargo install` fails on `link.exe` not found | Install MSVC Build Tools: `winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| `phantom: command not found` after install | Reopen PowerShell — Rustup adds `~/.cargo/bin` only after restart |
| Defender Firewall blocks port 7878 | Add inbound rule: `New-NetFirewallRule -DisplayName "phantom serve" -Direction Inbound -LocalPort 7878 -Protocol TCP -Action Allow` |
| `Scheduled Task` won't start | Task Scheduler GUI → right-click task → Run; check "Last Run Result" |
| `phantom autoevolve schedule install` says "macOS + Windows only" | This is the message you WANT — check `Get-ScheduledTask -TaskName "*phantom*"` |
| Tailscale GUI shows "Logged out" | Open Tailscale tray icon → Log In; or `tailscale up` from elevated PowerShell |
| `git clone` fails with "Unable to find remote helper for 'https'" | `winget install Git.Git` — must be the official Git for Windows, not WSL |
| `phantom doctor` output is garbled (ANSI codes) | PowerShell 5.x doesn't render ANSI — upgrade to PowerShell 7 (`winget install Microsoft.PowerShell`) |

---

## Performance baseline (Z13 Flow, i9-13900H, 32 GB RAM, Win 11 24H2)

| Operation | Time |
|---|---|
| Fresh `cargo install --path .` (first build) | ~3-4 min |
| Incremental rebuild after 1-file edit | 3-6 s |
| Cross-compiled .exe download (from a Mac via Tailscale 100 Mb/s) | ~3 s for 33 MB |
| `phantom doctor` cold | ~1 s |
| `phantom selftest --p0-only` | ~4 s |
| HTTP `/api/projects` cold | < 60 ms |

---

## Companion: `scripts/setup-z13.ps1`

For a hands-off bootstrap of a fresh Win 11 box as a cluster node,
the repo ships `scripts/setup-z13.ps1`. It:

1. Verifies Tailscale is connected
2. Pulls phantom.exe (from a local path or HTTP URL)
3. Writes a sensible `agents.toml` with peers preconfigured
4. Clones all 6 ecosystem repos
5. (Optional) installs streamlit for Data-Analysis demo
6. Schedules autoevolve hourly
7. Starts phantom serve in the background

Usage:
```powershell
cd phantom-mesh\scripts
Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass -Force
.\setup-z13.ps1 -PhantomBinarySource C:\path\to\phantom.exe -NodeName mywin
```

See the script's docstring for all params.
