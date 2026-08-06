#!/usr/bin/env bash
# setup-cloud-linux.sh — bootstrap a $5/mo Linux VPS as a spectyn-mesh
# cluster execution container.
#
# Designed for a freshly-provisioned Ubuntu 22.04 / Debian 12 box from
# DigitalOcean / Linode / Hetzner / etc. Run as root or with sudo.
#
# What you get:
#   - Tailscale joined to your tailnet
#   - spectyn binary on PATH
#   - spectyn serve running as systemd unit (auto-restart, on by default)
#   - agents.toml with the cluster peers + node_name = $NODE_NAME
#
# What you do NOT get (left to you, since cloud machines vary):
#   - Specific repos cloned (the cloud node is meant as a pure execution
#     container — you push tasks to it via subagent({node}), not run
#     specific repo demos on it)
#   - GPU / CUDA setup (this VPS is for orchestration, not training)
#   - Any data persistence beyond /var/lib/spectyn-mesh/
#
# Usage on a freshly-provisioned VPS:
#
#   curl -fsSL https://raw.githubusercontent.com/markl-a/spectyn-mesh/main/scripts/setup-cloud-linux.sh \
#       | sudo NODE_NAME=cloud-vps-1 \
#              CLUSTER_SECRET=<your-cluster-secret> \
#              TAILSCALE_AUTHKEY=tskey-... \
#              bash
#
# Idempotent. Safe to re-run.

set -euo pipefail

NODE_NAME="${NODE_NAME:-cloud-vps-$(hostname -s)}"
CLUSTER_SECRET="${CLUSTER_SECRET:?set CLUSTER_SECRET to your mesh shared secret}"
TAILSCALE_AUTHKEY="${TAILSCALE_AUTHKEY:-}"
SPECTYN_RELEASE_URL="${SPECTYN_RELEASE_URL:-}"
SERVE_PORT="${SERVE_PORT:-7878}"

step()  { echo; echo "── $*"; }
ok()    { echo "  ✓ $*"; }
warn()  { echo "  ⚠ $*"; }
fail()  { echo "  ✗ $*" >&2; exit 1; }

step "0. Privilege check"
[ "$(id -u)" = "0" ] || fail "Run as root or with sudo"

step "1. apt deps"
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq curl ca-certificates gnupg lsb-release git make python3 python3-pip
ok "base tools installed"

step "2. Tailscale"
if ! command -v tailscale >/dev/null 2>&1; then
    curl -fsSL https://tailscale.com/install.sh | sh
fi
if ! tailscale status >/dev/null 2>&1; then
    if [ -n "$TAILSCALE_AUTHKEY" ]; then
        tailscale up --authkey "$TAILSCALE_AUTHKEY" --hostname "$NODE_NAME"
    else
        warn "Tailscale not authed yet. Run: tailscale up --authkey tskey-..."
    fi
fi
ok "tailscale: $(tailscale status 2>&1 | head -1 || echo unknown)"

step "3. spectyn binary"
SPECTYN_BIN="/usr/local/bin/spectyn"

# Load the SHA256 + HTTPS verification helper. Prefer a co-located copy
# (when the operator cloned the repo and runs scripts/setup-cloud-linux.sh
# directly); fall back to fetching from raw.githubusercontent.com over HTTPS.
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd 2>/dev/null || echo "")"
VERIFY_HELPER=""
if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/_verify-download.sh" ]; then
    VERIFY_HELPER="$SCRIPT_DIR/_verify-download.sh"
else
    VERIFY_HELPER="$(mktemp -t spectyn-verify.XXXXXX)"
    HELPER_URL="https://raw.githubusercontent.com/markl-a/spectyn-mesh/main/scripts/_verify-download.sh"
    if ! curl -fsSL --max-time 10 "$HELPER_URL" -o "$VERIFY_HELPER"; then
        fail "Could not load $HELPER_URL — refusing to install unverified binary"
    fi
fi
# shellcheck disable=SC1090
. "$VERIFY_HELPER"

if [ -x "$SPECTYN_BIN" ]; then
    ok "spectyn already at $SPECTYN_BIN: $($SPECTYN_BIN --version | head -1)"
else
    if [ -z "$SPECTYN_RELEASE_URL" ]; then
        ARCH=$(uname -m)
        case "$ARCH" in
            x86_64|amd64)   SPECTYN_RELEASE_URL="https://github.com/markl-a/spectyn-mesh/releases/latest/download/spectyn-x86_64-unknown-linux" ;;
            aarch64|arm64)  SPECTYN_RELEASE_URL="https://github.com/markl-a/spectyn-mesh/releases/latest/download/spectyn-aarch64-unknown-linux" ;;
            *) fail "unknown arch $ARCH; set SPECTYN_RELEASE_URL=..." ;;
        esac
    fi
    echo "  downloading $SPECTYN_RELEASE_URL"
    require_https "$SPECTYN_RELEASE_URL" || fail "non-HTTPS SPECTYN_RELEASE_URL"
    curl -fsSL "$SPECTYN_RELEASE_URL" -o "$SPECTYN_BIN"
    # Verify SHA256 BEFORE chmod +x. verify_sha256 deletes the binary on
    # mismatch and exits non-zero (which `set -e` propagates).
    verify_sha256 "$SPECTYN_BIN" "$SPECTYN_RELEASE_URL"
    chmod +x "$SPECTYN_BIN"
    ok "installed to $SPECTYN_BIN"
fi

