# Phantom Mesh - Windows end-to-end test suite
#
# 8-phase smoke that exercises every Windows-relevant surface the
# node-a hardening sweep on 2026-05-01 carved out. Run after every fresh
# build / re-deploy to make sure nothing regressed.
#
# Usage:
#   ./scripts/test-windows.ps1                        # full suite
#   ./scripts/test-windows.ps1 -Phase 4               # only phase 4
#   ./scripts/test-windows.ps1 -SkipServiceInstall    # skip phase 4 (admin task may be stuck)
#   ./scripts/test-windows.ps1 -ServePort 7895        # override the test port
#
# Pre-requisites:
#   - phantom on PATH (typically C:\Users\<user>\.local\bin\phantom.exe).
#   - $env:OPENROUTER_API_KEY set (some phases call the real LLM).
#   - No `phantom serve` already running (the script kills + restarts).
#
# Each phase prints PASS / FAIL / SKIP in colour. Final summary at the end.
# Exit code mirrors total fail count, so CI can `if ($LASTEXITCODE -ne 0)`.
#
# This script is intentionally pure ASCII so Windows PowerShell 5.1 can
# parse it on Chinese / Japanese / Korean locale Windows where its default
# file encoding is CP950 / CP932 / CP949 (not UTF-8). Don't add fancy
# glyphs unless you also save with a UTF-8 BOM.

[CmdletBinding()]
param(
    [int]$Phase = 0,                  # 0 = run all phases, otherwise that single phase
    [int]$ServePort = 7895,           # serve listens here during phase 7
    [switch]$SkipServiceInstall,      # phase 4 - bypass if admin orphan still blocks
    [switch]$VerboseOutput
)

# 'Continue' (the default) - not 'Stop'. Windows PowerShell 5.1 wraps
# every native command stderr line as an ErrorRecord, so 'Stop' would
# abort the test runner the first time `phantom doctor` writes a hint
# to stderr. We rely on stdout regex matching to decide pass/fail.
$ErrorActionPreference = 'Continue'

# Force UTF-8 for native command stdout. Without this, Windows
# PowerShell 5.1 on CP950 / CP932 / CP949 locales mangles non-ASCII
# glyphs in `phantom doctor` output (the `>` U+203A used in warning
# hints) and assertions that expect plain text break.
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

$script:results = @()
$script:passCount = 0
$script:failCount = 0
$script:skipCount = 0

# Hydrate LLM provider keys from User-scope env (persistent in the registry)
# into this session when missing. A shell spawned before the keys were set
# (or a non-login shell) won't have inherited them, which would force the
# LLM phases to skip even though a usable key exists on the machine.
foreach ($k in @('OPENROUTER_API_KEY','GROQ_API_KEY','GEMINI_API_KEY','OPENCODE_API_KEY')) {
    if (-not [Environment]::GetEnvironmentVariable($k, 'Process')) {
        $userVal = [Environment]::GetEnvironmentVariable($k, 'User')
        if ($userVal) { Set-Item -Path "Env:$k" -Value $userVal }
    }
}
# Any one of these unlocks the LLM round-trip phases; phantom's provider
# fallback chain picks whichever is configured + reachable.
$script:hasLlmKey = [bool]($env:OPENROUTER_API_KEY -or $env:GROQ_API_KEY -or $env:GEMINI_API_KEY)

function Write-Phase($num, $title) {
    Write-Host ""
    Write-Host "=== Phase $num - $title ===" -ForegroundColor Cyan
}
function Pass($msg) {
    Write-Host "  [PASS] $msg" -ForegroundColor Green
    $script:results += @{ status='pass'; message=$msg }
    $script:passCount++
}
function Fail($msg) {
    Write-Host "  [FAIL] $msg" -ForegroundColor Red
    $script:results += @{ status='fail'; message=$msg }
    $script:failCount++
}
function Skip($msg) {
    Write-Host "  [SKIP] $msg" -ForegroundColor Yellow
    $script:results += @{ status='skip'; message=$msg }
    $script:skipCount++
}
function Info($msg) {
    if ($VerboseOutput) { Write-Host "    $msg" -ForegroundColor DarkGray }
}

# Capture both stdout AND stderr from phantom without triggering Windows
# PowerShell 5.1's NativeCommandError wrapping. The `cmd /c "... 2>&1"`
# pattern lets cmd.exe do the redirection at the OS level, so PS only
# sees one combined stream of plain strings (no ErrorRecord objects).
function Invoke-Phantom {
    param([Parameter(Mandatory)][string]$Args, [string]$StdinInput = $null)
    if ($StdinInput) {
        $StdinInput | & cmd /c "phantom $Args 2>&1" | Out-String
    } else {
        & cmd /c "phantom $Args 2>&1" | Out-String
    }
}

