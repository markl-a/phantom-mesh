#!/bin/bash
# ═══════════════════════════════════════════════════════════
# Clawtex Worker — Android (Termux) 一鍵安裝
# ═══════════════════════════════════════════════════════════
#
# 使用方式：
#   1. 安裝 Termux (F-Droid 版本)
#   2. 在 Termux 執行：
#      curl -sL https://raw.githubusercontent.com/anthropomorphic-AI/clawtex-core/master/deploy/mobile/android/install-termux.sh | bash
#
#   或手動：
#      git clone https://github.com/anthropomorphic-AI/clawtex-core.git
#      bash clawtex-core/deploy/mobile/android/install-termux.sh
#
set -e

# ── 設定 ─────────────────────────────────────────────────
HUB_IP="${CLAWTEX_HUB_IP:-192.168.1.104}"
HUB_PORT="${CLAWTEX_HUB_PORT:-7878}"
WORKER_NAME="${CLAWTEX_WORKER_NAME:-android-$(hostname | tr '[:upper:]' '[:lower:]')}"
WORKER_PORT="${CLAWTEX_WORKER_PORT:-7880}"
INSTALL_DIR="$HOME/clawtex"

echo "╔══════════════════════════════════════╗"
echo "║  Clawtex Worker — Android 安裝程式   ║"
echo "╚══════════════════════════════════════╝"
echo ""

# ── 1. 安裝依賴 ──────────────────────────────────────────
echo "[1/5] 安裝依賴..."
pkg update -y
pkg install -y python git curl

# ── 2. 建立目錄 ──────────────────────────────────────────
echo "[2/5] 建立工作目錄..."
mkdir -p "$INSTALL_DIR"

# ── 3. 下載 worker ───────────────────────────────────────
echo "[3/5] 下載 Worker..."
if [ -d "$INSTALL_DIR/.git" ]; then
    cd "$INSTALL_DIR"
    git pull --quiet
else
    git clone --depth 1 https://github.com/anthropomorphic-AI/clawtex-core.git "$INSTALL_DIR" 2>/dev/null || {
        # Fallback: 只下載 worker 檔案
        echo "Git clone 失敗，直接下載 worker..."
        curl -sL "https://raw.githubusercontent.com/anthropomorphic-AI/clawtex-core/master/deploy/lightweight-worker/clawtex-worker.py" \
            -o "$INSTALL_DIR/clawtex-worker.py"
    }
fi

# 找到 worker 腳本
WORKER_SCRIPT=""
if [ -f "$INSTALL_DIR/deploy/lightweight-worker/clawtex-worker.py" ]; then
    WORKER_SCRIPT="$INSTALL_DIR/deploy/lightweight-worker/clawtex-worker.py"
elif [ -f "$INSTALL_DIR/clawtex-worker.py" ]; then
    WORKER_SCRIPT="$INSTALL_DIR/clawtex-worker.py"
fi

if [ -z "$WORKER_SCRIPT" ]; then
    echo "ERROR: clawtex-worker.py 找不到"
    exit 1
fi

# ── 4. 設定 Hub 連線 ─────────────────────────────────────
echo "[4/5] 設定連線..."
echo ""
echo "  Hub IP:      $HUB_IP"
echo "  Hub Port:    $HUB_PORT"
echo "  Worker Name: $WORKER_NAME"
echo "  Worker Port: $WORKER_PORT"
echo ""

# 測試 Hub 連線
echo "  測試 Hub 連線..."
if curl -s --max-time 5 "http://${HUB_IP}:${HUB_PORT}/health" > /dev/null 2>&1; then
    echo "  ✓ Hub 連線成功"
else
    echo "  ✗ 無法連線到 Hub (http://${HUB_IP}:${HUB_PORT})"
    echo "    請確認："
    echo "    - Z13 Hub 正在運行"
    echo "    - 手機和 Z13 在同一個 WiFi 網路"
    echo "    - Z13 防火牆已開放 port ${HUB_PORT}"
    echo ""
    read -p "  要繼續安裝嗎？ (y/n) " -n 1 -r
    echo ""
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        exit 1
    fi
fi

# ── 5. 建立啟動腳本 ──────────────────────────────────────
echo "[5/5] 建立啟動腳本..."

cat > "$INSTALL_DIR/start.sh" << EOF
#!/bin/bash
# Clawtex Worker 啟動腳本
cd "$INSTALL_DIR"
python3 "$WORKER_SCRIPT" \\
    --hub "http://${HUB_IP}:${HUB_PORT}" \\
    --name "$WORKER_NAME" \\
    --port $WORKER_PORT
EOF
chmod +x "$INSTALL_DIR/start.sh"

# 建立 Termux 開機自啟（可選）
mkdir -p "$HOME/.termux/boot"
cat > "$HOME/.termux/boot/clawtex-worker.sh" << EOF
#!/bin/bash
# 開機自動啟動 Clawtex Worker
termux-wake-lock
sleep 5
cd "$INSTALL_DIR"
nohup bash start.sh > "$INSTALL_DIR/worker.log" 2>&1 &
EOF
chmod +x "$HOME/.termux/boot/clawtex-worker.sh"

echo ""
echo "╔══════════════════════════════════════╗"
echo "║         安裝完成！                    ║"
echo "╚══════════════════════════════════════╝"
echo ""
echo "啟動 Worker："
echo "  bash $INSTALL_DIR/start.sh"
echo ""
echo "背景執行："
echo "  nohup bash $INSTALL_DIR/start.sh > $INSTALL_DIR/worker.log 2>&1 &"
echo ""
echo "檢查狀態："
echo "  curl http://localhost:${WORKER_PORT}/health"
echo ""
echo "修改設定："
echo "  export CLAWTEX_HUB_IP=192.168.x.x"
echo "  export CLAWTEX_WORKER_NAME=my-phone"
echo "  bash $0"
echo ""

# 詢問是否立即啟動
read -p "現在啟動 Worker？ (y/n) " -n 1 -r
echo ""
if [[ $REPLY =~ ^[Yy]$ ]]; then
    bash "$INSTALL_DIR/start.sh"
fi
