# Clawtex 集群部署指令 — 每台設備一步步執行

## 網路架構

```
家裡 LAN (192.168.1.x)             辦公室 (Tailscale)
┌─────────────────────┐            ┌──────────────────┐
│ Z13 Hub             │            │ M1 Mac (Worker)  │
│  LAN: 10.0.1.1 │◄──100.x──►│  ├─ iPhone       │
│  TS:  <Z13_TS_IP>   │ Tailscale  │  └─ iPad         │
│                     │            └──────────────────┘
│ Ayaneo (Worker)     │
│ Aspire 5 (Worker)   │
│  ├─ Android 1       │
│  └─ Android 2       │
└─────────────────────┘
```

**家裡設備** (Ayaneo, Aspire 5, Android x2) → 用 LAN IP `10.0.1.1:7878`
**辦公室設備** (M1 Mac, iPhone, iPad) → 用 Tailscale IP `<Z13_TS_IP>:7878`

Repo: `https://github.com/markl-a/Clawtex.git` branch `ralph/money-agent`

---

## 前置作業：安裝 Tailscale

只有 4 台設備需要裝 Tailscale：**Z13、M1 Mac、iPhone、iPad**

### Z13 (Windows)
```
winget install Tailscale.Tailscale
# 或從 https://tailscale.com/download 下載安裝
# 登入後記下你的 Tailscale IP，例如 100.64.0.1
# 把下面所有 <Z13_TS_IP> 替換成這個 IP
```

### M1 Mac
```
brew install tailscale
# 或 App Store 搜尋 Tailscale
# 用同一個帳號登入
```

### iPhone / iPad
```
App Store 搜尋 Tailscale → 安裝 → 用同一個帳號登入
```

安裝完成後，確認 M1 Mac 能 ping 到 Z13 的 Tailscale IP：
```bash
ping <Z13_TS_IP>
```

---

## 節點 0：Z13 Hub（已啟動）

Hub 已在 Z13 上運行，綁定 `0.0.0.0:7878`（接受 LAN + Tailscale 連線）。

如果需要重新啟動：
```bash
cd Clawtex/clawtex-core
./target/release/clawtex-core.exe --host 0.0.0.0 --port 7878 daemon
```

---

## 節點 1：M1 Mac（Full Worker + 管理 iPhone/iPad）🌐 辦公室 via Tailscale

在 M1 Mac 上開 Claude Code，貼以下指令：

```
幫我設定 Clawtex cluster worker。這台機器在辦公室，透過 Tailscale 連回家裡的 Hub。步驟：

1. 確認 Tailscale 已連線：
   tailscale status
   ping <Z13_TS_IP>
   （如果 ping 不通，先確認 Tailscale 登入狀態）

2. Clone repo（如果還沒有的話）:
   git clone https://github.com/markl-a/Clawtex.git
   cd Clawtex/clawtex-core
   git checkout ralph/money-agent

3. Build release:
   cargo build --release

4. 啟動 Full Worker（背景執行）:
   nohup ./target/release/clawtex-core worker --hub http://<Z13_TS_IP>:7878 --name m1-mac --port 7879 > /tmp/clawtex-worker.log 2>&1 &

5. 驗證:
   curl http://localhost:7879/health
   curl http://<Z13_TS_IP>:7878/cluster/workers

6. 設定 iPhone light worker:
   把 Clawtex/clawtex-core/deploy/cluster-package/light-worker/clawtex-worker.py 傳到 iPhone
   在 iPhone 上用 iSH 或 a-Shell 執行:
   python3 clawtex-worker.py --hub http://<Z13_TS_IP>:7878 --name iphone --port 7880

7. 設定 iPad light worker:
   同樣把 clawtex-worker.py 傳到 iPad
   python3 clawtex-worker.py --hub http://<Z13_TS_IP>:7878 --name ipad --port 7880

每步完成後回報結果。
```

---

## 節點 2：Ayaneo（Full Worker）🏠 家裡 LAN

在 Ayaneo 上開 Claude Code，貼以下指令：

```
幫我設定 Clawtex cluster worker。步驟：

1. Clone repo（如果還沒有的話）:
   git clone https://github.com/markl-a/Clawtex.git
   cd Clawtex/clawtex-core
   git checkout ralph/money-agent

2. Build release:
   cargo build --release

3. 啟動 Full Worker（背景執行）:
   用 PowerShell 或 bash:
   ./target/release/clawtex-core.exe worker --hub http://10.0.1.1:7878 --name ayaneo --port 7879

   或者背景執行:
   Start-Process -NoNewWindow ./target/release/clawtex-core.exe -ArgumentList "worker --hub http://10.0.1.1:7878 --name ayaneo --port 7879"

4. 驗證:
   curl http://localhost:7879/health
   curl http://10.0.1.1:7878/cluster/workers

每步完成後回報結果。
```

