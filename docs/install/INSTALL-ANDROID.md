# spectyn 在 Android 上 — 安裝指南

兩種型態（flavor），你可以在**同一台裝置上同時執行兩者**：

| 型態 | 它能給你什麼 | 安裝時間 |
|---|---|---|
| **Tauri APK（精簡客戶端 thin client）** | 在主畫面放一個原生 app 圖示，點開後是一個 webview（網頁檢視容器），用來和你的 Mac/Linux 上的 spectyn serve 對話。觸控友善的聊天介面。 | 約 30 秒 |
| **Termux worker（無介面 headless 或 TUI）** | 在手機上跑一個真正的 `spectyn serve` 常駐程式（daemon）。它會加入叢集（cluster）、接受派發的任務，並提供和你在 Mac 上一樣的 ratatui TUI（文字使用者介面）。 | 約 2 分鐘 |

> 兩者的前置需求（prerequisites）：手機要連到與協調者（coordinator）相同的
> Tailscale tailnet（虛擬區網），而協調者（一台執行
> `spectyn serve` 的 Mac/Linux）要能透過它的 Tailscale IP 連到。從手機上：
> `ping <coord-ts-ip>` 應該要成功。

---

## A. Tauri APK — 原生精簡客戶端

當你想要一個**主畫面圖示**、點開後直接進入 spectyn 行動版介面時，就用這個。最適合「我只想跟我的叢集聊天」這種使用情境。

### A.1. 下載 APK

在手機的瀏覽器（Chrome / Firefox / Samsung Internet — 任何一個都行）中開啟：

```
http://<COORDINATOR-TS-IP>:7878/dist/spectyn-mesh-android.apk
```

把 `<COORDINATOR-TS-IP>` 換成你的 Mac/Linux 節點的 Tailscale IP
（例如 `<mac-tailscale-ip>`）。這個 APK 約 96 MB（通用版 universal：arm64-v8a + armv7
+ x86 + x86_64 全部打包在一起、已簽章、採用 v2/v3 簽章方案）。

### A.2. 安裝

點一下下載好的 APK。Android 會顯示「為了你的安全，你的手機
不允許從這個來源安裝未知的應用程式」— 點 **設定
→ 允許來自這個來源 → 返回 → 安裝**。

每個瀏覽器你只需要做一次。

### A.3. 首次啟動

點一下新出現的 **Spectyn Mesh** 圖示。你會看到一個設定表單：

```
Connect to spectyn serve

Host:  localhost          ← change to your coordinator's TS IP
Port:  7878
                          [Connect]
```

填入協調者的 IP，連接埠（port）維持 `7878`，點 **Connect**。
app 會把這個設定記在 localStorage（瀏覽器本機儲存）裡；之後再啟動就會直接
進到聊天介面。

### A.4. 你會看到什麼

和協調者上 `/m` 所提供的那個深色奶油（dark-cream）行動聊天介面一模一樣：
ANSI 著色的串流、修飾鍵列（`@` `/` `⇥` `↑` `↓` `■`）、
可摺疊的工具呼叫（tool call），以及即時的 SSE（伺服器推送事件 Server-Sent Events）token 串流。

### A.5. 之後切換協調者

開 Chrome → `spectyn://localhost:1430` 這招**行不通** — Tauri 的
WebView（網頁檢視）不會從外部暴露它的 localStorage。正確做法是直接
**解除安裝再重新安裝** APK 來重置，或使用行動版介面中即將推出的
`/Settings` 路由（見下方 roadmap）。

---

## B. Termux worker — 在手機上跑真正的 `spectyn serve`

當你想讓**手機成為一個真正的叢集成員**時就用這個 —
Mac（或任何對等節點 peer）都可以把 `subagent({node: "<phone-ts-ip>:7879"})`
任務派發給它。它同時也提供和 Mac 版一模一樣的 **ratatui TUI**，只是跑在 Termux 裡面。

### B.1. 從 F-Droid 安裝 Termux

