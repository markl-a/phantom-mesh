# win-fix-agent-providers.ps1
# Patch [agent.master] in $HOME\.spectyn-mesh\agents.toml to use the
# `provider:model` syntax in its `providers = [...]` priority list so each
# provider gets its own model (instead of inheriting the agent's `model =`
# field which is one Gemini name being sent to every provider).
#
# WHY: spectyn v0.4.0 call_with_fallback reads `agent.master.model` BEFORE
# the per-provider default_model — so [providers.groq] default_model is
# ignored when agent.master.model is non-empty (which it is on Win peers,
# set to "gemini-2.5-flash"). Solution: set providers list to explicit
# "<provider>:<model>" tokens so each entry carries its own model.
#
# Idempotent — safe to re-run.

$ErrorActionPreference = "Stop"
$toml = "$env:USERPROFILE\.spectyn-mesh\agents.toml"
if (-not (Test-Path $toml)) { Write-Error "no agents.toml at $toml"; exit 1 }

# Rollback to pristine pre-patch state if .bak exists (created by the v1
# win-fix-groq-model.ps1 BEFORE any of our patches). This makes the script
# idempotent even after a botched earlier run that left dangling array
# fragments in the file.
if (Test-Path "$toml.bak") {
  Write-Host "found $toml.bak; restoring pristine state before patching"
  Copy-Item -Path "$toml.bak" -Destination $toml -Force
}

$desiredProviders = 'providers    = ["groq:llama-3.1-8b-instant", "gemini:gemini-2.5-flash"]'
$desiredTools     = 'tools        = []'
$desiredInstr     = 'instructions = "Answer in one short line. Do NOT use any tools."'

$lines = Get-Content $toml
$newLines = @()
$inMaster = $false
$injected = $false
$replaced = $false
$skipUntilCloseBracket = $false   # NEW: when set, drop lines until we hit a line containing ']'

for ($i = 0; $i -lt $lines.Length; $i++) {
  $line = $lines[$i]
  $trimmed = $line.Trim()

  # NEW: if we're in the middle of consuming a multi-line array, drop lines
  # until we see the closing ']'. The closing line itself is also dropped.
  if ($skipUntilCloseBracket) {
    if ($trimmed -match '\]') { $skipUntilCloseBracket = $false }
    continue
  }

  if ($trimmed -match '^\[agent\.master\]') {
    $inMaster = $true
    $newLines += $line
    continue
  }

  if ($trimmed -match '^\[' -and $inMaster) {
    # Leaving [agent.master]
    if (-not $replaced) {
      $newLines += $desiredProviders
      $injected = $true
    }
    $inMaster = $false
    $newLines += $line
    continue
  }

  if ($inMaster -and $trimmed -match '^providers\s*=') {
    # Replace the existing providers line (single OR multi-line).
    $newLines += $desiredProviders
    $replaced = $true
    $injected = $true
    # NEW: if the providers value opens a [ but doesn't close on same line,
    # consume continuation lines until we see ].
    if ($trimmed -match '\[' -and -not ($trimmed -match '\].*$')) {
      $skipUntilCloseBracket = $true
    }
    continue
  }

  if ($inMaster -and $trimmed -match '^tools\s*=') {
    $newLines += $desiredTools
    if ($trimmed -match '\[' -and -not ($trimmed -match '\].*$')) {
      $skipUntilCloseBracket = $true
    }
    continue
  }

  if ($inMaster -and $trimmed -match '^instructions\s*=') {
    $newLines += $desiredInstr
    continue
  }

  $newLines += $line
}

# EOF case: still inside [agent.master]
if ($inMaster -and -not $replaced) {
  $newLines += $desiredProviders
  $injected = $true
}

if (-not $injected) {
  Write-Warning "no [agent.master] section found; nothing patched"
  exit 2
}

$tmp = "$toml.tmp.$([System.Guid]::NewGuid().ToString('N'))"
$newLines | Set-Content -Path $tmp -Encoding UTF8
Copy-Item -Path $toml -Destination "$toml.bak2" -Force
Move-Item -Path $tmp -Destination $toml -Force

Write-Host "patched [agent.master] providers list in $toml"
Write-Host "backup at $toml.bak2"

# Show the [agent.master] block after patch
$check = Select-String -Path $toml -Pattern '^\[agent\.master\]' -Context 0,6
if ($check) {
  Write-Host "section after patch:"
  $check.Context.PostContext | ForEach-Object { Write-Host "  $_" }
}

# Restart spectyn serve
Write-Host ""
Write-Host "Restarting spectyn serve..."
$proc = Get-Process -Name "spectyn" -ErrorAction SilentlyContinue
if ($proc) {
  $proc | ForEach-Object { Stop-Process -Id $_.Id -Force }
  Start-Sleep -Seconds 2
}
schtasks /Run /TN SpectynMeshServe 2>&1 | Out-Null
Start-Sleep -Seconds 4

$alive = Get-Process -Name "spectyn" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($alive) {
  Write-Host "spectyn serve up (pid=$($alive.Id))"
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:7878/healthz" -UseBasicParsing -TimeoutSec 5
    Write-Host "healthz: $($r.StatusCode) $($r.Content)"
  } catch { Write-Warning "healthz: $_" }
} else {
  Write-Error "spectyn did not start"; exit 3
}
Write-Host "DONE"
