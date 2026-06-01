// /install.ps1 + /dist/* — public binary distribution endpoints.
//
// Goal: any Windows box can install phantom in one command:
//   iwr -useb https://phantommesh.io/install.ps1 | iex
//
// /install.ps1 returns a self-contained PowerShell script (no auth).
// /dist/<name>  streams an R2 object through the Worker (no auth either —
// the binaries are public, the keys vault behind /api/me/* is what's
// authenticated).
//
// CORS: PowerShell's Invoke-WebRequest doesn't enforce CORS, so we don't
// need wildcards here. Browsers fetching .ps1 directly would be blocked
// by SOP, but no one does that — they paste the iwr line into PS.

import type { Context } from "hono";
import type { Env } from "../types";

// Each platform's binary lives in R2 under the key on the right. Keys
// not in this manifest 404 (so a typo doesn't leak arbitrary R2 reads).
// macOS-arm64 currently uploaded; Intel Mac + Linux entries return
// 503 until those binaries land in R2.
const BIN_OBJECTS: Record<string, { object: string; contentType: string }> = {
  "phantom-windows-x86_64.exe": {
    object: "phantom-windows-x86_64.exe",
    contentType: "application/octet-stream",
  },
  "phantom-darwin-arm64": {
    object: "phantom-darwin-arm64",
    contentType: "application/octet-stream",
  },
  "phantom-darwin-x86_64": {
    object: "phantom-darwin-x86_64",
    contentType: "application/octet-stream",
  },
  "phantom-linux-x86_64": {
    object: "phantom-linux-x86_64",
    contentType: "application/octet-stream",
  },
};

/// GET /dist/<name> — stream the binary out of R2.
///
/// Cache strategy: `must-revalidate` + ETag. The 1-hour `max-age=3600` we
/// had before was a footgun — a fresh upload took up to an hour to reach
/// users, breaking the "iwr | iex" one-liner workflow. Now every client
/// hit conditional-GETs against R2's ETag, so an unchanged binary still
/// 304s (zero body transfer) but a new upload is picked up immediately.
/// 404 when the name isn't in the manifest above (so a typo doesn't leak
/// arbitrary R2 reads).
export async function distHandler(c: Context<{ Bindings: Env }>) {
  const name = c.req.param("name") ?? "";
  const entry = BIN_OBJECTS[name];
  if (!entry) {
    return c.json({ error: `unknown binary '${name}'`, available: Object.keys(BIN_OBJECTS) }, 404);
  }

  // Conditional GET: if the client's If-None-Match matches the live R2
  // ETag, return 304 with no body. Saves bandwidth when re-running the
  // installer hasn't actually changed the binary.
  const ifNoneMatch = c.req.header("If-None-Match");
  if (ifNoneMatch) {
    const head = await c.env.BINARIES.head(entry.object);
    if (head && head.httpEtag === ifNoneMatch) {
      return new Response(null, {
        status: 304,
        headers: {
          "ETag":          head.httpEtag,
          "Cache-Control": "public, max-age=0, must-revalidate",
        },
      });
    }
  }

  const obj = await c.env.BINARIES.get(entry.object);
  if (!obj) {
    return c.json({ error: `binary '${name}' missing in R2 bucket` }, 503);
  }
  return new Response(obj.body, {
    headers: {
      "Content-Type":   entry.contentType,
      "Content-Length": String(obj.size),
      "Cache-Control":  "public, max-age=0, must-revalidate",
      "ETag":           obj.httpEtag,
    },
  });
}

/// GET /install.ps1 — return a fresh PowerShell installer.
/// Embeds the current host so the script downloads from wherever the user
/// fetched it (lets dev/staging deploys serve their own binaries without
/// a hardcoded URL).
export function installScript(c: Context<{ Bindings: Env }>) {
  const host = c.req.header("Host") ?? "phantommesh.io";
  const scheme = c.env.APP_URL.startsWith("https") ? "https" : "http";
  const baseUrl = `${scheme}://${host}`;
  const script = renderInstallPs1(baseUrl);
  return new Response(script, {
    headers: {
      "Content-Type":  "text/plain; charset=utf-8",
      "Cache-Control": "public, max-age=60",
    },
  });
}

