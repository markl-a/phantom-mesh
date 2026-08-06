#!/data/data/com.termux/files/usr/bin/bash
# spectyn-mesh Termux setup — Android worker join
#
# One-shot install. From Termux:
#   curl -fsSL http://<COORD-HOST>:7878/scripts/termux-setup.sh | sh
#
# Or with explicit coordinator + Groq key:
#   COORD=http://localhost:7878 GROQ_KEY=gsk_… \
#     curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
#
# Pulls the spectyn binary directly from the coordinator (avoids the
# GitHub-release dependency the old version had), writes a worker
# agents.toml pre-wired with the cluster_secret, and (if GROQ_KEY is
# given) starts `spectyn serve` in the background.

set -e
echo "[spectyn] Setting up on Android/Termux..."

# ── tunable env ──────────────────────────────────────────────────────────────
COORD="${COORD:-http://localhost:7878}"
PORT="${PORT:-7879}"
NODE_NAME="${NODE_NAME:-android-phone}"
SECRET="${SECRET:-changeme-cluster-secret}"
SPECTYN_URL="${SPECTYN_URL:-${COORD}/dist/spectyn-aarch64-linux-android}"

# Basic packages
pkg update -y -q
pkg install -y curl wget git termux-tools

# Create dirs
mkdir -p ~/.spectyn-mesh/bin
mkdir -p ~/.spectyn-mesh/data

# ── Load shared SHA256 + HTTPS verification helpers ──────────────────────────
# We trust the coordinator just enough to fetch the helper, then use the
# helper to enforce HTTPS + SHA256 on the actual binary download.
VERIFY_HELPER="$(mktemp -t spectyn-verify.XXXXXX 2>/dev/null || echo /tmp/spectyn-verify.$$)"
trap 'rm -f "$VERIFY_HELPER"' EXIT
if ! curl -fsSL --max-time 10 "$COORD/scripts/_verify-download.sh" -o "$VERIFY_HELPER"; then
  echo "[spectyn] ✗ Could not load $COORD/scripts/_verify-download.sh"
  echo "[spectyn]   Refusing to download a binary without the verifier."
  exit 1
fi
# shellcheck disable=SC1090
. "$VERIFY_HELPER"

# Download spectyn binary from coordinator (or override URL)
echo "[spectyn] Downloading from $SPECTYN_URL ..."
require_https "$SPECTYN_URL" || exit 1
curl -fsSL "$SPECTYN_URL" -o ~/.spectyn-mesh/bin/spectyn
# Fail-closed verification BEFORE chmod +x. On mismatch the binary is deleted.
verify_sha256 ~/.spectyn-mesh/bin/spectyn "$SPECTYN_URL"
chmod +x ~/.spectyn-mesh/bin/spectyn

# Add to PATH
if ! grep -q "spectyn-mesh" ~/.bashrc 2>/dev/null; then
  echo 'export PATH="$HOME/.spectyn-mesh/bin:$PATH"' >> ~/.bashrc
fi

# agents.toml — pre-wired; GROQ_KEY env var (if given) is substituted in
GROQ_KEY="${GROQ_KEY:-REPLACE_WITH_GROQ_KEY}"

cat > ~/.spectyn-mesh/agents.toml <<TOML
[core]
host = "0.0.0.0"
port = $PORT

[cluster]
node_name      = "$NODE_NAME"
cluster_secret = "$SECRET"
capabilities   = ["web_fetch", "search", "analysis", "mobile_llm"]
peers = [
  "$COORD",
]

[providers.groq]
base_url      = "https://api.groq.com/openai/v1"
api_key       = "$GROQ_KEY"
default_model = "llama-3.3-70b-versatile"

# Uncomment after: pkg install ollama && ollama pull qwen2.5:1.5b
# [providers.local-ollama]
# base_url      = "http://localhost:11434/v1"
# api_key       = "ollama"
# default_model = "qwen2.5:1.5b"