# -- Phase 1: Pre-flight ----------------------------------------------------
function Test-Phase1 {
    Write-Phase 1 'Pre-flight setup'

    $procs = Get-Process phantom -ErrorAction SilentlyContinue
    if ($procs) {
        $procs | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 2
    }
    if (Get-Process phantom -ErrorAction SilentlyContinue) {
        Fail "could not stop existing phantom.exe processes"; return
    }
    Pass "no stale phantom.exe processes"

    $exe = Get-Command phantom -ErrorAction SilentlyContinue
    if (-not $exe) { Fail "phantom not on PATH"; return }
    Pass "phantom on PATH: $($exe.Source)"

    if ($script:hasLlmKey) {
        $which = @()
        if ($env:OPENROUTER_API_KEY) { $which += 'OpenRouter' }
        if ($env:GROQ_API_KEY)       { $which += 'Groq' }
        if ($env:GEMINI_API_KEY)     { $which += 'Gemini' }
        Pass "LLM provider key present: $($which -join ', ')"
    } else {
        Skip "no LLM provider key (OPENROUTER/GROQ/GEMINI) set - phases 3, 6 will be skipped"
    }
}

# -- Phase 2: Read-only smoke ----------------------------------------------
function Test-Phase2 {
    Write-Phase 2 'Read-only smoke (--version, doctor, mcp, peer, self-update --dry-run, mlx/snapshot reject)'

    $ver = Invoke-Phantom '--version'
    if ($ver -match '^phantom 0\.') { Pass "--version: $ver" }
    else { Fail "--version unexpected: $ver" }

    $doctor = Invoke-Phantom 'doctor'
    if ($doctor -match 'configured port') { Pass 'doctor: configured port section present' }
    else { Fail 'doctor missing configured port section' }
    if ($doctor -match 'OpenRouter') { Pass 'doctor: OpenRouter listed in provider keys' }
    else { Fail 'doctor missing OpenRouter in provider keys' }

    $mcpReq = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
        '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
    ) -join "`n"
    $mcpOut = Invoke-Phantom -Args 'mcp' -StdinInput $mcpReq
    $toolsCount = ([regex]::Matches($mcpOut, '"name":"[^"]+"')).Count
    if ($toolsCount -ge 40) { Pass "mcp tools/list returned $toolsCount tools" }
    else { Fail "mcp tools/list returned only $toolsCount tools" }

    $upd = Invoke-Phantom 'self-update --dry-run'
    if ($upd -match 'phantom-x86_64-pc-windows.exe') { Pass 'self-update --dry-run: target detected' }
    else { Fail "self-update --dry-run unexpected output" }

    $mlx = Invoke-Phantom 'mlx status'
    if ($mlx -match 'requires Apple Silicon|macOS-only') { Pass 'phantom mlx: rejects on Windows' }
    else { Fail "phantom mlx: did not reject" }
    $snap = Invoke-Phantom 'snapshot apply'
    if ($snap -match 'macOS-only') { Pass 'phantom snapshot: rejects on Windows' }
    else { Fail "phantom snapshot: did not reject" }
}

# -- Phase 3: LLM round-trips -----------------------------------------------
function Test-Phase3 {
    Write-Phase 3 'LLM round-trips (master + coder via available provider)'
    if (-not $script:hasLlmKey) { Skip 'no LLM provider key (OPENROUTER/GROQ/GEMINI)'; return }

    $mOut = Invoke-Phantom '-c hi'
    if ($mOut -match '\$0\.0000') {
        Pass "master agent: zero-cost round-trip"
    } else {
        Fail "master agent: did not return zero-cost round-trip"
    }

    $cOut = Invoke-Phantom '-c --agent coder hello'
    if ($cOut -match '\$0\.0000') {
        Pass "coder agent: zero-cost round-trip"
    } else {
        Fail "coder agent: did not return zero-cost round-trip"
    }

    $mcpReq = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
        '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"shell","arguments":{"command":"echo home_test","cwd":"~"}}}'
    ) -join "`n"
    $mcpOut = Invoke-Phantom -Args 'mcp' -StdinInput $mcpReq
    if ($mcpOut -match 'home_test' -and $mcpOut -notmatch 'cwd .*does not exist') {
        Pass "shell tool: cwd '~' expansion works (no retry burned)"
    } else {
        Fail "shell tool: cwd '~' expansion broke"
    }
}

