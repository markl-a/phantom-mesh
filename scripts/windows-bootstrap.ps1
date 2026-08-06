#requires -RunAsAdministrator
[CmdletBinding(SupportsShouldProcess = $true)]
param(
    # OPT-IN: provide an SSH public key (full one-line `ssh-ed25519 AAAA... comment`)
    # to install into authorized_keys / administrators_authorized_keys.
    # If omitted, NO SSH KEY IS INSTALLED. This is intentional -- the prior
    # behavior of hardcoding a third party's pubkey was insecure (V10 HIGH-5 / C9).
    [string]$AddSshKey = '',

    # Tailscale IPv4 CIDR used to scope the inbound firewall rules.
    [string]$TailscaleCidr = '100.64.0.0/10'
)
# -----------------------------------------------------------------------------
# Spectyn Mesh -- Windows worker bootstrap (Z13 / Acer / AYANEO)
# -----------------------------------------------------------------------------
#
# Run this on EACH Windows worker (need admin PowerShell).
#
# What it does:
#   1. Enable + start OpenSSH server (sshd service)
#   2. (OPT-IN) If -AddSshKey is supplied, add that pubkey to authorized_keys
#      / administrators_authorized_keys. WITHOUT THE FLAG, NO KEY IS INSTALLED.
#   3. Open Windows Defender Firewall for inbound TCP 22 + 7878 from Tailscale
#   4. Print info for the operator: user@host, Tailscale IP, spectyn-mesh status
#
# SECURITY NOTE (C9 / T78 / V10 HIGH-5):
#   Previous versions of this script shipped with a HARDCODED ed25519 pubkey
#   belonging to the maintainer's Mac, and `irm | iex` invocations silently
#   wrote it to `C:\ProgramData\ssh\administrators_authorized_keys`. That was
#   a backdoor by default. It has been removed. To grant SSH access you must
#   now explicitly pass `-AddSshKey "<pubkey>"`.
#
# Usage:
#   # Local -- no SSH key added (recommended default):
#   powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\windows-bootstrap.ps1
#
#   # Local -- explicit opt-in to install a pubkey:
#   .\scripts\windows-bootstrap.ps1 -AddSshKey "ssh-ed25519 AAAA... me@host"
#
#   # Remote (no key installed):
#   irm https://raw.githubusercontent.com/markl-a/spectyn-mesh/main/scripts/windows-bootstrap.ps1 | iex
#
#   # Remote with explicit key (download first, then run with arg -- `iex` cannot
#   # accept script parameters):
#   irm https://raw.githubusercontent.com/markl-a/spectyn-mesh/main/scripts/windows-bootstrap.ps1 -OutFile bootstrap.ps1
#   .\bootstrap.ps1 -AddSshKey "ssh-ed25519 AAAA... me@host"
#
# Removal:
#   To revoke an installed key, run `spectyn uninstall --remove-ssh-key`
#   or manually edit:
#     C:\ProgramData\ssh\administrators_authorized_keys
#     $env:USERPROFILE\.ssh\authorized_keys
# -----------------------------------------------------------------------------

$ErrorActionPreference = 'Stop'

Write-Host "??? Spectyn Mesh Windows bootstrap ???" -ForegroundColor Cyan
Write-Host ""

# -- 1. Install + start OpenSSH server ----------------------------------------
Write-Host "[1/4] OpenSSH server" -ForegroundColor Yellow
if ($PSCmdlet.ShouldProcess('OpenSSH.Server capability', 'install + enable sshd')) {
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
        Write-Host "  ? OpenSSH install failed: $_" -ForegroundColor Red
    }
} else {
    Write-Host "  (WhatIf) would install OpenSSH.Server + enable sshd"
}

# -- 2. (OPT-IN) Install SSH pubkey -------------------------------------------
Write-Host ""
Write-Host "[2/4] authorized_keys" -ForegroundColor Yellow

$currentUser = $env:USERNAME