step "4. service user + state dir"
# Dedicated system user so spectyn-serve.service does not run as root.
# Hardened systemd unit (see step 5) blocks $HOME access, so SPECTYN_HOME
# must live under /var/lib/spectyn-mesh, not /root/.spectyn-mesh.
SPECTYN_USER="${SPECTYN_USER:-spectyn-mesh}"
SPECTYN_GROUP="${SPECTYN_GROUP:-spectyn-mesh}"
SPECTYN_HOME="${SPECTYN_HOME:-/var/lib/spectyn-mesh}"
if ! id -u "$SPECTYN_USER" >/dev/null 2>&1; then
    useradd --system --no-create-home --shell /usr/sbin/nologin \
            --home-dir "$SPECTYN_HOME" "$SPECTYN_USER" || true
    ok "created system user $SPECTYN_USER"
else
    ok "user $SPECTYN_USER already exists"
fi
mkdir -p "$SPECTYN_HOME"
chown -R "$SPECTYN_USER:$SPECTYN_GROUP" "$SPECTYN_HOME"
chmod 750 "$SPECTYN_HOME"

step "5. agents.toml"
CFG="$SPECTYN_HOME/agents.toml"
if [ ! -f "$CFG" ]; then
    cat > "$CFG" <<EOF
# spectyn-mesh agents.toml — generated by setup-cloud-linux.sh

[core]
host = "0.0.0.0"
port = $SERVE_PORT

[providers.anthropic]
type        = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[agent.master]
provider     = "anthropic"
tools        = ["shell", "file_read", "file_edit", "content_search",
                "git_status", "git_diff", "task"]
instructions = "You are spectyn on a cloud Linux execution container. Be terse."

[cluster]
node_name      = "$NODE_NAME"
cluster_secret = "$CLUSTER_SECRET"
peers = [
  "http://100.64.0.13:7878",   # node-b
  "http://100.64.0.12:7878",   # node-a
  "http://100.64.0.11:7879",   # node-a
  "http://100.64.0.10:7878",   # mac-coordinator
]
EOF
    chown "$SPECTYN_USER:$SPECTYN_GROUP" "$CFG"
    ok "wrote $CFG"
else
    ok "$CFG exists (NOT overwriting)"
fi

step "6. systemd unit"
SVC=/etc/systemd/system/spectyn-serve.service
if [ ! -f "$SVC" ]; then
    # Hardening notes (C10 / T79):
    #   - User/Group: drop root, run as dedicated system account.
    #   - ProtectSystem=full: /usr, /boot, /etc read-only. Start with `full`
    #     (not `strict`) so first-pass deployments do not regress on write
    #     paths we have not catalogued yet. Operator should validate on a
    #     real Pi for 24h, then tighten to ProtectSystem=strict if clean.
    #   - ReadWritePaths: explicit allow-list for state we must mutate.
    #   - NoNewPrivileges: blocks setuid escalation from child processes.
    #   - PrivateTmp/PrivateDevices: isolated /tmp + no /dev access.
    #   - ProtectHome=true: /root, /home, /run/user invisible.
    #   - ProtectKernelTunables/ProtectControlGroups: read-only /proc/sys,
    #     /sys, cgroup tree.
    #   - RestrictAddressFamilies: only the sockets serve actually needs.
    cat > "$SVC" <<EOF
[Unit]
Description=spectyn-mesh serve
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
Type=simple
User=$SPECTYN_USER
Group=$SPECTYN_GROUP
Environment=SPECTYN_HOME=$SPECTYN_HOME
EnvironmentFile=-/etc/spectyn-mesh/env
ExecStart=$SPECTYN_BIN serve --port $SERVE_PORT
Restart=on-failure
RestartSec=5
ProtectSystem=full
ProtectHome=true
NoNewPrivileges=true
PrivateTmp=true
ReadWritePaths=$SPECTYN_HOME
PrivateDevices=true
ProtectKernelTunables=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6

[Install]
WantedBy=multi-user.target
EOF
    systemctl daemon-reload
    systemctl enable spectyn-serve.service
    ok "wrote $SVC + enabled"
else
    ok "$SVC exists"
fi
mkdir -p /etc/spectyn-mesh
[ -f /etc/spectyn-mesh/env ] || cat > /etc/spectyn-mesh/env <<'EOF'
# put your provider keys here (chmod 600). e.g.:
# ANTHROPIC_API_KEY=sk-ant-...
# GROQ_API_KEY=gsk_...
EOF
# env must be readable by the spectyn-mesh service user (EnvironmentFile
# is read by systemd before the User= drop, but we still keep group-read
# narrow so secrets stay off other accounts on the box).
chown root:"$SPECTYN_GROUP" /etc/spectyn-mesh/env
chmod 640 /etc/spectyn-mesh/env

step "7. start"
systemctl restart spectyn-serve.service
sleep 2
if systemctl is-active --quiet spectyn-serve.service; then
    ok "spectyn-serve.service active"
else
    fail "spectyn-serve.service failed to start — see: journalctl -u spectyn-serve"
fi

step "8. verify"
TS_IP=$(tailscale ip -4 2>&1 | head -1 || echo "")
echo "  /healthz → $(curl -s "http://127.0.0.1:$SERVE_PORT/healthz" || echo 'unreachable')"
echo "  tailscale ip: ${TS_IP:-(not joined yet — run: tailscale up --authkey ...)}"

echo
echo "━━━ DONE ━━━"
echo "Add this peer to other nodes' agents.toml:"
echo "  \"http://${TS_IP}:${SERVE_PORT}\",   # $NODE_NAME"
echo
echo "From any other peer, confirm dispatch works:"
echo "  spectyn run --node $NODE_NAME 'echo hi from cloud'"
