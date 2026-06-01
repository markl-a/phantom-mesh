#requires -Version 5.1
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    # OPT-IN: SSH pubkey to install for incoming mac->this-host ssh.
    # Full line, e.g. "ssh-ed25519 AAAA... you@mac.local"
    # If omitted, the SSH/firewall steps are SKIPPED (no auth surface change).
    [string]$AddSshKey = '',

    # Mac coordinator base URL - defaults to the tailnet IP.
    [string]$CoordUrl = 'http://100.64.0.10:7878',

    # Logical node name on the mesh (defaults to $env:COMPUTERNAME).
    [string]$NodeName = $env:COMPUTERNAME,

    # Skip individual installer steps (for re-run / partial recovery).
    [switch]$SkipDevTools,
    [switch]$SkipPhantom,
    [switch]$SkipSsh
)

# -----------------------------------------------------------------------------
# Phantom Mesh - Windows dev-machine onboarding (node-b / node-a / new box)
# -----------------------------------------------------------------------------
#
# What it does (in order, each step idempotent + safe to re-run):
#
#   1. Dev tools  - winget node.js LTS + Google.Antigravity; npm -g claude+codex
#   2. Phantom    - pull binary from $CoordUrl/dist/, pull live agents.toml
#                   from $CoordUrl/onboarding/config (with real provider keys),
#                   register PhantomMeshServe scheduled task ONLOGON HIGHEST,
#                   open firewall for tailnet inbound :7878
#   3. SSH (opt)  - if -AddSshKey given AND running as admin:
#                     enable OpenSSH server, install pubkey, open :22 inbound.
#                   Without -AddSshKey: SSH step skipped (no key smuggling).
#
# Run from REGULAR PowerShell for steps 1+2 (no admin needed for user-scope
# scheduled task + winget). Run from ADMIN PowerShell to additionally do step 3.
#
# Idempotency notes:
#   - winget install detects already-installed packages and exits clean.
#   - npm install -g re-runs are no-ops if version already installed.
#   - schtasks /Create /F overwrites existing task with same name.
#   - SSH pubkey: appends if not already present (no duplicate lines).
# -----------------------------------------------------------------------------

$ErrorActionPreference = 'Stop'

function Write-Section($title) {
    Write-Host ""
    Write-Host "=== $title ===" -ForegroundColor Cyan
}
function Write-OK($msg)   { Write-Host "  [OK] $msg" -ForegroundColor Green }
function Write-Warn($msg) { Write-Host "  !  $msg" -ForegroundColor Yellow }
function Write-Err($msg)  { Write-Host "  [X] $msg" -ForegroundColor Red }

$IsAdmin = ([Security.Principal.WindowsPrincipal] `
    [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltinRole]::Administrator)

Write-Host ""
Write-Host "==============================================================="
Write-Host "  Phantom Mesh - Windows dev-machine onboarding"
Write-Host "==============================================================="
Write-Host "  Coordinator:  $CoordUrl"
Write-Host "  Node name:    $NodeName"
Write-Host "  Admin mode:   $IsAdmin"
Write-Host "  AddSshKey:    $(if ($AddSshKey) { 'YES (' + $AddSshKey.Substring(0,30) + '...)' } else { 'NO (SSH step will be skipped)' })"
Write-Host "==============================================================="

# -- 1. Dev tools ------------------------------------------------------------
if (-not $SkipDevTools) {
    Write-Section "1/3 Dev tools (node + claude + codex + antigravity)"

    # winget - node + antigravity
    foreach ($pkg in @('OpenJS.NodeJS.LTS','Google.Antigravity')) {
        Write-Host "  installing $pkg ..."
        $rc = & winget install --id $pkg --silent --accept-source-agreements --accept-package-agreements 2>&1
        if ($LASTEXITCODE -eq 0 -or $rc -match 'No applicable update') {
            Write-OK $pkg
        } else {
            Write-Warn "$pkg may already be installed (winget exit $LASTEXITCODE)"
        }
    }

    # refresh PATH for current session so npm becomes visible
    $env:Path = [System.Environment]::GetEnvironmentVariable('Path','Machine') + ';' + `
                [System.Environment]::GetEnvironmentVariable('Path','User')

    # npm globals - claude + codex
    $npm = (Get-Command npm -ErrorAction SilentlyContinue)
    if (-not $npm) {
        Write-Err "npm not on PATH after node install - open a NEW PowerShell and re-run with -SkipPhantom -SkipSsh, or run npm manually."
    } else {
        Write-Host "  npm install -g @anthropic-ai/claude-code @openai/codex ..."
        & npm install -g '@anthropic-ai/claude-code' '@openai/codex' 2>&1 | Out-Null
        if ($LASTEXITCODE -eq 0) {
            Write-OK "claude + codex"
        } else {
            Write-Err "npm install failed (exit $LASTEXITCODE)"
        }
    }

    # agy - Google Antigravity standalone CLI (separate from the IDE shim)
    Write-Host "  installing agy (Google Antigravity CLI) ..."
    try {
        Invoke-Expression (Invoke-RestMethod -Uri 'https://antigravity.google/cli/install.ps1' -TimeoutSec 30)
        Write-OK "agy -> %LOCALAPPDATA%\agy\bin\agy.exe (added to User PATH)"
    } catch {
        Write-Warn "agy install failed: $_"
    }
}

