#!/usr/bin/env bash
# cluster-install.sh — 一鍵部署 phantom serve 到所有節點
#
# 用法：
#   ./scripts/cluster-install.sh                         # 互動模式
#   ./scripts/cluster-install.sh --oracle ubuntu@100.x.x.x \
#       --z13 user@100.x.x.x --acer user@100.x.x.x
#
# 前提：
#   - 所有節點已裝 Tailscale，使用 100.x.x.x 互連
#   - 從這台機器可以 SSH 到各節點（公鑰已部署）
#   - dist/ 或 core/target/ 目錄有對應的 binary

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(dirname "$SCRIPT_DIR")"
DIST="$REPO_ROOT/dist"

# ── 顏色 ──────────────────────────────────────────────────────────────────────
GREEN='\033[0;32m'; YELLOW='\033[1;33m'; RED='\033[0;31m'; NC='\033[0m'
ok()   { echo -e "${GREEN}✓${NC} $*"; }
warn() { echo -e "${YELLOW}⚠${NC}  $*"; }
err()  { echo -e "${RED}✗${NC} $*"; exit 1; }
step() { echo -e "\n${YELLOW}── $* ──${NC}"; }

pick_binary() {
  local label="$1"
  shift
  local candidate
  for candidate in "$@"; do
    if [[ -n "$candidate" && -f "$candidate" ]]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  err "$label binary not found. Checked: $*"
}

# ── 解析參數 ──────────────────────────────────────────────────────────────────
ORACLE_SSH=""
Z13_SSH=""
ACER_SSH=""
AYANEO_SSH=""
EXTRA_NODES=()

while [[ $# -gt 0 ]]; do
  case $1 in
    --oracle) ORACLE_SSH="$2"; shift 2 ;;
    --z13)    Z13_SSH="$2";    shift 2 ;;
    --acer)   ACER_SSH="$2";   shift 2 ;;
    --ayaneo) AYANEO_SSH="$2"; shift 2 ;;
    --node)   EXTRA_NODES+=("$2"); shift 2 ;;
    --help|-h)
      echo "Usage: $0 [--oracle SSH] [--z13 SSH] [--acer SSH] [--ayaneo SSH] [--node SSH]"
      echo "  SSH format: user@100.x.x.x"
      exit 0 ;;
    *) err "Unknown option: $1" ;;
  esac
done

# ── 互動模式（沒傳參數時問） ──────────────────────────────────────────────────
if [[ -z "$ORACLE_SSH" && -z "$Z13_SSH" && -z "$ACER_SSH" ]]; then
  echo "Enter SSH targets (user@100.x.x.x), press Enter to skip:"
  read -p "  Oracle VM:  " ORACLE_SSH
  read -p "  Z13:        " Z13_SSH
  read -p "  Acer:       " ACER_SSH
  read -p "  AYANEO 2:   " AYANEO_SSH
fi

# ── 函數：部署到 Linux 節點 ───────────────────────────────────────────────────
deploy_linux() {
  local name="$1"
  local ssh_target="$2"
  local binary="$3"   # local path to binary

  step "Deploying to $name ($ssh_target)"

  [[ -f "$binary" ]] || err "Binary not found: $binary"

  # 建目錄，上傳 binary
  ssh "$ssh_target" "mkdir -p ~/.phantom-mesh/bin"
  scp "$binary" "$ssh_target:~/.phantom-mesh/bin/phantom"
  ssh "$ssh_target" "chmod +x ~/.phantom-mesh/bin/phantom"
  ok "Binary uploaded"

  # 生成 agents.toml（如果還沒有）
  ssh "$ssh_target" 'bash -s' << 'REMOTE'
if [[ ! -f ~/.phantom-mesh/agents.toml ]]; then
  cat > ~/.phantom-mesh/agents.toml << 'TOML'
[core]
host = "0.0.0.0"
port = 7878

# 填入你的 Tailscale IP 和其他節點的 IP
[cluster]
node_name = "CHANGE_ME"
cluster_secret = "CHANGE_ME_SHARED_SECRET"
peers = []

# 預設使用 Groq 免費 API（申請：console.groq.com）
[providers.groq]
base_url = "https://api.groq.com/openai/v1"
api_key_env = "GROQ_API_KEY"
default_model = "llama-3.1-70b-versatile"

[agent.master]
tools = ["shell","file_read","file_write","file_edit","content_search","glob_search","git_status","git_diff","git_commit","web_search"]
TOML
  echo "Created default agents.toml — edit ~/.phantom-mesh/agents.toml to configure"
fi
REMOTE
  ok "agents.toml ready"

  # 安裝 systemd service
  ssh "$ssh_target" 'bash -s' << REMOTE
cat > /tmp/phantom-serve.service << 'SERVICE'
[Unit]
Description=Phantom Mesh Agent Server
After=network.target tailscaled.service
Wants=network-online.target

[Service]
ExecStart=/root/.phantom-mesh/bin/phantom serve
Restart=on-failure
RestartSec=10
Environment="RUST_LOG=info"
EnvironmentFile=-/root/.phantom-mesh/env

[Install]
WantedBy=multi-user.target
SERVICE

# Try systemd (most Linux distros)
if command -v systemctl >/dev/null 2>&1; then
  sudo mv /tmp/phantom-serve.service /etc/systemd/system/phantom-serve.service
  sudo systemctl daemon-reload
  sudo systemctl enable phantom-serve
  sudo systemctl restart phantom-serve
  echo "systemd: phantom-serve started"
else
  # Fallback: run in tmux
  command -v tmux >/dev/null 2>&1 || sudo apt-get install -y tmux 2>/dev/null || true
  tmux new-session -d -s phantom "~/.phantom-mesh/bin/phantom serve" 2>/dev/null || \
  tmux send-keys -t phantom "" Enter 2>/dev/null || \
  nohup ~/.phantom-mesh/bin/phantom serve > ~/.phantom-mesh/serve.log 2>&1 &
  echo "Started phantom serve (no systemd, using nohup)"
fi
REMOTE
  ok "phantom-serve service installed and started"

  # 驗證
  sleep 2
  if ssh "$ssh_target" "curl -sf http://localhost:7878/healthz" > /dev/null 2>&1; then
    ok "$name is ONLINE ✓"
  else
    warn "$name healthz check failed — may still be starting up"
  fi
}

