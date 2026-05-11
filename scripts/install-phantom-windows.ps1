# Phantom Mesh — Windows worker install
#
# One-liner from a regular PowerShell window (no admin required for the
# user-mode Scheduled Task path):
#
#   $env:COORD = "http://100.87.93.58:7878"
#   $env:OPENROUTER_API_KEY = "sk-or-v1-..."   # optional — fill agents.toml later
#   iex (iwr "$env:COORD/scripts/install-phantom-windows.ps1").Content
#
# What it does (~2 min):
#   1. Download phantom.exe from the coordinator into ~/.phantom-mesh/bin/
#      (the same path docs/SESSION-ONBOARDING.md §3.1 expects, and the
#      same path `phantom service install` registers with the Scheduled
#      Task — three sources, one location.)
#   2. Write a minimal agents.toml using OpenRouter (free Llama tier);
#      api_key_env points at OPENROUTER_API_KEY so the secret never
#      lands inside agents.toml itself.
#   3. Open Defender Firewall inbound rule (Tailscale-only) on the
#      configured port — defaults to 7878, override with $env:PORT.
#   4. Register the "PhantomServe" Scheduled Task via the binary's own
#      `phantom service install` (which uses PowerShell's
#      Register-ScheduledTask under the hood, so it works without admin
#      on managed Windows where schtasks /SC ONLOGON is denied).
#   5. Healthz probe on the configured port.
#
# Requirements: Windows 10/11, PowerShell 5+, network access to $COORD.

$ErrorActionPreference = 'Stop'

$COORD     = if ($env:COORD)     { $env:COORD }     else { 'http://100.87.93.58:7878' }
$PORT      = if ($env:PORT)      { $env:PORT }      else { '7878' }
$NODE_NAME = if ($env:NODE_NAME) { $env:NODE_NAME } else { $env:COMPUTERNAME }
$SECRET    = if ($env:SECRET)    { $env:SECRET }    else { 'phantom-cluster-2026' }

# Both the binary itself and the SESSION-ONBOARDING doc agree the install
# location is ~/.phantom-mesh/bin/phantom.exe, not %LOCALAPPDATA%/PhantomMesh.
$CFG_DIR     = Join-Path $env:USERPROFILE '.phantom-mesh'
$INSTALL_DIR = Join-Path $CFG_DIR         'bin'
$BIN         = Join-Path $INSTALL_DIR     'phantom.exe'
$CFG         = Join-Path $CFG_DIR         'agents.toml'
$LOG_DIR     = Join-Path $CFG_DIR         'data'

Write-Host "=== Phantom Mesh Windows install ===" -ForegroundColor Cyan
Write-Host ""
Write-Host "  coordinator: $COORD"
Write-Host "  node name  : $NODE_NAME"
Write-Host "  install to : $INSTALL_DIR"
Write-Host "  config     : $CFG"
Write-Host "  serve port : $PORT"
Write-Host ""

# ── 1. Download binary ───────────────────────────────────────────────────────
Write-Host "[1/5] Downloading phantom.exe ..." -ForegroundColor Cyan
New-Item -ItemType Directory -Force $INSTALL_DIR | Out-Null
New-Item -ItemType Directory -Force $CFG_DIR     | Out-Null
New-Item -ItemType Directory -Force $LOG_DIR     | Out-Null

# Stop a running phantom (if any) before overwriting the binary, otherwise
# Windows refuses to replace the .exe. Best-effort.
Get-Process phantom -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

$exeUrl = "$COORD/dist/phantom-x86_64-pc-windows.exe"
Invoke-WebRequest -Uri $exeUrl -OutFile $BIN -UseBasicParsing
if (-not (Test-Path $BIN)) { throw "Download failed: $exeUrl" }
$size = (Get-Item $BIN).Length
Write-Host "  -> $BIN ($([math]::Round($size/1MB, 1)) MB)" -ForegroundColor Green

# ── 2. agents.toml ───────────────────────────────────────────────────────────
Write-Host "[2/5] Writing agents.toml ..." -ForegroundColor Cyan
$cfgContent = @"
[core]
host = "0.0.0.0"
port = $PORT

[cluster]
node_name      = "$NODE_NAME"
cluster_secret = "$SECRET"
capabilities   = ["build", "test", "shell", "windows"]
peers = ["$COORD"]

# Provider keys read from environment variables — never written here.
# Set these before running the binary:
#   PowerShell:  [Environment]::SetEnvironmentVariable('OPENROUTER_API_KEY', 'sk-or-v1-...', 'User')
[providers.openrouter]
type          = "openrouter"
base_url      = "https://openrouter.ai/api/v1"
api_key_env   = "OPENROUTER_API_KEY"
default_model = "meta-llama/llama-3.3-70b-instruct"