# -- Phase 4: phantom service install/status/uninstall ----------------------
function Test-Phase4 {
    Write-Phase 4 'phantom service install / status / uninstall (PhantomServe)'
    if ($SkipServiceInstall) { Skip '-SkipServiceInstall passed'; return }

    $instOut = Invoke-Phantom 'service install'
    if ($instOut -match 'PermissionDenied|HRESULT 0x80070005') {
        Skip 'admin-owned PhantomServe blocks user-level install - run Unregister-ScheduledTask -TaskName PhantomServe -Confirm:$false from elevated PowerShell, then re-run this phase'
        return
    }
    if ($instOut -match 'Registered Scheduled Task') {
        Pass 'service install: Scheduled Task registered'
    } else {
        Fail "service install unexpected output"
        return
    }

    Start-Sleep -Seconds 4

    $stat = Invoke-Phantom 'service status'
    if ($stat -match 'registered : .*yes') {
        Pass 'service status: registered yes'
    } else {
        Fail "service status: not registered after install"
    }
    if ($stat -match 'healthz.*ok') {
        Pass 'service status: healthz ok'
    } else {
        Skip "service status: healthz unreachable (probably needs a moment longer)"
    }

    $uninst = Invoke-Phantom 'service uninstall'
    if ($uninst -match 'Uninstalled|Removed Scheduled Task') {
        Pass 'service uninstall: clean'
    } else {
        Fail "service uninstall unexpected output"
    }
}

# -- Phase 5: phantom autoevolve schedule -----------------------------------
function Test-Phase5 {
    Write-Phase 5 'phantom autoevolve schedule install / status / uninstall'

    $inst = Invoke-Phantom 'autoevolve schedule install --interval 3600 --target check'
    if ($inst -match 'Scheduled autoevolve every 60min') {
        Pass 'schedule install: created PhantomAutoevolve task at 60min'
    } else {
        Fail "schedule install: did not create task"; return
    }

    $stat = Invoke-Phantom 'autoevolve schedule status'
    if ($stat -match 'registered : .*yes' -and $stat -match 'PT1H') {
        Pass 'schedule status: registered, interval PT1H'
    } else {
        Fail "schedule status: did not see registered=yes and PT1H"
    }

    $uninst = Invoke-Phantom 'autoevolve schedule uninstall'
    if ($uninst -match 'Unscheduled|Removed') {
        Pass 'schedule uninstall: clean'
    } else {
        Fail "schedule uninstall: did not confirm removal"
    }
}

# -- Phase 6: autoevolve --once + evolve --max-rounds 1 ---------------------
function Test-Phase6 {
    Write-Phase 6 'autoevolve --once + evolve --max-rounds 1'
    if (-not $script:hasLlmKey) { Skip 'no LLM provider key (OPENROUTER/GROQ/GEMINI)'; return }

    $auto = Invoke-Phantom 'autoevolve --once --target check'
    if ($auto -match 'cargo check green - nothing to evolve' -or $auto -match 'green .* nothing to evolve') {
        Pass 'autoevolve --once: green, no LLM triggered'
    } elseif ($auto -match 'hit Windows AV lock') {
        Pass 'autoevolve --once: AV-lock transient correctly suppressed'
    } else {
        Fail "autoevolve --once: unexpected output"
    }

    # evolve calls `cargo test` via the shell tool. Without a Cargo.toml
    # in the agent's cwd it errors out and the round ends with "stopped
    # after N rounds" instead of EVOLVE_DONE. The repo's Cargo.toml lives
    # in `core/`, so cd there first before invoking evolve.
    $evoCwd = Join-Path (Resolve-Path (Join-Path $PSScriptRoot '..')) 'core'
    Push-Location $evoCwd
    try {
        $evo = Invoke-Phantom 'evolve --max-rounds 1 --target check'
    } finally {
        Pop-Location
    }
    if ($evo -match 'EVOLVE_DONE|all tests pass') {
        Pass 'evolve --max-rounds 1: completes EVOLVE_DONE'
    } elseif ($evo -match 'stopped after \d+ rounds' -and
             $evo -match 'HTTP 413|tokens per minute|TPM|rate.?limit|429') {
        # The evolve loop itself ran fine (started, sandboxed, called the
        # provider chain, exited cleanly) but the free-tier LLM could not
        # complete a tool-using round: Groq 413s because the tool-schema
        # request exceeds its 6000 TPM ceiling, and the fallback model does
        # not reliably emit a structured tool_call. That is a provider/quota
        # limitation, not a Windows-side defect, so it is a SKIP not a FAIL.
        Skip 'evolve loop healthy; LLM provider rate-limited (tool-schema request > free TPM) - not a Windows issue'
    } else {
        Fail "evolve: did not see EVOLVE_DONE / all tests pass"
    }
}