**重要**：請從 [F-Droid 安裝 Termux](https://f-droid.org/en/packages/com.termux/)，
**不要**從 Play Store 安裝。Play Store 版本自 2020 年起就不再更新，
也缺少現行的套件。

### B.2. 執行 bootstrap 啟動腳本

開啟 Termux，貼上這**一行**（把 Groq key 換成你自己的，
可從協調者上的 `~/.spectyn-mesh/env` 取得）：

```bash
COORD=http://<mac-tailscale-ip>:7878 \
GROQ_KEY=gsk_your_groq_key_here \
  curl -fsSL "$COORD/scripts/termux-setup.sh" | sh
```

這個腳本會做的事（約 2 分鐘）：
- `pkg install` 安裝 curl/wget/git/termux-tools
- 從 `<COORD>/dist/...` 拉取最新的
  `spectyn-aarch64-linux-android`
- 寫入 `~/.spectyn-mesh/agents.toml`，內含 cluster_secret（叢集密鑰）+
  協調者 URL
- 在背景啟動 `spectyn serve --port 7879` 並驗證
  healthz（健康檢查端點）
- 印出一個三選一選單（TUI / 瀏覽器 / 叢集 worker），讓你知道
  接下來該做什麼

### B.3. 三種使用方式

腳本跑完後：

**1) ratatui TUI** — 全螢幕的互動介面，和 Mac 上完全相同：
```bash
spectyn
```
這會佔住（block）整個 Termux 工作階段（session）。如果你想讓 worker
在你使用 TUI 的同時繼續執行，請另外開一個 Termux 工作階段。

**2) 瀏覽器 / PWA**（漸進式網頁應用程式 Progressive Web App）— 聊天式介面：
```
http://<phone-ts-ip>:7879/    ← this device's serve
http://<coord-ts-ip>:7878/m   ← Mac coordinator's mobile UI
```
加到主畫面就會有一個 PWA 風格的圖示。

**3) 維持無介面（只當 worker）** — 由 Mac 派發任務給它：
```ts
mcp__spectyn__subagent({
  agent: "master",
  prompt: "echo hello from node-a",
  node: "<phone-ts-ip>:7879",
})
```

### B.4. Termux 開機時自動啟動（選用）

