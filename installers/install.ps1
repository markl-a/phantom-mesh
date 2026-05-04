# phantom mesh — Windows installer
# Run via:
#   iwr -useb https://phantommesh.io/install.ps1 | iex
#
# What it does:
#   1. Downloads phantom.exe to ~/.local/bin (no admin needed)
#   2. Adds ~/.local/bin to user PATH if missing
#   3. Auto-runs `phantom login` to bind to broker + pull LLM keys
#      (skipped on re-install if you already have a fresh token saved).
#      Set $env:PHANTOM_INSTALL_SKIP_LOGIN='1' before piping to skip.
#
# Re-running upgrades the binary in place (kills any running phantom first).
# Does NOT touch ~/.phantom-mesh/ — your config + auth survive upgrades.

$ErrorActionPreference = 'Stop'

$BIN_URL  = 'https://phantommesh.io/dist/phantom-windows-x86_64.exe'
$BIN_DIR  = Join-Path $env:USERPROFILE '.local\bin'
$BIN_PATH = Join-Path $BIN_DIR 'phantom.exe'

Write-Host '=== phantom mesh installer ==='
Write-Host "  source:  $BIN_URL"
Write-Host "  target:  $BIN_PATH"
Write-Host ''

# 1. Stop any running phantom (lets us overwrite the file)
Write-Host '[1/4] stopping running phantom...'
Stop-Process -Name phantom -Force -ErrorAction SilentlyContinue
Start-Sleep -Milliseconds 400

# 2. Download
Write-Host '[2/4] downloading...'
New-Item -ItemType Directory -Force -Path $BIN_DIR | Out-Null
Invoke-WebRequest -Uri $BIN_URL -OutFile $BIN_PATH -UseBasicParsing
$size = (Get-Item $BIN_PATH).Length
Write-Host "  -> $([math]::Round($size/1MB, 1)) MB"

# Drop in ~/bin too if that's where the user's PATH-shim lives (some setups)
if (Test-Path "$env:USERPROFILE\bin") {
    Copy-Item $BIN_PATH "$env:USERPROFILE\bin\phantom.exe" -Force
}

# 3. Add to PATH (User scope) if not already present
Write-Host '[3/4] ensuring PATH contains ~/.local/bin ...'
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath -notlike "*$BIN_DIR*") {
    $newPath = if ([string]::IsNullOrEmpty($userPath)) { $BIN_DIR } else { "$userPath;$BIN_DIR" }
    [Environment]::SetEnvironmentVariable('Path', $newPath, 'User')
    # Reflect in current session so the version check below works
    $env:Path = "$env:Path;$BIN_DIR"
    Write-Host "  -> appended to user PATH"
} else {
    Write-Host '  -> already in PATH'
}

# 4. Smoke test
Write-Host '[4/4] verifying...'
$ver = & $BIN_PATH --version
Write-Host "  $ver"

