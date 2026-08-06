# win-set-llm-keys.ps1
# Set Groq + Gemini API keys at user-env level and restart spectyn serve.
# WHY: Windows peers in the spectyn-mesh cluster need an LLM key so
#      fan-out swarm jobs can actually produce text. The keys live in
#      USER env (not machine env) so they only affect the current user.
# USAGE: powershell -ExecutionPolicy Bypass -File win-set-llm-keys.ps1 <GROQ_KEY> <GEMINI_KEY>

param(
  [Parameter(Mandatory=$true)][string]$GroqKey,
  [Parameter(Mandatory=$true)][string]$GeminiKey
)

Write-Host "Setting user env vars..."
[System.Environment]::SetEnvironmentVariable("GROQ_API_KEY", $GroqKey, "User")
[System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY", $GeminiKey, "User")
Write-Host "  GROQ_API_KEY set (length=$($GroqKey.Length))"
Write-Host "  GEMINI_API_KEY set (length=$($GeminiKey.Length))"

# Refresh current process env so the relaunched spectyn inherits them.
$env:GROQ_API_KEY = $GroqKey
$env:GEMINI_API_KEY = $GeminiKey

Write-Host "Stopping any running spectyn serve..."
$proc = Get-Process -Name "spectyn" -ErrorAction SilentlyContinue
if ($proc) {
  $proc | ForEach-Object { Write-Host "  killing pid $($_.Id)"; Stop-Process -Id $_.Id -Force }
  Start-Sleep -Seconds 2
} else {
  Write-Host "  no spectyn process running"
}

# Find the spectyn binary (try common install locations)
$candidates = @(
  "$env:USERPROFILE\.local\bin\spectyn.exe",
  "$env:USERPROFILE\.cargo\bin\spectyn.exe",
  "C:\Users\$env:USERNAME\.local\bin\spectyn.exe",
  "C:\spectyn\spectyn.exe"
)
$spectyn = $null
foreach ($c in $candidates) {
  if (Test-Path $c) { $spectyn = $c; break }
}
if (-not $spectyn) {
  # try PATH
  $cmd = Get-Command spectyn -ErrorAction SilentlyContinue
  if ($cmd) { $spectyn = $cmd.Source }
}
if (-not $spectyn) {
  Write-Error "spectyn.exe not found in any known location"
  exit 1
}
Write-Host "Found spectyn: $spectyn"

Write-Host "Starting spectyn serve as detached scheduled task..."
$logDir = "$env:USERPROFILE\.spectyn-mesh\logs"
if (-not (Test-Path $logDir)) { New-Item -ItemType Directory -Path $logDir | Out-Null }
$out = "$logDir\serve.out.log"
$err = "$logDir\serve.err.log"

# Use schtasks for true detach (survives SSH disconnect). Like launchd on Mac.
$taskName = "SpectynMeshServe"
# Delete existing task if present
schtasks /Delete /TN $taskName /F 2>$null | Out-Null

# Wrapper batch that sets env + launches spectyn + redirects logs.
$wrapper = "$env:USERPROFILE\.spectyn-mesh\serve-wrapper.cmd"
@"
@echo off
set GROQ_API_KEY=$GroqKey
set GEMINI_API_KEY=$GeminiKey
"$spectyn" serve --port 7878 >> "$out" 2>> "$err"
"@ | Set-Content -Path $wrapper -Encoding ASCII

# Create + start a one-off task that runs the wrapper now (no schedule).
schtasks /Create /TN $taskName /TR "`"$wrapper`"" /SC ONCE /ST 00:00 /F | Out-Null
schtasks /Run /TN $taskName | Out-Null

Start-Sleep -Seconds 3
$p = Get-Process -Name "spectyn" -ErrorAction SilentlyContinue | Select-Object -First 1
if ($p) { Write-Host "Started spectyn pid=$($p.Id)" } else { Write-Error "spectyn did not start"; exit 3 }

# Give it 3s to bind, then check
Start-Sleep -Seconds 3
$alive = Get-Process -Id $p.Id -ErrorAction SilentlyContinue
if ($alive) {
  Write-Host "spectyn serve up (pid=$($p.Id))"
} else {
  Write-Error "spectyn died within 3s — check $err"
  exit 2
}

# Sanity: hit localhost healthz
try {
  $r = Invoke-WebRequest -Uri "http://127.0.0.1:7878/healthz" -UseBasicParsing -TimeoutSec 5
  Write-Host "healthz: $($r.StatusCode) $($r.Content)"
} catch {
  Write-Warning "healthz probe failed: $_"
}

Write-Host "DONE"