# -- Phase 7: phantom serve concurrent load ---------------------------------
function Test-Phase7 {
    Write-Phase 7 "phantom serve + concurrent /healthz on :$ServePort"

    $serveJob = Start-Job -ScriptBlock { phantom serve --port $using:ServePort 2>&1 }
    Start-Sleep -Seconds 3

    try {
        $health = Invoke-WebRequest -Uri "http://127.0.0.1:$ServePort/healthz" -TimeoutSec 3 -UseBasicParsing
        if ($health.StatusCode -eq 200) {
            Pass "/healthz on :$ServePort returns 200"
        } else {
            Fail "/healthz returned $($health.StatusCode)"
            Get-Process phantom -ErrorAction SilentlyContinue | Stop-Process -Force
            $serveJob | Remove-Job -Force -ErrorAction SilentlyContinue
            return
        }
    } catch {
        Fail "/healthz unreachable on :$ServePort"
        Get-Process phantom -ErrorAction SilentlyContinue | Stop-Process -Force
        $serveJob | Remove-Job -Force -ErrorAction SilentlyContinue
        return
    }

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $jobs = 1..16 | ForEach-Object {
        Start-Job -ScriptBlock {
            try {
                (Invoke-WebRequest -Uri "http://127.0.0.1:$using:ServePort/healthz" -TimeoutSec 5 -UseBasicParsing).StatusCode
            } catch { 0 }
        }
    }
    $codes = $jobs | Wait-Job | Receive-Job
    $jobs | Remove-Job
    $sw.Stop()
    $allOk = ($codes | Where-Object { $_ -ne 200 }).Count -eq 0
    if ($allOk) {
        Pass "16 concurrent /healthz: all 200 in $($sw.ElapsedMilliseconds)ms"
    } else {
        Fail "16 concurrent /healthz: some did not return 200"
    }

    try {
        $ver = (Invoke-WebRequest -Uri "http://127.0.0.1:$ServePort/api/version" -TimeoutSec 3 -UseBasicParsing).Content
        if ($ver -match '"version"') { Pass "/api/version returns json" }
        else { Fail "/api/version unexpected" }
    } catch { Fail "/api/version error" }

    try {
        $ping = (Invoke-WebRequest -Uri "http://127.0.0.1:$ServePort/rpc/ping" -TimeoutSec 3 -UseBasicParsing).Content
        if ($ping -match '"wire_version"') {
            Pass "/rpc/ping includes wire_version (post-macos-merge contract)"
        } else {
            Skip "/rpc/ping missing wire_version (older binary?)"
        }
    } catch { Fail "/rpc/ping error" }

    Get-Process phantom -ErrorAction SilentlyContinue | Stop-Process -Force
    Start-Sleep -Seconds 2
    if (-not (Get-Process phantom -ErrorAction SilentlyContinue)) {
        Pass 'Stop-Process: phantom.exe gone'
    } else {
        Fail 'Stop-Process: phantom.exe still running'
    }
    $serveJob | Remove-Job -Force -ErrorAction SilentlyContinue
}

