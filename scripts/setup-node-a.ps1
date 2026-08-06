# setup-node-a.ps1 — one-shot bootstrap for the node-a (Win 11) cluster hub.
#
# Run this from PowerShell on node-a (RDP / direct keyboard, doesn't matter).
# Idempotent: safe to re-run; it skips work that's already done.
#
# What this gets you:
#   - Tailscale running (interactive `tailscale up` if needed)
#   - spectyn binary at ~\AppData\Local\Programs\spectyn-mesh\spectyn.exe
#   - 6 pinned repos cloned to %USERPROFILE%\Documents\GitHub\
#   - agents.toml at ~\.spectyn-mesh\agents.toml with cluster peers wired
#   - Windows Scheduled Task running `spectyn autoevolve --once` hourly
#   - spectyn serve started as a Windows service (manual mode for now)
#
# Where to get spectyn.exe:
#   The cross-compiled Windows binary is built on the Mac side via:
#     cd <spectyn-mesh>/core
#     cargo build --release --target x86_64-pc-windows-gnu --bin spectyn
#   resulting binary lives at:
#     dist/spectyn-x86_64-pc-windows-gnu.exe        (~33 MB)
#   Three ways to get it to node-a:
#     A. SCP from Mac:  scp dist/spectyn-x86_64-pc-windows-gnu.exe \
#                            <user>@<node-a-tailnet-ip>:Downloads/spectyn.exe
#     B. SMB share:     copy via Finder ⇄ File Explorer
#     C. -SpectynBinaryUrl flag below: pass a URL the script can curl from
#   Once present anywhere on node-a, point this script at it via the
#   $SpectynBinarySource parameter (path or URL).
#
# What this does NOT do:
#   - Install Rust (only needed if you build from source — we use prebuilt binary)
#   - Install Python / Streamlit (only needed if node-a hosts Data-Analysis demo;
#     tweak $InstallStreamlit at top to enable)
#   - Install Docker (spectyn-secops's full lab needs it; demo-mock doesn't)
#
# After this completes successfully, from the Mac (or any device on Tailscale):
#   curl http://<node-a-tailscale-ip>:7879/projects   # → 200 OK with dashboard

[CmdletBinding()]
param(
    # Where to find spectyn.exe. Accepts:
    #   - a local path (e.g. C:\Users\me\Downloads\spectyn.exe)
    #   - an HTTP(S) URL (e.g. http://mac.tailnet:8080/spectyn.exe)
    # Empty string → script just verifies an existing install or bails.
    [string]$SpectynBinarySource = "",
    [string]$ClusterSecret    = "<your-cluster-secret>",
    [string]$NodeName         = "node-b",
    [int]$ServePort           = 7879,
    [bool]$InstallStreamlit   = $true,   # set $false if node-a won't host Data-Analysis
    [bool]$CloneAllRepos      = $true,   # set $false to skip the slow repo clone
    [bool]$ScheduleAutoevolve = $true
)

$ErrorActionPreference = "Continue"
$step = 0
function Step($msg) { $script:step++; Write-Host "`n[$script:step] $msg" -ForegroundColor Cyan }
function OK($msg)   { Write-Host "    ✓ $msg" -ForegroundColor Green }
function Warn($msg) { Write-Host "    ⚠ $msg" -ForegroundColor Yellow }
function Fail($msg) { Write-Host "    ✗ $msg" -ForegroundColor Red }

Write-Host "━━━ spectyn-mesh node-a cluster hub bootstrap ━━━" -ForegroundColor Magenta
Write-Host "    node: $NodeName · port: $ServePort"

# ── 1. Tailscale ─────────────────────────────────────────────────────────────
Step "Tailscale — verify installed + connected"
if (Get-Command tailscale -ErrorAction SilentlyContinue) {
    OK "tailscale binary found"
    $status = (tailscale status 2>&1) -join "`n"
    if ($status -match "stopped|Logged out") {
        Warn "Tailscale not connected. Running `tailscale up`…"
        tailscale up
    } elseif ($status -match "^\d") {
        OK "tailscale connected"
    }
} else {
    Fail "tailscale not installed. Get it from https://tailscale.com/download/windows then re-run this script."
    exit 2
}

