#!/usr/bin/env bash
# Phantom Mesh — Oracle Cloud (and generic Linux) coordinator setup.
#
# Run this ON THE LINUX VM after `scripts/build-linux.sh` has produced
# `dist/phantom-<triple>`. Idempotent — safe to re-run.
#
# What it does, in order:
#   1. Pre-flight: OS family + arch + binary present
#   2. swap setup if RAM < 2 GB (Oracle E2.1.Micro survival)
#   3. Install package deps via dnf (RHEL family) or apt (Debian family)
#   4. Install Tailscale + non-interactive `tailscale up` via TAILSCALE_AUTH_KEY
#   5. Open firewall for tailscale0 only (not public)
#   6. Install `phantom` binary to ~/.local/bin/phantom
#   7. Render templates/phantom-mesh.service.tmpl → ~/.config/systemd/user/
#   8. `loginctl enable-linger` so the user service persists across logout
#   9. Bootstrap ~/.phantom-mesh/{agents.toml,local.toml} from cloud template
#  10. Print next-step checklist (fill keys, systemctl --user start)
#
# Usage:
#   TAILSCALE_AUTH_KEY=tskey-auth-... NODE_NAME=oci-singapore-coord \
#     ./scripts/setup-oci.sh
#
# Optional env:
#   TAILSCALE_AUTH_KEY   pre-auth key from login.tailscale.com/admin/settings/keys
#                        (required unless --skip-tailscale)
#   NODE_NAME            hostname this node advertises (default: $(hostname -s))
#   SKIP_TAILSCALE=1     skip Tailscale steps (already configured)
#   PHANTOM_BIN          override binary path (default: dist/phantom-<host-triple>)

set -euo pipefail

# ── 0. sanity ───────────────────────────────────────────────────────────
if [ "$(uname -s)" != "Linux" ]; then
  echo "✗ setup-oci.sh must run on Linux — host is $(uname -s)"
  exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOST_USER="$(whoami)"
HOST_HOME="$HOME"

# Detect OS family from /etc/os-release
if [ ! -r /etc/os-release ]; then
  echo "✗ /etc/os-release missing — cannot detect distro"
  exit 1
fi
# shellcheck disable=SC1091
. /etc/os-release
case "${ID:-}" in
  ol|rhel|centos|rocky|almalinux|fedora)
    OS_FAMILY="rhel"
    PKG_INSTALL="sudo dnf install -y"
    FIREWALL_TOOL="firewalld"
    ;;
  ubuntu|debian)
    OS_FAMILY="debian"
    PKG_INSTALL="sudo apt-get install -y"
    FIREWALL_TOOL="ufw"
    ;;
  *)
    echo "✗ unsupported distro: ${ID:-unknown}"
    echo "  Supported: Oracle Linux / RHEL / Fedora / Ubuntu / Debian"
    exit 1
    ;;
esac

# Detect arch + locate binary
case "$(uname -m)" in
  aarch64|arm64)  HOST_TRIPLE="aarch64-unknown-linux-gnu" ;;
  x86_64|amd64)   HOST_TRIPLE="x86_64-unknown-linux-gnu" ;;
  *)              echo "✗ unsupported arch: $(uname -m)"; exit 1 ;;
esac

PHANTOM_BIN="${PHANTOM_BIN:-$REPO_ROOT/dist/phantom-$HOST_TRIPLE}"
NODE_NAME="${NODE_NAME:-$(hostname -s | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9-')}"
NODE_NAME="${NODE_NAME:-oci-coordinator}"

if [ ! -x "$PHANTOM_BIN" ]; then
  echo "✗ phantom binary not found at $PHANTOM_BIN"
  echo "  Run scripts/build-linux.sh first, or set PHANTOM_BIN=/path/to/phantom"
  exit 1
fi

# RAM
RAM_KB=$(awk '/^MemTotal:/ {print $2}' /proc/meminfo)
RAM_MB=$(( RAM_KB / 1024 ))

echo "  ◆ phantom-mesh — OCI / Linux coordinator setup"
echo "    distro    : ${PRETTY_NAME:-$ID}"
echo "    family    : $OS_FAMILY ($PKG_INSTALL)"
echo "    arch      : $HOST_TRIPLE"
echo "    user      : $HOST_USER"
echo "    node_name : $NODE_NAME"
echo "    binary    : $PHANTOM_BIN"
echo "    ram       : ${RAM_MB} MB"
echo

