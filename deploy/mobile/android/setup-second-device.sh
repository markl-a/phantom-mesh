#!/bin/bash
# Acer 連接的第二台 Android — 快速設定
# 在第二台 Android 的 Termux 執行

export CLAWTEX_HUB_IP="${CLAWTEX_HUB_IP:-192.168.1.104}"
export CLAWTEX_WORKER_NAME="android-2"
export CLAWTEX_WORKER_PORT="7884"

# 用同一個安裝腳本
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
bash "$SCRIPT_DIR/install-termux.sh"