[agent.master]
provider = "openrouter"
model    = "meta-llama/llama-3.3-70b-instruct"
tools    = ["shell", "file_read", "file_write", "content_search", "git_status"]
"@
Set-Content -Path $CFG -Value $cfgContent -Encoding UTF8
Write-Host "  -> $CFG" -ForegroundColor Green

# ── 3. Defender firewall rule (Tailscale subnet only) ────────────────────────
Write-Host "[3/5] Opening firewall for TCP $PORT (Tailscale 100.64.0.0/10) ..." -ForegroundColor Cyan
try {
    Get-NetFirewallRule -DisplayName 'PhantomMesh-Inbound' -ErrorAction SilentlyContinue |
        Remove-NetFirewallRule -ErrorAction SilentlyContinue
    New-NetFirewallRule `
        -DisplayName 'PhantomMesh-Inbound' `
        -Direction Inbound `
        -Action Allow `
        -Protocol TCP `
        -LocalPort $PORT `
        -RemoteAddress '100.64.0.0/10' `
        -Profile Any `
        -ErrorAction Stop | Out-Null
    Write-Host "  -> rule installed" -ForegroundColor Green
} catch {
    Write-Host "  ! firewall step skipped: $_" -ForegroundColor Yellow
    Write-Host "    (re-run from an admin PowerShell to enable inbound :$PORT)" -ForegroundColor Yellow
    Write-Host "    `phantom service install` from admin shell also installs the rule." -ForegroundColor Yellow
}

# ── 4. Register Scheduled Task via the binary itself ─────────────────────────
Write-Host "[4/5] Registering Scheduled Task 'PhantomServe' ..." -ForegroundColor Cyan
& $BIN service install
if ($LASTEXITCODE -ne 0) {
    Write-Host "  ! phantom service install returned $LASTEXITCODE - check output above" -ForegroundColor Yellow
}

# ── 5. Verify ────────────────────────────────────────────────────────────────
Write-Host "[5/5] Verifying ..." -ForegroundColor Cyan
Start-Sleep -Seconds 4
try {
    $resp = Invoke-WebRequest -Uri "http://127.0.0.1:$PORT/healthz" -UseBasicParsing -TimeoutSec 3
    if ($resp.StatusCode -eq 200) {
        Write-Host "  OK phantom serve responding on :$PORT" -ForegroundColor Green
    } else {
        Write-Host "  ! healthz returned $($resp.StatusCode)" -ForegroundColor Yellow
    }
} catch {
    Write-Host "  ! healthz unreachable yet - may need a logon to start" -ForegroundColor Yellow
    Write-Host "    Run manually now:  & '$BIN' serve" -ForegroundColor Yellow
}

Write-Host ""
Write-Host "==================================================================="
Write-Host "    Use phantom on this Windows box - three ways:" -ForegroundColor Cyan
Write-Host "==================================================================="
Write-Host ""
Write-Host "  1) Interactive TUI (blocks the terminal):"
Write-Host "     & '$BIN'"
Write-Host ""
Write-Host "  2) Browser:"
$tsIp = (Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Where-Object { $_.IPAddress -like '100.*' } |
        Select-Object -First 1 -ExpandProperty IPAddress)
if ($tsIp) {
    Write-Host "     http://${tsIp}:${PORT}/   (this device)"
}
Write-Host "     $COORD/m                  (Mac coordinator's mobile UI)"
Write-Host ""
Write-Host "  3) Cluster worker - already wired up via Scheduled Task PhantomServe."
if ($tsIp) {
    Write-Host "     From Mac: subagent({ node: '${tsIp}:${PORT}', ... })"
}
Write-Host ""
if (-not $env:OPENROUTER_API_KEY) {
    Write-Host "  ! OPENROUTER_API_KEY not set - phantom will have no LLM backend." -ForegroundColor Yellow
    Write-Host "    Set it permanently with:" -ForegroundColor Yellow
    Write-Host "      [Environment]::SetEnvironmentVariable('OPENROUTER_API_KEY','sk-or-v1-...','User')" -ForegroundColor Yellow
}
Write-Host "==================================================================="
Write-Host ""
Write-Host "Useful follow-ups:" -ForegroundColor Cyan
Write-Host "  & '$BIN' doctor"
Write-Host "  & '$BIN' service status"
Write-Host "  & '$BIN' --version"
