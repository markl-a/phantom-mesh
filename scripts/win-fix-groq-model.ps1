# win-fix-groq-model.ps1
# Inject `default_model = "llama-3.1-8b-instant"` into the [providers.groq]
# section of $HOME\.phantom-mesh\agents.toml on a Windows phantom-mesh peer,
# then restart phantom serve via schtasks so the new model takes effect.
#
# WHY: Win v0.4.0 phantom's [providers.groq] section silently inherits the
# global default_model = "gemini-2.5-flash", which Groq returns 404 for. Adding
# an explicit default_model line under [providers.groq] overrides the global.
#
# USAGE: powershell -ExecutionPolicy Bypass -File win-fix-groq-model.ps1
#        (idempotent — safe to re-run)

$ErrorActionPreference = "Stop"
$toml = "$env:USERPROFILE\.phantom-mesh\agents.toml"
if (-not (Test-Path $toml)) {
  Write-Error "no agents.toml at $toml"; exit 1
}

$desiredModel = "llama-3.1-8b-instant"
$lines = Get-Content $toml
$newLines = @()
$inGroq = $false
$injected = $false
$alreadyHasDefaultModel = $false

# Walk top-to-bottom. When we enter [providers.groq], remember it. When we hit
# the NEXT section header (or EOF), if we never saw a default_model line under
# [providers.groq], inject one right before the new section/EOF.
for ($i = 0; $i -lt $lines.Length; $i++) {
  $line = $lines[$i]
  $trimmed = $line.Trim()

  if ($trimmed -match '^\[providers\.groq\]') {
    $inGroq = $true
    $newLines += $line
    continue
  }

  if ($trimmed -match '^\[' -and $inGroq) {
    # Leaving the [providers.groq] section
    if (-not $alreadyHasDefaultModel -and -not $injected) {
      $newLines += "default_model = `"$desiredModel`""
      $injected = $true
    }
    $inGroq = $false
    $newLines += $line
    continue
  }

  if ($inGroq -and $trimmed -match '^default_model\s*=') {
    # Already has a default_model — replace it
    $newLines += "default_model = `"$desiredModel`""
    $alreadyHasDefaultModel = $true
    $injected = $true
    continue
  }

  $newLines += $line
}

# EOF case: was still inside [providers.groq] at end of file
if ($inGroq -and -not $alreadyHasDefaultModel -and -not $injected) {
  $newLines += "default_model = `"$desiredModel`""
  $injected = $true
}

if (-not $injected) {
  Write-Warning "no [providers.groq] section found in $toml; appending one"
  $newLines += ""
  $newLines += "[providers.groq]"
  $newLines += "default_model = `"$desiredModel`""
  $injected = $true
}

# Write back atomically: write to temp, then rename
$tmp = "$toml.tmp.$([System.Guid]::NewGuid().ToString('N'))"
$newLines | Set-Content -Path $tmp -Encoding UTF8
# Backup the original first
Copy-Item -Path $toml -Destination "$toml.bak" -Force
Move-Item -Path $tmp -Destination $toml -Force

Write-Host "patched [providers.groq] default_model = `"$desiredModel`" in $toml"
Write-Host "backup at $toml.bak"

# Verify
$check = Select-String -Path $toml -Pattern '^\[providers\.groq\]' -Context 0,4
if ($check) {
  Write-Host "section after patch:"
  $check.Context.PostContext | ForEach-Object { Write-Host "  $_" }
}

# Restart phantom serve via schtasks so the new agents.toml is picked up.
Write-Host ""
Write-Host "Restarting phantom serve..."

$proc = Get-Process -Name "phantom" -ErrorAction SilentlyContinue
if ($proc) {
  $proc | ForEach-Object {
    Write-Host "  killing pid $($_.Id)"
    Stop-Process -Id $_.Id -Force
  }
  Start-Sleep -Seconds 2
}

$taskName = "PhantomMeshServe"
schtasks /Run /TN $taskName 2>&1 | Out-Null

Start-Sleep -Seconds 4
$alive = Get-Process -Name "phantom" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($alive) {
  Write-Host "phantom serve up (pid=$($alive.Id))"
  try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:7878/healthz" -UseBasicParsing -TimeoutSec 5
    Write-Host "healthz: $($r.StatusCode) $($r.Content)"
  } catch { Write-Warning "healthz: $_" }
} else {
  Write-Error "phantom did not start after schtasks /Run"
  exit 2
}

Write-Host "DONE"