# 4b. Seed ~/.phantom-mesh/agents.toml on FIRST install only.
# Without this, fresh boxes have no [agent.master].providers priority list,
# so the runtime falls through to alphabetical of all configured providers
# -- which usually puts groq first, and if groq has a stale key in the
# vault that's a 401 every prompt. Write a minimal config that pins master
# to opencode:minimax-m2.5-free (verified to actually call tools) and
# leaves the user free to edit. NEVER overwrite an existing file.
$cfgDir  = Join-Path $env:USERPROFILE '.phantom-mesh'
$cfgPath = Join-Path $cfgDir 'agents.toml'
# Detect corrupt existing agents.toml (we have one specific repeat
# offender: install-peer.ps1's earlier regex-strip left duplicate
# capabilities/peers lines inside [cluster], which trips
# "duplicate key" in TOML parsing). If `phantom providers list` errors
# out, back up + replace.
$shouldSeed = -not (Test-Path $cfgPath)
if (-not $shouldSeed) {
    # Use cmd /c so PS 5.1's native-stderr-as-ErrorRecord quirk doesn't
    # fire $ErrorActionPreference=Stop on a phantom error message --
    # we EXPECT the command to fail when the file is corrupt, that's
    # the whole point of running it. cmd dumps stderr into the same
    # output stream which we then grep for the parse error keywords.
    $tmpOut = Join-Path $env:TEMP "phantom-providers-check-$PID.txt"
    & cmd /c "`"$BIN_PATH`" providers list > `"$tmpOut`" 2>&1"
    $providersOut = if (Test-Path $tmpOut) { Get-Content $tmpOut -Raw } else { '' }
    Remove-Item $tmpOut -ErrorAction SilentlyContinue
    if ($providersOut -match 'TOML parse error|duplicate key') {
        Write-Host '  ! existing agents.toml fails TOML parse -- backing up + reseeding'
        $bak = "$cfgPath.bak-$(Get-Date -UFormat %s)-corrupt"
        Copy-Item $cfgPath $bak -Force
        Write-Host "  backup: $bak"
        Remove-Item $cfgPath
        $shouldSeed = $true
    }
}
if ($shouldSeed) {
    Write-Host '  seeding default agents.toml'
    New-Item -ItemType Directory -Force -Path $cfgDir | Out-Null
    $defaultCfg = @'
# phantom-mesh default config — written by phantommesh.io install.ps1
# Edit freely. `phantom config pull` only touches ~/.phantom-mesh/env
# (LLM API keys), never this file.

[core]
host = "0.0.0.0"
port = 7878

[agent.master]
provider = "opencode"
model    = "minimax-m2.5-free"
tools    = ["shell","file_read","file_write","file_edit","content_search","glob_search","git_status","git_diff","git_log","git_commit","cluster_status","cluster_peers","cluster_sessions"]
# Failover order — phantom tries each provider:model in turn until one
# returns a real response. ANY single provider failing (rate limit,
# wrong key, model removed, network blip) just demotes that entry for
# this call; the next one runs.
#
#   opencode:minimax-m2.5-free      free, vault-managed key, has tool support
#   groq:llama-3.3-70b-versatile     free, very fast, second opinion
#   openrouter:meta-llama/llama-3.3-70b-instruct:free   free with openrouter key
#   local-ollama:qwen3:8b           offline last-resort (only if Ollama installed)
#
# Edit interactively in TUI: /priority   (arrow-key reorder + save)
providers = [
  "opencode:minimax-m2.5-free",
  "groq:llama-3.3-70b-versatile",
  "openrouter:meta-llama/llama-3.3-70b-instruct:free",
  "local-ollama:qwen3:8b",
]
instructions = """
You are a senior software engineer. Use tools to do work — never claim
to have done something without first calling the matching tool. Quote
real tool output back to the user. Prefer short honest 'no' over long
fabricated 'yes'. Respond in Traditional Chinese unless the user
writes in another language.
"""

[providers.opencode]
type          = "opencode"
base_url      = "https://opencode.ai/zen/v1"
api_key_env   = "OPENCODE_API_KEY"
default_model = "minimax-m2.5-free"

[providers.openrouter]
type          = "openrouter"
base_url      = "https://openrouter.ai/api/v1"
api_key_env   = "OPENROUTER_API_KEY"
default_model = "meta-llama/llama-3.3-70b-instruct"

[providers.groq]
type          = "groq"
base_url      = "https://api.groq.com/openai/v1"
api_key_env   = "GROQ_API_KEY"
default_model = "llama-3.3-70b-versatile"
'@
    Set-Content -Path $cfgPath -Value $defaultCfg -Encoding UTF8
    Write-Host "  -> $cfgPath"
} else {
    Write-Host "  agents.toml at $cfgPath parses OK -- left mostly untouched"
}

