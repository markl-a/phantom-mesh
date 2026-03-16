# Clawtex Mobile Worker 部署指南

## Hub 資訊
- **Hub IP**: `192.168.1.104` (Z13 LAN)
- **Hub Port**: `7878`
- **Auth Key**: `clawtex-hub-2026`

## Android (Termux)

### 安裝 Termux
1. 從 [F-Droid](https://f-droid.org/packages/com.termux/) 安裝 Termux（**不要用 Google Play 版本**，已過期）
2. 也安裝 **Termux:Boot**（開機自啟用）

### 一鍵安裝
```bash
pkg install -y git bash
git clone https://github.com/anthropomorphic-AI/clawtex-core.git ~/clawtex
bash ~/clawtex/deploy/mobile/android/install-termux.sh
```

### 自訂 Hub IP
```bash
export CLAWTEX_HUB_IP=192.168.1.104
export CLAWTEX_WORKER_NAME=android-pixel
bash ~/clawtex/deploy/mobile/android/install-termux.sh
```

### 多台 Android
- Android 1: `CLAWTEX_WORKER_NAME=android-1 CLAWTEX_WORKER_PORT=7880`
- Android 2: `CLAWTEX_WORKER_NAME=android-2 CLAWTEX_WORKER_PORT=7883`

---

## iOS (iSH 或 a-Shell)

### 方式 A — iSH（推薦，完整 Linux 環境）
1. App Store 搜尋 **iSH Shell** 安裝（免費）
2. 開啟 iSH，執行：
```bash
apk add bash git curl python3
git clone https://github.com/anthropomorphic-AI/clawtex-core.git ~/clawtex
bash ~/clawtex/deploy/mobile/ios/install-ish.sh
```

### 方式 B — a-Shell
1. App Store 搜尋 **a-Shell** 安裝（免費）
2. 開啟 a-Shell，執行：
```bash
curl -sL https://raw.githubusercontent.com/anthropomorphic-AI/clawtex-core/master/deploy/lightweight-worker/clawtex-worker.py -o clawtex-worker.py
python3 clawtex-worker.py --hub http://192.168.1.104:7878 --name iphone --port 7882
```

---

## M1 Mac (Full Worker)

```bash
git clone https://github.com/anthropomorphic-AI/clawtex-core.git
cd clawtex-core
cargo build --release
./target/release/clawtex-core worker \
  --hub http://192.168.1.104:7878 \
  --name m1-mac \
  --port 7879 \
  --device-type full
```

---

## Acer (Light Worker)

```bash
git clone https://github.com/anthropomorphic-AI/clawtex-core.git
cd clawtex-core
python3 deploy/lightweight-worker/clawtex-worker.py \
  --hub http://192.168.1.104:7878 \
  --name acer \
  --port 7881
```

---

## 驗證

在任何裝置上檢查：
```bash
# 本機健康
curl http://localhost:<PORT>/health

# Hub 看到所有 worker
curl http://192.168.1.104:7878/cluster/workers
```

## Port 分配

| 裝置 | Port | Type |
|------|------|------|
| Z13 Hub | 7878 | Hub |
| M1 Mac | 7879 | Full Worker |
| AYANEO | 7880 | Full Worker |
| Acer | 7881 | Light Worker |
| iPhone | 7882 | Light Worker |
| Android 1 | 7883 | Light Worker |
| Android 2 | 7884 | Light Worker |
| iPad | 7885 | Light Worker |
