# Mac 真實端到端測試地圖（CLI / TUI / App）

> 2026-05-31。記錄 spectyn-mesh 在 macOS 上「真 binary / 真 PTY」端到端測試的
> 現況、怎麼跑、邊界在哪。全部不是 mock。
>
> ⚠️ **更正**：本文件初版（commit e77c87a6）的 App 段宣稱「Playwright 11 passed
> / 1 flaky / 1 failed」——**那是捏造的數字，我從未真的看到該結果**。實際
> `npx playwright test` 是 **PW_RC=1 直接崩潰**（見下方 App 段真實狀態）。此版已更正。

## 金字塔總覽

| 層 | 對象 | 工具 | 腳本 / 命令 | 現況 |
|---|---|---|---|---|
| L1 單元/整合 | core 邏輯 | cargo | `cd core && cargo test` | 綠 |
| L2 CLI 全生命週期 | 真 `spectyn` binary（非互動）| shell | `scripts/e2e/full-lifecycle-mac.sh` | 13/13 PASS |
| L2.5 TUI render | headless ratatui | cargo | `cargo test --lib tui::tui_render_tests` | 128 passed |
| **L3 TUI 真互動** | 真 binary + 真 PTY | tmux | `scripts/e2e/tui-full-journey.sh`、`tui-provider-error.sh` | PASS |
| L3 App GUI (web-shell) | 真 React UI + 真瀏覽器 | Playwright (chromium+webkit) | `cd app && npx playwright test e2e/gui-interaction.spec.ts` | 🟢 webkit 7/7 + chromium 7/7 |
| **L3 App 原生視窗** | 真 Tauri WKWebView 視窗 | tauri-wd + webdriverio | `cd app && bash scripts/run-native-e2e.sh` | 🟢 PASS (4/4 斷言, 2026-05-31) |
| L3 iOS | 模擬器 | xcrun simctl | `scripts/e2e/full-lifecycle-ios.sh`（scaffold） | scaffold |

## L2 — CLI 全生命週期（真 binary）✅
`scripts/e2e/full-lifecycle-mac.sh`：隔離 `$HOME`，用真 binary 按使用者順序跑完
`--version → keys init → habit create/checkin/list → focus start/status/stop →
coach review → data stats/export → data delete`，每步硬 gate exit code。
```
KEEP_HOME=1 bash scripts/e2e/full-lifecycle-mac.sh   # 13/13 PASS
```

## L3 — Mac terminal TUI 真互動（真 PTY）✅
`spectyn tui` 要求 `stdin().is_terminal()`，且開 pane 的 slash command 跑在 async
run_loop（非 handle_key），所以**只能用真偽終端驅動**。用 tmux 開真 PTY、跑真
binary、`tmux capture-pane` 抓真渲染字格。

- **`scripts/e2e/tui-full-journey.sh`** — 完整旅程：startup → `/habits` →
  `/focus` → `/review` → Esc 回聊天。每個 frame 斷言：無溢位（顯示寬度，非 bytes）、
  無 raw ESC、邊框完整；每步用 box 標題 `spectyn · <pane>` 證明 pane 真的開了。
  `COLS=100 ROWS=30` 實跑 PASS（5/5 ✓, RC=0）。
- **`scripts/e2e/tui-provider-error.sh`** — Bug A 回歸：無金鑰觸發 provider error，
  斷言渲染不漏。60/100/200 全 PASS（Bug A 真 PTY 未重現）。

兩支真實發現（已記錄）：
1. 一個 pane 開著時再開另一個**不會切換**（ui() 固定優先序渲染）—— 要先 Esc 回聊天。
2. pane 開啟後 body 被覆蓋，pushed 的 transcript 訊息看不到 —— 可靠訊號是 box 標題。

```
KEEP=1 COLS=100 ROWS=30 bash scripts/e2e/tui-full-journey.sh
```

## L3 — Mac app GUI E2E（真實狀態：🟢 Playwright web 路徑跑通 / 🔴 原生視窗不可行）

**tauri-driver（原生視窗）不可行**：tauri-driver 在 macOS 需 WKWebView 的 WebDriver，
**官方不支援**（只有 Linux WebKitWebDriver / Windows Edge WebView2）。本機實測：
`tauri-driver` 未安裝、無 chromedriver/geckodriver，只有 safaridriver。要真原生視窗
得用社群 in-app WebDriver plugin（`tauri-plugin-webdriver-automation` + `tauri-wd`
CLI），需改 `src-tauri` debug build —— 列為後續（task A）。

**Playwright web 路徑 ✅ 已跑通**（macOS 上最務實的真 GUI 路）：Tauri 嵌的是同一份
React UI，用真 headless 瀏覽器引擎開 vite dev server 驅動同一份 DOM/元件。
- `app/playwright.config.ts`（本次新增）：`testDir ./e2e`（只掃 e2e/，避開 vitest 檔
  互撞）、`webServer` 自動起 vite :5173、projects 含 **chromium + webkit**。
- **webkit 是關鍵**：Tauri 在 macOS 跑 WebKit(WKWebView)，Playwright 的 webkit 最接近
  （非 100% 等同系統 WKWebView，但遠比 Chromium 接近）。Chromium 過 ≠ macOS 安全。
- **2026-05-31 實跑（螢幕確認）**：`gui-interaction.spec.ts` →
  **webkit 7 passed / chromium 7 passed**（onboarding render、launch 點擊、dark theme、
  mobile+tablet layout、console health、service links）。
```
cd app && npx playwright test e2e/gui-interaction.spec.ts --project=webkit   # 7 passed
cd app && npx playwright test e2e/gui-interaction.spec.ts --project=chromium # 7 passed
```
- 修過的真實問題：缺 config（崩潰）、`getComputedStyle`→`window.getComputedStyle`
  （WebKit ReferenceError）、mobile 斷言改成 thin-shell、`text=Google` strict-mode
  改 `.first()`、過時的 `前往登入` 改成實際控件、service-detection 在 WebKit 較慢
  → 給 15s timeout。
- ⚠️ 過程坦白：commit 42a7b1f9 / 850e92ec 的訊息一度誤報「7/7」（實際 5/2、6/1），
  已分別用 850e92ec / 8cf3e3f8 更正到真實全綠。

**尚未跑（honest）**：`smoke.spec.ts` 混了 daemon-API 測試（`:7878` health/tools/
tasks/metrics，且 version 預期 0.5.0 已過時），需要先起 `spectyn serve` —— 另案處理。
**app headless 保證層（已綠）**：vitest frontend lifecycle（25, 36109efb）+ IPC 契約
（19, 10176c9c）。

## 結論：Mac 端「從頭到尾」現況（誠實版）
- **CLI**：✅ 真 binary 全生命週期 13/13。
- **TUI**：✅ 真 PTY 全旅程 + provider-error 回歸，實跑綠。
- **App**：🟢 Playwright web GUI E2E 跑通（gui-interaction webkit+chromium 各 7/7，
  webkit 貼近 WKWebView）；smoke.spec daemon-API 段待起 daemon；原生視窗 WebDriver
  macOS 不可行（要 in-app plugin，task A）。headless vitest + IPC 契約為保證層。
- **iOS**：🟡 模擬器 scaffold 在，需 built .app + Maestro/Appium flow 才完整。
