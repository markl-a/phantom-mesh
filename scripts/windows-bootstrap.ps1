#requires -RunAsAdministrator
# ─────────────────────────────────────────────────────────────────────────────
# Phantom Mesh — Windows worker bootstrap (Z13 / Acer / AYANEO)
# ─────────────────────────────────────────────────────────────────────────────
#
# Run this on EACH of the three Windows machines (need admin PowerShell).
#
# What it does:
#   1. Enable + start OpenSSH server (sshd service)
#   2. Add Mac's public key to authorized_keys (so Mac can SSH in)
#   3. Open Windows Defender Firewall for inbound TCP 22 + 7878 from Tailscale
#   4. Print info for Mac side: user@host, Tailscale IP, phantom-mesh status
#
# Usage:
#   1. 開「以系統管理員身分執行」的 PowerShell（cmd 不行）
#   2. 複製整個檔案內容貼上去 Enter
#   3. 把最後印出的「報給 Mac」那段 copy 給對面
#
# To run a remote one-liner instead:
#   irm https://raw.githubusercontent.com/markl-a/phantom-mesh/main/scripts/windows-bootstrap.ps1 | iex
# ─────────────────────────────────────────────────────────────────────────────

$ErrorActionPreference = 'Stop'
$MAC_PUBKEY = 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINEWSlyia5LO8HzKHMmHtmsSG0YZ235MOqVend+JrgSF marklight@MarkdeMacBook-Air.local'
$TAILSCALE_CIDR = '100.64.0.0/10'  # Tailscale 4-IPv4 subnet

Write-Host "═══ Phantom Mesh Windows bootstrap ═══" -ForegroundColor Cyan
Write-Host ""

# ── 1. Install + start OpenSSH server ────────────────────────────────────────
Write-Host "[1/4] OpenSSH server" -ForegroundColor Yellow
try {
    $cap = Get-WindowsCapability -Online -Name 'OpenSSH.Server*' -ErrorAction Stop
    if ($cap.State -ne 'Installed') {
        Write-Host "  installing..."
        Add-WindowsCapability -Online -Name $cap.Name | Out-Null
    }
    Start-Service -Name sshd -ErrorAction SilentlyContinue
    Set-Service -Name sshd -StartupType Automatic
    $svc = Get-Service sshd
    Write-Host "  sshd: $($svc.Status) (StartType=Automatic)"
} catch {
    Write-Host "  ⚠ OpenSSH install failed: $_" -ForegroundColor Red
}

# ── 2. Add Mac's pubkey to authorized_keys ───────────────────────────────────
Write-Host ""
Write-Host "[2/4] authorized_keys" -ForegroundColor Yellow

# Determine which file Windows OpenSSH uses
# Admin user (Administrators group) → C:\ProgramData\ssh\administrators_authorized_keys
# Regular user → $env:USERPROFILE\.ssh\authorized_keys
$isAdmin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
$currentUser = $env:USERNAME

# Always populate user file
$userKeyPath = Join-Path $env:USERPROFILE '.ssh\authorized_keys'
$userKeyDir = Split-Path $userKeyPath -Parent
if (-not (Test-Path $userKeyDir)) {
    New-Item -ItemType Directory -Path $userKeyDir | Out-Null
}
$existing = if (Test-Path $userKeyPath) { Get-Content $userKeyPath } else { @() }
if ($existing -notcontains $MAC_PUBKEY) {
    Add-Content -Path $userKeyPath -Value $MAC_PUBKEY
    Write-Host "  + $userKeyPath"
} else {
    Write-Host "  ✓ already in $userKeyPath"
}

# If logged-in user is in Administrators group, also populate admin file
$inAdminGroup = (Get-LocalGroupMember -Group 'Administrators' -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -like "*\$currentUser" }) -ne $null
if ($inAdminGroup) {
    $adminKeyPath = 'C:\ProgramData\ssh\administrators_authorized_keys'
    if (-not (Test-Path 'C:\ProgramData\ssh')) {
        New-Item -ItemType Directory -Path 'C:\ProgramData\ssh' | Out-Null
    }
    $existing = if (Test-Path $adminKeyPath) { Get-Content $adminKeyPath } else { @() }
    if ($existing -notcontains $MAC_PUBKEY) {
        Add-Content -Path $adminKeyPath -Value $MAC_PUBKEY
        Write-Host "  + $adminKeyPath (admin user)"
    } else {
        Write-Host "  ✓ already in $adminKeyPath"
    }
    # Tighten permissions per Windows OpenSSH requirements
    icacls $adminKeyPath /inheritance:r /grant 'Administrators:F' /grant 'SYSTEM:F' | Out-Null
}

# ── 3. Firewall rules: inbound TCP 22 + 7878 from Tailscale subnet ───────────
Write-Host ""
Write-Host "[3/4] Firewall (Tailscale $TAILSCALE_CIDR → TCP 22, 7878)" -ForegroundColor Yellow

function Add-FwRule {
    param([string]$Name, [int]$Port)
    $existing = Get-NetFirewallRule -DisplayName $Name -ErrorAction SilentlyContinue
    if ($existing) {
        Write-Host "  ✓ exists: $Name (port $Port)"
    } else {
        New-NetFirewallRule -DisplayName $Name -Direction Inbound -Protocol TCP `
            -LocalPort $Port -RemoteAddress $TAILSCALE_CIDR -Action Allow -Profile Any | Out-Null
        Write-Host "  + added: $Name (port $Port)"
    }
}

Add-FwRule "Phantom Mesh - Tailscale SSH"     22
Add-FwRule "Phantom Mesh - Tailscale phantom" 7878

# ── 4. Report info for Mac ───────────────────────────────────────────────────
Write-Host ""
Write-Host "[4/4] Status & report" -ForegroundColor Yellow

$hostname = $env:COMPUTERNAME
$tailscaleIp = ""
try {
    $ts = & 'C:\Program Files\Tailscale\tailscale.exe' ip -4 2>$null
    if ($LASTEXITCODE -eq 0) { $tailscaleIp = ($ts -split "`n")[0].Trim() }
} catch {}

$phantomRunning = Get-Process -Name 'phantom-mesh' -ErrorAction SilentlyContinue
$phantomStatus = if ($phantomRunning) { "✓ pid=$($phantomRunning.Id)" } else { "✗ not running" }

Write-Host ""
Write-Host "═══ 報給 Mac 用 ═══" -ForegroundColor Cyan
Write-Host "Hostname:      $hostname"
Write-Host "User:          $currentUser"
Write-Host "Tailscale IP:  $tailscaleIp"
Write-Host "phantom-mesh:  $phantomStatus"
Write-Host ""
Write-Host "Mac 端可以這樣連："
Write-Host "  ssh $currentUser@$tailscaleIp 'hostname'"
Write-Host ""
Write-Host "═══════════════════════════════════════"