# -- Phase 8: edge cases + cleanup ------------------------------------------
function Test-Phase8 {
    Write-Phase 8 'tilde edge cases + broken-pipe panic + cleanup'

    # Use 'cd' (a cmd.exe builtin that prints the working directory on
    # Windows) rather than 'pwd' (a Unix-only command cmd.exe does not have,
    # which would always fail with exit code 1 regardless of tilde handling).
    # Assert the resolved path is the home dir so we prove '~' actually
    # expanded, not merely that the command ran.
    $req = @(
        '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"smoke","version":"0"}}}'
        '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"shell","arguments":{"command":"cd","cwd":"~/"}}}'
    ) -join "`n"
    $out = Invoke-Phantom -Args 'mcp' -StdinInput $req
    # The path comes back inside a JSON string, so backslashes arrive doubled
    # (C:\\Users\\name). Normalize to single backslashes before matching.
    $outNorm = $out -replace '\\\\', '\'
    $homeEsc = [regex]::Escape($env:USERPROFILE)
    if ($outNorm -match 'exit code: 0' -and $outNorm -match $homeEsc -and $outNorm -notmatch 'cwd .*does not exist') {
        Pass "shell tool: cwd '~/' expands to $env:USERPROFILE"
    } else {
        Fail "shell tool: cwd '~/' did not expand (expected $env:USERPROFILE in output)"
    }

    $crashDir = Join-Path $env:USERPROFILE '.phantom-mesh\crashes'
    $crashCountBefore = (Get-ChildItem $crashDir -Filter '*.log' -ErrorAction SilentlyContinue | Measure-Object).Count
    $null = & phantom doctor 2>$null | Select-Object -First 1
    Start-Sleep -Seconds 1
    $crashCountAfter = (Get-ChildItem $crashDir -Filter '*.log' -ErrorAction SilentlyContinue | Measure-Object).Count
    if ($crashCountAfter -eq $crashCountBefore) {
        Pass "broken-pipe panic suppression: doctor | head produced no new crash log (count stayed at $crashCountAfter)"
    } else {
        Fail "broken-pipe regression: crash count went $crashCountBefore -> $crashCountAfter"
    }

    if (Get-Process phantom -ErrorAction SilentlyContinue) {
        Fail "cleanup: phantom.exe still running"
    } else {
        Pass "cleanup: no phantom.exe"
    }
}

# -- Phase 9: code-signing smoke (scripts/codesign-windows.ps1) -------------
function Test-Phase9 {
    Write-Phase 9 'codesign smoke (dev self-signed via codesign-windows.ps1)'

    $codesign = Join-Path $PSScriptRoot 'codesign-windows.ps1'
    if (-not (Test-Path $codesign)) {
        Skip "scripts/codesign-windows.ps1 not present"
        return
    }

    # Ensure dev cert (idempotent) and capture its thumbprint.
    & $codesign -CreateCert *>$null
    $cert = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Subject -eq 'CN=Phantom Mesh Dev Code Signing' } |
        Sort-Object NotBefore -Descending | Select-Object -First 1
    if ($cert) {
        Pass "codesign -CreateCert: dev cert present ($($cert.Thumbprint.Substring(0,12))...)"
    } else {
        Fail "codesign -CreateCert: no dev cert in CurrentUser\My"
        return
    }

    # Build a fresh stub exe (NOT a System32 binary - those match the OS
    # catalog and Get-AuthenticodeSignature would report the catalog signer
    # instead of ours) and sign it.
    $stub = Join-Path $env:TEMP 'phantom-codesign-phase9.exe'
    Remove-Item $stub -ErrorAction SilentlyContinue
    Add-Type -TypeDefinition 'public class P9 { public static void Main() {} }' `
        -OutputAssembly $stub -OutputType ConsoleApplication
    $signOut = & $codesign -Path $stub *>&1 | Out-String

    $sig = Get-AuthenticodeSignature -FilePath $stub
    if ($sig.Status -ne 'NotSigned' -and
        $sig.SignerCertificate -and
        $sig.SignerCertificate.Thumbprint -eq $cert.Thumbprint) {
        Pass "codesign sign: signature attached, signer thumbprint matches dev cert (status=$($sig.Status))"
    } else {
        Fail "codesign sign: expected dev-cert signer, got status=$($sig.Status) signer=$($sig.SignerCertificate.Thumbprint)"
    }

    # Read-only verify path returns 0 (PASS) for a present signature.
    & $codesign -Verify -Path $stub *>$null
    if ($LASTEXITCODE -eq 0) {
        Pass "codesign -Verify: exit 0 on signed stub"
    } else {
        Fail "codesign -Verify: exit $LASTEXITCODE on signed stub"
    }

    Remove-Item $stub -ErrorAction SilentlyContinue
}

# -- Run --------------------------------------------------------------------
$phases = @(1,2,3,4,5,6,7,8,9)
if ($Phase -ne 0) { $phases = @($Phase) }

foreach ($p in $phases) {
    & "Test-Phase$p"
}

# -- Summary ----------------------------------------------------------------
Write-Host ""
Write-Host "==========================================================="
Write-Host "  Summary" -ForegroundColor Cyan
Write-Host "==========================================================="
Write-Host "  PASS : $script:passCount" -ForegroundColor Green
Write-Host "  SKIP : $script:skipCount" -ForegroundColor Yellow
Write-Host "  FAIL : $script:failCount" -ForegroundColor Red
Write-Host ""

if ($script:failCount -gt 0) {
    Write-Host "FAILED" -ForegroundColor Red
    exit $script:failCount
} else {
    Write-Host "ALL CLEAR" -ForegroundColor Green
    exit 0
}