if ([string]::IsNullOrWhiteSpace($AddSshKey)) {
    Write-Host "  (skipped) no -AddSshKey provided -- no SSH pubkey installed."
    Write-Host "  To grant SSH access, re-run with: -AddSshKey ""ssh-ed25519 AAAA... comment"""
} else {
    # Validate format (very loose -- just ensure it looks like an OpenSSH key line)
    $trimmedKey = $AddSshKey.Trim()
    if ($trimmedKey -notmatch '^(ssh-ed25519|ssh-rsa|ecdsa-sha2-\S+|sk-ssh-ed25519@openssh\.com|sk-ecdsa-sha2-\S+@openssh\.com)\s+\S+') {
        throw "-AddSshKey does not look like a valid OpenSSH public key line: '$trimmedKey'"
    }

    Write-Warning "ADDING SSH PUBLIC KEY TO administrators_authorized_keys -- REMOVE WITH 'spectyn uninstall --remove-ssh-key' OR EDIT MANUALLY"
    Write-Warning "  key: $trimmedKey"

    if ($PSCmdlet.ShouldProcess('authorized_keys files', "install pubkey")) {
        # Always populate user file
        $userKeyPath = Join-Path $env:USERPROFILE '.ssh\authorized_keys'
        $userKeyDir = Split-Path $userKeyPath -Parent
        if (-not (Test-Path $userKeyDir)) {
            New-Item -ItemType Directory -Path $userKeyDir | Out-Null
        }
        $existing = if (Test-Path $userKeyPath) { Get-Content $userKeyPath } else { @() }
        if ($existing -notcontains $trimmedKey) {
            Add-Content -Path $userKeyPath -Value $trimmedKey
            Write-Host "  + $userKeyPath"
        } else {
            Write-Host "  [OK] already in $userKeyPath"
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
            if ($existing -notcontains $trimmedKey) {
                Add-Content -Path $adminKeyPath -Value $trimmedKey
                Write-Host "  + $adminKeyPath (admin user)"
            } else {
                Write-Host "  [OK] already in $adminKeyPath"
            }
            # Tighten permissions per Windows OpenSSH requirements
            icacls $adminKeyPath /inheritance:r /grant 'Administrators:F' /grant 'SYSTEM:F' | Out-Null
        }
    } else {
        Write-Host "  (WhatIf) would install pubkey to authorized_keys files"
    }
}

# -- 3. Firewall rules: inbound TCP 22 + 7878 from Tailscale subnet -----------
Write-Host ""
Write-Host "[3/4] Firewall (Tailscale $TailscaleCidr -> TCP 22, 7878)" -ForegroundColor Yellow

function Add-FwRule {
    param([string]$Name, [int]$Port)
    $existing = Get-NetFirewallRule -DisplayName $Name -ErrorAction SilentlyContinue
    if ($existing) {
        Write-Host "  [OK] exists: $Name (port $Port)"
    } else {
        if ($PSCmdlet.ShouldProcess("firewall rule '$Name' (port $Port)", 'create')) {
            New-NetFirewallRule -DisplayName $Name -Direction Inbound -Protocol TCP `
                -LocalPort $Port -RemoteAddress $TailscaleCidr -Action Allow -Profile Any | Out-Null
            Write-Host "  + added: $Name (port $Port)"
        } else {
            Write-Host "  (WhatIf) would add: $Name (port $Port)"
        }
    }
}

Add-FwRule "Spectyn Mesh - Tailscale SSH"     22
Add-FwRule "Spectyn Mesh - Tailscale spectyn" 7878

# -- 4. Report info for operator ----------------------------------------------
Write-Host ""
Write-Host "[4/4] Status & report" -ForegroundColor Yellow

$hostname = $env:COMPUTERNAME
$tailscaleIp = ""
try {
    $ts = & 'C:\Program Files\Tailscale\tailscale.exe' ip -4 2>$null
    if ($LASTEXITCODE -eq 0) { $tailscaleIp = ($ts -split "`n")[0].Trim() }
} catch {}

$spectynRunning = Get-Process -Name 'spectyn-mesh' -ErrorAction SilentlyContinue
$spectynStatus = if ($spectynRunning) { "[OK] pid=$($spectynRunning.Id)" } else { "[X] not running" }

Write-Host ""
Write-Host "??? ????? ???" -ForegroundColor Cyan
Write-Host "Hostname:      $hostname"
Write-Host "User:          $currentUser"
Write-Host "Tailscale IP:  $tailscaleIp"
Write-Host "spectyn-mesh:  $spectynStatus"
Write-Host ""
if ([string]::IsNullOrWhiteSpace($AddSshKey)) {
    Write-Host "(no SSH pubkey installed; re-run with -AddSshKey to grant access)"
} else {
    Write-Host "SSH access enabled for the supplied pubkey. Connect via:"
    Write-Host "  ssh $currentUser@$tailscaleIp 'hostname'"
}
Write-Host ""
Write-Host "???????????????????????????????????????"
