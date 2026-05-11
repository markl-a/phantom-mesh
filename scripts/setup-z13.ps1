# setup-z13.ps1 — one-shot bootstrap for the Z13 (Win 11) cluster hub.
#
# Run this from PowerShell on the Z13 (RDP / direct keyboard, doesn't matter).
# Idempotent: safe to re-run; it skips work that's already done.
#
# What this gets you:
#   - Tailscale running (interactive `tailscale up` if needed)
#   - phantom binary at ~\AppData\Local\Programs\phantom-mesh\phantom.exe
#   - 6 pinned repos cloned to %USERPROFILE%\Documents\GitHub\
#   - agents.toml at ~\.phantom-mesh\agents.toml with cluster peers wired
#   - Windows Scheduled Task running `phantom autoevolve --once` hourly
#   - phantom serve started as a Windows service (manual mode for now)
#
# Where to get phantom.exe:
#   The cross-compiled Windows binary is built on the Mac side via:
#     cd <phantom-mesh>/core
#     cargo build --release --target x86_64-pc-windows-gnu --bin phantom
#   resulting binary lives at:
#     dist/phantom-x86_64-pc-windows-gnu.exe        (~33 MB)
#   Three ways to get it to the Z13:
#     A. SCP from Mac:  scp dist/phantom-x86_64-pc-windows-gnu.exe \
#                            <user>@<z13-tailnet-ip>:Downloads/phantom.exe
#     B. SMB share:     copy via Finder ⇄ File Explorer
#     C. -PhantomBinaryUrl flag below: pass a URL the script can curl from
#   Once present anywhere on Z13, point this script at it via the
#   $PhantomBinarySource parameter (path or URL).
#
# What this does NOT do:
#   - Install Rust (only needed if you build from source — we use prebuilt binary)
#   - Install Python / Streamlit (only needed if Z13 hosts Data-Analysis demo;
#     tweak $InstallStreamlit at top to enable)
#   - Install Docker (phantom-secops's full lab needs it; demo-mock doesn't)
#
# After this completes successfully, from the Mac (or any device on Tailscale):
#   curl http://<z13-tailscale-ip>:7879/projects   # → 200 OK with dashboard

[CmdletBinding()]
param(
    # Where to find phantom.exe. Accepts:
    #   - a local path (e.g. C:\Users\me\Downloads\phantom.exe)
    #   - an HTTP(S) URL (e.g. http://mac.tailnet:8080/phantom.exe)
    # Empty string → script just verifies an existing install or bails.
    [string]$PhantomBinarySource = "",
    [string]$ClusterSecret    = "phantom-cluster-2026",
    [string]$NodeName         = "yoyogood",
    [int]$ServePort           = 7879,
    [bool]$InstallStreamlit   = $true,   # set $false if Z13 won't host Data-Analysis
    [bool]$CloneAllRepos      = $true,   # set $false to skip the slow repo clone
    [bool]$ScheduleAutoevolve = $true
)

$ErrorActionPreference = "Continue"
$step = 0
function Step($msg) { $script:step++; Write-Host "`n[$script:step] $msg" -ForegroundColor Cyan }
function OK($msg)   { Write-Host "    ✓ $msg" -ForegroundColor Green }
function Warn($msg) { Write-Host "    ⚠ $msg" -ForegroundColor Yellow }
function Fail($msg) { Write-Host "    ✗ $msg" -ForegroundColor Red }

Write-Host "━━━ phantom-mesh Z13 cluster hub bootstrap ━━━" -ForegroundColor Magenta
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

