#!/bin/bash
# phantom-mesh — one-line installer for macOS.
#
# Use:
#   curl -fsSL http://<coordinator-tailscale-ip>:7878/scripts/install-mac.sh \
#     | COORD=http://<coordinator-tailscale-ip>:7878 bash
#
# OR (auto-detect the URL the script came from — works because we're piped
# from `curl` and our parent's command line still references the URL):
#   curl -fsSL http://<coord>:7878/scripts/install-mac.sh | bash
#
# Pulls:
#   - phantom binary           → ~/.cargo/bin/phantom (+ TCC-safe copy)
#   - cluster bootstrap        → ~/.phantom-mesh/agents.toml (cluster_secret +
#                                peer list, NO API keys)
#   - launchd registration     → phantom serve auto-starts on login
#
# Does NOT touch your provider API keys — set those interactively after via
#   phantom
#   /keys add groq
#   /keys add gemini

set -euo pipefail

# ── Detect coordinator URL ──────────────────────────────────────────────
# Either passed via env (COORD=...) or extracted from how we were invoked.
if [ -z "${COORD:-}" ]; then
  echo "✗ COORD not set. Pass via:"
  echo "    curl -fsSL <url>/scripts/install-mac.sh | COORD=<url> bash"
  exit 1
fi

# Strip trailing slash for clean concatenation.
COORD="${COORD%/}"

# ── Banner ──────────────────────────────────────────────────────────────
echo
echo "  ◆ phantom-mesh installer — macOS"
echo "    coordinator: $COORD"
echo

# ── Pre-flight ──────────────────────────────────────────────────────────
echo "  [1/6] Pre-flight checks..."
case "$(uname -m)" in
  arm64) ARCH="aarch64-apple-darwin" ;;
  x86_64) echo "    ✗ Intel Macs not supported in v0.1.0 — Apple Silicon only."; exit 1 ;;
  *) echo "    ✗ Unknown arch $(uname -m)"; exit 1 ;;
esac

if ! command -v curl >/dev/null 2>&1; then
  echo "    ✗ curl not found"; exit 1
fi

# Tailscale check — we don't refuse install (you might be on the same LAN as
# the coordinator), but we warn so the user knows.
if ! command -v tailscale >/dev/null 2>&1; then
  echo "    ⚠ Tailscale not installed. If the coordinator is on a tailnet,"
  echo "      you'll need: brew install tailscale && sudo tailscale up"
elif ! tailscale status >/dev/null 2>&1; then
  echo "    ⚠ Tailscale installed but not running — sudo tailscale up"
fi

# Verify the coordinator is reachable BEFORE making any local changes.
if ! curl -fsS --max-time 3 "$COORD/healthz" >/dev/null 2>&1; then
  echo "    ✗ Cannot reach $COORD/healthz — check the URL + tailnet"
  exit 1
fi
echo "    ✓ coordinator reachable"

# ── Make install dirs ───────────────────────────────────────────────────
mkdir -p \
  "$HOME/.cargo/bin" \
  "$HOME/.phantom-mesh" \
  "$HOME/Library/Application Support/phantom-mesh/bin"

# ── Download binary ─────────────────────────────────────────────────────
echo "  [2/6] Downloading phantom ($ARCH)..."
TMP_BIN="$(mktemp -t phantom.XXXXXX)"
trap 'rm -f "$TMP_BIN"' EXIT
if ! curl -fsSL --max-time 60 "$COORD/dist/phantom-$ARCH" -o "$TMP_BIN"; then
  echo "    ✗ download failed — coordinator may not have a Mac binary in dist/"
  echo "      run on coordinator:  cd phantom-mesh && cargo build --release --bin phantom"
  echo "      then:                cp core/target/release/phantom dist/phantom-$ARCH"
  exit 1
fi
chmod +x "$TMP_BIN"
mv "$TMP_BIN" "$HOME/.cargo/bin/phantom"
trap - EXIT
# TCC-safe copy (mirrors the launchd path used by `phantom service install`)
cp "$HOME/.cargo/bin/phantom" "$HOME/Library/Application Support/phantom-mesh/bin/phantom"

# Ad-hoc re-sign both copies. A `cp`/`mv` over a Mach-O strips the kernel-
# valid signature; amfid then SIGKILLs the next launch silently (exit 137,
# zero stdout/stderr) — see commit 85c8377. `codesign --force --sign -`
# produces an ad-hoc signature, identical to what `brew` and `cargo
# install` leave behind, which is enough for amfid on the user's own
# machine. Silent-tolerant: codesign should be present on every Mac, but
# if it isn't we still surface the failure on first daemon start.
for bin in \
  "$HOME/.cargo/bin/phantom" \
  "$HOME/Library/Application Support/phantom-mesh/bin/phantom"; do
  codesign --force --sign - "$bin" 2>/dev/null || true