// Exported so scripts/check-install-ps1.ts can render + validate the
// embedded PowerShell regex literals at CI time. Without that gate the
// `\s` → `s` template-literal escape bug ships to users.
export function renderInstallPs1(baseUrl: string): string {
  return `# phantom mesh — Windows installer
# Run via:
#   iwr -useb ${baseUrl}/install.ps1 | iex
#
# What it does:
#   1. Downloads phantom.exe to ~/.local/bin (no admin needed)
#   2. Adds ~/.local/bin to user PATH if missing
#   3. Auto-runs \`phantom login\` to bind to broker + pull LLM keys
#      (skipped on re-install if you already have a fresh token saved).
#      Set $env:PHANTOM_INSTALL_SKIP_LOGIN='1' before piping to skip.
#
# Re-running upgrades the binary in place (kills any running phantom first).
# Does NOT touch ~/.phantom-mesh/ — your config + auth survive upgrades.

$ErrorActionPreference = 'Stop'

$BIN_URL  = '${baseUrl}/dist/phantom-windows-x86_64.exe'
$BIN_DIR  = Join-Path $env:USERPROFILE '.local\\bin'
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
if (Test-Path "$env:USERPROFILE\\bin") {
    Copy-Item $BIN_PATH "$env:USERPROFILE\\bin\\phantom.exe" -Force
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
# "duplicate key" in TOML parsing). If \`phantom providers list\` errors
# out, back up + replace.
$shouldSeed = -not (Test-Path $cfgPath)
if (-not $shouldSeed) {
    # Use cmd /c so PS 5.1's native-stderr-as-ErrorRecord quirk doesn't
    # fire $ErrorActionPreference=Stop on a phantom error message --
    # we EXPECT the command to fail when the file is corrupt, that's
    # the whole point of running it. cmd dumps stderr into the same
    # output stream which we then grep for the parse error keywords.
    $tmpOut = Join-Path $env:TEMP "phantom-providers-check-$PID.txt"
    & cmd /c "\`"$BIN_PATH\`" providers list > \`"$tmpOut\`" 2>&1"
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
# Edit freely. \`phantom config pull\` only touches ~/.phantom-mesh/env
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
# \`phantom config pull\`.
$cfgText = Get-Content $cfgPath -Raw -ErrorAction SilentlyContinue
# Note: backslashes are doubled so the TS template literal outputs them
# verbatim — '\\[' here renders as '\[' in the PS source. Without that,
# TS eats the backslash, PowerShell sees '[providers.opencode]' as a
# regex character class (matches any one of p,r,o,v,i,d,e,s,.,n,c)
# which matches EVERY file -> -notmatch is always false -> block never
# gets appended. That was the actual root cause for the node-b machine.
if ($cfgText -and $cfgText -notmatch '\\[providers\\.opencode\\]') {
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
# Note: backslashes are doubled in regex literals (\\s, \\[, etc.) so
# the TS template literal outputs them verbatim into the PowerShell
# source. Without this the rendered PS sees 'providerss*=s*[' which
# is an unterminated character class — same bug pattern that bit us
# on the [providers.opencode] block check earlier.
$cfgText = Get-Content $cfgPath -Raw -ErrorAction SilentlyContinue
$hasMultiProvider = ($cfgText -match 'providers\\s*=\\s*\\[') -and ($cfgText -match '"groq:')
if (-not $hasMultiProvider) {
    Write-Host '  setting [agent.master].providers = [opencode, groq, openrouter, local-ollama]  (multi-provider failover)'
    & cmd /c "\`"$BIN_PATH\`" providers priority master \`"opencode:minimax-m2.5-free\`" \`"groq:llama-3.3-70b-versatile\`" \`"openrouter:meta-llama/llama-3.3-70b-instruct:free\`" \`"local-ollama:qwen3:8b\`" >nul 2>nul"
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
    Write-Host 'Run \`phantom login\` later to bind this device + pull LLM keys.'
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
$envFile = "$env:USERPROFILE\\.phantom-mesh\\env"
if (Test-Path $envFile) {
    Get-Content $envFile | ForEach-Object {
        if ($_ -match '^([A-Z_][A-Z0-9_]*)=(.*)$') {
            [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], 'Process')
        }
    }
}

# 5c-pre. Empty-vault check.
# A brand-new account has no LLM keys saved on the broker — phantom
# config pull then writes an env file that contains only CLUSTER_SECRET
# (or nothing at all), and the agent runtime 401s on the very first
# prompt with no clear hint about why. Detect that case here so we can
# (a) print a clear "go set keys at /account first" instruction and
# (b) skip the TUI auto-launch at the bottom (would just immediately
# fail in raw-mode and confuse the user).
$llmKeyVars = @(
    'OPENAI_API_KEY','ANTHROPIC_API_KEY','GROQ_API_KEY',
    'GOOGLE_API_KEY','GEMINI_API_KEY','OPENROUTER_API_KEY',
    'OPENCODE_API_KEY','MISTRAL_API_KEY','TOGETHER_API_KEY',
    'CEREBRAS_API_KEY','DEEPSEEK_API_KEY','NVIDIA_API_KEY'
)
$hasAnyLlmKey = $false
foreach ($var in $llmKeyVars) {
    $val = [Environment]::GetEnvironmentVariable($var, 'Process')
    if (-not [string]::IsNullOrEmpty($val)) { $hasAnyLlmKey = $true; break }
}
$emptyVaultHint = -not $hasAnyLlmKey

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
        Write-Host "  ! schtasks /Create failed (rc=\$rc) -- falling back to background launch"
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
$cfg = "$env:USERPROFILE\\.phantom-mesh\\agents.toml"
if (Test-Path $cfg) {
    $portLine = Select-String -Path $cfg -Pattern '^\\s*port\\s*=\\s*(\\d+)' -ErrorAction SilentlyContinue
    if ($portLine) { $port = [int]$portLine.Matches[0].Groups[1].Value }
}
try {
    $r = Invoke-WebRequest -Uri "http://127.0.0.1:$port/healthz" -UseBasicParsing -TimeoutSec 3
    Write-Host "  healthz on :$port -> $($r.StatusCode) $($r.Content)"
} catch {
    Write-Host "  ! healthz probe failed on :$port : $_"
    Write-Host '    (run \`phantom doctor\` to diagnose)'
}

Write-Host ''
Write-Host '=== installed ==='
Write-Host ''

# 7. Empty-vault hint — printed before the launch prompt so it's the
# last thing the user sees if they have no LLM keys saved yet.
# Auto-launching the TUI in that state just produces a 401 on the
# first prompt with no clear next-step, so we suppress the launch
# (same effect as PHANTOM_INSTALL_NOLAUNCH=1) and tell them where to go.
if ($emptyVaultHint) {
    Write-Host '! No LLM provider keys saved on your account yet.'
    Write-Host ''
    Write-Host '  Open https://phantommesh.io/account in a browser and save'
    Write-Host '  at least one provider key (Groq has a free tier and is the'
    Write-Host '  fastest to get started — link inline on that page).'
    Write-Host ''
    Write-Host '  Once saved, on this machine:'
    Write-Host '     phantom config pull   # fetch the new keys to ~/.phantom-mesh/env'
    Write-Host '     phantom               # start the TUI'
    Write-Host ''
    Write-Host '  (Skipping TUI auto-launch — would just 401 immediately without keys.)'
}

# 8. Drop the user straight into the TUI. Read-Host waits for Enter so
#    the workflow reads "iwr -useb ... | iex; <Enter> -> TUI" — no need
#    to remember the binary name. Ctrl-C still works to back out.
#    Set $env:PHANTOM_INSTALL_NOLAUNCH='1' before piping for CI / scripted
#    setups that don't want a TUI to appear. Empty-vault hint above also
#    suppresses launch so a brand-new user lands on a clear instruction
#    instead of a cryptic auth failure.
$noLaunch = ($env:PHANTOM_INSTALL_NOLAUNCH -eq '1') -or $emptyVaultHint
if ($noLaunch) {
    if (-not $emptyVaultHint) {
        Write-Host 'PHANTOM_INSTALL_NOLAUNCH=1 set -- skipping launch.'
        Write-Host 'Run this when ready:'
        Write-Host '   phantom         # the TUI'
    }
} else {
    Write-Host 'Press Enter to start phantom (or Ctrl-C to exit and run it later).'
    [void](Read-Host)
    & $BIN_PATH
}
`;
}

