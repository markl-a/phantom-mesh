# Phantom Mesh Mobile Worker 部署指南

## Hub 資訊
- **Hub IP**: `10.0.1.1` (Z13 LAN)
- **Hub Port**: `7878`
- **Auth Key**: `your-hub-token-here`

## Android (Termux)

### 安裝 Termux
1. 從 [F-Droid](https://f-droid.org/packages/com.termux/) 安裝 Termux（**不要用 Google Play 版本**，已過期）
2. 也安裝 **Termux:Boot**（開機自啟用）

### 一鍵安裝
```bash
pkg install -y git bash
git clone https://github.com/anthropomorphic-AI/phantom-mesh.git ~/phantom-mesh
bash ~/phantom-mesh/deploy/mobile/android/install-termux.sh
```

### 自訂 Hub IP
```bash
export PHANTOM_MESH_HUB_IP=10.0.1.1
export PHANTOM_MESH_WORKER_NAME=android-pixel
bash ~/phantom-mesh/deploy/mobile/android/install-termux.sh
```

### 多台 Android
- Android 1: `PHANTOM_MESH_WORKER_NAME=android-1 PHANTOM_MESH_WORKER_PORT=7880`
- Android 2: `PHANTOM_MESH_WORKER_NAME=android-2 PHANTOM_MESH_WORKER_PORT=7883`

---

## iOS (iSH 或 a-Shell)

### 方式 A — iSH（推薦，完整 Linux 環境）
1. App Store 搜尋 **iSH Shell** 安裝（免費）
2. 開啟 iSH，執行：
```bash
apk add bash git curl python3
git clone https://github.com/anthropomorphic-AI/phantom-mesh.git ~/phantom-mesh
bash ~/phantom-mesh/deploy/mobile/ios/install-ish.sh
```

### 方式 B — a-Shell
1. App Store 搜尋 **a-Shell** 安裝（免費）
2. 開啟 a-Shell，執行：
```bash
curl -sL https://raw.githubusercontent.com/anthropomorphic-AI/phantom-mesh/master/deploy/lightweight-worker/phantom-mesh-worker.py -o phantom-mesh-worker.py
python3 phantom-mesh-worker.py --hub http://10.0.1.1:7878 --name iphone --port 7882
```

---

## M1 Mac (Full Worker)

```bash
git clone https://github.com/anthropomorphic-AI/phantom-mesh.git
cd phantom-mesh
cargo build --release
./target/release/phantom-mesh worker \
  --hub http://10.0.1.1:7878 \
  --name m1-mac \
  --port 7879 \
  --device-type full
```

---

## Acer (Light Worker)

```bash
git clone https://github.com/anthropomorphic-AI/phantom-mesh.git
cd phantom-mesh
python3 deploy/lightweight-worker/phantom-mesh-worker.py \
  --hub http://10.0.1.1:7878 \
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
curl http://10.0.1.1:7878/cluster/workers
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