安裝 [Termux:Boot](https://f-droid.org/packages/com.termux.boot/)，
然後建立開機腳本：

```bash
mkdir -p ~/.termux/boot
cat > ~/.termux/boot/spectyn-serve <<'EOF'
#!/data/data/com.termux/files/usr/bin/sh
~/.spectyn-mesh/bin/spectyn serve >> ~/.spectyn-mesh/data/spectyn-serve.log 2>&1 &
EOF
chmod +x ~/.termux/boot/spectyn-serve
```

手機重新開機後，Termux:Boot 會在背景靜默啟動 worker。

---

## 在同一台裝置上結合 A 與 B

兩者不衝突 — A 是一個 webview 客戶端，B 是一個常駐程式（daemon）。node-a 上常見的
設定方式：

- **B 以無介面方式執行** → 100.64.0.10:7879 就是一個叢集可以派發任務的
  真正 worker。
- **A 放在主畫面** → 點它就會打開聊天介面（指向
  Mac 的 `:7878`，或它自己本機 serve 的 `:7879` 都可以）。

如果你把 A 指向**手機自己的** serve（`localhost:7879`），
手機就同時跑介面和它自己的 LLM（大型語言模型）呼叫 — 完全可離線運作
（只要填好供應商 provider 的金鑰）。

---

## 疑難排解

### 「App not installed as package appears to be invalid」（應用程式未安裝，套件似乎無效）

APK 是在不穩定的連線下載的，檔案毀損了。重新下載
並驗證：

```bash
# On the phone after download:
curl -I http://<coord>:7878/dist/spectyn-mesh-android.apk
# Content-Length should be ≥ 90 MB. If not, re-download.
```

### Tauri app 顯示「Connection refused」（連線被拒）

協調者的 `spectyn serve` 從手機這邊連不到：

1. 手機的 Tailscale 開了嗎？（通知列上的 VPN 圖示）
2. 協調者的 `spectyn doctor` 顯示 healthz OK 嗎？
3. macOS 防火牆有沒有擋住 :7878 的對外連線？系統設定 →
   網路 → 防火牆 → 允許 spectyn

### Termux 腳本在 `pkg install` 階段失敗

```bash
pkg upgrade -y
pkg install -y curl wget git termux-tools
# then re-run the bootstrap line
```

### 手機一直把 worker 砍掉（尤其是 Xiaomi/Vivo）

這是 OEM（原始裝置製造商）電池最佳化的怪癖。設定 → 應用程式 → Termux → 電池 →
**不受限制（Unrestricted）**。在小米 MIUI 上你可能還需要：
設定 → 應用程式 → Termux → 其他權限 → 自動啟動（Autostart）**開啟**。

### `spectyn doctor` 中「APFS snapshots: tmutil reachable」這一列看起來很怪

那一列是 macOS 專用的。Android 版的 spectyn 二進位檔沒有
APFS 快照（snapshot）工具（它被 `#[cfg(target_os = "macos")]` 條件編譯擋住了）。在
Android 上 `spectyn doctor` 會直接省略 macOS 整合那一段。

### Worker 已啟動但 Mac 無法派發 — 找不到 `node`

手機的 TS IP 可能變了：

```bash
# In Termux:
ip -4 addr show | awk '/100\./ {print $2}' | cut -d/ -f1
```

依此更新你 subagent 呼叫中的 `node:` 參數（或你
`agents.toml` 中的 `[cluster].peers`）。

---

## 驗證來回往返（round trip）

從 Mac 上執行（Mac 上跑著 spectyn serve、手機上跑著 worker
時）：

```bash
# 1. Phone reachable on its Tailscale IP from the Mac:
curl -fsS http://<phone-ts-ip>:7879/healthz
# Expect: ok

# 2. HMAC dispatch through cluster RPC:
# 請設定你自己的共享密鑰（須與各節點 SPECTYN_CLUSTER_SECRET 一致）
SECRET="changeme-cluster-secret"
BODY='{"agent":"master","prompt":"reply: OK from android"}'
AUTH=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$SECRET" -hex | awk '{print $2}')
RESP=$(curl -s -X POST "http://<phone-ts-ip>:7879/rpc/task/assign" \
  -H "X-Cluster-Auth: $AUTH" -H "Content-Type: application/json" -d "$BODY")
JOB=$(echo "$RESP" | sed -n 's/.*"job_id":"\([^"]*\)".*/\1/p')
sleep 5
curl -s "http://<phone-ts-ip>:7879/rpc/task/status/$JOB"
# Expect: {"status":"done","output":"OK from android",...}
```

如果兩項都通過，node-a（或任何 Android 裝置）就是一個功能完整的
叢集 worker。

若要進行更深入的端對端（end-to-end）驗證（15 個階段，涵蓋 CLI / 常駐程式 /
端點矩陣 / web / MCP stdio / HMAC / 真實 LLM / 叢集 /
TUI / autoevolve / Termux:Boot / 壓力測試 / 失效模式 / 清理），
請見 [`../mobile/SMOKE-ANDROID.md`](../mobile/SMOKE-ANDROID.md)。

---

## 發展藍圖（Roadmap，尚未推出）

- **內建協調者切換器** — 不必解除安裝/重新安裝，直接在 app 內部
  變更主機/連接埠
- **APK 內的前景服務（Foreground Service）** — 即使 Android 想砍掉背景
  app，也能讓 worker 保持存活
- **以 GitHub Releases 作為 APK 的正式來源** — 屆時 `spectyn serve`
  會直接從那裡拉取，不再需要某個對等節點來代管二進位檔
- **推播通知（Push notifications）** — 當長時間執行的任務完成時通知你
