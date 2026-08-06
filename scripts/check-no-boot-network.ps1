#!/usr/bin/env pwsh
#requires -version 5
<#
.SYNOPSIS
  check-no-boot-network.ps1 — P0-7 S3 CI mirror of the cold-start no-network gate.

.DESCRIPTION
  Static grep mirror of core/tests/p0_7_no_boot_network_static.rs, so a
  regression that wires a remote URL into the cold-start surface is caught by
  the doc-tree / CI lint even outside `cargo test`. Asserts the (d) guarantee:

    1. The onboarding writer (onboarding_config.rs) emits no hardcoded remote
       host and only loopback URLs.
    2. Local-server detection (local_servers.rs) probes only loopback.
    3. The broker default URL (https://phantommesh.io) is only *resolved* via a
       token-gated `unwrap_or_else` read — never on the fresh-install path.

  ALL-GREEN-or-exit-1. No piped exit-code masking (CLAUDE.md verification rule):
  each check sets $fail directly; the script exits 1 if any check failed.
#>

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$core = Join-Path $root 'core'
$fail = $false

function Fail($msg) {
    Write-Host "FAIL: $msg" -ForegroundColor Red
    $script:fail = $true
}
function Pass($msg) {
    Write-Host "  ok: $msg" -ForegroundColor Green
}

# --- 1. onboarding writer: no remote host, only loopback URLs ---------------
$onb = Get-Content (Join-Path $core 'src/onboarding_config.rs') -Raw
foreach ($needle in @('phantommesh.io', 'demo.spectynmesh', '192.0.2.')) {
    if ($onb.Contains($needle)) { Fail "onboarding_config.rs hardcodes a remote host ($needle)" }
}
# Every http(s):// literal must be loopback.
$urlMatches = [regex]::Matches($onb, 'https?://([^/"''\s\)`]+)')
foreach ($m in $urlMatches) {
    $h = $m.Groups[1].Value
    if (-not ($h.StartsWith('127.0.0.1') -or $h.StartsWith('localhost'))) {
        Fail "onboarding_config.rs emits a non-loopback URL: $($m.Value)"
    }
}
if (-not $onb.Contains('http://127.0.0.1:11434/v1')) {
    Fail 'onboarding_config.rs lost the localhost ollama url'
}
if (-not $fail) { Pass 'onboarding writer: loopback-only, no remote host' }

# --- 2. local-server detection: loopback only -------------------------------
$ls = Get-Content (Join-Path $core 'src/providers/local_servers.rs') -Raw
$before = $fail
$lsMatches = [regex]::Matches($ls, 'https?://([^/"''\s\)`]+)')
foreach ($m in $lsMatches) {
    $h = $m.Groups[1].Value
    if (-not ($h.StartsWith('127.0.0.1') -or $h.StartsWith('localhost'))) {
        Fail "local_servers.rs probes a non-loopback URL: $($m.Value)"
    }
}
if ($ls.Contains('phantommesh.io')) { Fail 'local_servers.rs references a broker host' }
if ($fail -eq $before) { Pass 'local detection: loopback-only' }

# --- 3. broker default URL only in token-gated reads ------------------------
$cfg = Get-Content (Join-Path $core 'src/cli_config.rs') -Raw
$needle = 'unwrap_or_else(|| "https://phantommesh.io"'
$before = $fail
$idx = 0
$count = 0
while (($idx = $cfg.IndexOf($needle, $idx)) -ge 0) {
    $count++
    $start = [Math]::Max(0, $idx - 800)
    $end = [Math]::Min($idx + 200, $cfg.Length)
    $window = $cfg.Substring($start, $end - $start)
    $gated = $window.Contains('broker_token') -or $window.Contains('read_broker_config') `
        -or $window.Contains('auth::load') -or $window.Contains('no broker token') `
        -or $window.Contains('no token') -or $window.Contains('spectyn login')
    if (-not $gated) { Fail "phantommesh.io default at byte $idx is not token-gated" }
    $idx += $needle.Length
}
if ($count -eq 0) { Fail 'broker default-URL resolution pattern not found (silent rename?)' }
if ($fail -eq $before) { Pass "broker default URL token-gated ($count reads)" }

if ($fail) {
    Write-Host 'check-no-boot-network: FAILED' -ForegroundColor Red
    exit 1
}
Write-Host 'check-no-boot-network: ALL GREEN' -ForegroundColor Green
exit 0