/// GET /install.sh — POSIX shell installer for macOS + Linux.
/// Mirrors installScript's dynamic-host trick so dev/staging deploys
/// serve their own binaries without a hardcoded URL.
export function installShellScript(c: Context<{ Bindings: Env }>) {
  const host = c.req.header("Host") ?? "phantommesh.io";
  const scheme = c.env.APP_URL.startsWith("https") ? "https" : "http";
  const baseUrl = `${scheme}://${host}`;
  const script = renderInstallSh(baseUrl);
  return new Response(script, {
    headers: {
      "Content-Type":  "text/plain; charset=utf-8",
      "Cache-Control": "public, max-age=60",
    },
  });
}

export function renderInstallSh(baseUrl: string): string {
  return `#!/bin/sh
# phantom mesh — macOS / Linux installer
# Run via:
#   curl -fsSL ${baseUrl}/install.sh | sh
#
# What it does (mirrors install.ps1 step-for-step):
#   1. Detects OS + arch (darwin/linux × arm64/x86_64)
#   2. Stops running phantom, downloads binary to ~/.local/bin
#   3. (macOS) ad-hoc resigns + adds ~/.local/bin to PATH
#   4. Verifies version
#   5. (macOS) installs launchd plist for 'phantom serve' auto-start
#   6. Seeds ~/.phantom-mesh/agents.toml on FIRST install + ensures
#      [providers.opencode] block + sets [agent.master] priority list
#      (auto-recovers from corrupt TOML by backing up + reseeding)
#   7. Auto-runs 'phantom login' to bind to broker + pull LLM keys
#      (incl. CLUSTER_SECRET). Login itself also auto-registers this
#      machine on the cluster + writes the [cluster] block to
#      agents.toml. Skip with PHANTOM_INSTALL_SKIP_LOGIN=1.
#   8. (macOS) Verifies cluster wiring via 'phantom cluster status'.
#      Skip with PHANTOM_INSTALL_SKIP_CLUSTER=1.
#   Tail: healthz probe + run-this-next hint. We don't auto-launch the
#      TUI under curl|sh (the pipe breaks raw-mode init).
#
# Re-running upgrades the binary in place.
# Does NOT touch ~/.phantom-mesh/ — your config + auth survive upgrades.
#
# Opt-outs:
#   PHANTOM_INSTALL_NO_LAUNCHD=1     skip macOS launchd plist install
#   PHANTOM_INSTALL_SKIP_LOGIN=1     skip 'phantom login'
#   PHANTOM_INSTALL_SKIP_CLUSTER=1   skip cluster status check

set -e

case "\$(uname -s)" in
  Darwin) os=darwin ;;
  Linux)  os=linux  ;;
  *) echo "phantom: unsupported OS '\$(uname -s)' (need Darwin or Linux)"; exit 1 ;;
esac
case "\$(uname -m)" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64)  arch=x86_64 ;;
  *) echo "phantom: unsupported arch '\$(uname -m)' (need arm64 or x86_64)"; exit 1 ;;
esac

if [ "\$os" = "linux" ] && [ "\$arch" = "arm64" ]; then
  echo "phantom: linux/arm64 build not yet available — see https://phantommesh.io"
  exit 1
fi

asset="phantom-\${os}-\${arch}"
url="${baseUrl}/dist/\${asset}"
bin_dir="\$HOME/.local/bin"
bin_path="\$bin_dir/phantom"

echo "=== phantom mesh installer ==="
echo "  source:  \$url"
echo "  target:  \$bin_path"
echo

# 1. Stop any running phantom (and let launchd quiet for a moment so
# amfid doesn't try to validate the half-written binary)
echo "[1/8] stopping running phantom..."
if [ "\$os" = "darwin" ]; then
  launchctl bootout "gui/\$(id -u)/ai.phantommesh.serve" 2>/dev/null || true
fi
pkill -f "phantom serve" 2>/dev/null || true
sleep 0.5

echo "[2/8] downloading..."
mkdir -p "\$bin_dir"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "\$url" -o "\$bin_path"
elif command -v wget >/dev/null 2>&1; then
  wget -q "\$url" -O "\$bin_path"
else
  echo "phantom: need curl or wget to download"; exit 1
fi
chmod +x "\$bin_path"
size_bytes=\$(wc -c < "\$bin_path" | tr -d ' ')
echo "  -> \$bin_path (\$(( size_bytes / 1024 / 1024 )) MB)"

# 3. macOS: strip Gatekeeper quarantine + ad-hoc re-sign on this machine.
# Re-signing on the install host avoids the amfid SIGKILL race that hits
# when an mmap'd binary gets overwritten by curl while a launchd process
# was about to exec it (see commit 85c8377 for the same issue under
# 'phantom service install').
if [ "\$os" = "darwin" ]; then
  xattr -d com.apple.quarantine "\$bin_path" 2>/dev/null || true
  if command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - "\$bin_path" 2>/dev/null || true
  fi
fi

echo "[3/8] ensuring PATH contains \$bin_dir..."
case ":\$PATH:" in
  *":\$bin_dir:"*) echo "  -> already in PATH" ;;
  *)
    line='export PATH="\$HOME/.local/bin:\$PATH"'
    for rc in "\$HOME/.zshrc" "\$HOME/.bashrc"; do
      if [ -f "\$rc" ] && ! grep -q "\\.local/bin" "\$rc"; then
        echo "" >> "\$rc"
        echo "# Added by phantom mesh installer" >> "\$rc"
        echo "\$line" >> "\$rc"
      fi
    done
    export PATH="\$bin_dir:\$PATH"
    echo "  -> appended to ~/.zshrc + ~/.bashrc (open a new terminal to pick up)"
    ;;
esac

echo "[4/8] verifying..."
"\$bin_path" --version

if [ "\$os" = "darwin" ] && [ "\${PHANTOM_INSTALL_NO_LAUNCHD:-0}" != "1" ]; then
  echo "[5/8] installing launchd plist for 'phantom serve' auto-start..."
  plist="\$HOME/Library/LaunchAgents/ai.phantommesh.serve.plist"
  mkdir -p "\$HOME/Library/LaunchAgents" "\$HOME/.phantom-mesh"
  cat > "\$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.phantommesh.serve</string>
  <key>ProgramArguments</key>
  <array>
    <string>\$bin_path</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>\$HOME/.phantom-mesh/serve.out.log</string>
  <key>StandardErrorPath</key><string>\$HOME/.phantom-mesh/serve.err.log</string>
</dict>
</plist>
PLIST
  launchctl bootstrap "gui/\$(id -u)" "\$plist" 2>/dev/null || launchctl load "\$plist" 2>/dev/null || true
  echo "  -> ai.phantommesh.serve loaded (logs: ~/.phantom-mesh/serve.{out,err}.log)"
else
  echo "[5/8] skipped launchd plist (linux or PHANTOM_INSTALL_NO_LAUNCHD=1)"
fi

# 6. Seed ~/.phantom-mesh/agents.toml on FIRST install only.
# Mirrors install.ps1 §4b/§4c — without a [agent.master].providers list,
# runtime falls back to alphabetical of all configured providers, and
# stale groq keys silently 401 every prompt with no failover. Pin master
# to opencode:minimax-m2.5-free (verified to actually call tools) and
# leave user free to edit. NEVER overwrite an existing valid file.
echo "[6/8] ensuring agents.toml has master priority + opencode block..."
cfg_dir="\$HOME/.phantom-mesh"
cfg_path="\$cfg_dir/agents.toml"
mkdir -p "\$cfg_dir"

should_seed=0
if [ ! -f "\$cfg_path" ]; then
  should_seed=1
else
  # Detect corrupt config (TOML parse error / duplicate-key from older
  # install scripts). Same recovery logic as install.ps1.
  providers_out=\$("\$bin_path" providers list 2>&1 || true)
  if echo "\$providers_out" | grep -qE 'TOML parse error|duplicate key'; then
    ts=\$(date +%s)
    bak="\$cfg_path.bak-\${ts}-corrupt"
    echo "  ! existing agents.toml fails TOML parse — backing up + reseeding"
    cp "\$cfg_path" "\$bak"
    echo "  backup: \$bak"
    rm -f "\$cfg_path"
    should_seed=1
  fi
fi

if [ "\$should_seed" = "1" ]; then
  echo "  seeding default agents.toml"
  cat > "\$cfg_path" <<'TOML'
# phantom-mesh default config — written by phantommesh.io install.sh
# Edit freely. \`phantom config pull\` only touches ~/.phantom-mesh/env
# (LLM API keys + CLUSTER_SECRET), never this file.

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
#   opencode:minimax-m2.5-free                          free, vault-managed key, has tool support
#   groq:llama-3.3-70b-versatile                        free, very fast, second opinion
#   openrouter:meta-llama/llama-3.3-70b-instruct:free   free with openrouter key
#   local-ollama:qwen3:8b                               offline last-resort (only if Ollama installed)
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

[providers.local-ollama]
type          = "openai-compat"
base_url      = "http://127.0.0.1:11434/v1"
default_model = "qwen3:8b"
TOML
  echo "  -> \$cfg_path"
else
  echo "  agents.toml at \$cfg_path parses OK — left mostly untouched"
fi

# Always (re)ensure [providers.opencode] block exists, even on existing
# configs that pre-date this installer.
if ! grep -qE '^\\[providers\\.opencode\\]' "\$cfg_path" 2>/dev/null; then
  echo "  appending [providers.opencode] block (was missing)"
  cat >> "\$cfg_path" <<'BLOCK'

[providers.opencode]
type          = "opencode"
base_url      = "https://opencode.ai/zen/v1"
api_key_env   = "OPENCODE_API_KEY"
default_model = "minimax-m2.5-free"
BLOCK
fi
# Re-set master priority ONLY when the file contains the legacy
# single-provider line. Existing users who manually edited the chain
# via /priority should not have their order silently overwritten on
# every install. Mirrors install.ps1's gating logic.
cfg_text=\$(cat "\$cfg_path" 2>/dev/null || echo "")
single_legacy=\$(echo "\$cfg_text" | grep -E '^\\s*providers\\s*=\\s*\\["opencode:minimax-m2.5-free"\\]\\s*\$' | head -1)
has_groq_in_chain=\$(echo "\$cfg_text" | grep -E '"groq:' | head -1)
if [ -z "\$has_groq_in_chain" ] || [ -n "\$single_legacy" ]; then
  echo '  setting [agent.master].providers = [opencode, groq, openrouter, local-ollama]  (multi-provider failover)'
  "\$bin_path" providers priority master \\
    "opencode:minimax-m2.5-free" \\
    "groq:llama-3.3-70b-versatile" \\
    "openrouter:meta-llama/llama-3.3-70b-instruct:free" \\
    "local-ollama:qwen3:8b" >/dev/null 2>&1 || true
else
  echo '  [agent.master].providers already has multi-provider chain — leaving alone'
fi

# 7. Auto-login + pull LLM keys (and CLUSTER_SECRET, used by step 8).
# Re-runs the OAuth dance on a brand-new install; on a re-install where
# auth.json already has a non-expired broker_token, login short-circuits
# to a key refresh (no browser).
if [ "\${PHANTOM_INSTALL_SKIP_LOGIN:-0}" = "1" ]; then
  echo
  echo "[7/8] PHANTOM_INSTALL_SKIP_LOGIN=1 set — skipping login."
  echo '   Run \`phantom login\` later to bind this device + pull LLM keys.'
elif ! (exec </dev/tty) 2>/dev/null; then
  # \`[ -e /dev/tty ]\` is true even from a non-interactive subshell where
  # /dev/tty exists but isn't actually attached — the open() then fails
  # with "Device not configured" mid-login (seen in the 2026-05-04 e2e
  # repro from a Claude-spawned bash). Probe by trying to open it for
  # read in a subshell; that's the only reliable check.
  echo
  echo "[7/8] /dev/tty not openable (non-interactive shell) — skipping login."
  echo '   Run \`phantom login\` manually after install completes.'
else
  echo
  echo "[7/8] === running phantom login ==="
  echo "   (opens browser -> Google sign-in -> auto-pulls LLM keys + CLUSTER_SECRET."
  echo "    Press Ctrl-C to skip; you can run phantom login later.)"
  echo
  "\$bin_path" login </dev/tty || true
fi

# 8. Verify cluster wiring. \`phantom login\` already auto-registers this
# machine + writes the [cluster] block (it pulls peers from the broker
# and calls cluster_join_lines internally), so an explicit cluster-join
# call here would just rewrite the block redundantly. We instead probe
# \`phantom cluster status\` — if that errors out (most often: bad TOML
# left by the login auto-write), surface the recovery hint.
if [ "\$os" = "darwin" ] && [ "\${PHANTOM_INSTALL_SKIP_CLUSTER:-0}" != "1" ]; then
  echo
  echo "[8/8] === cluster status ==="
  status_out=\$("\$bin_path" cluster status 2>&1 || true)
  echo "\$status_out"
  if echo "\$status_out" | grep -qE 'TOML parse error|not valid TOML'; then
    echo
    echo "  ! agents.toml is broken — 'phantom login' may have written"
    echo "    an invalid [cluster] block. Manual recovery:"
    echo "      \$EDITOR ~/.phantom-mesh/agents.toml    # fix or delete [cluster]"
    echo "      phantom cluster join mac-coordinator   # rewrite cleanly"
  fi
elif [ "\$os" = "linux" ]; then
  echo
  echo "[8/8] cluster: skipped (Linux — no default node name in topology)"
  echo "   set up manually after install:"
  echo "     phantom cluster join <name>     # see 'phantom cluster --help'"
fi

# Restart serve so newly-pulled env keys (and updated [cluster] block)
# take effect — env + agents.toml are both read at process start.
if [ "\$os" = "darwin" ]; then
  launchctl kickstart -k "gui/\$(id -u)/ai.phantommesh.serve" 2>/dev/null || true
fi
sleep 1

# Healthz probe — read [core].port from agents.toml so a customized
# port still gets probed correctly.
port=7878
if [ -f "\$cfg_path" ]; then
  pl=\$(grep -E '^[[:space:]]*port[[:space:]]*=' "\$cfg_path" | head -1 | sed -E 's/.*=[[:space:]]*([0-9]+).*/\\1/')
  if [ -n "\$pl" ]; then port="\$pl"; fi
fi
hz=\$(curl -s -m 3 -o /dev/null -w "%{http_code}" "http://127.0.0.1:\$port/healthz" 2>/dev/null || echo "000")
if [ "\$hz" = "200" ]; then
  echo "  healthz on :\$port -> 200 ok"
else
  echo "  ! healthz probe on :\$port returned \$hz (run 'phantom doctor' to diagnose)"
fi

echo
echo "=== installed ==="
echo

# Empty-vault check: a brand-new account has no LLM keys saved on the
# broker yet. \`phantom login\` then drops an env file with only
# CLUSTER_SECRET (or nothing). The TUI would 401 on the first prompt
# with no clear next-step. Detect by re-reading the env file and grep
# for any well-known LLM provider key.
empty_vault=1
if [ -f "\$HOME/.phantom-mesh/env" ]; then
  if grep -qE '^(OPENAI|ANTHROPIC|GROQ|GOOGLE|GEMINI|OPENROUTER|OPENCODE|MISTRAL|TOGETHER|CEREBRAS|DEEPSEEK|NVIDIA)_API_KEY=..' "\$HOME/.phantom-mesh/env"; then
    empty_vault=0
  fi
fi

if [ "\$empty_vault" = "1" ]; then
  echo "! No LLM provider keys saved on your account yet."
  echo
  echo "  Open https://phantommesh.io/account in a browser and save at"
  echo "  least one provider key (Groq has a free tier and is the fastest"
  echo "  way to get started — link inline on that page)."
  echo
  echo "  Once saved, on this machine:"
  echo "     phantom config pull   # fetch the new keys to ~/.phantom-mesh/env"
  echo "     phantom               # start the TUI"
  echo
  exit 0
fi

# Print run-this-next hint. We deliberately do NOT auto-launch the TUI:
# under 'curl | sh', sh inherits stdin from the curl pipe (script body),
# so an 'exec phantom </dev/tty' only redirects stdin — stdout/stderr
# are still pipe-attached, which fails the TUI's raw-mode init with
# "Failed to initialize input reader". install.ps1 gets away with this
# because PowerShell's Read-Host bypasses the pipe; sh has no equivalent
# clean enough to be worth the fragility. The user just types 'phantom'
# in their existing terminal after the installer exits.
echo "Run when ready:"
echo "   phantom         # the TUI"
echo "   phantom doctor  # cluster + provider sanity check"
`;
}