# 4c. ALWAYS ensure [providers.opencode] block + [agent.master].providers
# priority list exist, even on existing configs. The reason: older
# install scripts (install-phantom-windows.ps1) wrote agents.toml with
# only [providers.groq] inline-keyed, plus [agent.master].provider="groq".
# If that groq key was ever invalid (e.g. rotated, never set right),
# every prompt 401s with no failover. Append the opencode block + set
# the priority so opencode:minimax-m2.5-free is tried FIRST -- it has a
# vault-managed key via OPENCODE_API_KEY which auto-refreshes via
# `phantom config pull`.
$cfgText = Get-Content $cfgPath -Raw -ErrorAction SilentlyContinue
# Note: backslashes are doubled so the TS template literal outputs them
# verbatim — '\[' here renders as '[' in the PS source. Without that,
# TS eats the backslash, PowerShell sees '[providers.opencode]' as a
# regex character class (matches any one of p,r,o,v,i,d,e,s,.,n,c)
# which matches EVERY file -> -notmatch is always false -> block never
# gets appended. That was the actual root cause for the acer machine.
if ($cfgText -and $cfgText -notmatch '\[providers\.opencode\]') {
    Write-Host '  appending [providers.opencode] block (was missing)'
    @'

[providers.opencode]
type          = "opencode"
base_url      = "https://opencode.ai/zen/v1"
api_key_env   = "OPENCODE_API_KEY"
default_model = "minimax-m2.5-free"
'@ | Add-Content -Path $cfgPath
}
# Re-set master priority ONLY when the file contains the legacy
# single-provider line. Existing users who manually edited the chain
# via /priority should not have their order silently overwritten on
# every install. Also writes a 4-provider failover chain (matches the
# default agents.toml block above) so single-provider failures don't
# brick chat.
#
# Note: backslashes are doubled in regex literals (\s, \[, etc.) so
# the TS template literal outputs them verbatim into the PowerShell
# source. Without this the rendered PS sees 'providerss*=s*[' which
# is an unterminated character class — same bug pattern that bit us
# on the [providers.opencode] block check earlier.
$cfgText = Get-Content $cfgPath -Raw -ErrorAction SilentlyContinue
$hasMultiProvider = ($cfgText -match 'providers\s*=\s*\[') -and ($cfgText -match '"groq:')
if (-not $hasMultiProvider) {
    Write-Host '  setting [agent.master].providers = [opencode, groq, openrouter, local-ollama]  (multi-provider failover)'
    & cmd /c "`"$BIN_PATH`" providers priority master `"opencode:minimax-m2.5-free`" `"groq:llama-3.3-70b-versatile`" `"openrouter:meta-llama/llama-3.3-70b-instruct:free`" `"local-ollama:qwen3:8b`" >nul 2>nul"
} else {
    Write-Host '  [agent.master].providers already has multi-provider chain -- leaving alone'
}

# 5. Auto-login + pull LLM keys (unless explicitly skipped or already
#    logged in with a fresh broker_token). Re-runs the OAuth dance every
#    time on a brand-new install; on a re-install where ~/.phantom-mesh/
#    auth.json already has a non-expired broker_token, phantom login
#    short-circuits to a key refresh (no browser).
$skipLogin = $env:PHANTOM_INSTALL_SKIP_LOGIN -eq '1'
if ($skipLogin) {
    Write-Host ''
    Write-Host 'PHANTOM_INSTALL_SKIP_LOGIN=1 set -- skipping login.'
    Write-Host 'Run `phantom login` later to bind this device + pull LLM keys.'
} else {
    Write-Host ''
    Write-Host '=== running phantom login ==='
    Write-Host '  (opens browser -> Google sign-in -> auto-pulls your LLM keys.'
    Write-Host '   Press Ctrl-C to skip; you can run phantom login later.)'
    Write-Host ''
    & $BIN_PATH login
}