[agent.master]
provider = "groq"
model    = "llama-3.3-70b-versatile"
tools    = ["shell", "file_read", "file_write", "web_fetch", "content_search"]
instructions = """
You are an Android agent in a distributed AI mesh.
Specialties: web fetching, content scraping, network requests from mobile IP.
Use shell tool for curl/wget. Respond in Traditional Chinese.
"""

[agent.fetcher]
provider = "groq"
model    = "llama-3.3-70b-versatile"
tools    = ["shell", "web_fetch"]
instructions = """
Web fetch specialist. Retrieve URLs, scrape content, extract data.
Use curl with proper headers to avoid bot detection.
Always return structured JSON when possible.
"""
TOML

echo ""
echo "[spectyn] Files installed. Coordinator: $COORD"
echo ""

# ── auto-start if we have a usable Groq key (otherwise leave it to the user)
if [ "$GROQ_KEY" != "REPLACE_WITH_GROQ_KEY" ] && [ -n "$GROQ_KEY" ]; then
  echo "[spectyn] Starting worker on :$PORT ..."
  nohup ~/.spectyn-mesh/bin/spectyn serve \
    > ~/.spectyn-mesh/data/spectyn-serve.log 2>&1 &
  PID=$!
  sleep 3
  WORKER_LIVE=0
  if curl -fsS --max-time 3 "http://127.0.0.1:$PORT/healthz" >/dev/null 2>&1; then
    WORKER_LIVE=1
    echo ""
    echo "✓ spectyn worker live (pid $PID, port $PORT)"
    echo "  Log:  ~/.spectyn-mesh/data/spectyn-serve.log"
  else
    echo "✗ spectyn serve did not bind :$PORT — check log:"
    tail -20 ~/.spectyn-mesh/data/spectyn-serve.log
  fi
fi

# ── Visual UI menu (always shown, regardless of GROQ_KEY) ──────────────────
TS_IP="$(ip -4 addr show 2>/dev/null | awk '/100\./ {print $2}' | cut -d/ -f1 | head -1)"
echo ""
echo "═══════════════════════════════════════════════════════════════════"
echo " 🎨  How to USE spectyn on this device — three ways:"
echo "═══════════════════════════════════════════════════════════════════"
echo ""
echo "  1) ratatui TUI (full-screen terminal UI, same as on Mac):"
echo "     ─ Open a NEW Termux session"
echo "     ─ Type:    spectyn"
echo "     (or:       ~/.spectyn-mesh/bin/spectyn)"
echo ""
echo "  2) mobile chat UI in Chrome / Firefox / browser:"
if [ -n "$TS_IP" ]; then
  echo "     ─ Visit:   http://${TS_IP}:${PORT}/   (this device's serve)"
fi
echo "     ─ Or:      ${COORD}/m              (Mac coordinator's mobile UI)"
echo "     (Add to home screen for a PWA-style icon.)"
echo ""
echo "  3) Cluster worker — already running in the background."
if [ "$WORKER_LIVE" = 1 ]; then
  echo "     ─ Mac can dispatch tasks to this node:"
  echo "         mcp__spectyn__subagent({ node: \"${TS_IP:-<this-ts-ip>}:${PORT}\", … })"
  echo "     ─ Mac coordinator's running serve at: ${COORD}"
fi
echo ""
echo "  Useful follow-up commands:"
echo "    spectyn doctor      — health check"
echo "    spectyn snapshot list  — APFS snapshots (no-op on Linux)"
echo "    spectyn --version   — provenance"
echo ""
if [ "$GROQ_KEY" = "REPLACE_WITH_GROQ_KEY" ] || [ -z "$GROQ_KEY" ]; then
  echo "  ⚠ GROQ_KEY was not provided — open ~/.spectyn-mesh/agents.toml"
  echo "    and replace REPLACE_WITH_GROQ_KEY before option (3) works."
fi
echo "═══════════════════════════════════════════════════════════════════"