# ── 1. swap ─────────────────────────────────────────────────────────────
if [ "$RAM_MB" -lt 2000 ] && [ ! -f /swapfile ]; then
  echo "  [1/9] adding 2 GB /swapfile (RAM ${RAM_MB} MB)"
  sudo fallocate -l 2G /swapfile || sudo dd if=/dev/zero of=/swapfile bs=1M count=2048
  sudo chmod 600 /swapfile
  sudo mkswap /swapfile
  sudo swapon /swapfile
  if ! grep -q '^/swapfile ' /etc/fstab; then
    echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab >/dev/null
  fi
  echo "    ✓ swap on ($(swapon --show --noheadings --bytes | awk '{print $3}' | head -1) bytes)"
else
  echo "  [1/9] swap: skipping (RAM ${RAM_MB} MB or /swapfile exists)"
fi

# ── 2. package deps ─────────────────────────────────────────────────────
echo "  [2/9] installing distro packages ($OS_FAMILY)"
case "$OS_FAMILY" in
  rhel)
    sudo dnf install -y curl tar firewalld policycoreutils-python-utils >/dev/null
    sudo systemctl enable --now firewalld >/dev/null 2>&1 || true
    ;;
  debian)
    sudo apt-get update -qq
    sudo apt-get install -y curl tar ufw >/dev/null
    ;;
esac
echo "    ✓ deps installed"

# ── 3. Tailscale ────────────────────────────────────────────────────────
if [ "${SKIP_TAILSCALE:-0}" = "1" ]; then
  echo "  [3/9] tailscale: SKIP_TAILSCALE=1 — assuming already configured"
elif command -v tailscale >/dev/null 2>&1 && tailscale status >/dev/null 2>&1; then
  echo "  [3/9] tailscale: already up ($(tailscale ip -4 2>/dev/null | head -1))"
else
  if [ -z "${TAILSCALE_AUTH_KEY:-}" ]; then
    echo "  ✗ TAILSCALE_AUTH_KEY required for non-interactive setup."
    echo "    Generate one at: https://login.tailscale.com/admin/settings/keys"
    echo "    Then re-run:  TAILSCALE_AUTH_KEY=tskey-auth-... ./scripts/setup-oci.sh"
    exit 1
  fi
  echo "  [3/9] installing Tailscale"
  if ! command -v tailscale >/dev/null 2>&1; then
    curl -fsSL https://tailscale.com/install.sh | sh
  fi
  sudo tailscale up \
    --hostname="$NODE_NAME" \
    --auth-key="$TAILSCALE_AUTH_KEY" \
    --accept-routes
  # Wait up to 30s for tailscale0 to come up
  for _ in $(seq 1 30); do
    ip link show tailscale0 >/dev/null 2>&1 && break
    sleep 1
  done
  if ! ip link show tailscale0 >/dev/null 2>&1; then
    echo "  ✗ tailscale0 interface did not come up"
    exit 1
  fi
  echo "    ✓ tailscale up ($(tailscale ip -4 | head -1))"
fi

# ── 4. firewall ─────────────────────────────────────────────────────────
echo "  [4/9] firewall: allowing 7878 only on tailscale0"
case "$OS_FAMILY" in
  rhel)
    # firewalld: assign tailscale0 to 'trusted' zone, allow 7878 there
    sudo firewall-cmd --permanent --zone=trusted --add-interface=tailscale0 >/dev/null 2>&1 || true
    sudo firewall-cmd --permanent --zone=trusted --add-port=7878/tcp >/dev/null 2>&1 || true
    sudo firewall-cmd --reload >/dev/null
    ;;
  debian)
    # ufw: allow 7878 from tailscale0 only
    sudo ufw --force enable >/dev/null 2>&1 || true
    sudo ufw allow in on tailscale0 to any port 7878 proto tcp >/dev/null
    ;;
esac
echo "    ✓ port 7878 reachable only via tailscale0"

# ── 5. install phantom binary ───────────────────────────────────────────
mkdir -p "$HOST_HOME/.local/bin"
install -m 755 "$PHANTOM_BIN" "$HOST_HOME/.local/bin/phantom"
echo "  [5/9] phantom → $HOST_HOME/.local/bin/phantom"

# SELinux context for the binary on RHEL family
if [ "$OS_FAMILY" = "rhel" ] && command -v restorecon >/dev/null 2>&1; then
  sudo semanage fcontext -a -t bin_t "$HOST_HOME/.local/bin/phantom" 2>/dev/null || true
  sudo restorecon "$HOST_HOME/.local/bin/phantom" 2>/dev/null || true
fi