# 5b. Load ~/.phantom-mesh/env into THIS PowerShell process so the
#     cluster sync below + the serve we spawn next both see the keys
#     phantom login just dropped onto disk. Without this, cluster sync
#     skips with "CLUSTER_SECRET not in env" even though the value is
#     literally one directory away — that was the actual root cause for
#     the 4 boxes that registered but failed to wire up cluster RPC.
$envFile = "$env:USERPROFILE\.phantom-mesh\env"
if (Test-Path $envFile) {
    Get-Content $envFile | ForEach-Object {
        if ($_ -match '^([A-Z_][A-Z0-9_]*)=(.*)$') {
            [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
        }
    }
}

# 5c. Wire this machine into the cluster: rewrites [cluster] block with
#     up-to-date peers list AND the shared cluster_secret (sourced from
#     the env we just loaded). Idempotent — re-running on a fresh install
#     OR a returning machine both result in the right config.
Write-Host ''
Write-Host '=== joining cluster ==='
& $BIN_PATH cluster sync

# 6. Start phantom serve in the background so cluster RPC works + the
#    just-pulled LLM keys take effect (env is read at process start).
#    If we're elevated, also register a Scheduled Task so serve survives
#    reboot. Otherwise just spawn it for this session and tell the user
#    how to make it persistent.
$isAdmin = ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltinRole]::Administrator)
Write-Host ''
Write-Host '=== starting phantom serve ==='
# Always stop first so a stale serve (from before this install) doesn't
# hold the port + serve old key state.
Stop-Process -Name phantom -Force -ErrorAction SilentlyContinue
& cmd /c 'schtasks /End /TN PhantomMeshServe >nul 2>nul'
Start-Sleep -Milliseconds 400

if ($isAdmin) {
    Write-Host '  (elevated PowerShell -- installing PhantomMeshServe Scheduled Task)'
    & cmd /c 'schtasks /Delete /F /TN PhantomMeshServe >nul 2>nul'
    # /TR needs the exe path quoted (in case it contains spaces) followed
    # by the serve arg. Building the value separately + passing as one
    # arg avoids the nested-quote-escape mess we had before.
    $tr = '"' + $BIN_PATH + '" serve'
    & schtasks /Create /TN PhantomMeshServe /TR $tr /SC ONLOGON /RL HIGHEST /F | Out-Null
    $rc = $LASTEXITCODE
    if ($rc -eq 0) {
        & cmd /c 'schtasks /Run /TN PhantomMeshServe >nul 2>nul'
        Write-Host '  -> task created + started (will autorun on every login)'
    } else {
        Write-Host "  ! schtasks /Create failed (rc=$rc) -- falling back to background launch"
        Start-Process -FilePath $BIN_PATH -ArgumentList 'serve' -WindowStyle Hidden | Out-Null
    }
} else {
    Write-Host '  (not elevated -- starting serve in user background only)'
    Write-Host '  for autostart on reboot, re-run this installer in an elevated PowerShell once.'
    Start-Process -FilePath $BIN_PATH -ArgumentList 'serve' -WindowStyle Hidden | Out-Null
}

Start-Sleep -Seconds 2
# Read [core].port from agents.toml so the smoke check hits the right
# port even when the user customized it.
$port = 7878
$cfg = "$env:USERPROFILE\.phantom-mesh\agents.toml"
if (Test-Path $cfg) {
    $portLine = Select-String -Path $cfg -Pattern '^\s*port\s*=\s*(\d+)' -ErrorAction SilentlyContinue
    if ($portLine) { $port = [int]$portLine.Matches[0].Groups[1].Value }
}
try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:$port/healthz" -UseBasicParsing -TimeoutSec 3
    Write-Host "  healthz on :$port -> $($r.StatusCode) $($r.Content)"
} catch {
    Write-Host "  ! healthz probe failed on :$port : $_"
    Write-Host '    (run `phantom doctor` to diagnose)'
}

Write-Host ''
Write-Host '=== installed ==='
Write-Host ''

# 7. Drop the user straight into the TUI. Read-Host waits for Enter so
#    the workflow reads "iwr -useb ... | iex; <Enter> -> TUI" — no need
#    to remember the binary name. Ctrl-C still works to back out.
#    Set $env:PHANTOM_INSTALL_NOLAUNCH='1' before piping for CI / scripted
#    setups that don't want a TUI to appear.
$noLaunch = $env:PHANTOM_INSTALL_NOLAUNCH -eq '1'
if ($noLaunch) {
    Write-Host 'PHANTOM_INSTALL_NOLAUNCH=1 set -- skipping launch.'
    Write-Host 'Run this when ready:'
    Write-Host '   phantom         # the TUI'
} else {
    Write-Host 'Press Enter to start phantom (or Ctrl-C to exit and run it later).'
    [void](Read-Host)
    & $BIN_PATH
}
