# SPEC-21 Capture Focus — Windows Prototype（原型）

> **Stage 3/3** · [線框稿（Windows）](./SPEC-21-capture-focus-windows-wireframe.md) → [視覺稿（Windows）](./SPEC-21-capture-focus-windows-mockup.md) → 原型（Windows）
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Windows only（**hero 平台是 iOS**；見 [`SPEC-21-capture-focus-prototype.md`](./SPEC-21-capture-focus-prototype.md) for cross-platform 互動骨架 + Nielsen 5 / SUS 樣板）。本檔只列 Windows **deltas** — Tray right-click → context menu pop / 每 menu item tap behavior / `Win+Shift+F` hotkey opt-in flow / ActionCenter toast click → cold launch sequence / `scenario="urgent"` Interrupted 穿透 Focus Assist / Focus Assist 偵測 + in-app fallback banner / SMTC v0.7+ defer / Narrator focus order / AUMID anchoring / DPI scaling on multi-monitor。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-42-PLATFORM-Windows-foundations`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43-PLATFORM-Windows-screens-flows`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)
> **這份的工作範圍**：把 Windows Mockup 變「可操作」 — 每個 tap target / right-click menu item 點下去發生什麼、XAML Storyboard duration、toast scenario timing、cold-launch route 解析、Focus Assist 退化路徑、Narrator focus cycle、DPI swap timing。為 5-user usability test（node-a / node-b / node-a / surface / OEM laptop）準備 walkthrough script + SUS 對齊。
> **三檔分層**：視覺 token / 字級 / 元件尺寸歸 mockup；佈局 / FSM / 螢幕結構歸 wireframe；互動 timing / 手勢 / Tauri command / 失敗路徑歸本檔。

## 為什麼這份要寫獨立 Windows prototype

iOS hero prototype 鎖了「PTT-first mobile / iOS perm prompt / lock-screen MPNowPlayingInfoCenter / haptic」的互動腳本。Windows 完全沒對等：

1. **沒 PTT、沒 haptic** — 桌機鍵盤情境，互動全靠 mouse / keyboard / accelerator
2. **Tray right-click 是唯一 ambient 入口** — Windows tray context menu pop / item dispatch / re-build timing 是 v0.6.0 主流量入口（per SPEC-43 §8.2）
3. **toast persistent + AUMID-anchored** — 跟 iOS UNUserNotification 完全兩套機制，cold-launch route 解析鏈不同
4. **Focus Assist 折疊** — Windows-only 系統態，需 in-app fallback banner 退化路徑
5. **Global hotkey opt-in + fallback chain** — `Win+Shift+F` 預設關閉（避撞 enterprise app），user 開啟後撞了要走 §8.5 fallback 三段（primary → `Ctrl+Alt+F` → user capture mode）
6. **DPI scaling 多顯示器** — node-a 200% / 外接 4K 100% 切換時 tray icon swap timing
7. **AUMID missing self-heal** — 從 v0.5.x in-place upgrade 漏 MSI 場景，首次 toast emit 前要 self-register

→ 七點獨立寫成本檔，不塞 hero prototype 的 Windows 短段。

## Nielsen 5 易用性檢核（Windows 對應）

