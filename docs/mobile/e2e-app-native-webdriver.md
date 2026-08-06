# Mac app 原生視窗 E2E（in-app WebDriver）— ✅ 跑通

> **2026-05-31 ✅ 真跑通**（user 機器 MarkdeAir）：`bash scripts/run-native-e2e.sh`
> → 真的開了 Spectyn Mesh **原生 WKWebView 視窗**、WebDriver session 驅動它、
> 4 個斷言全過：`✓ native window has a <body>`、`✓ onboarding shows "Spectyn
> Mesh"`、`✓ provider list shows "OpenRouter"`、`✓ "Anthropic"` →
> `NATIVE-WDIO: PASS (0 failed)` / `NATIVE-E2E RESULT: PASS (rc=0)`。
> 這是 macOS 上「官方說做不到」的原生視窗 E2E（無系統 WKWebView WebDriver），
> 靠 in-app plugin + tauri-wd bridge 達成。
>
> **怎麼跑**：`cd app && bash scripts/run-native-e2e.sh`（會自動起 vite :5173 +
> tauri-wd :4444，開真視窗跑 `node tests/wdio/native-window.mjs`）。

---

# （以下為建置過程 runbook）

> 2026-05-31。task #10。在 macOS 真正驅動 Tauri app 的**原生 WKWebView 視窗**
> （不是瀏覽器 web-shell）。macOS 沒有系統 WKWebView WebDriver，所以用社群方案
> （`tauri-plugin-webdriver-automation` + `tauri-wd` CLI）把 WebDriver server 塞進
> app 自己。架構：`wdio :4444 → tauri-wd → 啟動 debug app → plugin server(動態 port)`。

## ✅ 已完成（本機實證）

1. **plugin dep**：`app/src-tauri/Cargo.toml` → `tauri-plugin-webdriver-automation = "0.1.3"`（commit 2a7241cf）。Cargo.lock 已 commit（05444ffa，+4 crates）。
2. **plugin init**：`app/src-tauri/src/lib.rs` 在 `#[cfg(all(debug_assertions, desktop))]` 下 `builder.plugin(tauri_plugin_webdriver_automation::init())`（commit 32426f59）。release 不含。
3. **debug app 已重 build 並含 plugin**：`cargo build --bin spectyn-mesh-app` → BUILD_RC=0、0 errors；binary 105MB、2026-05-31 20:25；`strings` 抓到 **56 個 "webdriver"**（plugin 真的編進去）。
4. **tauri-wd CLI 已裝**：`~/.cargo/bin/tauri-wd` present（user 跑 `cargo install tauri-webdriver-automation`）；`tauri-wd --help` → W3C server，`--port` 預設 4444。
5. **harness 已寫 + syntax-check 過**（commit 3cef81fa）：
   - `app/wdio.conf.mjs` — 連 tauri-wd :4444，capabilities `[{ 'tauri:options': { binary: <abs debug binary> } }]`（tauri-wd README 文件的確切 key）。
   - `app/tests/wdio/native-window.e2e.mjs` — 斷言原生視窗 render onboarding（"Spectyn Mesh"）+ 真 provider 列表（OpenRouter / Anthropic）。
   - `app/scripts/run-native-e2e.sh` — orchestrate vite :5173 + tauri-wd :4444 + wdio，每步硬 gate、失敗 exit≠0。

## 🔴 唯一卡點：webdriverio 裝不起來（環境 network/proxy 擋）

跑 `run-native-e2e.sh` 目前停在 prereq gate `✗ wdio not installed`：
```
npm i -D webdriverio @wdio/cli @wdio/local-runner @wdio/mocha-framework
→ npm error network 'proxy' config ... NPM_RC=196   (與 cargo install 同類的網路限制)
```
這台機器的 npm 在這個 session 連不出去裝套件。**這是唯一還缺的一塊。**

### 你只要跑這一步，原生視窗 E2E 就能真的跑：
```sh
cd app && npm i -D webdriverio @wdio/cli @wdio/local-runner @wdio/mocha-framework
# 然後（vite + tauri-wd + 真視窗，一鍵）：
cd app && bash scripts/run-native-e2e.sh
```
腳本會自動起 vite、起 tauri-wd、`wdio run wdio.conf.mjs`，啟動**真的 app 視窗**驅動
WKWebView，印 `NATIVE-E2E RESULT: PASS|FAIL`。我會在能裝 wdio 後讀真實 wdio
passing/failing 數字才回報——在那之前**不宣稱**它通過。

## 現有 app 測試保證層（已綠、真實）

原生視窗那一哩接通前，app 已有的真覆蓋：
- **Playwright web-shell GUI E2E**：`gui-interaction.spec.ts` webkit 7/7 + chromium 7/7（webkit 引擎最接近 WKWebView）；`smoke.spec.ts` GUI 2 passed + daemon-API 6 skip（無 daemon 誠實 skip）。
- **headless vitest**：frontend lifecycle 25 + IPC 契約 19。

## 小結
原生視窗層：**plugin 編進去了、CLI 裝好了、harness 寫好驗證了**——只差 `npm i wdio`
（被環境 network 擋），裝完一行腳本就能跑真視窗。其餘 Mac 層（CLI 13/13、TUI 真 PTY
全旅程、App web-shell GUI 7/7×2）都已真跑綠。
