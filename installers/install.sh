#!/bin/sh
# phantom mesh — macOS / Linux installer
# Run via:
#   curl -fsSL https://phantommesh.io/install.sh | sh
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

case "$(uname -s)" in
  Darwin) os=darwin ;;
  Linux)  os=linux  ;;
  *) echo "phantom: unsupported OS '$(uname -s)' (need Darwin or Linux)"; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) arch=arm64 ;;
  x86_64|amd64)  arch=x86_64 ;;
  *) echo "phantom: unsupported arch '$(uname -m)' (need arm64 or x86_64)"; exit 1 ;;
esac

if [ "$os" = "linux" ] && [ "$arch" = "arm64" ]; then
  echo "phantom: linux/arm64 build not yet available — see https://phantommesh.io"
  exit 1
fi

asset="phantom-${os}-${arch}"
url="https://phantommesh.io/dist/${asset}"
bin_dir="$HOME/.local/bin"
bin_path="$bin_dir/phantom"

echo "=== phantom mesh installer ==="
echo "  source:  $url"
echo "  target:  $bin_path"
echo

# 1. Stop any running phantom (and let launchd quiet for a moment so
# amfid doesn't try to validate the half-written binary)
echo "[1/8] stopping running phantom..."
if [ "$os" = "darwin" ]; then
  launchctl bootout "gui/$(id -u)/ai.phantommesh.serve" 2>/dev/null || true
fi
pkill -f "phantom serve" 2>/dev/null || true
sleep 0.5

echo "[2/8] downloading..."
mkdir -p "$bin_dir"
if command -v curl >/dev/null 2>&1; then
  curl -fsSL "$url" -o "$bin_path"
elif command -v wget >/dev/null 2>&1; then
  wget -q "$url" -O "$bin_path"
else
  echo "phantom: need curl or wget to download"; exit 1
fi
chmod +x "$bin_path"
size_bytes=$(wc -c < "$bin_path" | tr -d ' ')
echo "  -> $bin_path ($(( size_bytes / 1024 / 1024 )) MB)"

# 3. macOS: strip Gatekeeper quarantine + ad-hoc re-sign on this machine.
# Re-signing on the install host avoids the amfid SIGKILL race that hits
# when an mmap'd binary gets overwritten by curl while a launchd process
# was about to exec it (see commit 85c8377 for the same issue under
# 'phantom service install').
if [ "$os" = "darwin" ]; then
  xattr -d com.apple.quarantine "$bin_path" 2>/dev/null || true
  if command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - "$bin_path" 2>/dev/null || true
  fi
fi

echo "[3/8] ensuring PATH contains $bin_dir..."
case ":$PATH:" in
  *":$bin_dir:"*) echo "  -> already in PATH" ;;
  *)
    line='export PATH="$HOME/.local/bin:$PATH"'
    for rc in "$HOME/.zshrc" "$HOME/.bashrc"; do
      if [ -f "$rc" ] && ! grep -q "\.local/bin" "$rc"; then
        echo "" >> "$rc"
        echo "# Added by phantom mesh installer" >> "$rc"
        echo "$line" >> "$rc"
      fi
    done
    export PATH="$bin_dir:$PATH"
    echo "  -> appended to ~/.zshrc + ~/.bashrc (open a new terminal to pick up)"
    ;;
esac

echo "[4/8] verifying..."
"$bin_path" --version

if [ "$os" = "darwin" ] && [ "${PHANTOM_INSTALL_NO_LAUNCHD:-0}" != "1" ]; then
  echo "[5/8] installing launchd plist for 'phantom serve' auto-start..."
  plist="$HOME/Library/LaunchAgents/ai.phantommesh.serve.plist"
  mkdir -p "$HOME/Library/LaunchAgents" "$HOME/.phantom-mesh"
  cat > "$plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>ai.phantommesh.serve</string>
  <key>ProgramArguments</key>
  <array>
    <string>$bin_path</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>StandardOutPath</key><string>$HOME/.phantom-mesh/serve.out.log</string>
  <key>StandardErrorPath</key><string>$HOME/.phantom-mesh/serve.err.log</string>
</dict>
</plist>
PLIST
  launchctl bootstrap "gui/$(id -u)" "$plist" 2>/dev/null || launchctl load "$plist" 2>/dev/null || true
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
cfg_dir="$HOME/.phantom-mesh"
cfg_path="$cfg_dir/agents.toml"
mkdir -p "$cfg_dir"

should_seed=0
if [ ! -f "$cfg_path" ]; then
  should_seed=1
else
  # Detect corrupt config (TOML parse error / duplicate-key from older
  # install scripts). Same recovery logic as install.ps1.
  providers_out=$("$bin_path" providers list 2>&1 || true)
  if echo "$providers_out" | grep -qE 'TOML parse error|duplicate key'; then
    ts=$(date +%s)
    bak="$cfg_path.bak-${ts}-corrupt"
    echo "  ! existing agents.toml fails TOML parse — backing up + reseeding"
    cp "$cfg_path" "$bak"
    echo "  backup: $bak"
    rm -f "$cfg_path"
    should_seed=1
  fi
fi

if [ "$should_seed" = "1" ]; then
  echo "  seeding default agents.toml"
  cat > "$cfg_path" <<'TOML'
