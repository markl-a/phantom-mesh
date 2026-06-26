# self-update.ps1 — Windows node auto-catch-up to the latest framework version.
#
# The Windows sibling of self-update.sh (S7). Same exit-code contract and the
# same hardened safety (build-before-swap, ancestor guard against downgrade,
# skip-when-busy/dirty, marker-only-after-verified-restart, detached contract),
# with ONE structural difference forced by Windows: a running phantom.exe holds
# an EXCLUSIVE FILE LOCK, so cargo can't relink it while serve runs (the os
# error 5 we hit live). Order is therefore stop-serve -> build -> start-serve;
# the node is offline for the whole build (the honest cost of the file lock).
#
# Drives the keepalive task (task 4) to stop/start serve, falling back to direct
# process control if the task isn't registered.
#
# Usage:
#   powershell -NoProfile -ExecutionPolicy Bypass -File scripts\dev-loop\self-update.ps1 -Repo C:\Users\<you>\phantom-mesh
# Exit:  0 up-to-date · 1 updated+serve-healthy · 2 build failed (tree restored,
#        old serve restarted) · 3 skipped (busy/dirty) · 4 setup error · 5
#        updated but serve restart UNVERIFIED (tree restored, needs attention)
param(
  [string]$Repo = "$(Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $MyInvocation.MyCommand.Path)))",
  [string]$Base = 'step3-coach-install-schedule',
  [string]$TaskName = 'phantom-serve'
)
$ErrorActionPreference = 'Continue'
$Repo = $Repo.TrimEnd('\')
$state = Join-Path $env:USERPROFILE '.phantom-mesh'
New-Item -ItemType Directory -Force -Path $state | Out-Null
$marker = Join-Path $state 'built-commit'
$port = if ($env:PHANTOM_PORT) { $env:PHANTOM_PORT } else { 7878 }

$logfile = Join-Path $state 'self-update.log'
function GitR { param([Parameter(ValueFromRemainingArguments=$true)]$a) & git -C $Repo @a }
function Healthz { try { (Invoke-WebRequest -UseBasicParsing -TimeoutSec 4 "http://127.0.0.1:$port/healthz").StatusCode } catch { 0 } }
# Tee to a log file as well as the host — Write-Host alone isn't captured by a
# caller's stdout redirect, and a cron/scheduled run needs a durable record.
function Say($m) { $l = "self-update: $m"; Write-Host $l; Add-Content -Path $logfile -Value "$(Get-Date -Format o)  $l" }

if (-not (Test-Path (Join-Path $Repo '.git'))) { Say "no repo at $Repo"; exit 4 }

# ── guards ──────────────────────────────────────────────────────────────────
$cur = (GitR symbolic-ref -q --short HEAD)
if ($cur -match '^(dev|feat)/') { Say "on work branch $cur (mid-task) — skipping"; exit 3 }
$dirty = (GitR status --porcelain) | Where-Object { $_ -notmatch '^\?\?' }
if ($dirty) { Say "tracked changes present — skipping (won't clobber edits)"; exit 3 }

# ── detect ──────────────────────────────────────────────────────────────────
GitR fetch origin "+refs/heads/${Base}:refs/remotes/origin/$Base" 2>$null
if ($LASTEXITCODE -ne 0) { Say "cannot fetch origin/$Base"; exit 4 }
$target = (GitR rev-parse "origin/$Base"); if (-not $target) { Say "no origin/$Base"; exit 4 }
$target = $target.Trim()
# Marker = the commit the installed binary was built from (seeded at arm time).
# Null-safe: an empty marker or a failed `git rev-parse` must not crash on
# .Trim() of $null (review: agy).
$mc = if (Test-Path $marker) { Get-Content $marker -Raw } else { $null }
if ($mc) { $built = $mc.Trim() }
else {
  $h = GitR rev-parse HEAD
  if (-not $h) { Say "cannot resolve HEAD"; exit 4 }
  $built = $h.Trim()
}

if ($target -eq $built) { Say "up-to-date ($(GitR rev-parse --short $target))"; exit 0 }
# Only update when origin is genuinely AHEAD of what we built (built is an
# ancestor of target). Diverged/ahead -> do nothing, never downgrade.
GitR rev-parse --verify -q "$built^{commit}" 2>$null | Out-Null
if ($LASTEXITCODE -eq 0) {
  GitR merge-base --is-ancestor $built $target 2>$null
  if ($LASTEXITCODE -ne 0) {
    Say "built $(GitR rev-parse --short $built) is not an ancestor of origin/$Base ($(GitR rev-parse --short $target)) — diverged/ahead, NOT downgrading"
    exit 0
  }
}

Say "origin/$Base at $(GitR rev-parse --short $target) is ahead of built $(GitR rev-parse --short $built) — updating"

# Capture where the tree was so ANY failure restores it (branch-form preserving).
$or = GitR symbolic-ref -q --short HEAD
if (-not $or) { $or = GitR rev-parse HEAD }
$origRef = "$or".Trim()
function Restore-Tree { GitR checkout -q $origRef 2>$null | Out-Null }

# ── serve control (drives the keepalive task; falls back to direct) ──────────
$script:HasTask = [bool](Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue)
function Stop-Serve {
  # DISABLE the task (not just stop) so a trigger firing during the offline
  # build window can't relaunch serve and re-lock the .exe (review: codex — the
  # exact risk this script exists to avoid). Re-enabled by Start-Serve.
  if ($script:HasTask) {
    Disable-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue | Out-Null
    Stop-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue
  }
  taskkill /f /im phantom.exe 2>$null | Out-Null
  # Wait for the .exe lock to release: healthz must stop returning 200 (≤15s).
  foreach ($i in 1..15) { if ((Healthz) -ne 200) { return $true }; Start-Sleep -Seconds 1 }
  return ((Healthz) -ne 200)
}
function Start-Serve {
  if ($script:HasTask) {
    Enable-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue | Out-Null
    Start-ScheduledTask -TaskName $TaskName
  } else {
    # No keepalive task — launch the loop wrapper detached as a fallback. Single
    # argument string (not array+manual-quotes) so paths with spaces survive
    # (review: agy).
    $loop = Join-Path $Repo 'scripts\dev-cluster\serve-loop.ps1'
    if (Test-Path $loop) {
      Start-Process powershell -WindowStyle Hidden -ArgumentList "-NoProfile -ExecutionPolicy Bypass -File `"$loop`" -Repo `"$Repo`""
    } else {
      $bin = Join-Path $Repo 'core\target\release\phantom.exe'
      if (Test-Path $bin) { Start-Process $bin -ArgumentList 'serve' -WindowStyle Hidden }
    }
  }
  foreach ($i in 1..20) { if ((Healthz) -eq 200) { return $true }; Start-Sleep -Seconds 1 }
  return ((Healthz) -eq 200)
}
# Re-enable the task on ANY exit path (so a disabled task never outlives this
# script and silently stops serve from ever auto-restarting again).
function Ensure-TaskEnabled { if ($script:HasTask) { Enable-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue | Out-Null } }

# ── move source to the new base, then STOP serve before building (file lock) ─
GitR checkout -q --detach $target 2>$null
if ($LASTEXITCODE -ne 0) { Say "cannot checkout $target"; exit 4 }

# Guard cargo BEFORE stopping serve — no point taking the node offline if we
# can't build (review: agy — and a missing cargo with ErrorActionPreference
# 'Continue' would otherwise leave $LASTEXITCODE at the prior git's 0 and FALSELY
# report build success, advancing the marker and stranding the node on the old
# binary forever).
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
  Say "cargo not on PATH — cannot build; leaving serve as-is"
  Restore-Tree
  exit 4
}

