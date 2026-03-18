#!/bin/bash
# ═══════════════════════════════════════════════════════════
# Clawtex Worker — 通用快速啟動（任何有 Python3 的裝置）
# ═══════════════════════════════════════════════════════════
# 只需要這一個檔案，不需要 git clone 整個 repo
#
# 使用方式：
#   curl -sL <raw URL>/quick-start.sh | bash
#   或：
#   bash quick-start.sh
#
set -e

HUB_IP="${CLAWTEX_HUB_IP:-10.0.1.1}"
HUB_PORT="${CLAWTEX_HUB_PORT:-7878}"
WORKER_NAME="${CLAWTEX_WORKER_NAME:-worker-$(hostname 2>/dev/null | tr '[:upper:]' '[:lower:]' || echo 'unknown')}"
WORKER_PORT="${CLAWTEX_WORKER_PORT:-7880}"

echo "Clawtex Worker 快速啟動"
echo "========================"
echo "Hub: http://${HUB_IP}:${HUB_PORT}"
echo "Name: ${WORKER_NAME}"
echo "Port: ${WORKER_PORT}"
echo ""

# 下載 worker
WORKER_URL="https://raw.githubusercontent.com/anthropomorphic-AI/clawtex-core/master/deploy/lightweight-worker/clawtex-worker.py"
WORKER_FILE="/tmp/clawtex-worker.py"

echo "下載 worker..."
if command -v curl &> /dev/null; then
    curl -sL "$WORKER_URL" -o "$WORKER_FILE"
elif command -v wget &> /dev/null; then
    wget -q "$WORKER_URL" -O "$WORKER_FILE"
else
    echo "ERROR: 需要 curl 或 wget"
    exit 1
fi

echo "啟動..."
python3 "$WORKER_FILE" \
    --hub "http://${HUB_IP}:${HUB_PORT}" \
    --name "$WORKER_NAME" \
    --port "$WORKER_PORT"