# ── 函數：部署到 Windows（WSL2）節點 ─────────────────────────────────────────
deploy_windows_wsl() {
  local name="$1"
  local ssh_target="$2"
  local binary="$3"

  step "Deploying to $name/$ssh_target (WSL2)"

  [[ -f "$binary" ]] || err "Binary not found: $binary"

  ssh "$ssh_target" "mkdir -p ~/.phantom-mesh/bin"
  scp "$binary" "$ssh_target:~/.phantom-mesh/bin/phantom"
  ssh "$ssh_target" "chmod +x ~/.phantom-mesh/bin/phantom"
  ok "Binary uploaded"

  # 在 WSL2 裡用 nohup 啟動，並盡量補上 /etc/wsl.conf 開機自啟
  ssh "$ssh_target" 'bash -s' << 'REMOTE'
mkdir -p ~/.phantom-mesh
if [[ ! -f ~/.phantom-mesh/agents.toml ]]; then
  cat > ~/.phantom-mesh/agents.toml << 'TOML'
[core]
host = "0.0.0.0"
port = 7878

[cluster]
node_name = "CHANGE_ME"
cluster_secret = "CHANGE_ME_SHARED_SECRET"
peers = []

[providers.groq]
base_url = "https://api.groq.com/openai/v1"
api_key_env = "GROQ_API_KEY"
default_model = "llama-3.1-70b-versatile"

[agent.master]
tools = ["shell","file_read","file_write","file_edit","content_search","glob_search","git_status","git_diff","git_commit","web_search"]
TOML
fi

# 建立啟動腳本（idempotent）
cat > ~/.phantom-mesh/start.sh << 'START'
#!/bin/bash
set -euo pipefail
pkill -f "phantom serve" 2>/dev/null || true
sleep 1
mkdir -p ~/.phantom-mesh/logs
export RUST_LOG=info
nohup ~/.phantom-mesh/bin/phantom serve >> ~/.phantom-mesh/logs/serve.log 2>&1 &
START
chmod +x ~/.phantom-mesh/start.sh

~/.phantom-mesh/start.sh
sleep 2
curl -sf http://localhost:7878/healthz >/dev/null && echo "healthz ok"

# Best effort: 若有免密 sudo，寫入 WSL boot hook
if command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
  USER_NAME="$(id -un)"
  HOME_DIR="$(getent passwd "$USER_NAME" | cut -d: -f6)"
  sudo tee /etc/wsl.conf > /dev/null <<EOF
[boot]
command = "su - $USER_NAME -c $HOME_DIR/.phantom-mesh/start.sh"
EOF
  echo "wsl.conf updated"
else
  echo "wsl.conf skipped (sudo unavailable or requires password)"
fi
REMOTE
  ok "phantom serve started on $name"
}

# ── 主流程 ────────────────────────────────────────────────────────────────────
echo ""
echo "╔══════════════════════════════════════╗"
echo "║   phantom-mesh cluster-install.sh    ║"
echo "╚══════════════════════════════════════╝"
echo ""

LINUX_ARM64="$(pick_binary \
  'Linux ARM64' \
  "$DIST/phantom-linux-arm64" \
  "$REPO_ROOT/core/target/aarch64-unknown-linux-gnu/release/phantom-mesh")"
LINUX_X86="$(pick_binary \
  'Linux x86_64' \
  "$DIST/phantom-linux-x86_64" \
  "$REPO_ROOT/core/target/x86_64-unknown-linux-gnu/release/phantom-mesh")"

[[ -z "$ORACLE_SSH" ]] || deploy_linux  "Oracle VM"  "$ORACLE_SSH"  "$LINUX_ARM64"
[[ -z "$Z13_SSH" ]]    || deploy_windows_wsl "Z13"    "$Z13_SSH"    "$LINUX_X86"
[[ -z "$ACER_SSH" ]]   || deploy_windows_wsl "Acer"   "$ACER_SSH"   "$LINUX_X86"
[[ -z "$AYANEO_SSH" ]] || deploy_windows_wsl "AYANEO" "$AYANEO_SSH" "$LINUX_X86"

for node in "${EXTRA_NODES[@]}"; do
  deploy_linux "node:$node" "$node" "$LINUX_ARM64"
done

echo ""
echo "═══════════════════════════════════════════"
echo "Done! Next steps:"
echo "  1. Edit agents.toml on each node:"
echo "       node_name = \"oracle\"  # unique per node"
echo "       cluster_secret = \"your-shared-secret\""
echo "       peers = [\"http://100.x.x.mac:7878\", ...]"
echo ""
echo "  2. Add API keys on each node:"
echo "       echo 'GROQ_API_KEY=gsk_...' >> ~/.phantom-mesh/env"
echo ""
echo "  3. Verify from Mac M1:"
echo "       phantom peer list"
echo "═══════════════════════════════════════════"