if (-not (Stop-Serve)) {
  Say "WARNING — could not stop the old serve (still holding :$port / the .exe lock) — aborting before a doomed build"
  Ensure-TaskEnabled                 # serve is still up; don't leave the task disabled
  Restore-Tree
  exit 2
}

# ── build (single; serve is down so the .exe is writable) ────────────────────
$buildOk = $false
Push-Location (Join-Path $Repo 'core')
try { & cargo build --release --bin phantom; $buildOk = ($LASTEXITCODE -eq 0) }
catch { $buildOk = $false }
Pop-Location
if (-not $buildOk) {
  Say "BUILD FAILED at $(GitR rev-parse --short $target) — restoring old tree, restarting old serve, will retry next run"
  Restore-Tree
  Start-Serve | Out-Null            # re-enables the task + brings the OLD binary back (still on disk)
  exit 2
}

# ── start the new serve, verify healthy, THEN advance the marker ─────────────
if (Start-Serve) {
  ($target) | Out-File -Encoding ascii -NoNewline $marker
  Say "OK updated to $(GitR rev-parse --short $target) — serve restarted and healthy (detached at origin/$Base, per contract)"
  exit 1
}
# Built but serve didn't come back: restore tree (so a no-marker node can't
# deadlock on 'up-to-date'), hold the marker, surface for attention.
Say "WARNING — updated binary to $(GitR rev-parse --short $target) but serve restart UNVERIFIED — tree restored, will retry next run; needs attention"
Restore-Tree
exit 5