# phantom-mesh default config — written by phantommesh.io install.sh
# Edit freely. `phantom config pull` only touches ~/.phantom-mesh/env
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
  echo "  -> $cfg_path"
else
  echo "  agents.toml at $cfg_path parses OK — left mostly untouched"
fi

# Always (re)ensure [providers.opencode] block exists, even on existing
# configs that pre-date this installer.
if ! grep -qE '^\[providers\.opencode\]' "$cfg_path" 2>/dev/null; then
  echo "  appending [providers.opencode] block (was missing)"
  cat >> "$cfg_path" <<'BLOCK'

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
cfg_text=$(cat "$cfg_path" 2>/dev/null || echo "")
single_legacy=$(echo "$cfg_text" | grep -E '^\s*providers\s*=\s*\["opencode:minimax-m2.5-free"\]\s*$' | head -1)
has_groq_in_chain=$(echo "$cfg_text" | grep -E '"groq:' | head -1)
if [ -z "$has_groq_in_chain" ] || [ -n "$single_legacy" ]; then
  echo '  setting [agent.master].providers = [opencode, groq, openrouter, local-ollama]  (multi-provider failover)'
  "$bin_path" providers priority master \
    "opencode:minimax-m2.5-free" \
    "groq:llama-3.3-70b-versatile" \
    "openrouter:meta-llama/llama-3.3-70b-instruct:free" \
    "local-ollama:qwen3:8b" >/dev/null 2>&1 || true
else
  echo '  [agent.master].providers already has multi-provider chain — leaving alone'
fi

# 7. Auto-login + pull LLM keys (and CLUSTER_SECRET, used by step 8).
# Re-runs the OAuth dance on a brand-new install; on a re-install where
# auth.json already has a non-expired broker_token, login short-circuits
# to a key refresh (no browser).
if [ "${PHANTOM_INSTALL_SKIP_LOGIN:-0}" = "1" ]; then
  echo
  echo "[7/8] PHANTOM_INSTALL_SKIP_LOGIN=1 set — skipping login."
  echo '   Run `phantom login` later to bind this device + pull LLM keys.'
elif ! (exec </dev/tty) 2>/dev/null; then
  # `[ -e /dev/tty ]` is true even from a non-interactive subshell where
  # /dev/tty exists but isn't actually attached — the open() then fails
  # with "Device not configured" mid-login (seen in the 2026-05-04 e2e
  # repro from a Claude-spawned bash). Probe by trying to open it for
  # read in a subshell; that's the only reliable check.
  echo
  echo "[7/8] /dev/tty not openable (non-interactive shell) — skipping login."
  echo '   Run `phantom login` manually after install completes.'
else
  echo
  echo "[7/8] === running phantom login ==="
  echo "   (opens browser -> Google sign-in -> auto-pulls LLM keys + CLUSTER_SECRET."
  echo "    Press Ctrl-C to skip; you can run phantom login later.)"
  echo
  "$bin_path" login </dev/tty || true
fi

# 8. Verify cluster wiring. `phantom login` already auto-registers this
# machine + writes the [cluster] block (it pulls peers from the broker
# and calls cluster_join_lines internally), so an explicit cluster-join
# call here would just rewrite the block redundantly. We instead probe
# `phantom cluster status` — if that errors out (most often: bad TOML
# left by the login auto-write), surface the recovery hint.
if [ "$os" = "darwin" ] && [ "${PHANTOM_INSTALL_SKIP_CLUSTER:-0}" != "1" ]; then
  echo
  echo "[8/8] === cluster status ==="
  status_out=$("$bin_path" cluster status 2>&1 || true)
  echo "$status_out"
  if echo "$status_out" | grep -qE 'TOML parse error|not valid TOML'; then
    echo
    echo "  ! agents.toml is broken — 'phantom login' may have written"
    echo "    an invalid [cluster] block. Manual recovery:"
    echo "      $EDITOR ~/.phantom-mesh/agents.toml    # fix or delete [cluster]"
    echo "      phantom cluster join mac-coordinator   # rewrite cleanly"
  fi
elif [ "$os" = "linux" ]; then
  echo
  echo "[8/8] cluster: skipped (Linux — no default node name in topology)"
  echo "   set up manually after install:"
  echo "     phantom cluster join <name>     # see 'phantom cluster --help'"
fi

# Restart serve so newly-pulled env keys (and updated [cluster] block)
# take effect — env + agents.toml are both read at process start.
if [ "$os" = "darwin" ]; then
  launchctl kickstart -k "gui/$(id -u)/ai.phantommesh.serve" 2>/dev/null || true
fi
sleep 1

# Healthz probe — read [core].port from agents.toml so a customized
# port still gets probed correctly.
port=7878
if [ -f "$cfg_path" ]; then
  pl=$(grep -E '^[[:space:]]*port[[:space:]]*=' "$cfg_path" | head -1 | sed -E 's/.*=[[:space:]]*([0-9]+).*/\1/')
  if [ -n "$pl" ]; then port="$pl"; fi
fi
hz=$(curl -s -m 3 -o /dev/null -w "%{http_code}" "http://127.0.0.1:$port/healthz" 2>/dev/null || echo "000")
if [ "$hz" = "200" ]; then
  echo "  healthz on :$port -> 200 ok"
else
  echo "  ! healthz probe on :$port returned $hz (run 'phantom doctor' to diagnose)"
fi

echo
echo "=== installed ==="
echo

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
