#!/bin/bash
# 樹莓派節點一鍵設定腳本
# 在你的 Mac 上執行，自動部署到指定 Pi
#
# 用法：
#   ./scripts/setup-pi.sh <PI_TAILSCALE_IP> <NODE_NAME> [pi4|pi3|pi2]
#
# 範例：
#   ./scripts/setup-pi.sh 100.64.0.10 pi-living-room pi4
#   ./scripts/setup-pi.sh 100.64.0.11 pi-bedroom     pi3
#   ./scripts/setup-pi.sh 100.64.0.12 pi-old         pi2

set -euo pipefail

PI_IP="${1:?請提供 Pi 的 Tailscale IP，例如: 100.64.0.10}"
NODE_NAME="${2:?請提供節點名稱，例如: pi-01}"
MODEL="${3:-pi4}"  # pi4, pi3, pi2

PI_USER="${PI_USER:-pi}"   # Pi OS 預設使用者；Ubuntu Server 是 ubuntu
SSH_KEY="${SSH_KEY:-}"     # 留空使用預設 SSH key

SSH_CMD="ssh"
SCP_CMD="scp"
if [ -n "$SSH_KEY" ]; then
  SSH_CMD="ssh -i $SSH_KEY"
  SCP_CMD="scp -i $SSH_KEY"
fi

REMOTE="$PI_USER@$PI_IP"

# ─ 選擇正確的 binary ──────────────────────────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"

case "$MODEL" in
  pi4|pi5|pi3)
    BINARY="$ROOT/core/target/aarch64-unknown-linux-gnu/release/phantom-mesh"
    ;;
  pi2)
    BINARY="$ROOT/core/target/armv7-unknown-linux-gnueabihf/release/phantom-mesh"
    ;;
  *)
    echo "未知型號: $MODEL (支援: pi2 pi3 pi4 pi5)"
    exit 1
    ;;
esac

if [ ! -f "$BINARY" ]; then
  echo "找不到 binary: $BINARY"
  echo "請先執行:"
  echo "  cargo zigbuild --target aarch64-unknown-linux-gnu --manifest-path core/Cargo.toml --bin phantom-mesh --release"
  exit 1
fi

echo "==> 部署到 $NODE_NAME ($PI_IP)..."

# ─ 1. 傳送 binary ─────────────────────────────────────────────────────────────
echo "  傳送 binary..."
$SCP_CMD "$BINARY" "$REMOTE:/tmp/phantom-mesh-new"
$SSH_CMD "$REMOTE" "sudo mv /tmp/phantom-mesh-new /usr/local/bin/phantom-mesh && sudo chmod +x /usr/local/bin/phantom-mesh"

# ─ 2. 傳送 config ─────────────────────────────────────────────────────────────
echo "  傳送設定檔..."
$SSH_CMD "$REMOTE" "mkdir -p ~/.config/phantom-mesh"
$SCP_CMD "$ROOT/configs/agents.raspberrypi.toml" "$REMOTE:/tmp/agents.toml"
$SSH_CMD "$REMOTE" "
  # 替換 node_name
  sed -i 's/node_name = \"pi-1\"/node_name = \"$NODE_NAME\"/' /tmp/agents.toml
  mv /tmp/agents.toml ~/.config/phantom-mesh/agents.toml
"

# ─ 3. 設定 systemd 服務 ────────────────────────────────────────────────────────
echo "  設定 systemd..."
$SSH_CMD "$REMOTE" "
sudo tee /etc/systemd/system/phantom-mesh.service > /dev/null << 'EOF'
[Unit]
Description=Phantom Mesh Node
After=network-online.target tailscaled.service
Wants=network-online.target

[Service]
Type=simple
User=$PI_USER
ExecStart=/usr/local/bin/phantom-mesh
WorkingDirectory=/home/$PI_USER
EnvironmentFile=-/home/$PI_USER/.config/phantom-mesh/env
Restart=on-failure
RestartSec=10
StandardOutput=journal
StandardError=journal

[Install]
WantedBy=multi-user.target
EOF

sudo systemctl daemon-reload
sudo systemctl enable phantom-mesh
sudo systemctl restart phantom-mesh
"

# ─ 4. 安裝 Tailscale（如果還沒裝）─────────────────────────────────────────────
$SSH_CMD "$REMOTE" "
if ! command -v tailscale &>/dev/null; then
  echo '安裝 Tailscale...'
  curl -fsSL https://tailscale.com/install.sh | sh
  sudo tailscale up
  echo 'Tailscale 已安裝，請在瀏覽器中完成登入'
fi
tailscale ip -4
"

# ─ 5. 等待並確認 ──────────────────────────────────────────────────────────────
echo "  等待服務啟動..."
sleep 5
$SSH_CMD "$REMOTE" "
  systemctl is-active phantom-mesh && echo '✓ 服務已啟動' || echo '✗ 服務未啟動'
  curl -s http://localhost:7878/health 2>/dev/null | python3 -m json.tool || echo '  (等待 port 7878 開啟...)'
"

echo ""
echo "✓ $NODE_NAME 部署完成！"
echo ""
echo "後續步驟："
echo "  1. 編輯 config 填入真實 IP："
echo "     ssh $REMOTE nano ~/.config/phantom-mesh/agents.toml"
echo "  2. 設定 API keys："
echo "     ssh $REMOTE 'echo GROQ_API_KEY=你的key >> ~/.config/phantom-mesh/env'"
echo "  3. 查看日誌："
echo "     ssh $REMOTE journalctl -fu phantom-mesh"
