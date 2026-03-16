#!/bin/bash
# ═══════════════════════════════════════════════════════════
# Clawtex Worker — iOS (iSH / a-Shell) 一鍵安裝
# ═══════════════════════════════════════════════════════════
#
# 方式 A — iSH (App Store 免費, Alpine Linux 環境)：
#   apk add bash curl git python3
#   git clone https://github.com/anthropomorphic-AI/clawtex-core.git
#   bash clawtex-core/deploy/mobile/ios/install-ish.sh
#
# 方式 B — a-Shell (App Store 免費, 內建 Python)：
#   curl -sL <raw URL> -o install.sh && bash install.sh
#
set -e

HUB_IP="${CLAWTEX_HUB_IP:-192.168.1.104}"
HUB_PORT="${CLAWTEX_HUB_PORT:-7878}"
WORKER_NAME="${CLAWTEX_WORKER_NAME:-ios-$(hostname 2>/dev/null | tr '[:upper:]' '[:lower:]' || echo 'iphone')}"
WORKER_PORT="${CLAWTEX_WORKER_PORT:-7882}"
INSTALL_DIR="$HOME/clawtex"

echo "╔══════════════════════════════════════╗"
echo "║   Clawtex Worker — iOS 安裝程式      ║"
echo "╚══════════════════════════════════════╝"
echo ""

# ── 偵測環境 ─────────────────────────────────────────────
ENV="unknown"
if [ -f /etc/alpine-release ]; then
    ENV="ish"
    echo "偵測到 iSH (Alpine Linux)"
elif command -v pickFolder &> /dev/null; then
    ENV="ashell"
    echo "偵測到 a-Shell"
else
    echo "偵測到一般 Linux/macOS 環境"
    ENV="generic"
fi

# ── 1. 安裝依賴 ──────────────────────────────────────────
echo "[1/5] 安裝依賴..."
case "$ENV" in
    ish)
        apk update
        apk add python3 git curl
        ;;
    ashell)
        echo "a-Shell 已內建 Python，跳過"
        ;;
    *)
        # 假設已有 python3
        if ! command -v python3 &> /dev/null; then
            echo "ERROR: 找不到 python3"
            exit 1
        fi
        ;;
esac

# ── 2. 建立目錄 ──────────────────────────────────────────
echo "[2/5] 建立工作目錄..."
mkdir -p "$INSTALL_DIR"

# ── 3. 下載 worker ───────────────────────────────────────
echo "[3/5] 下載 Worker..."
if command -v git &> /dev/null; then
    if [ -d "$INSTALL_DIR/.git" ]; then
        cd "$INSTALL_DIR" && git pull --quiet
    else
        git clone --depth 1 https://github.com/anthropomorphic-AI/clawtex-core.git "$INSTALL_DIR" 2>/dev/null || {
            echo "Git clone 失敗，直接下載..."
            curl -sL "https://raw.githubusercontent.com/anthropomorphic-AI/clawtex-core/master/deploy/lightweight-worker/clawtex-worker.py" \
                -o "$INSTALL_DIR/clawtex-worker.py"
        }
    fi
else
    curl -sL "https://raw.githubusercontent.com/anthropomorphic-AI/clawtex-core/master/deploy/lightweight-worker/clawtex-worker.py" \
        -o "$INSTALL_DIR/clawtex-worker.py"
fi

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

# ── 4. 測試連線 ──────────────────────────────────────────
echo "[4/5] 測試 Hub 連線..."
echo "  Hub: http://${HUB_IP}:${HUB_PORT}"

if curl -s --max-time 5 "http://${HUB_IP}:${HUB_PORT}/health" > /dev/null 2>&1; then
    echo "  ✓ Hub 連線成功"
else
    echo "  ✗ 無法連線到 Hub"
    echo "    確認 iPhone/iPad 和 Z13 在同一個 WiFi"
fi

# ── 5. 建立啟動腳本 ──────────────────────────────────────
echo "[5/5] 建立啟動腳本..."

cat > "$INSTALL_DIR/start.sh" << EOF
#!/bin/bash
cd "$INSTALL_DIR"
python3 "$WORKER_SCRIPT" \\
    --hub "http://${HUB_IP}:${HUB_PORT}" \\
    --name "$WORKER_NAME" \\
    --port $WORKER_PORT
EOF
chmod +x "$INSTALL_DIR/start.sh"

echo ""
echo "╔══════════════════════════════════════╗"
echo "║         安裝完成！                    ║"
echo "╚══════════════════════════════════════╝"
echo ""
echo "啟動：bash $INSTALL_DIR/start.sh"
echo "狀態：curl http://localhost:${WORKER_PORT}/health"
echo ""

read -p "現在啟動 Worker？ (y/n) " -n 1 -r
echo ""
if [[ $REPLY =~ ^[Yy]$ ]]; then
    bash "$INSTALL_DIR/start.sh"
fi