# ── 6. systemd USER unit (rendered from template) ───────────────────────
echo "  [6/9] systemd user unit"
mkdir -p "$HOST_HOME/.config/systemd/user"
TMPL="$REPO_ROOT/templates/phantom-mesh.service.tmpl"
if [ ! -f "$TMPL" ]; then
  echo "  ✗ template missing: $TMPL"
  exit 1
fi
LOG_FILE="$HOST_HOME/.local/state/phantom-mesh.log"
mkdir -p "$(dirname "$LOG_FILE")"
sed \
  -e "s|__PHANTOM_BIN__|$HOST_HOME/.local/bin/phantom|g" \
  -e "s|__WORK_DIR__|$HOST_HOME|g" \
  -e "s|__LOG__|$LOG_FILE|g" \
  -e "s|__HOME__|$HOST_HOME|g" \
  -e "s|__EXTRA_ENV__|EnvironmentFile=-$HOST_HOME/.phantom-mesh/env|g" \
  "$TMPL" > "$HOST_HOME/.config/systemd/user/phantom-mesh.service"
systemctl --user daemon-reload
echo "    ✓ ~/.config/systemd/user/phantom-mesh.service"

# ── 7. linger (so user service runs without active login) ───────────────
if ! loginctl show-user "$HOST_USER" 2>/dev/null | grep -q '^Linger=yes'; then
  echo "  [7/9] enabling linger for $HOST_USER"
  sudo loginctl enable-linger "$HOST_USER"
else
  echo "  [7/9] linger already enabled for $HOST_USER"
fi

# ── 8. config bootstrap ─────────────────────────────────────────────────
mkdir -p "$HOST_HOME/.phantom-mesh"
chmod 700 "$HOST_HOME/.phantom-mesh"

# agents.toml: use configs/agents.cloud.toml as base.
# (When agents.base.toml lands per MULTI-DEVICE-COORDINATION.md Rule 4,
#  switch this line. See EVOLVE-GOALS.md.)
if [ ! -f "$HOST_HOME/.phantom-mesh/agents.toml" ]; then
  cp "$REPO_ROOT/configs/agents.cloud.toml" "$HOST_HOME/.phantom-mesh/agents.toml"
  sed -i "s/node_name = \"gcp-cloud\"/node_name = \"$NODE_NAME\"/" \
    "$HOST_HOME/.phantom-mesh/agents.toml"
  echo "  [8/9] agents.toml ← configs/agents.cloud.toml (node=$NODE_NAME)"
else
  echo "  [8/9] agents.toml already present — leaving alone"
fi

# local.toml skeleton (cluster_secret + overrides; user fills it)
if [ ! -f "$HOST_HOME/.phantom-mesh/local.toml" ]; then
  cat > "$HOST_HOME/.phantom-mesh/local.toml" <<EOF
# Per-machine overrides — never commit this file.
# See docs/MULTI-DEVICE-COORDINATION.md Rule 4.

[cluster]
node_name      = "$NODE_NAME"
cluster_secret = ""   # paste shared HMAC secret from 1Password (same on every node)
EOF
  chmod 600 "$HOST_HOME/.phantom-mesh/local.toml"
  echo "        local.toml ← skeleton (fill cluster_secret before starting)"
fi

# env file for API keys (referenced by the systemd unit)
if [ ! -f "$HOST_HOME/.phantom-mesh/env" ]; then
  cat > "$HOST_HOME/.phantom-mesh/env" <<'EOF'
# API keys read by phantom-mesh.service. Keep chmod 600. Never commit.
# ANTHROPIC_API_KEY=sk-ant-...
# OPENROUTER_API_KEY=sk-or-...
# TELEGRAM_BOT_TOKEN=...
# PHANTOM_CLUSTER_SECRET=...
EOF
  chmod 600 "$HOST_HOME/.phantom-mesh/env"
fi

# ── 9. summary ──────────────────────────────────────────────────────────
echo
echo "  ✓ setup-oci.sh complete."
echo
echo "  Next steps (operator):"
echo "    1. Edit  ~/.phantom-mesh/local.toml  — paste cluster_secret"
echo "    2. Edit  ~/.phantom-mesh/env         — fill API keys"
echo "    3. Start: systemctl --user start phantom-mesh"
echo "    4. Watch: journalctl --user -fu phantom-mesh"
echo "    5. Verify: curl -s http://localhost:7878/rpc/ping | head"
echo "    6. From another tailnet device:"
echo "         curl -s http://$NODE_NAME:7878/rpc/ping"
echo "         (should match local core_sha + wire_version)"
echo
echo "  See docs/INSTALL-OCI.md for OCI console steps + troubleshooting."