# -- 2. Phantom mesh ---------------------------------------------------------
if (-not $SkipPhantom) {
    Write-Section "2/3 Phantom mesh (binary + agents.toml + schtasks)"

    $CFG_DIR     = Join-Path $env:USERPROFILE '.phantom-mesh'
    $INSTALL_DIR = Join-Path $CFG_DIR         'bin'
    $LOG_DIR     = Join-Path $CFG_DIR         'logs'
    $BIN         = Join-Path $INSTALL_DIR     'phantom.exe'
    $CFG         = Join-Path $CFG_DIR         'agents.toml'

    New-Item -ItemType Directory -Force $INSTALL_DIR | Out-Null
    New-Item -ItemType Directory -Force $LOG_DIR     | Out-Null

    # Stop running phantom so binary can be replaced
    Get-Process phantom -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 1

    # Download binary
    $exeUrl = "$CoordUrl/dist/phantom-x86_64-pc-windows.exe"
    Write-Host "  downloading $exeUrl ..."
    Invoke-WebRequest -Uri $exeUrl -OutFile $BIN -UseBasicParsing -TimeoutSec 30
    Unblock-File -Path $BIN -ErrorAction SilentlyContinue
    $size = [math]::Round((Get-Item $BIN).Length/1MB, 1)
    Write-OK "phantom.exe ($size MB) -> $BIN"

    # Backup existing agents.toml then pull fresh from /onboarding/config
    if (Test-Path $CFG) {
        $ts = Get-Date -Format 'yyyyMMddHHmmss'
        Copy-Item $CFG "$CFG.bk-$ts"
        Write-Host "  backed up existing agents.toml -> $CFG.bk-$ts"
    }
    Write-Host "  fetching /onboarding/config from coordinator ..."
    $token = (Invoke-RestMethod -Uri "$CoordUrl/onboarding/token" -TimeoutSec 5).token
    # NB: '&' in URL must be escaped past cmd, but PowerShell handles fine when interpolated
    $configUrl = "$CoordUrl/onboarding/config?token=$token" + "&node_name=$NodeName"
    $config    = Invoke-RestMethod -Uri $configUrl -TimeoutSec 10
    $config | Out-File -FilePath $CFG -Encoding utf8 -NoNewline
    $cfgSize = (Get-Item $CFG).Length
    Write-OK "agents.toml ($cfgSize bytes, real provider keys baked in)"

    # Firewall (tailnet only) - admin needed; skip cleanly without
    if ($IsAdmin) {
        try {
            Get-NetFirewallRule -DisplayName 'PhantomMesh-Inbound' -ErrorAction SilentlyContinue |
                Remove-NetFirewallRule -ErrorAction SilentlyContinue
            New-NetFirewallRule -DisplayName 'PhantomMesh-Inbound' `
                -Direction Inbound -Action Allow -Protocol TCP `
                -LocalPort 7878 -RemoteAddress '100.64.0.0/10' `
                -Profile Any -ErrorAction Stop | Out-Null
            Write-OK "firewall :7878 (tailnet only)"
        } catch {
            Write-Warn "firewall rule failed: $_"
        }
    } else {
        Write-Warn "skip firewall (not admin) - re-run from admin to enable inbound :7878"
    }

    # Scheduled task - ONLOGON, HIGHEST priv
    Write-Host "  registering scheduled task 'PhantomMeshServe' ..."
    $trCmd = "cmd /c `"$BIN`" serve >> `"$LOG_DIR\serve.out`" 2>&1"
    & schtasks /Create /F /SC ONLOGON /RL HIGHEST `
        /TN 'PhantomMeshServe' `
        /TR $trCmd 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
        Write-OK "schtasks created"
        & schtasks /Run /TN 'PhantomMeshServe' 2>&1 | Out-Null
        Start-Sleep -Seconds 8
        try {
            $st = Invoke-RestMethod -Uri 'http://127.0.0.1:7878/api/status' -TimeoutSec 3
            Write-OK "phantom serve responding: node_name=$($st.cluster.node_name) peers=$($st.cluster.peers) providers=$($st.providers -join ',')"
        } catch {
            Write-Warn "healthz check failed: $_"
            Write-Host "    tail of serve log:"
            Get-Content "$LOG_DIR\serve.out" -Tail 5 -ErrorAction SilentlyContinue | ForEach-Object { Write-Host "      $_" }
        }
    } else {
        Write-Err "schtasks create failed (exit $LASTEXITCODE)"
    }
}

# -- 3. SSH (opt-in, admin-only) ---------------------------------------------
if (-not $SkipSsh -and $AddSshKey) {
    Write-Section "3/3 SSH server + pubkey install"
    if (-not $IsAdmin) {
        Write-Err "SSH step requires admin - re-run from admin PowerShell"
    } else {
        # Enable OpenSSH Server (Windows feature)
        $cap = Get-WindowsCapability -Online -Name 'OpenSSH.Server*' -ErrorAction SilentlyContinue |
               Select-Object -First 1
        if ($cap -and $cap.State -ne 'Installed') {
            Write-Host "  installing OpenSSH.Server feature ..."
            Add-WindowsCapability -Online -Name $cap.Name | Out-Null
        }
        Set-Service -Name sshd -StartupType Automatic
        Start-Service sshd
        Write-OK "OpenSSH Server running"

        # Install pubkey - administrators_authorized_keys for admin users,
        # else %USERPROFILE%\.ssh\authorized_keys.
        $isAdminUser = ([Security.Principal.WindowsPrincipal] `
            [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
                [Security.Principal.WindowsBuiltinRole]::Administrator)
        if ($isAdminUser) {
            $authKeysPath = "$env:ProgramData\ssh\administrators_authorized_keys"
        } else {
            $authKeysPath = "$env:USERPROFILE\.ssh\authorized_keys"
            New-Item -ItemType Directory -Force "$env:USERPROFILE\.ssh" | Out-Null
        }
        $existing = if (Test-Path $authKeysPath) { Get-Content $authKeysPath -Raw } else { '' }
        if ($existing -notmatch [regex]::Escape($AddSshKey.Substring(0,40))) {
            Add-Content -Path $authKeysPath -Value $AddSshKey
            Write-OK "pubkey added to $authKeysPath"
        } else {
            Write-OK "pubkey already present in $authKeysPath"
        }
        # Fix permissions on administrators_authorized_keys (sshd requires)
        if ($isAdminUser) {
            icacls $authKeysPath /inheritance:r 2>&1 | Out-Null
            icacls $authKeysPath /grant 'Administrators:F' 'SYSTEM:F' 2>&1 | Out-Null
        }

        # Firewall :22
        try {
            Get-NetFirewallRule -DisplayName 'PhantomMesh-SSH' -ErrorAction SilentlyContinue |
                Remove-NetFirewallRule -ErrorAction SilentlyContinue
            New-NetFirewallRule -DisplayName 'PhantomMesh-SSH' `
                -Direction Inbound -Action Allow -Protocol TCP `
                -LocalPort 22 -RemoteAddress '100.64.0.0/10' `
                -Profile Any -ErrorAction Stop | Out-Null
            Write-OK "firewall :22 (tailnet only)"
        } catch {
            Write-Warn "firewall :22 failed: $_"
        }
    }
} elseif (-not $SkipSsh) {
    Write-Section "3/3 SSH - SKIPPED (no -AddSshKey provided)"
    Write-Host "  Re-run with -AddSshKey '<pubkey line>' from admin PowerShell to enable."
}

# -- Final summary -----------------------------------------------------------
Write-Section "Summary"
$tsIp = (Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue |
        Where-Object { $_.IPAddress -like '100.*' } |
        Select-Object -First 1 -ExpandProperty IPAddress)
Write-Host "  Tailscale IP: $tsIp"
Write-Host "  Phantom dir:  $env:USERPROFILE\.phantom-mesh\"
Write-Host "  Log:          $env:USERPROFILE\.phantom-mesh\logs\serve.out"
Write-Host "  Manual test:  curl http://127.0.0.1:7878/api/status"
Write-Host "  From mac:     curl http://${tsIp}:7878/api/status"
Write-Host ""
Write-Host "  Dev tool versions:"
foreach ($cmd in @('node','npm','claude','codex','antigravity')) {
    $found = Get-Command $cmd -ErrorAction SilentlyContinue
    if ($found) {
        try {
            $v = & $cmd --version 2>&1 | Select-Object -First 1
            Write-Host "    $cmd : $v"
        } catch {
            Write-Host "    $cmd : (installed but --version failed)"
        }
    } else {
        Write-Host "    $cmd : NOT FOUND on PATH (may need new shell)"
    }
}
Write-Host ""