繼承 [hero prototype Nielsen 5 表格](./SPEC-21-capture-focus-prototype.md#nielsen-5-易用性檢核總攬每個-screen-再對照一次)。Windows-specific：

| 原則 | Windows Focus 表現方式 |
|---|---|
| **可學習性（Learnability）** | tray icon hover tooltip 第一次就揭示「right-click for controls」；Start window 真窗結構直白（duration + Start button）；首次安裝後 onboarding 第 5 步示意 tray menu |
| **效率性（Efficiency）** | tray menu → "Start Focus..." → Start window → Enter 開始（3 tap）；user opt-in `Win+Shift+F` 後 1 hotkey 直達；Recording 中 `Ctrl+Shift+S` 主視窗 active 即停 |
| **記憶性（Memorability）** | tray icon 在右下角 OS 標準位置；menu item 順序鎖定（SPEC-43 §8.2 鎖序）；Recording 時 tray icon 橘變 ambient 提醒不必記 |
| **失誤性（Errors）** | mic 被 enterprise app 搶 → Interrupted toast `scenario="urgent"` 穿透 Focus Assist；hotkey 衝突走 fallback chain；Focus Assist 折疊走 in-app banner |
| **成就感（Satisfaction）** | Done toast persists 到 user dismiss（不會錯過）；takeaway card 落 main window Focus tab；Action Center 歷史可回查 |

## 6 大資料狀態速查（Windows 對應）

繼承 [Windows mockup §6 大資料狀態 — 視覺對映](./SPEC-21-capture-focus-windows-mockup.md#6-大資料狀態--windows-mockup-視覺對映)。本檔列「互動行為」（視覺請見 mockup）：

| 狀態 | 互動觸發 | i18n key 對應 |
|---|---|---|
| **理想（Ideal）** | Done toast persists + 點下去 cold-launch route 到 Focus tab takeaway card | `focus.done.title` + `focus.btn.review` |
| **空白（Empty — History）** | main window Focus tab 首次進入無 session | `focus.empty.history` + `focus.empty.go_to_focus` |
| **空白（Empty — ASR 無語音）** | Finalizing 完成但 transcript 全空 → 不發 toast | `focus.empty.no_speech` |
| **極限（Limit）** | chunk count ≥ 100 顯示 `99+` / takeaway > 800 字截斷 / toast body row 2 > 60 字截斷 | `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` |
| **錯誤（Error）** | mic disabled / Interrupted toast / `R.windows.toast_emit_fail` / Focus Assist fallback | `focus.windows.mic_disabled_by_system` / `focus.interrupted.*` / `focus.windows.focus_assist_fallback` |
| **局部（Partial）** | E Finalizing inline chunk_failed | `focus.partial.chunk_failed` |
| **載入中（Loading）** | E Finalizing + tray icon 橘 + tray header 動態更新 | `focus.finalizing.asr` / `focus.finalizing.llm` |

## FSM 主骨架（per wireframe + SPEC-43）

繼承 [hero 9-state FSM](./SPEC-21-capture-focus-prototype.md#9-state-fsm-主骨架per-wireframe-v03)。Windows 對應 FSM state → screen：

| FSM state | Windows 螢幕 / surface | 互動重點 |
|---|---|---|
| `Idle` | Start window（真窗 480×320px）+ tray icon idle muted | duration picker / Start button / `Win+Enter` accelerator |
| `Recording` | main window C + tray icon `spectyn-tray-focus.ico`（橘）+ tray context menu rebuild | Stop & finalize 提到首項、`Ctrl+Shift+S` accelerator |
| `Chunking` (sub) | tray icon hover tooltip + main window chunk counter | `focus.tray.tooltip_recording` 每 1s update |
| `Interrupted` (sub) | tray icon `spectyn-tray-error.ico`（紅）+ ActionCenter toast `scenario="urgent"` | toast 穿透 Focus Assist + Alarm2 audio |
| `Finalizing` | tray icon 橘持續 + tray header `focus.finalizing.asr` | header 1s debounce update |
| `Done` | main window F takeaway card + Done toast persists | `scenario="default"` 可被 Focus Assist 折疊 |

---

## 螢幕 A — Start Window（真窗 480×320px）

### Nielsen 5 對應（Windows）

- Learnability：title bar 「Start Focus Session」直白；duration chip 三檔 + Start button 兩層結構（不像 macOS popover squeeze 進 menu bar 旁）
- Efficiency：開啟即焦點落「Start」button（per Narrator focus order）→ Enter 直接開
- Memorability：上次 duration `last_duration_min` 預選（Tauri store via `@tauri-apps/plugin-store`）
- Errors：mic disabled → B' 覆蓋層 + deep-link 設定（不靜默失敗）
- Satisfaction：trust badge 解釋「本地加密」減少 enterprise 機 user 焦慮

### Tap targets（按下去發生什麼）

| Target | 動作 |
|---|---|
| `[_]` minimize 鈕 | OS 標準行為 — 縮到 taskbar；session 未啟動，**不**寫 events |
| `[□]` maximize 鈕 | **disabled**（per mockup §122 — window non-resizable）；hover 顯示 OS 預設 "Maximize" tooltip 但點下去無效；建議改用 OS API `WS_MAXIMIZEBOX` 移除 |
| `[X]` close 鈕 | 等同 Cancel — 不寫 events、不啟動 session；同 `Escape` / `Alt+F4` |
| `15` chip | `onClick` → 即時 select（Tauri command `focus_panel_set_duration({minutes: 15})`）；其他兩 chip deselect；display 區「00:00 / 15:00」更新；無 sound / animation 之外的回饋（桌機無 haptic）|
| `25` chip | 同上，「00:00 / 25:00」更新；**預設選中**（首次開啟）|
| `50` chip | 同上，「00:00 / 50:00」更新 |
| **Start button** (label `focus.btn.start_timer`) | (1) 若 `vault_setup_status` 偵測 mic disabled → 切 B' 覆蓋層；(2) 若 mic OK → 0ms 視覺 press（press 樣式 mockup `spectyn-primary @ 80%`）→ 100ms 內 invoke Tauri `focus_session_start({duration: N, source: "start_window"})` → 主視窗（main window）activate + route 到 Focus tab Recording state；Start window 同 frame 關閉（不留殘窗）|
| trust badge（caption 文字）| 不可點（純資訊；hero iOS trust badge 是 tappable，桌機桌面 user 不需深入）|

### Keyboard targets

| Key | 動作 |
|---|---|
| `Tab` | focus cycle：15 chip → 25 chip → 50 chip → Start button → `[X]` close（last）→ wrap 回 15 chip |
| `Shift+Tab` | 反向 cycle |
| `1` / `2` / `3` | accelerator — 直接選 15/25/50 chip（per mockup §133）；focus ring 同步移到該 chip |
| `Enter` on Start button | 等同 Start tap |
| `Win+Enter`（在任何 chip / button focus 時）| 等同 Start tap（per SPEC-43 §12.2 keyboard 導覽契約）|
| `Escape` | 關閉視窗（cancel）— 同 `[X]` |
| `Alt+F4` | OS 規範關閉 |

### Animations / Timings（XAML Storyboard 等同 timing）

- Start window 開啟：Tauri `Window.show()` 後 ≤ 50ms first paint（per SPEC-43 G10：focus panel < 100ms p95）；**不做 fade-in 動畫**（per mockup §361 — user opt-in 從 tray menu 期望立刻 ready）
- chip select：background color transition 200ms `ease-out`（與 hero iOS 同；Win 平台用 CSS `transition: background-color 200ms cubic-bezier(0.0, 0.0, 0.2, 1.0)`，對齊 Fluent `EaseOutCubic`）
- chip focus ring：focus-visible 出現 0ms（無 transition；keyboard nav 需即時回饋）
- Start button press：CSS `transform: scale(0.98)` 80ms `ease-out`；release 80ms `ease-out` 回 1.0；總共 ≤ 160ms
- Start button → 主視窗 transition：Start window `close()` + 主視窗 `setFocus()` 同 frame；無 cross-fade（兩個獨立 window，OS 自然切換）
- `prefers-reduced-motion: reduce` 時：全部 200ms transition 改 instant（per SPEC-43 §12.2 動效契約）

### Failure paths

- **Mic disabled by system**：Start button click → invoke `focus_session_start` 拋 `WASAPI_ERROR_DEVICE_DISABLED` → 覆蓋 B' 顯示 `focus.windows.mic_disabled_by_system`；保留 Start window 不關閉（user 開完設定回來繼續）
- **Mic 被佔用**（其他 app exclusive mode 抓 mic）：拋 `R.windows.mic_busy` → Start window 內顯示 inline toast「麥克風被其他程式使用，請關閉後再試」+ 「重試」按鈕；3s 後自動 retry init
- **AUMID missing**（v0.5.x in-place upgrade → 漏 MSI shortcut）：Start button click 前先 `Window.show()` 後 100ms 內 detect AUMID → 若 missing 直接 self-register via shortcut metadata（per SPEC-43 §7.1.3 + SPEC-42 §8.5）；user 不感知；register 失敗則 toast 全程退到 in-app banner（per `R.windows.toast_emit_fail`）

### Narrator focus order（per SPEC-43 §12.2 + WCAG 2.2 AA）

1. Window title `"Start Focus Session"`（OS auto-read on window open）
2. `15 chip`：`"Duration 15 minutes, button"` + AccessKey `"1"` + AutomationProperty `selected=false`
3. `25 chip`：`"Duration 25 minutes, button, selected"` + AccessKey `"2"` + AutomationProperty `selected=true`（首次開預設）
4. `50 chip`：`"Duration 50 minutes, button"` + AccessKey `"3"`
5. `Start button`：`"Start timer recording, button, Enter to activate"` — focus 落在這（per mockup §138）
6. `trust badge`：作為 `aria-describedby` 掛在 Start button（讀時補上「Encrypted on device, local microphone ASR. Audio never uploaded.」）— Narrator 不 cycle 進

### Walkthrough script（usability test：「請開一段 25 分鐘的 focus」）

1. 預期 user 從 tray 右鍵 → 「Start Focus...」（route：tray menu）開 Start window；或從主視窗 Focus tab 開始（route：main window）
2. user 看到 25 已預選 → 點 Start（無需動 duration）
3. **觀察點 1**：user 是否找到 tray menu？若 30%+ user 直開主視窗，要在 onboarding 第 5 步強化 tray menu 教學
4. **觀察點 2**：user 是否預期 Start window 關閉後 session 還在跑？若 user 焦慮「window 消失就沒了」→ 加 toast「Focus 已開始」（per `focus.start.confirmed` — 待補 i18n key）

---

## 螢幕 B' — Mic disabled 變體（覆蓋 Start window）

### 6 大資料狀態：Error 路徑專屬

per [mockup §141-162](./SPEC-21-capture-focus-windows-mockup.md#螢幕-b--mic-disabled-變體覆蓋-idle)。覆蓋層覆蓋 Start window content（不關 window；Start window 底下 chip / button 套 `overlay-disabled-40` 不可點）。

### Tap targets

| Target | 動作 |
|---|---|
| 「打開設定」按鈕 | invoke Tauri `shell.open("ms-settings:privacy-microphone")` → OS 開系統設定 → 焦點落「麥克風存取」開關；無 confirm；無 sound |
| 「重試」按鈕 | invoke Tauri `focus_session_start({retry: true})` → re-init WASAPI → 若成功覆蓋層淡出 200ms ease-out + 接續 Recording flow；失敗則覆蓋層留住、按鈕 disabled 1s 防連點 |
| 覆蓋層 dim 區（非 button 區）| 不可點（modal-style）；click 無效 |
| `Escape` | 同「重試」（user 開完設定回來常按 Escape；對齊「快速重試」直覺）|

### Animations / Timings

- 覆蓋層 fade-in：200ms `ease-out`（disabled mockup invariant `spectyn-bg @ 92%`）
- 「打開設定」按鈕 hover：bg `spectyn-primary @ 90%` 100ms transition
- 「重試」按鈕 disabled state（剛點過）：套 `overlay-disabled-40` 維持 1s；spinner 16px spectyn-primary stroke 顯示於按鈕中間，1Hz rotate
- 覆蓋層淡出（retry 成功）：200ms `ease-out` opacity 0；之後 cross-fade 250ms 切主視窗 Recording

### Failure paths

- 「打開設定」open shell 失敗（OS deep-link broken；rare）→ inline toast「無法開啟設定，請手動至 Windows 設定 → 隱私 → 麥克風」+ 顯示路徑文字（user 可手動 navigate）
- 「重試」連續 3 次失敗 → 顯示「請重啟 Spectyn Mesh 或檢查麥克風裝置」+ link to support page

### Narrator focus order

1. Heading：`"Microphone disabled in Windows settings"` （從 `focus.windows.mic_disabled_by_system` 解析）
2. 「打開設定」button：`"Open Windows microphone settings, button"`
3. 「重試」button：`"Retry microphone access, button"`
4. 焦點預設落「打開設定」（多數 user 第一次撞 mic disabled 都需要去設定）

---

## 螢幕 C — Recording（main window + tray icon + tray context menu）

### Nielsen 5 對應（Windows）

- Learnability：tray icon 橘（`spectyn-tray-focus.ico`）+ hover tooltip 提示「right-click for controls」；主視窗 Pause / Stop 標籤對齊 hero iOS
- Efficiency：Stop 兩條路徑（主視窗 Stop button 或 tray menu Stop & finalize）≤ 2 操作；`Ctrl+Shift+S` 主視窗 active 即停
- Memorability：tray icon 橘 ambient → user 漏看主視窗瞄一眼右下角即知「我還在錄」
- Errors：mic 被搶 → Interrupted sub-state + `scenario="urgent"` toast 穿透 Focus Assist
- Satisfaction：waveform 跟麥克風連動；chunk count 累積感

### Tap targets — 主視窗（同 hero macOS C）

| Target | 動作 |
|---|---|
| `⏸ 暫停` button | label `focus.btn.pause`。(1) WASAPI session pause；(2) waveform 凍結；(3) 計時 1Hz 閃爍（warning 色，per mockup invariant）；(4) 按鈕變 `▶ 繼續`；(5) tray icon 切 `spectyn-tray-paused.ico`（mic-off muted）；(6) tray hover tooltip 變「Focus 已暫停 — 右鍵繼續」；(7) tray menu rebuild：Pause 變 Resume |
| `▶ 繼續` button | label `focus.btn.resume`。反向：session 重啟、waveform 重跑、計時繼續、按鈕變回 Pause、tray icon 切 `spectyn-tray-focus.ico`（橘）、tray header 切回 recording |
| `⏹ 停止` button | label `focus.btn.stop_finalize`（desktop 用長版）。立即 transition 到 Finalizing screen（**不**加 confirm dialog — 同 hero 效率優先）；invoke Tauri `focus_session_stop` → flush chunk → Chunking → Finalizing |
| chunk count `已落地 chunk: {n}` | 99 → 100 切到 `99+`（`focus.limit.chunk_overflow`，per mockup §341）；tap 無動作 |
| trust badge | 不可點（同 Idle）|
| 主視窗 `[X]` close button | **不關 session** — 主視窗 hide-to-tray（per SPEC-43 §8.2 「tray icon 必常駐」）+ tray icon 維持橘；user click tray menu「Open Spectyn Mesh」可重開 |

### Right-click → Tray context menu pop timing

**從 right-click 到 menu pixel 確認 ≤ 150ms p95**（per SPEC-43 G1）：

1. user `WM_RBUTTONUP` on tray icon
2. `windows_tray.rs::on_right_click()` 0ms
3. `ClusterCache.get_peer_count()` cached 5s → 即時回（< 1ms）
4. Detect Recording state via Tauri shared state（`AppState.focus_session.is_recording`）→ < 1ms
5. **menu rebuild**：Recording 期間 menu items reorder（per wireframe §99）— 重組 5 items（header + Stop + Pause + Open + Settings）≤ 10ms
6. `TrackPopupMenu` 呼叫 OS render menu → OS 控制 paint（典型 50-100ms）
7. 總計 p95 < 150ms（OS render 是主要 budget）

**debounce**：tray icon state 切換（Recording → Paused → Recording 連續 chunk boundary）有 1s debounce（per SPEC-43 §8.1） — 避免 icon flicker。debounce 期間 menu rebuild **使用 current debounced state**（不是 instant state）以保持 visual + interaction 一致。

### Tap targets — Tray context menu items（Recording 期間）

per wireframe §92-103 + mockup §187-201。每 item tap 行為：

| Menu item | Accelerator | Tap 動作 | Tauri command |
|---|---|---|---|
| `Spectyn Mesh · Focus 05:23 / 25:00`（header）| — | **不可點**（per SPEC-43 §8.2 item 1）；hover 也不亮 | n/a |
| `⏹ Stop & finalize` | `Ctrl+Shift+S` | 等同主視窗 `⏹ 停止` button — invoke `focus_session_stop` + activate 主視窗 + route 到 Focus tab Finalizing；menu 自動關閉 | `focus_session_stop({source: "tray_menu"})` |
| `⏸ Pause` | — | 等同主視窗 Pause button；menu 自動關閉；下次 right-click menu rebuild 顯示 `▶ Resume` | `focus_session_pause` |
| `Open Spectyn Mesh` | `Ctrl+O` | activate 主視窗 + route 到 Focus tab（保留 Recording state visible）| `window_focus_main` |
| `Settings...` | — | activate 主視窗 + route `/settings/general` | `settings_open({tab: "general"})` |
| `Quick Log` | — | **disabled**（per wireframe §105 + mockup §210 — Recording 期間避撞）| n/a |
| `Start Focus...` | — | **disabled**（Recording 期間避撞）| n/a |

**accelerator 行為**：`Ctrl+Shift+S` 在主視窗 active 時即生效（全域 menu accelerator；不需註冊 global hotkey）；user 不必右鍵 menu 就可停。menu 開啟時按 accelerator 等同 click 該 item。

**menu 關閉時機**：
- click 任何 item（含 disabled — disabled 不觸發 action 但 menu 仍關）
- click menu 外（包括 tray icon 自身二次 click）→ menu dismiss
- press `Escape` → menu dismiss
- 30s timeout（OS 預設）→ menu dismiss

### Animations / Timings

- waveform refresh：60fps；audio buffer 1024 samples 為一禎（per hero）
- 計時器數字：每秒 update；OS render（無 CSS animation）
- pause/resume icon morph：CSS 200ms `ease-in-out`（同 hero iOS）
- stop → Finalizing：主視窗內 cross-fade 250ms（per hero）
- **tray icon state swap**：1s debounce 後 `spectyn-tray-idle.ico` ↔ `spectyn-tray-focus.ico`（Recording → Paused → Recording 不會閃；連續 chunk boundary 不重新 swap）
- **tray hover tooltip 動態 update**：每 1s 更新計時（取 `focus.tray.tooltip_recording` 變數）；OS tooltip render，無 fade
- **tray menu rebuild**：right-click 觸發 lazy rebuild（rebuild 在 `on_right_click` 內同步跑 ≤ 10ms）；menu 不 cache（每次 right-click 都拿 latest state）

### Background / Multi-monitor / DPI scaling

- **主視窗最小化或關閉**：session 持續，spectyn serve 背景跑（per SPEC-43 §6.3 Flow 2 — spectyn serve emit toast 不依賴 webview alive）
- **Win+L 鎖屏**：session 持續錄音（WASAPI 不受鎖屏影響）；**v0.6.0 無鎖屏控制 UI**（SMTC 留 v0.7+，per wireframe §開放問題 #1）；user 解鎖回主視窗仍見計時器與 waveform 接續
- **DPI scaling 切換**（user 從筆電 200% 切到外接 4K 100%；或反向）：tray icon 從 ico 多 frame 自動挑（16/32/48 三 frame，per SPEC-43 §OoS3）；切換偵測延遲 ≤ 500ms（OS WM_DPICHANGED）；主視窗自動 reflow（Tauri webview 接 system DPI）；Start window 已關不受影響
- **多顯示器 Start window 開啟**：position = active focus monitor 中央（per mockup §124）；user 從 monitor A tray menu 開啟 → window 開在 monitor A；切到 monitor B 時 window 不跟（OS 預設行為）

### Failure paths

- **Mic 被其他 app 搶**（WASAPI exclusive mode，e.g. Discord PTT 切入）：`AudioSessionDisconnectReason::ExclusiveModeOverride` → 進 Interrupted sub-state → 觸發 D' Interrupted toast（見下）；tray icon 切 `spectyn-tray-error.ico`（紅）；主視窗 waveform 不凍結（per hero invariant — desktop interrupt 多源自 mic 被搶、狀態不明顯，靠 toast 提醒）
- **系統 sleep（S3/S4）/ Modern Standby**：spectyn serve 進入 sleep 一同停；wake 後 session 標 `interrupted=true`；剩餘 chunk 仍 finalize；user 回前景見「session 已中斷於 OS sleep」inline message
- **藍牙耳機切換**（mic source 從內建 → BT mic）：`AudioEndpointVolume` event → 平滑切（不進 Interrupted）；UI 無提示（per hero invariant — 桌面 mic source 切換不通知，避過度打擾）；chunk boundary 落在切換點以保兩段 transcript 分離
- **儲存空間滿**（chunk encrypt 失敗）：toast `focus.err.disk_full` + 切 Finalizing（保留已落地 chunk，同 hero）

### Narrator focus order（主視窗 Recording state）

1. Window title `"Spectyn Mesh · Focus Recording"`
2. Pause / Stop button group（focus 預設落 Pause — 常用順序）
3. 計時器（`role="timer"` aria-live="polite"）— 每 30s announce 一次（避免每秒打擾）
4. waveform（`role="img"` aria-label="Recording waveform, animated"）— Narrator 不 cycle 進
5. chunk count（`aria-live="polite"`）— 變 `99+` 時 announce 一次
6. trust badge（`aria-describedby` 掛在 Stop button）

**tray icon 不在 Narrator scope**（OS 限制，per SPEC-43 §12.2）— user 用 `Win+B` 聚焦 system tray 後可方向鍵讀 tooltip。

### Walkthrough script（usability test：「開始錄音、暫停、繼續、停止」）

1. 從 Start window 進 Recording state
2. **觀察點 1**：user 是否注意到 tray icon 變橘？若 < 30% user 看到，需在 onboarding 強化（如首次 Recording 時 toast「右下角橘色圖示表示錄音中」— v0.7+ 評估）
3. user 從主視窗按 Pause → 繼續 → Stop
4. **觀察點 2**：user 是否嘗試 right-click tray menu Stop？若 50%+ user 不知 tray menu 可停，重新考慮 menu 教學
5. **觀察點 3**：user 是否預期關主視窗 = 停 session？若 30%+ user 焦慮，加 first-time hint「關視窗不會停止錄音，從 tray menu 控制」

---

## 螢幕 C' — Interrupted sub-state

### 觸發來源

per [wireframe §111-119](./SPEC-21-capture-focus-windows-wireframe.md#螢幕-c--interrupted-sub-state)：

1. WASAPI exclusive mode 被搶（Discord PTT / Teams / OBS）
2. 系統 sleep（S3/S4）/ Modern Standby
3. 藍牙耳機切換（**不**進 Interrupted — 平滑切，per §螢幕 C Failure paths）
4. Focus Assist activate（**不**影響錄音，只折疊 toast — 走 §螢幕 D 與下方 fallback banner）

### 觸發後動作（per hero invariant + SPEC-43 §15）

1. tray icon 切 `spectyn-tray-error.ico`（紅）— 1s debounce
2. tray header 切 `focus.interrupted.*`（依來源）
3. 主視窗 waveform **不凍結**（per hero invariant — desktop interrupt 視覺上不明顯，靠 toast）
4. 主視窗計時 **不停**（OS interrupt 期間音訊雖斷但 session timer 持續）
5. 30s 寬限內 OS event `.ended` → 自動 resume → tray icon 回橘；無 toast、無 UI 變動
6. 30s 寬限超時 → 強制 finalize（標記 `interrupted=true`）→ 切 Finalizing；toast D' 不再彈（已 finalize；改彈 Done toast D）
7. **若主視窗非 active focus**（user 在別 app）→ **必發 D' Interrupted toast**（per hero invariant line 350 + wireframe §121）

### 寬限數字（per wireframe FSM）

30 秒（同 hero iOS）— Win 平台沿用；user 接電話 / Discord 開關語音通話的常見短中斷涵蓋。長中斷（> 30s）自動 finalize 保資料。

---

## 螢幕 D — Done ActionCenter toast（scenario="default"）

### Tap targets

per [mockup §216-260](./SPEC-21-capture-focus-windows-mockup.md#螢幕-d--done-actioncenter-toastxml-樣板)。toast OS-rendered：

| Target | 動作 |
|---|---|
| toast body（非 action button 區）| trigger toast `launch` URI → `spectyn-mesh://focus/{session_id}` deep-link → 走 `coach_review_open` 同樣機制（per SPEC-43 §9.3）→ cold-launch 主視窗（若已關）→ route `/focus/:id` takeaway card |
| `開啟回顧` action button | label `focus.btn.review`；arguments=`action=open`；activationType=`protocol`；觸發同 toast body click |
| dismiss（user 滑出 / Action Center 全清 / Win+N 開 AC 後逐個關）| toast 進 Action Center 歷史；不寫 events、不啟動任何流程 |
| 自動消失 | **無**（Win 11 toast 在 Action Center 中 persists 直到 user dismiss；不像 macOS NC banner 5s 自動消，per wireframe §136-141）|

### Cold-launch route 解析（toast click → app launch sequence）

per SPEC-43 §6.3 Flow 2 + SPEC-42 §8.5：

1. user click toast / action button
2. OS 解析 `launch` URI → `spectyn-mesh://focus/{session_id}`
3. 若 spectyn-mesh app 已在跑（tray icon 在）→ Tauri `deep-link` plugin 接 URI → emit event `deep-link://focus/{session_id}` → React Router `navigate("/focus/" + session_id)`；耗時 ≤ 100ms p95
4. 若 spectyn-mesh app 未跑（tray icon 不在；user 已 quit）→ OS via `HKCR\spectyn-mesh` registry 找 binary → 啟動 `spectyn-mesh.exe spectyn-mesh://focus/{session_id}` → Tauri 殼 cold-launch → first paint → deep-link route；耗時 ≤ 60s p95（per SPEC-43 perf budget `MSI install + cold-start = 90s combined`，cold-start 自身 ≤ 60s）
5. 主視窗 route 落定 → 顯示 Focus tab takeaway card；scroll 到該 session

**AUMID 驗證**：toast emit 前 `windows_toast.rs` 驗 AUMID 已註冊（`com.spectyn-mesh.app`）；缺失 → 走 §AUMID missing self-heal（見下）

### Animations / Timings

- toast emit budget：spectyn serve `toast_show` call → Action Center render p50 < 250ms / p95 < 500ms（per SPEC-43 G2）
- toast 進場：OS 預設 slide-up 200-300ms（OS 控、Win 11 用 SystemAnimations.SlideUpAnimation）
- toast 在屏顯示：約 5-7s（OS 預設）後自動進 Action Center 歷史 — **不**消失（與 macOS NC 5s 後消失不同）
- 主視窗 cold-launch 後 first paint → takeaway card 顯示：cross-fade 250ms（per hero）

### Focus Assist 互動（Done toast 被折疊路徑）

per wireframe §117 + SPEC-43 §15：

- Done toast `scenario="default"` → 可被 Focus Assist 折疊（user 在 dnd 模式下）
- 折疊狀態：toast **不**即時彈出，但**仍進 Action Center 歷史**（user 之後 `Win+N` 開 AC 可補看）
- **無 in-app fallback banner**（Done 不是 urgent；user 主動看 AC 即可）
- 若 user 完全關閉 toast permission（Settings → Notifications → Spectyn Mesh OFF）→ `R.windows.toast_emit_fail` → 退化到主視窗頂部 in-app banner（見下方 §Focus Assist + in-app fallback banner）

### Failure paths

- **AUMID missing self-heal**：emit 前 detect AUMID 缺 → `windows_toast.rs` self-register（per SPEC-43 §7 line 622）→ retry emit；自 register 失敗 → `R.windows.toast_emit_fail` → in-app banner fallback
- **toast permission OFF**：直接走 in-app banner（per SPEC-43 §9.4 `R.windows.toast_emit_fail` recovery action）
- **Action Center 滿 50 通知**（OS 限制）：舊 toast 自動 evict；新 toast 仍 emit OK
- **Empty 情境（ASR 全靜音）**：**不 emit toast**（per wireframe §172 + mockup §260 — 避免空通知打擾）；主視窗 F 仍顯示安撫文案

### Narrator 行為

Win 10/11 內建 Narrator 整合自動讀 toast（per SPEC-43 §12.2）：
- Read 順序：title `"Spectyn Mesh"` → body line 1 `"Focus 25 min · takeaway ready"` → body line 2 (60 字截斷後的 takeaway) → action button `"開啟回顧, button"`
- user 按 `Caps+Space` 啟動 action button（Narrator 快捷）

---

## 螢幕 D' — Interrupted toast（scenario="urgent"，穿透 Focus Assist）

### Tap targets

per [mockup §262-294](./SPEC-21-capture-focus-windows-mockup.md#螢幕-d--interrupted-toast系統強制觸發)：

| Target | 動作 |
|---|---|
| toast body | trigger `launch` URI → `spectyn-mesh://focus/{session_id}/stop` → cold-launch / activate 主視窗 → invoke `focus_session_stop` → 切 Finalizing → Done |
| `開啟並停止` action button | label `focus.desktop.interrupt_notif_action`；arguments=`action=stop`；activationType=`protocol`；同 toast body click |
| dismiss（user 主動關）| 進 Action Center 歷史；session 仍在 Interrupted state（可能 30s 內 OS event `.ended` 自動 resume；或超時強制 finalize）|

### scenario="urgent" 行為（per mockup §286）

- **穿透 Focus Assist**（Quiet hours / Do not disturb / Focus mode）— Win 11 對 `scenario="urgent"` 給予優先級
- **audio: `Alarm2` looping=false**（per mockup XML §281）— 高優先提示音；user 可在 Win 11 Settings → Notifications → Priority notifications 微調，但預設穿透
- 進場時機：interrupt event 觸發後 ≤ 500ms emit（per G2 same budget）— OS render 即時

### Focus Assist 穿透路徑

per wireframe §159 + SPEC-43 §15：

1. user 在 Focus Assist dnd mode 中
2. Recording → mic 被搶（e.g. Teams 來電）→ 進 Interrupted sub-state
3. 主視窗非 active focus → 必發 D' toast（per hero invariant line 350）
4. toast `scenario="urgent"` → Focus Assist **不折疊**（Win 11 對 urgent 給穿透）
5. 進屏 + Alarm2 音 → user 注意到 → click toast → cold-launch / activate → finalize

### Focus Assist + in-app fallback banner（toast permission OFF 路徑）

per [wireframe §117](./SPEC-21-capture-focus-windows-wireframe.md#螢幕-d--interrupted-toast系統強制觸發) + mockup §294：

**觸發條件**（任一）：
- user 在 Win 11 Settings → Notifications → Spectyn Mesh OFF（完全關閉 app toast）
- `R.windows.toast_emit_fail` 發生（AUMID register 失敗、HRESULT 非 0、ToastNotifier.Show 拋例外）
- Focus Assist 在 "Alarms only" mode（連 urgent 也擋掉，rare）

**Fallback banner 行為**：
- **觸發時機**：toast emit 失敗 detect 後 ≤ 100ms 顯示
- **位置**：主視窗頂部（z-index 在 main content 之上，不阻擋 navigation）；若主視窗未開 → 暫存 banner state，user 開主視窗時即顯
- **視覺**（per mockup §294）：icon Lucide `triangle-alert` 16px spectyn-warning + bg `overlay-recording-16` + 文案 `focus.windows.focus_assist_fallback`「Focus Assist 開啟中，通知改在 app 內顯示」
- **可點關閉**：右上 `×` button → banner 淡出 200ms `ease-out`；user 主動 dismiss 後該 session 不再顯示（per-session state，非 global setting）
- **action button**：「開啟並停止」（Interrupted 變體）或「開啟回顧」（Done 變體）—  click 等同對應 toast action
- **Narrator**：`role="alert" aria-live="assertive"` — Narrator 即時讀出全文（不需 user navigate）

### Failure paths

- 30s 寬限超時 + user 沒 click toast → 強制 finalize → toast 自動 dismiss + Done toast D 接著彈（per Flow 2）
- user click toast 但主視窗 cold-launch 失敗（rare：app crash）→ OS 顯示 spectyn-mesh.exe 啟動失敗 → user 重啟 app + session 已標 `interrupted=true`、`finalized_at=now`，takeaway 仍會在 Focus tab 找到

---

## 螢幕 E — Finalizing（過渡）

per hero E + Windows delta（[wireframe §163-172](./SPEC-21-capture-focus-windows-wireframe.md#螢幕-e--f--finalizing--done同-macos--tray-icon-同步--toast-觸發)）：

### Tap targets

繼承 hero E 全表（`取消並先看逐字稿` / `重試 ASR` / `先用空白 transcript 跑 LLM`）。Windows delta：
- **取消並先看逐字稿**：點下 → 主視窗內 cross-fade 250ms → Done screen；tray icon 仍橘 3 秒後回 idle muted；tray header 切回 idle
- 主視窗 close button：**不**中斷 Finalizing（hide-to-tray，session 仍跑完）— 與 Recording 同 hide-to-tray 行為

### Animations / Timings

- spinner：60°/s 常數（per hero）
- Progress bar：實際反映 chunk transcribed / total
- **tray icon 維持橘** 整段 Finalizing（不切回 idle）— ambient「還在處理」提示
- **tray header 動態更新** `focus.finalizing.asr` →「整理逐字稿 (2/5)」→ `focus.finalizing.llm` →「產生 takeaway 中…」；1s debounce update（與 Recording header update 同 budget）
- ASR 預期：5 chunks ≤ 80s（whisper-cpp small on node-a M.2 NVMe）

### Failure paths

繼承 hero E 全表（ASR 全掛 / LLM 失敗 / 取消 LLM）。Windows-specific：
- **背景 finalize**：主視窗 hide-to-tray 期間 finalize 仍跑（spectyn serve 接管）；Done 時 tray icon flash 橘 3 秒 → 切回 idle；**Done toast 必 emit**（即使主視窗 active 或未開 — per wireframe §170，Windows 不抑制 active focus 通知，與 macOS focus-suppressed banner 行為不同）

---

## 螢幕 F — Done（Takeaway card）

per hero F + Windows delta。

### Tap targets

| Target | 動作 |
|---|---|
| `看完整逐字稿` button | label `focus.done.view_full`。主視窗內 route to `/focus/:id/transcript`；不開新 window |
| `新 session` button | label `focus.done.new_session`。route 回 Focus tab Idle；不開 Start window（直接顯示 Focus tab Idle state — 因主視窗已在）|
| takeaway card（truncated state）| tap 整張 / `focus.limit.view_full_takeaway` CTA → 等同 `看完整逐字稿`（per hero F）|
| 主視窗 `[X]` close | hide-to-tray；下次 tray menu「Open Spectyn Mesh」可重開 + takeaway card 仍在 |
| Done toast click（D 已彈）| 若主視窗已在 takeaway card → 無動作（route 已 settle）；若主視窗未開 → cold-launch + route |

### Entry animation

- 從 Finalizing 進入：cross-fade 250ms（per hero）
- success icon：CSS scale spring `cubic-bezier(0.34, 1.56, 0.64, 1)` duration 400ms 0→1 — 對齊 hero iOS 但 Windows 無 haptic（用 audio 的 `Notification.Default` 對等）
- takeaway card 從 cardOrigin fade-in + slide-up 12px 350ms `ease-out`
- **tray icon Done flash**：橘維持 3s（per mockup §300）→ fade 200ms 切 idle muted
- Done toast emit：與 takeaway card 顯示同 frame（≤ 100ms 差）

### Failure paths

繼承 hero F 全表。Windows-specific：
- **AUMID register 失敗** → Done toast 退化 in-app banner（同 D' fallback 機制）
- **主視窗未開且 toast emit 失敗** → user 不會看到 Done 訊號（rare）；下次開主視窗 → Focus tab 仍見 takeaway card；建議 v0.7+ 加 tray icon Done badge（綠 +）3 秒 ambient

---

## 跨螢幕互動

### Global Hotkey `Win+Shift+F` Opt-in Flow

**v0.6.0 預設關閉**（per wireframe §32 + SPEC-43 §17 Alt-C）— user 至 Settings → Hotkeys tab 手動 enable。

**Opt-in 流程**：
1. user 開 Settings → Hotkeys tab
2. 看到 `Focus start` row + 「啟用全域熱鍵」toggle（預設 OFF）
3. toggle ON → invoke Tauri `hotkey_register({action: "focus_start", accelerator: "Win+Shift+F"})`
4. 走 §8.5 fallback chain：
   - primary `Win+Shift+F` 成功 → toggle 變綠 + 顯示「Win+Shift+F」實際 binding
   - primary fail → 自動退到 `Ctrl+Alt+F`；toggle 變黃（warn）+ 顯示「Ctrl+Alt+F（Win+Shift+F 被佔用）」 + 「Customize...」按鈕
   - 兩個都 fail → toggle 紅 + `R.windows.hotkey_all_fail` 文案 + 「Customize...」進 capture mode
5. capture mode：user 按下任意組合鍵 → `hotkey_capture` 阻塞 10s → 抓到 → 寫進 settings + retry register

**Hotkey 觸發後**：
- user 按 `Win+Shift+F`（或 actual binding）任何時候
- invoke `focus_panel_show({source: "hotkey"})` → 開 Start window（同 tray menu「Start Focus...」path）

### Toast Click → Cold-launch Sequence（完整）

per SPEC-43 §6.3 Flow 2 + 上方螢幕 D：

```
spectyn serve emit toast (background, AUMID-anchored)
    ↓ (Action Center render ≤ 500ms p95)
toast 進屏 + Action Center 歷史
    ↓ (user click)
OS 解析 launch URI: spectyn-mesh://focus/{session_id}
    ↓
    ├─ app 已在跑 (tray icon visible)
    │   → Tauri deep-link plugin 接 URI → React Router navigate
    │     ≤ 100ms 到 takeaway card
    │
    └─ app 未跑 (user 已 quit)
        → HKCR\spectyn-mesh registry → 啟動 spectyn-mesh.exe with URI arg
        → Tauri cold-launch → first paint ≤ 60s p95
        → deep-link route → takeaway card
```

### Tray Icon State Machine + Debounce

per SPEC-43 §8.1 + mockup §168-185：

```
Idle (mic muted, spectyn-muted per mockup)
    ↓ Start session
Recording (mic, spectyn-warning per mockup 橘) ────┐
    ↓ Pause                                      │
Paused (mic-off, spectyn-muted)                  │
    ↓ Resume                                     │
Recording ──────────────────────────────────────┘
    ↓ Mic 被搶 / OS interrupt
Error (mic + 紅點 overlay, spectyn-danger per mockup)
    ↓ Resume (30s 寬限內 OS event .ended)
Recording
    OR
    ↓ 寬限超時
Finalizing (橘維持) → Done (橘 3s) → Idle
```

**Debounce**：每個 state transition 1s debounce — 連續 chunk boundary（每 5 min close + next open）不觸發 swap；rapid pause/resume（< 1s）只 swap 一次（取 debounce 期末 state）。

### Back-button / Window close

| 螢幕 | close button [X] | Escape |
|---|---|---|
| Start window | Cancel — 不寫 events，不啟動 session | 同 close |
| 主視窗 Recording | hide-to-tray，session 持續 | n/a（無 modal）|
| 主視窗 Finalizing | hide-to-tray，finalize 持續 | n/a |
| 主視窗 Done | hide-to-tray，takeaway 已寫 events | n/a |
| B' Mic disabled 覆蓋 | （覆蓋層）關閉 Start window | 同「重試」按鈕 |

---

## 通用 Empty / Limit / Error — 互動補充（視覺 spec 全在 mockup）

繼承 [hero prototype 通用節](./SPEC-21-capture-focus-prototype.md#通用-empty--maximum--error--互動補充視覺-spec-全在-mockup)。Windows 特化：

### Empty state（Focus tab History 首次進入）

- 文案取 `focus.empty.history`；button label `focus.empty.go_to_focus`
- tap 「前往 Focus」→ React Router 切到 Focus tab Idle state（**不開 Start window** — 主視窗內 navigation；Start window 是獨立進入流程）
- transition 250ms cross-fade（Tab switch 標準）

### Limit state（chunk overflow + takeaway truncated）

- chunk count ≥ 100：顯示 `99+`（取 `focus.limit.chunk_overflow`）；chunk 區塊 min-width 48px（per mockup §341）；無 animation
- takeaway > 800 字：fade gradient 底部漸層 + inline `focus.limit.takeaway_truncated_hint` + CTA `focus.limit.view_full_takeaway`（按下 → 同「看完整逐字稿」）

### Error state（global toast / banner）

- 全螢幕級 error（FOCUS-001 perm denied / `R.windows.toast_emit_fail` 退化）：in-app banner（非 OS toast）
- auto-dismiss 6s OR user click `×` close button
- 多個 error 排隊（非疊加）
- 無 sound（避免高頻 error 連響；user 已在 dnd 場景）

---

## SUS（System Usability Scale）題目對齊

繼承 [hero prototype SUS 表](./SPEC-21-capture-focus-prototype.md#sus-system-usability-scale-題目對齊)。Windows-specific 預期差異：

| 題目（簡寫）| Windows 預期評分 | Windows 風險 / 設計依據 |
|---|---|---|
| 1. 想常用 | 3-5 | tray icon ambient 隨手用；風險：enterprise 機 toast permission 預設關 |
| 2. 不必要的複雜 | 2-3 | tray menu rebuild + Start window 兩段；風險：user 不知 tray menu 可控 |
| 3. 容易使用 | 3-5 | Start window 直白；風險：global hotkey opt-in 流程隱藏 Settings 內 |
| 4. 需要技術支援 | 1-3 | mic disabled deep-link 自帶；風險：Focus Assist 折疊 toast 無 surface 提示 |
| 5. 各功能整合度高 | 3-5 | tray + main window + toast 三 surface 對齊；風險：Start window 與主視窗分立或被誤認重複 |
| 6. 太多不一致 | 1-3 | tray icon 配色橘維持「Recording = warning 系」；風險：與 SPEC-43 §8.1 綠點 working 看似衝突（已決定橘，mockup §28-35 拍板）|
| 7. 多數人能很快學會 | 3-5 | 無 onboarding 即可開始；風險：global hotkey 預設關，user 不知有 |
| 8. 使用不靈活 | 1-3 | Start window 不可 resize；風險：multi-monitor user 想拉到第二屏（已決定固定置中於 active monitor）|
| 9. 使用上有信心 | 3-5 | tray icon 橘 + waveform + chunk count；風險：背景 finalize 期間關主視窗 user 不知還在跑 |
| 10. 學了很多才能用 | 1-3 | 預設 25 min Start 即可；風險：hotkey opt-in 進階流程 |

**目標 SUS：65-80 範圍**（同 hero 標準）。**若 < 68**，優先檢討：
1. tray menu 教學（observation：< 30% user 嘗試 right-click tray）
2. Start window 開啟與主視窗的差異（observation：user 把 Start window 當主視窗）
3. global hotkey opt-in 流程 discoverability

---

## 開放問題（prototype 層面，Windows-specific）

1. **Stop & finalize 是否加 confirm**：tray menu 點 Stop 一鍵停（無 confirm，per hero 效率優先）— 但 Windows tray menu click 比 button 更易誤觸（accidental right-click + arrow key）；usability test 若 30%+ user 誤觸 → 考慮 long-hover Stop item 0.5s 才 commit。
2. **`Ctrl+Shift+S` Stop accelerator 是否預設 global**：目前只在主視窗 active 時生效；若改 global 衝突風險高（撞 「Save as...」、PowerPoint Slide Show 等）。傾向維持非 global，user 想要可在 Settings → Hotkeys 自選。
3. **tray icon Done flash 3 秒夠嗎**：3s 短促；user 漏看 → 只能靠 toast。可改 5s 或 toast-driven（toast 一彈即 flash）— 待 usability test 量測。
4. **AUMID self-heal 觸發時機**：目前 toast emit 前 detect；可否改 cold launch detect（避免延遲）— 但 v0.5.x → v0.6.0 in-place upgrade 場景太邊緣，傾向不優化。
5. **Focus Assist 偵測 polling vs event**：Win 11 提供 `Windows.UI.Notifications.Management.UserNotificationListener` API 可取 Focus Assist state，但 v0.6.0 Tauri 2 windows-rs binding 未直接支援；目前走 toast emit 失敗 reactive fallback（不主動 poll）— v0.7+ 評估主動偵測。
6. **DPI scaling 切換時 Start window 行為**：若 user 在 Start window 開啟期間插拔外接顯示器（DPI 變）→ 目前 Tauri 自動 reflow；但 480×320 固定 size 可能在 200% 機看起來太小。考慮加 DPI-aware size（100% = 480×320 / 200% = 960×640）— 但會破壞 mockup 鎖定 §122 「固定 size」決策。

---

## 易用性測試準備

### 7 個 user task — 涵蓋 6 大資料狀態 + Nielsen 5

對齊 [hero 7-task 結構](./SPEC-21-capture-focus-prototype.md#7-個-user-task--涵蓋-6-大資料狀態--nielsen-5)。Windows 特化版：

| # | Task | 測項 | 6-state 覆蓋 |
|---|---|---|---|
| 1 | **首次使用 + Empty state**：「請從 tray 開啟 Spectyn Mesh，看 Focus tab 的 History（預期空白），再開一段 25 分鐘 focus」 | tray menu 教學 + Empty state + Start window flow | **Empty**（History）/ Loading（mic init）/ Ideal（done）|
| 2 | **Hotkey opt-in**：「請從 Settings 啟用 Win+Shift+F 全域熱鍵，並用它開始一段 focus」 | Settings → Hotkeys tab + hotkey register fallback chain + capture mode | Ideal / Error（若衝突走 fallback）|
| 3 | **背景 + tray ambient**：「開始錄音後關閉主視窗，繼續工作 5 分鐘後從 tray 停止錄音」 | tray icon ambient + hide-to-tray + tray menu Stop | Loading（背景 finalize 感）/ Ideal |
| 4 | **Interrupted**：「錄音中請開啟 Discord 並 join 語音通道（觸發 mic 搶佔），檢查 toast」 | WASAPI exclusive mode + Interrupted toast scenario=urgent + 30s 寬限 | Error（中斷態）|
| 5 | **Done flow**：「Recording 完成後從 toast 點開回顧，看 takeaway」 | toast cold-launch / activate + deep-link route | Ideal |
| 6 | **Maximum state**：「設定 50 分鐘 timer，模擬連續 100+ chunk（測試環境注入）後停止」 | chunk 99+ overflow + takeaway truncation | Maximum |
| 7 | **Focus Assist + Partial state**：「開 Win 11 Focus Assist 並錄音，模擬第 3 個 chunk ASR 失敗，從 Finalizing 跟 Done 卡讀懂」 | Focus Assist 折疊 toast + in-app fallback banner + partial inline | Partial / Error（fallback banner）/ Maximum |

### Sampling

- 目標 5-7 user（per Nielsen「5 個 user 找 80% 問題」）
- 角色：3 power user OEM laptop + 2 OSS contributor dev box + 2 enterprise 環境 group policy 機（per SPEC-43 §5 personas）
- 環境：
  - **node-a**（200% DPI / 內建 mic / Win 11 23H2）
  - **node-b**（100% DPI / Bluetooth headset / Win 11 22H2）
  - **node-a**（150% DPI / 觸控 + 鍵鼠 / Win 11 23H2）
  - **surface**（200% DPI / Studio mic / Win 11 23H2 + Enterprise GPO）
  - **OEM laptop**（100% DPI / 預裝 SmartScreen 嚴格 / Win 10 22H2）

### 觀察重點

- **screen A**：Start window 開啟後 user 是否找到 Start button？25 min 預選是否引起「為什麼是 25？」疑問？
- **screen C**：user 是否注意 tray icon 變橘？關主視窗後是否焦慮「session 還在嗎」？
- **screen C'**：mic 被搶後 Interrupted toast 是否在 dnd / Focus Assist 中真正穿透？
- **screen D**：toast click cold-launch 是否被 user 誤認「重啟 app」（rather than「打開回顧」）？
- **screen E**：Finalizing 期間 user 切走 app → 再回來時是否預期 session 已 done？tray icon 橘是否被 user 看到並理解？
- **Hotkey opt-in**：user 是否能找到 Settings → Hotkeys？fallback warning 訊息是否清楚？
- **Focus Assist task**：fallback banner 是否被 user 視為「正常」而非「異常」？

### 紀錄方式

- 螢幕錄影 + camera（user 同意後；camera 拍實機 + 外接顯示器，捕捉 tray icon 變化）
- think-aloud protocol
- 結束後 SUS 問卷 + 5 題開放問題（最讚 / 最差 / 困惑點 / 期望加 / 期望砍）
- 量測：tray dropdown render p95（per SPEC-43 G1，target < 150ms）、toast emit p95（G2，target < 500ms）、cold-launch p95（target < 60s）

---

## 下一步

→ 五個 Windows 實機（node-a / node-b / node-a / surface / OEM laptop）跑 7-task usability test → 收 SUS 分數 + tray dropdown / toast 實機 p95 + Focus Assist 穿透實測
→ 若 < 68 SUS 或實機 p95 不達 SPEC-43 G1/G2 → 回頭修 wireframe / mockup / prototype 對應點
→ Windows prototype 通過 usability 驗證後，**SMTC v0.7+ 鎖屏控制**評估啟動（per wireframe §開放問題 #1）— 桌面三平台一起加（mac / Win / Linux）
→ 跟 macOS prototype + Android prototype（即將寫）對齊 cross-platform invariants 矩陣 — 確保「Recording = warning 系」「Stop ≤ 2 操作」「desktop 中斷強制系統通知」三條共線
→ Re-baseline SPEC-43 §8.1 tray icon state matrix — 補進「focus-recording」第 5 列（mockup §35 建議）