# ── 2. Spectyn binary ────────────────────────────────────────────────────────
Step "spectyn binary — install if missing"
$spectynDir  = Join-Path $env:LOCALAPPDATA "Programs\spectyn-mesh"
$spectynExe  = Join-Path $spectynDir "spectyn.exe"
$spectynCmd  = Get-Command spectyn -ErrorAction SilentlyContinue
if ($spectynCmd) {
    OK "spectyn on PATH at $($spectynCmd.Source)"
    spectyn --version
} elseif (Test-Path $spectynExe) {
    OK "spectyn found at $spectynExe (not on PATH yet)"
    & $spectynExe --version
    Warn "Add $spectynDir to PATH manually OR re-run after restart"
} else {
    if (-not $SpectynBinarySource) {
        Fail "spectyn not found and no -SpectynBinarySource provided."
        Write-Host "    Get the cross-compiled binary from your Mac's dist/ folder:"
        Write-Host "      dist/spectyn-x86_64-pc-windows-gnu.exe   (~33 MB)"
        Write-Host "    via SCP / SMB / OneDrive, then re-run with:"
        Write-Host "      .\setup-node-a.ps1 -SpectynBinarySource C:\path\to\spectyn.exe"
        Write-Host "    or pass an HTTP URL:"
        Write-Host "      .\setup-node-a.ps1 -SpectynBinarySource http://mac.tailnet:8080/spectyn.exe"
        exit 2
    }
    New-Item -ItemType Directory -Force -Path $spectynDir | Out-Null
    if ($SpectynBinarySource -match "^https?://") {
        Write-Host "    downloading spectyn.exe from $SpectynBinarySource …"
        Invoke-WebRequest -Uri $SpectynBinarySource -OutFile $spectynExe
    } else {
        Write-Host "    copying spectyn.exe from $SpectynBinarySource …"
        Copy-Item -Path $SpectynBinarySource -Destination $spectynExe -Force
    }
    OK "installed at $spectynExe"
    # Add to PATH for this session (User-PATH update is left for restart)
    $env:PATH = "$spectynDir;$env:PATH"
    & $spectynExe --version
}

# ── 3. agents.toml ───────────────────────────────────────────────────────────
Step "agents.toml — write cluster config if missing"
$cfgDir  = Join-Path $env:USERPROFILE ".spectyn-mesh"
$cfgPath = Join-Path $cfgDir "agents.toml"
New-Item -ItemType Directory -Force -Path $cfgDir | Out-Null
if (Test-Path $cfgPath) {
    OK "agents.toml exists at $cfgPath (NOT overwriting; review by hand if cluster_secret drifts)"
} else {
    @"
# spectyn-mesh agents.toml — auto-generated by setup-node-a.ps1 on $(Get-Date -Format 'yyyy-MM-dd HH:mm')

[core]
host = "0.0.0.0"
port = $ServePort
hub_api_key = ""

[providers.anthropic]
type        = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[providers.groq]
type        = "groq"
api_key_env = "GROQ_API_KEY"

[agent.master]
provider = "anthropic"
tools    = ["shell", "file_read", "file_edit", "content_search",
            "git_status", "git_diff", "git_commit", "task"]
instructions = "You are spectyn on node-a (Win 11) cluster hub. Be terse."

[cluster]
node_name      = "$NodeName"
cluster_secret = "$ClusterSecret"
peers = [
  "http://100.64.0.13:7878",   # node-b
  "http://100.64.0.12:7878",   # node-a
  "http://100.64.0.10:7878",   # mac-coordinator
]
"@ | Set-Content -Path $cfgPath -Encoding UTF8
    OK "wrote $cfgPath"
}