---

## 節點 3：Aspire 5 / Acer（Full Worker + 管理 Android x2）🏠 家裡 LAN

在 Aspire 5 上開 Claude Code，貼以下指令：

```
幫我設定 Clawtex cluster worker。步驟：

1. Clone repo（如果還沒有的話）:
   git clone https://github.com/markl-a/Clawtex.git
   cd Clawtex/clawtex-core
   git checkout ralph/money-agent

2. Build release:
   cargo build --release

3. 啟動 Full Worker（背景執行）:
   nohup ./target/release/clawtex-core worker --hub http://10.0.1.1:7878 --name aspire5 --port 7879 > /tmp/clawtex-worker.log 2>&1 &

   如果是 Windows:
   ./target/release/clawtex-core.exe worker --hub http://10.0.1.1:7878 --name aspire5 --port 7879

4. 驗證:
   curl http://localhost:7879/health
   curl http://10.0.1.1:7878/cluster/workers

5. 設定 Android 1 light worker:
   在 Android 上安裝 Termux，執行:
   pkg install python
   把 clawtex-worker.py 傳到 Android（用 adb push 或檔案分享）
   python3 clawtex-worker.py --hub http://10.0.1.1:7878 --name android-1 --port 7880

6. 設定 Android 2 light worker:
   同樣方式:
   python3 clawtex-worker.py --hub http://10.0.1.1:7878 --name android-2 --port 7880

每步完成後回報結果。
```

---

## 驗證全部節點（在 Z13 上執行）

所有節點部署完後，在 Z13 Hub 執行：

```bash
# 查看所有 workers
curl -s http://127.0.0.1:7878/cluster/workers | python3 -m json.tool

# 預期結果：7 個 workers online
# - m1-mac (full) — via Tailscale
# - ayaneo (full) — LAN
# - aspire5 (full) — LAN
# - android-1 (light) — LAN
# - android-2 (light) — LAN
# - iphone (light) — via Tailscale
# - ipad (light) — via Tailscale

# 查看集群指標
curl -s http://127.0.0.1:7878/cluster/metrics | python3 -m json.tool

# 測試 hand 執行
curl -X POST http://127.0.0.1:7878/hand/cluster_health/run \
  -H "Content-Type: application/json" \
  -d '{"prompt": "Check all nodes"}'
```

---

## 快速參考

| 設備 | 角色 | 位置 | Hub 地址 | 啟動指令 |
|------|------|------|----------|---------|
| Z13 | Hub | 家裡 | — | `clawtex-core --host 0.0.0.0 --port 7878 daemon` |
| M1 Mac | Full Worker | 辦公室 | `<Z13_TS_IP>:7878` | `clawtex-core worker --hub http://<Z13_TS_IP>:7878 --name m1-mac` |
| Ayaneo | Full Worker | 家裡 | `10.0.1.1:7878` | `clawtex-core worker --hub http://10.0.1.1:7878 --name ayaneo` |
| Aspire 5 | Full Worker | 家裡 | `10.0.1.1:7878` | `clawtex-core worker --hub http://10.0.1.1:7878 --name aspire5` |
| Android 1 | Light Worker | 家裡 | `10.0.1.1:7878` | `python3 clawtex-worker.py --hub http://10.0.1.1:7878 --name android-1` |
| Android 2 | Light Worker | 家裡 | `10.0.1.1:7878` | `python3 clawtex-worker.py --hub http://10.0.1.1:7878 --name android-2` |
| iPhone | Light Worker | 辦公室 | `<Z13_TS_IP>:7878` | `python3 clawtex-worker.py --hub http://<Z13_TS_IP>:7878 --name iphone` |
| iPad | Light Worker | 辦公室 | `<Z13_TS_IP>:7878` | `python3 clawtex-worker.py --hub http://<Z13_TS_IP>:7878 --name ipad` |

## 注意事項
- `<Z13_TS_IP>` = Z13 安裝 Tailscale 後拿到的 100.x.x.x IP
- 家裡設備用 LAN IP `10.0.1.1`，辦公室設備用 Tailscale IP
- Hub 綁定 `0.0.0.0` 所以 LAN 和 Tailscale 都能連入
- Full Worker 需要 Rust 工具鏈（cargo）
- Light Worker 只需要 Python 3（純 stdlib，無外部依賴）
- Worker 斷線後會自動用指數退避重連（15s → 120s）
- Tailscale 斷線重連後，Worker 會自動恢復心跳

## Tailscale 故障排除
- `tailscale status` — 查看連線狀態
- `tailscale ping <Z13_TS_IP>` — 測試延遲
- 如果連不上：確認兩端都已登入同一個 Tailscale 帳號
- iOS/iPad 如果 Tailscale 被殺：重開 Tailscale app，Worker 會自動重連