# ── 2. Phantom binary ────────────────────────────────────────────────────────
Step "phantom binary — install if missing"
$phantomDir  = Join-Path $env:LOCALAPPDATA "Programs\phantom-mesh"
$phantomExe  = Join-Path $phantomDir "phantom.exe"
$phantomCmd  = Get-Command phantom -ErrorAction SilentlyContinue
if ($phantomCmd) {
    OK "phantom on PATH at $($phantomCmd.Source)"
    phantom --version
} elseif (Test-Path $phantomExe) {
    OK "phantom found at $phantomExe (not on PATH yet)"
    & $phantomExe --version
    Warn "Add $phantomDir to PATH manually OR re-run after restart"
} else {
    if (-not $PhantomBinarySource) {
        Fail "phantom not found and no -PhantomBinarySource provided."
        Write-Host "    Get the cross-compiled binary from your Mac's dist/ folder:"
        Write-Host "      dist/phantom-x86_64-pc-windows-gnu.exe   (~33 MB)"
        Write-Host "    via SCP / SMB / OneDrive, then re-run with:"
        Write-Host "      .\setup-z13.ps1 -PhantomBinarySource C:\path\to\phantom.exe"
        Write-Host "    or pass an HTTP URL:"
        Write-Host "      .\setup-z13.ps1 -PhantomBinarySource http://mac.tailnet:8080/phantom.exe"
        exit 2
    }
    New-Item -ItemType Directory -Force -Path $phantomDir | Out-Null
    if ($PhantomBinarySource -match "^https?://") {
        Write-Host "    downloading phantom.exe from $PhantomBinarySource …"
        Invoke-WebRequest -Uri $PhantomBinarySource -OutFile $phantomExe
    } else {
        Write-Host "    copying phantom.exe from $PhantomBinarySource …"
        Copy-Item -Path $PhantomBinarySource -Destination $phantomExe -Force
    }
    OK "installed at $phantomExe"
    # Add to PATH for this session (User-PATH update is left for restart)
    $env:PATH = "$phantomDir;$env:PATH"
    & $phantomExe --version
}

# ── 3. agents.toml ───────────────────────────────────────────────────────────
Step "agents.toml — write cluster config if missing"
$cfgDir  = Join-Path $env:USERPROFILE ".phantom-mesh"
$cfgPath = Join-Path $cfgDir "agents.toml"
New-Item -ItemType Directory -Force -Path $cfgDir | Out-Null
if (Test-Path $cfgPath) {
    OK "agents.toml exists at $cfgPath (NOT overwriting; review by hand if cluster_secret drifts)"
} else {
    @"
# phantom-mesh agents.toml — auto-generated by setup-z13.ps1 on $(Get-Date -Format 'yyyy-MM-dd HH:mm')

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
instructions = "You are phantom on the Z13 (Win 11) cluster hub. Be terse."

[cluster]
node_name      = "$NodeName"
cluster_secret = "$ClusterSecret"
peers = [
  "http://100.106.176.125:7878",   # acer
  "http://100.107.205.98:7878",    # ayaneo
  "http://100.87.93.58:7878",      # mac-coordinator
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
        @{name="phantom-mesh";              url="https://github.com/markl-a/phantom-mesh.git"},
        @{name="phantom-secops";            url="https://github.com/markl-a/phantom-secops.git"},
        @{name="phantom-mobile";            url="https://github.com/markl-a/phantom-mobile.git"},
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

# ── 5. Streamlit (only if Z13 hosts Data-Analysis demo) ──────────────────────
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
    if (-not (Get-Command phantom -ErrorAction SilentlyContinue)) {
        $env:PATH = "$phantomDir;$env:PATH"
    }
    if (Get-Command phantom -ErrorAction SilentlyContinue) {
        # phantom's own subcommand handles Windows specifics. If your phantom
        # version's Windows fallback prints "macOS + Windows only" then this
        # works; otherwise it'll bail and you can fall back to schtasks below.
        phantom autoevolve schedule install --interval 3600 --target check --max-rounds 5 --agent master
        OK "scheduled (or attempted) — check with: phantom autoevolve schedule status"
    } else { Warn "phantom not callable; schedule deferred" }
}

# ── 7. Start phantom serve in background ────────────────────────────────────
Step "phantom serve — start"
$existing = Get-Process -Name phantom -ErrorAction SilentlyContinue
if ($existing) {
    OK "phantom already running (pid $($existing.Id))"
} else {
    Start-Process -FilePath phantom -ArgumentList "serve","--port",$ServePort `
                  -WindowStyle Hidden -PassThru | ForEach-Object { OK "phantom serve started (pid $($_.Id))" }
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
} catch { Warn "/api/projects unreachable (older phantom version?): $_" }

Write-Host "`n━━━ DONE ━━━" -ForegroundColor Magenta
$tsIp = (tailscale ip -4 2>&1 | Select-Object -First 1)
Write-Host "From any device on Tailscale, open:"
Write-Host "  http://${tsIp}:${ServePort}/projects" -ForegroundColor Cyan
Write-Host "`nFrom your Mac, also confirm `phantom run --node $NodeName 'hi'` works."
