#!/bin/bash
# Acer 連接的第二台 Android — 快速設定
# 在第二台 Android 的 Termux 執行

export PHANTOM_MESH_HUB_IP="${PHANTOM_MESH_HUB_IP:-10.0.1.1}"
export PHANTOM_MESH_WORKER_NAME="android-2"
export PHANTOM_MESH_WORKER_PORT="7884"

# 用同一個安裝腳本
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
bash "$SCRIPT_DIR/install-termux.sh"