# ── 4. Clone the 6 pinned repos ──────────────────────────────────────────────
Step "Clone 6 pinned repos to %USERPROFILE%\Documents\GitHub\"
if ($CloneAllRepos) {
    if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
        Fail "git not on PATH. Install via 'winget install Git.Git' then re-run."
        exit 2
    }
    $base = Join-Path $env:USERPROFILE "Documents\GitHub"
    New-Item -ItemType Directory -Force -Path $base | Out-Null
    $repos = @(
        @{name="spectyn-mesh";              url="https://github.com/markl-a/spectyn-mesh.git"},
        @{name="spectyn-secops";            url="https://github.com/markl-a/spectyn-secops.git"},
        @{name="spectyn-mobile";            url="https://github.com/markl-a/spectyn-mobile.git"},
        @{name="Data-Analysis-with-Agents"; url="https://github.com/markl-a/Data-Analysis-with-Agents.git"},
        @{name="Automation_with_Agent";     url="https://github.com/markl-a/Automation_with_Agent.git"},
        @{name="My-AI-Learning-Notes";      url="https://github.com/markl-a/My-AI-Learning-Notes.git"}
    )
    foreach ($r in $repos) {
        $target = Join-Path $base $r.name
        if (Test-Path $target) {
            OK "$($r.name) already cloned"
        } else {
            Write-Host "    cloning $($r.name) …"
            git clone --depth 1 $r.url $target 2>&1 | Out-Null
            if ($?) { OK "cloned $($r.name)" } else { Warn "clone of $($r.name) failed (private repo? check gh auth)" }
        }
    }
} else {
    Warn "CloneAllRepos disabled; skipping"
}

# ── 5. Streamlit (only if node-a hosts Data-Analysis demo) ──────────────────────
Step "Streamlit (optional — for Data-Analysis demo)"
if ($InstallStreamlit) {
    if (Get-Command python -ErrorAction SilentlyContinue) {
        $sl = Get-Command streamlit -ErrorAction SilentlyContinue
        if ($sl) { OK "streamlit already at $($sl.Source)" }
        else {
            Write-Host "    pip install streamlit pandas scikit-learn …"
            python -m pip install --user --quiet streamlit pandas scikit-learn
            if ($?) { OK "streamlit installed" } else { Warn "pip install failed; install python first" }
        }
    } else { Warn "python not on PATH — streamlit skipped" }
}

# ── 6. Schedule autoevolve hourly via Windows Scheduled Task ────────────────
Step "autoevolve — Windows Scheduled Task"
if ($ScheduleAutoevolve) {
    if (-not (Get-Command spectyn -ErrorAction SilentlyContinue)) {
        $env:PATH = "$spectynDir;$env:PATH"
    }
    if (Get-Command spectyn -ErrorAction SilentlyContinue) {
        # spectyn's own subcommand handles Windows specifics. If your spectyn
        # version's Windows fallback prints "macOS + Windows only" then this
        # works; otherwise it'll bail and you can fall back to schtasks below.
        spectyn autoevolve schedule install --interval 3600 --target check --max-rounds 5 --agent master
        OK "scheduled (or attempted) — check with: spectyn autoevolve schedule status"
    } else { Warn "spectyn not callable; schedule deferred" }
}

# ── 7. Start spectyn serve in background ────────────────────────────────────
Step "spectyn serve — start"
$existing = Get-Process -Name spectyn -ErrorAction SilentlyContinue
if ($existing) {
    OK "spectyn already running (pid $($existing.Id))"
} else {
    Start-Process -FilePath spectyn -ArgumentList "serve","--port",$ServePort `
                  -WindowStyle Hidden -PassThru | ForEach-Object { OK "spectyn serve started (pid $($_.Id))" }
    Start-Sleep 2
}

# ── 8. Verify ────────────────────────────────────────────────────────────────
Step "Verify"
try {
    $h = Invoke-RestMethod "http://127.0.0.1:$ServePort/healthz" -TimeoutSec 5
    OK "/healthz → $h"
} catch { Fail "/healthz unreachable: $_" }
try {
    $p = Invoke-RestMethod "http://127.0.0.1:$ServePort/api/projects" -TimeoutSec 5
    OK "/api/projects → $($p.Count) entries"
} catch { Warn "/api/projects unreachable (older spectyn version?): $_" }

Write-Host "`n━━━ DONE ━━━" -ForegroundColor Magenta
$tsIp = (tailscale ip -4 2>&1 | Select-Object -First 1)
Write-Host "From any device on Tailscale, open:"
Write-Host "  http://${tsIp}:${ServePort}/projects" -ForegroundColor Cyan
Write-Host "`nFrom your Mac, also confirm `spectyn run --node $NodeName 'hi'` works."