done
echo "    ✓ installed to ~/.cargo/bin/phantom (ad-hoc signed)"

# ── Fetch cluster bootstrap ─────────────────────────────────────────────
echo "  [3/6] Fetching cluster bootstrap..."
TOKEN_RESP="$(curl -fsS --max-time 5 "$COORD/onboarding/token" 2>/dev/null || true)"
TOKEN="$(echo "$TOKEN_RESP" | sed -nE 's/.*"token"[[:space:]]*:[[:space:]]*"([^"]+)".*/\1/p')"
if [ -z "$TOKEN" ]; then
  echo "    ✗ Could not get onboarding token from $COORD/onboarding/token"
  echo "      Coordinator may not have cluster_secret set in agents.toml [cluster]"
  exit 1
fi

NODE_NAME="$(scutil --get LocalHostName 2>/dev/null || hostname -s)"
NODE_NAME="$(echo "$NODE_NAME" | tr -c 'A-Za-z0-9_-' '_' | head -c 40)"

CFG_TARGET="$HOME/.phantom-mesh/agents.toml"
if [ -e "$CFG_TARGET" ]; then
  BACKUP="$CFG_TARGET.backup-$(date +%Y%m%d-%H%M%S)"
  echo "    ⚠ Existing agents.toml → $BACKUP"
  cp "$CFG_TARGET" "$BACKUP"
fi

if ! curl -fsS --max-time 5 \
  "$COORD/onboarding/config?token=$TOKEN&node_name=$NODE_NAME" \
  -o "$CFG_TARGET"; then
  echo "    ✗ Failed to fetch /onboarding/config"
  exit 1
fi
chmod 0600 "$CFG_TARGET"
echo "    ✓ wrote $CFG_TARGET (node_name: $NODE_NAME)"

# ── Verify config ───────────────────────────────────────────────────────
echo "  [4/6] Verifying config..."
if ! grep -q '\[cluster\]' "$CFG_TARGET"; then
  echo "    ✗ Bootstrap response missing [cluster] section"
  exit 1
fi
PROVIDER_COUNT=$(grep -c '^\[providers\.' "$CFG_TARGET" 2>/dev/null || echo 0)
echo "    ✓ [cluster] + $PROVIDER_COUNT [providers.*] sections written"
echo "    ⚠ API keys are NOT auto-configured — set via /keys add inside REPL"

# ── launchd registration ────────────────────────────────────────────────
echo "  [5/6] Registering launchd auto-start..."
if "$HOME/.cargo/bin/phantom" service install >/dev/null 2>&1; then
  echo "    ✓ launchd registered (phantom serve will auto-start on login)"
else
  echo "    ⚠ launchd registration failed — you can run \`phantom serve\` manually"
fi

# ── Verify peer ────────────────────────────────────────────────────────
echo "  [6/6] Verifying mesh round-trip..."
if "$HOME/.cargo/bin/phantom" peer list 2>/dev/null | grep -q "online"; then
  echo "    ✓ at least one peer online — mesh ready"
else
  echo "    ⚠ no peers responded yet (may take ~30s for heartbeat)"
fi

# ── Detect this node's Tailscale IP for reverse-registration hint ──────
MY_TS_IP=""
if command -v tailscale >/dev/null 2>&1; then
  MY_TS_IP="$(tailscale ip -4 2>/dev/null | head -1 || true)"
fi
MY_URL=""
if [ -n "$MY_TS_IP" ]; then
  MY_URL="http://$MY_TS_IP:7878"
fi

# ── Done ───────────────────────────────────────────────────────────────
echo
echo "  ✓ Install complete."
echo
echo "  ── Next steps on THIS Mac ─────────────────────────────────────"
echo "    1. \`phantom\`                       — start the REPL/TUI"
echo "    2. \`/keys add groq\`                — paste your Groq API key"
echo "    3. \`/keys add gemini\`              — paste your Gemini API key"
echo "    4. \`/keys test groq\`               — verify"
echo "    5. \`/model fetch groq\`             — see available models"
echo
if [ -n "$MY_URL" ]; then
  echo "  ── Register this Mac on the COORDINATOR (one-time) ──────────"
  echo
  echo "    On the coordinator (the Mac whose URL you used as COORD), run:"
  echo
  echo "      phantom peer discover                 # Tailscale auto-pickup"
  echo "                                            # — should show $MY_URL"
  echo
  echo "      OR edit ~/.phantom-mesh/agents.toml on the coordinator and"
  echo "      add to [cluster].peers:"
  echo
  echo "        peers = ["
  echo "          # ... your existing peers ..."
  echo "          \"$MY_URL\","
  echo "        ]"
  echo
  echo "      Then on the coordinator:"
  echo "        launchctl kickstart -k gui/\$UID/ai.phantommesh.serve"
  echo
fi
