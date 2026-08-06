# SPEC-21 Capture Focus — Prototype（原型）· Linux

> **Stage 3/3** of the Linux user-flow chain · [線框稿（Linux Wireframe）](./SPEC-21-capture-focus-linux-wireframe.md) → [視覺稿（Linux Mockup）](./SPEC-21-capture-focus-linux-mockup.md) → 原型（Linux Prototype）
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Linux only。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-prototype.md](./SPEC-21-capture-focus-prototype.md)），本檔只列 Linux **interaction deltas** + Linux-specific timing / DE fallback 互動鏈。Linux 是 **best-effort 平台**（DE 碎片化 — GNOME / KDE / Xfce / sway / i3 行為各異），寫法比 mac/win 更務實 — **接受視覺與行為的降級**，不追求所有 compositor 完美對齊。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-44-PLATFORM-Linux-foundations`](../specs/v060-deep-spec/SPEC-44-PLATFORM-Linux-foundations.md) · [`SPEC-45-PLATFORM-Linux-screens-flows`](../specs/v060-deep-spec/SPEC-45-PLATFORM-Linux-screens-flows.md)
> **這份的工作範圍**：把 Linux Mockup 變「可操作」 — 每個 tap target（GTK CSD button / SNI tray click / libnotify action / .desktop action / deep link）點下去發生什麼、Linux-specific timing（GTK animation、webkit2gtk 渲染、D-Bus round-trip）、6 大資料狀態 + Nielsen 5 + 7 tasks + SUS 對齊。視覺 token / 字級 / 色彩 / 元件尺寸歸 Linux mockup；佈局 / 螢幕結構 / FSM 規格歸 Linux wireframe — 三檔嚴格分層。

## 為什麼 Linux prototype 比 mac / Win 更短

hero iOS prototype 488 行涵蓋 9-state FSM 全互動。Linux 共用 FSM（per Linux wireframe §跨 OS 對映） — 不重抄。Linux 獨有需要鎖：

1. **GTK CSD button** 互動（hover / press / focus ring）— 跟 web `:hover` / `:focus-visible` 對映
2. **SNI tray icon click** D-Bus 行為（left click / right click / middle click）+ AppIndicator extension fallback
3. **libnotify action button** D-Bus 回呼鏈（`ActionInvoked` signal） + body click `default` action + 無 actions compositor 救濟
4. **Wayland portal vs X11** 視窗 raise / focus 行為差異
5. **Sway / i3 tiling** 下 main window 行為（接受被 tile）
6. **DE detection** 鏈（`XDG_CURRENT_DESKTOP` → exec 對映 → fallback）
7. **不承諾 global shortcut**（user 自接 wmctrl / KWin bindsym）— prototype 給教學文件指引

→ 約 350-400 行，比 hero 精簡。

## Nielsen 5 易用性檢核（Linux 對映）

| 原則 | Linux Focus 表現方式（vs iOS hero） |
|---|---|
| **可學習性（Learnability）** | main window Focus tab 為 canonical surface（永遠在）；tray / libnotify 是 bonus。首次進 Focus tab 不需 onboarding 即可看懂 25/50/自訂 + 開始按鈕；trust badge 文案一字不差跨平台 |
| **效率性（Efficiency）** | GTK CSD 真窗單擊即開始；無 mic perm prompt（PA/PW 直接成功）→ 進 Recording 比 mobile 快；tray click 在 KDE / Xfce 為 1-click 入口 |
| **記憶性（Memorability）** | Focus tab 版式跟 hero iOS Idle 對得起來（trust badge / 三檔 duration / start button 位置同）；tray icon 三態（idle / recording / paused）跨 distro 一致 |
| **失誤性（Errors）** | 無 OS perm gate → 改走 B' device-error 螢幕（mic 找不到 → 「打開音訊設定」DE-aware deep link）；headless / libnotify 不可用 → main window inline 訊息救濟；不靜默失敗 |
| **成就感（Satisfaction）** | F Done takeaway card 落在 main window；libnotify low 通知（不打斷）；tray icon 從 warning 切回 idle 給「結束了」視覺訊號 |

## 6 大資料狀態 — Linux 互動補充

per Linux wireframe §6 大資料狀態 — Linux 對映表，prototype 層補互動行為（視覺不重述）：

| 狀態 | Linux 互動 |
|---|---|
| **理想（Ideal）** | F Done card 落 main window Focus tab + libnotify low 通知 + tray icon 切回 idle mono |
| **空白（Empty - History）** | Focus tab 內 history 區無 row → 顯示空狀態 illustration + 「前往 Focus」按鈕 → 點擊 focus radio panel 自動 scroll into view |
| **空白（Empty - ASR 全靜音）** | F 卡片顯示安撫文 + 「重錄這次」/「完成」按鈕；libnotify **不發**（避免騷擾） |
| **極限（Limit）** | C chunk count 數字 ≥ 100 切 `99+` 純態切換（無轉場動畫，per mockup invariant）；F takeaway > 800 字 truncate + 「看完整摘要」CTA 等同 `[看完整逐字稿]` 主按鈕 |
| **錯誤（Error）** | B' No-mic 卡覆蓋 Focus tab 主區；interrupted libnotify critical + timeout 0 → user 須處理；headless / `LINUX-SCREEN-LIBNOTIFY-NO-SERVICE` → main window inline banner |
| **局部（Partial）** | E Finalizing inline `focus.partial.chunk_failed` 出現後續續執行不阻擋（progress 跳過失敗 chunk） |
| **載入中（Loading）** | E spinner + progress 持續更新；tray icon 切 attention 變體（best-effort — GNOME mono 限制下靠 main window title 補強） |

## 9-state FSM 共用（per Linux wireframe §跨 OS 對映）

FSM 共用 hero iOS 9-state（Idle / Requesting / Recording / Chunking / Interrupted / Finalizing / Transcribing / SummaryGen / Done）。Linux delta：

- **Requesting state 退化為 micro-state**（per Linux wireframe §B）— PulseAudio / PipeWire 無 OS prompt，`Requesting` 只在 `cpal::default_input_device()` 抓 device 那 ≤ 50ms 出現；多半 user 看不到
- **Interrupted sub-state** 在 desktop **無專屬 UI 變體**（waveform 不凍結、計時不停）但**強制 libnotify critical 通知**（per hero invariants + Linux wireframe §C'）

---

## 螢幕 A — Focus Tab / Start Window（Idle 等價）

### Nielsen 5 對應

- Learnability：Focus tab 為 canonical surface — 永遠在 main window；3 個 duration radio + goal tag input + start button 跟 hero iOS Idle 版式對得起來
- Efficiency：GTK CSD 真窗單擊開始；無 mic perm prompt
- Memorability：上次 duration 預選（store `last_duration_min` in `~/.config/spectyn-mesh/state.json` per SPEC-44 storage spec）
- Errors：start 按下若 mic 抓不到 → 走 B' device-error，**不靜默失敗**
- Satisfaction：trust badge `focus.trust_badge` 一字不差跨平台

### 6 大資料狀態

per Linux mockup §A — 視覺 spec 在 mockup，此處只列互動行為。

| 狀態 | 互動 |
|---|---|
| 理想 | 3 radio 可選；start button enabled；trust badge 顯示 |
| 空白 | 同理想（Focus tab Idle 無「無資料」一說） |
| 極限 | 自訂 input > 180 → 失焦時 clamp 回 180 + GTK style class `error` 加 80ms × 3 shake（CSS animation，**比 iOS shake 略短** — GTK transition 預設 200ms 怕被吃掉，鎖死 80×3=240ms） |
| 錯誤 | mic 硬體不存在 → start button **不 disabled**（避免 user 困惑「為什麼按不下」）；按下後走 B' device-error 螢幕（per Linux wireframe §A → B' flow） |
| 局部 | n/a |
| 載入中 | n/a |

### Tap targets

| Target | 動作 |
|---|---|
| `25 分鐘 Pomodoro` radio | GTK toggle button group — 0ms 切換；其他 radio deselect；webkit2gtk render frame ~16ms；無 haptic（Linux 桌面無 haptic） |
| `50 分鐘` radio | 同上 |
| `自訂 [N] 分鐘` radio | 切換 + 顯示 inline number input（5-180） — input focus 進去後 user keyboard input 觸發 IME composition（中文 IM 不打斷 input flow） |
| 自訂 input 失焦 | clamp 5-180；out-of-range 觸發 shake animation 80ms × 3（總 240ms）+ webkit2gtk render 額外 2 frames jank |
| `目標標籤` text input | 接 user 鍵盤 input；逗號分隔多 tag；無 dropdown autocomplete v0.6.0（v0.7+ 加） |
| **`開始` button — click** | (1) attempt `cpal::default_input_device()` → 若 `None` 走 B' device-error；(2) 若有 device → 跳 Recording screen（Focus tab 內容區切換）；webkit2gtk transition ≤ 100ms（用 CSS `transition: opacity 80ms ease-out` 而非 GTK 原生 stack transition — 為跨 distro 一致） |
| `取消` button | 關閉 Start window（若是獨立 sheet）或 reset Focus tab 內 form state（若在 main window 內）；無 confirm |
| trust badge tap | 開啟 `TrustExplainerView` — 用 GTK `Gtk.Dialog` modal（**不是 web modal**） — 為跟 GTK 視窗排序對齊；ESC 關閉 |

### Animations / Timings

- Radio 切換：CSS `transition: background-color 150ms ease-out`（比 iOS pill 切換 200ms 略快 — GTK 視覺慣例偏快速）
- Start button hover：`transition: background-color 80ms ease-out`（per Linux mockup §A `spectyn-primary @ 90%`）
- Start button press：`transition: background-color 40ms ease-in`（press 比 hover 快、release 比 press 慢，給「按下去」反饋）
- Custom input shake：keyframes 0/33/66/100% × 80ms = 240ms total，translate -10px / +10px / -10px / 0
- Start → Recording 切換：webkit2gtk content swap ~80ms ease-out（避免 jank 加 reduced-motion `prefers-reduced-motion` query 直接 0ms 切）

### Failure paths

- **mic 不存在**：start click → `cpal::default_input_device() == None` → 200ms 內切到 B' device-error 卡（覆蓋 Focus tab 主區）；卡上「打開音訊設定」按鈕走 DE detection 鏈
- **PipeWire socket 不存在**（headless / minimal distro）：start click → `pw_context_connect` fail → B' device-error 卡 + 文案改顯「無 PipeWire 服務」（v0.7+ 補 i18n key，v0.6.0 fallback 用 `focus.err.no_mic`）
- **webkit2gtk 渲染失敗**（極罕見，distro packaging 問題）：fallback 用 GTK native widget（v0.7+，v0.6.0 接受 crash log + restart）

### Walkthrough script（usability test：「請開始一段 25 分鐘的專注錄音」）

1. 預期 user 從 main window Focus tab 進入 → 看到 25/50/自訂 radio 與 start button
2. 預期 user 直接點「開始」（25 是預設）→ 切到 Recording screen
3. **觀察點**：user 是否預期 mic perm prompt？若 50% 以上 user 等待 perm prompt 然後困惑「為什麼沒問就開始錄」，要在 trust badge 加「Linux 桌面無 mic 權限提示，本 app 直接使用系統預設麥克風」hint（v0.7+，v0.6.0 接受降級）

---

## 螢幕 B — Mic Permission（Linux 無此螢幕）

per Linux wireframe §B + Linux mockup §B — PulseAudio / PipeWire 不 prompt，直接成功走到 C。**prototype 不需描述互動**。

---

## 螢幕 B' — No-mic Device Error

### Nielsen 5 對應

- Learnability：Lucide `mic-off` 64px + 標題「找不到麥克風裝置」直白；不需文檔即懂
- Efficiency：「打開音訊設定」一按到位（DE-aware deep link）
- Errors：若 DE 偵測不到對應 binary → button disabled + 顯示文字提示「請從系統設定開啟音訊面板」，**不假裝可點**

### 6 大資料狀態

| 狀態 | 互動 |
|---|---|
| 理想 | mic-off icon + 標題 + body + 「打開音訊設定」button enabled |
| 錯誤 | DE 偵測測不到任何 audio control binary → button disabled（per Linux mockup §B' 對映表）|

### Tap targets

| Target | 動作 |
|---|---|
| **`打開音訊設定` button** | (1) 讀 `XDG_CURRENT_DESKTOP` env；(2) 對映表 dispatch（per Linux mockup §B' 對映表）— GNOME → exec `gnome-control-center sound` / KDE → `kcmshell5 kcm_pulseaudio` / Xfce → `pavucontrol` / 其他 → fallback `xdg-open audio:///`；(3) exec 用 `std::process::Command::spawn` 不 block UI；(4) 200-800ms 後音訊設定 panel 出現（時間 distro 不一） |
| 卡片外點擊 | **不關卡片**（device error 是阻擋態，必須處理）— 跟 iOS Denied 卡同邏輯 |
| ESC 鍵 | 關卡片 + 回 Focus tab Idle 狀態（user 可重試但 mic 仍不存在）— 給 escape hatch 避免卡死 |

### Animations / Timings

- 卡片進入：webkit2gtk content swap fade-in 250ms ease-out（比 hero iOS Denied 350ms 略快 — GTK 視覺偏快）
- button hover/press 同 Screen A start button

### Failure paths

- **DE detection 命中但 binary 不存在**（罕見 — user 自家刪了 `gnome-control-center`）：`Command::spawn` returns `ErrNotFound` → fallback 走 `xdg-open audio:///` → 若 URI scheme handler 不存在 → toast `focus.err.no_audio_panel` + button 變 disabled 並顯示文字「請從系統設定開啟音訊面板」
- **權限不足無法 spawn**（極罕見 — SELinux / AppArmor 限制）：spawn 回 PermissionDenied → toast + 文字 fallback

### Walkthrough script（usability test，假裝 mic 不存在）

1. user 點 start → 預期切到 Recording → 實際看到 B' device-error
2. **觀察點 1**：user 是否能在 5 秒內理解「mic 找不到」？若 30% 以上 user 困惑「為什麼點開始卻顯示這個」，要在 button text 加「重試」對等 v0.7+
3. user 點「打開音訊設定」→ 預期音訊面板開啟
4. **觀察點 2**：對應 DE 開出對的 panel 嗎？（測 GNOME / KDE / Xfce 三家）

---

## 螢幕 C — Recording

### Nielsen 5 對應

- Learnability：計時器大數字 + Pause/Stop 兩按鈕直白；tray icon 變色當輔助提示
- Efficiency：Stop ≤ 2 操作（hero invariant 跨平台一致） — main window 直接點 Stop / 或 tray right-click → Stop
- Memorability：版面結構與 hero iOS C 對應（trust badge / 計時器 / waveform / chunk count 位置同）
- Errors：interrupted → libnotify critical（強制觸發 per hero invariants） + main window 內 inline 提示
- Satisfaction：waveform 即時跳動 + chunk count 累積；tray icon warning 色給「在錄」感

### 6 大資料狀態

| 狀態 | 互動 |
|---|---|
| 理想 | 計時器跑；waveform 連動；chunk count 每 5min +1（觸發 Chunking sub-state） |
| 空白 | n/a（Recording 必有 audio stream） |
| 極限 | 50min / custom max 180min 到 → 自動 stop；計時器最後 1s 閃紅切 Finalizing；chunk count ≥ 100 切 `99+` 純態（無轉場動畫，per mockup invariant） |
| 錯誤 | interrupted → libnotify critical + main window 內 inline 提示（per Linux wireframe §C'）|
| 局部 | n/a（per chunk fail 在 E 顯示） |
| 載入中 | n/a |

### Tap targets

| Target | 動作 |
|---|---|
| `⏸ 暫停` button | label 取 `focus.btn.pause`。(1) cpal / PipeWire stream pause；(2) waveform 凍結（CSS class `paused` 加 `spectyn-muted` 色）；(3) 計時 1Hz 閃爍（CSS `animation: blink 1s infinite`）；(4) button label 切 `▶ 繼續`（取 `focus.btn.resume`）；(5) tray icon 切 paused 變體（Lucide `mic-off` spectyn-muted） |
| `▶ 繼續` button | 反向：stream resume；waveform 重跑；計時繼續；button label 切回 `⏸ 暫停`；tray icon 切回 recording 變體 |
| `⏹ 停止` button | label 取 `focus.btn.stop_finalize`（desktop 用長版，per hero invariant + Linux mockup §C-tray）。立即 transition 到 Finalizing screen（**不加 confirm dialog** — per hero iOS C 同邏輯）；webkit2gtk cross-fade 250ms；AudioRecorder.close；flush 殘留 chunk → 觸發 Chunking → Finalizing 鏈 |
| chunk count `已落地 chunk: {n}` | 顯示用變量；99 → 100 切 `99+`（取 `focus.limit.chunk_overflow`）；tap 無動作（純資訊）；v0.7+ 可 tap 跳 history |
| trust badge tap | 同 Screen A trust badge |

### Background / Tray 行為

- **進背景（user 切到其他 app）**：main window 失焦 — Recording 繼續（systemd `--user` spectyn.service 在 background 跑，不是 Android FGS 概念，per Linux wireframe §C）
- **Tray icon 同步**：app 進背景時 tray icon 切 `NeedsAttention` 狀態（SNI status，GNOME extension `appindicatorsupport` 支援、KDE / Xfce 預設可見）；GNOME 無 tray 情境靠 main window title 加 `[Recording]` prefix 救濟（per Linux mockup §C-tray + Linux wireframe §6 大資料狀態）
- **systemd-logind suspend signal**：`PrepareForSleep` D-Bus signal → 進 Interrupted sub-state；libnotify critical（per Linux wireframe §C'）
- **mic 被搶**（PipeWire `node-removed` 或 PA `source-output-removed`）：同上，進 Interrupted + libnotify critical

### Animations / Timings

- waveform refresh：60fps（CSS canvas 或 SVG path animation，**避免 GPU 不可得情境的降級** — 若 webkit2gtk 偵測無硬體加速 → 降到 30fps）
- 計時器數字：每秒 update，無 animation
- pause/resume icon morph：CSS `transition: transform 200ms ease-in-out`（icon 用 Lucide 兩個 SVG fade-cross 而非 morph — 跨 distro 一致）
- chunk +1 flash toast：bottom-up enter 100ms / hold 1.5s / exit 200ms（同 hero iOS）
- stop → Finalizing：webkit2gtk content swap cross-fade 250ms
- **tray icon 切換 timing**：SNI `IconThemePath` 改變後 100-500ms 內 panel 反映（DE-dependent — KDE 快、GNOME extension 慢）

### Failure paths

- **mic 被搶**：PipeWire `node-removed` event → 觸發 Interrupted sub-state；fire libnotify critical（per Linux mockup §C'）；waveform **不凍結**（per hero invariants desktop variant）；計時 **不停**；user 可從 main window Stop 或 libnotify action button 「開啟並停止」
- **libnotify daemon 不在**（`LINUX-SCREEN-LIBNOTIFY-NO-SERVICE` — headless server / minimal distro）：D-Bus call fail → fallback main window inline banner（紅色 banner 顯示「焦點時段中斷 — 5:23/25:00 · mic 被佔用」）；tray icon 仍切 attention 變體
- **儲存空間滿**：chunk encrypt 失敗 → toast `focus.err.disk_full` + 切 Finalizing（保留已落地 chunk）
- **Wayland portal 拒絕 raise window**（libnotify action → 試圖 raise main window）：portal call fail → fallback `wmctrl -a spectyn-mesh`（X11 only）/ sway IPC `[app_id="spectyn-mesh"] focus`（Sway only）/ KWin DBus（KDE only）— prototype 接受「使用者可能要自己切窗」降級

### Walkthrough script（usability test：「開始錄音 30 秒、暫停、繼續、停止」）

1. user 從 Idle → 25min start → Recording screen
2. **觀察點 1**：user 是否注意到 tray icon 變色？若 50% 以上 user 沒看 tray，要在 main window 加更明顯的 recording 指示（v0.7+，v0.6.0 接受降級）
3. user 按暫停 → 預期 waveform 凍結 + tray icon 切灰
4. **觀察點 2**：user 是否認得「▶ 繼續」按鈕？icon morph 是否清楚？
5. user 按繼續 → 按停止 → 切 Finalizing

---

## 螢幕 C-tray — Tray right-click menu（best-effort）

### Tap targets（per Linux mockup §C-tray）

| Target | 動作 |
|---|---|
| Tray icon **left click**（KDE / Plasma） | raise main window（D-Bus org.kde.StatusNotifierItem `Activate`）；視窗 raise timing：KWin / X11 ~ 50-200ms / Wayland portal 看 compositor |
| Tray icon **left click**（GNOME + appindicatorsupport） | extension 通常 open dropdown menu 而非 raise window；v0.6.0 接受此差異 |
| Tray icon **middle click** | v0.6.0 不接（避免誤觸） |
| Tray icon **right click** | open dropdown menu（per Linux mockup §C-tray）— 跨 SNI / AppIndicator 一致 |
| Menu item `⏹ 停止並收工` | 等同 main window 內 Stop button；觸發 D-Bus `Activate` 後 panel 處理 menu close；100-300ms 後 Finalizing screen 出現於 main window |
| Menu item `⏸ 暫停` | 等同 main window 內 Pause；同樣 100-300ms timing |
| Menu item `Open Spectyn Mesh` | raise main window + 切 Focus tab；行為同 left click on KDE |

### Failure paths

- **GNOME 無 `appindicatorsupport` extension**：tray 不存在 → menu 不存在；user 須從 main window 控制；onboarding（per SPEC-45 §6.4 step 4）已提示安裝
- **Sway / i3wm**：tray 行為依 swaybar / i3bar 配置，v0.6.0 接受「可能 tray 不可見」；CLI fallback `spectyn-mesh-app --focus-stop`（v0.7+）

---

## 螢幕 C' — Interrupted（libnotify critical）

### Nielsen 5 對應

- Errors：強制 libnotify critical（per hero invariants） + main window inline 訊息雙保險；body click + action button 雙救濟（per Linux mockup §C' compositor 落差）
- Satisfaction：n/a（這是錯誤態）

### libnotify interaction

per Linux mockup §C' libnotify hint 規格 — prototype 鎖互動：

| Tap target | 動作 |
|---|---|
| **Action button「開啟並停止」** | D-Bus `ActionInvoked` signal → spectyn 收到 → (1) raise main window（DE-aware：KWin / sway IPC / wmctrl）；(2) trigger Stop flow；(3) main window 切 Finalizing screen；總 timing ~ 100-500ms |
| **Notification body click** | D-Bus `default` action invoked → 同上 raise main window；KDE / GNOME Shell 多支援；dunst / mako 視 user 設定 |
| **Notification dismiss**（user 揮掉 / X） | `NotificationClosed` signal → spectyn 收到；session **保持 Interrupted state**（30s 寬限繼續跑）；若寬限過 → 強制 finalize（per wireframe FSM） |

### Compositor 支援度落差 — prototype 鎖二級救濟

| Compositor | actions 顯示 | 救濟 |
|---|---|---|
| KDE Plasma / GNOME Shell | OK | 一級：action button click；二級：body click |
| xfce4-notifyd | 部分支援 | 一級：action button click（若有）；二級：body click；三級：main window banner |
| dunst / mako（看 user config） | 多半不顯示 actions | 一級：body click（若 user 啟用 `default` action）；二級：tray icon attention 變體 + main window banner |

**main window banner 救濟**：libnotify D-Bus call 不論成功失敗，**同時** 在 main window 顯示 inline banner「焦點時段中斷 — 5:23/25:00 · mic 被佔用 — 30 秒內回復將自動繼續 — [開啟並停止]」（紅色 banner，固定 main window 頂部） — 避免單一通道失敗導致 user 完全感受不到

### Timing

- libnotify D-Bus call：spectyn 觸發 → D-Bus round-trip ~ 5-50ms（local D-Bus，快）→ 通知出現於 panel ~ 50-300ms（compositor 處理）
- main window banner：immediate（webkit2gtk render ≤ 16ms）
- Interrupt 30s 寬限：per wireframe FSM；prototype 不重述秒數

### Failure paths

- **D-Bus session bus 不存在**（極罕見 — headless 或 user 沒登 graphical session）：libnotify call fail → fallback main window banner（已並行觸發）+ tray icon attention（若 tray 存在）
- **`org.freedesktop.Notifications` 服務未註冊**：同上，fallback main window banner

---

## 螢幕 D — Lock-screen（不存在）

per Linux wireframe §D + Linux mockup §D + hero NG6 — **Linux 無對等品**，loginctl 不對等 macOS MPRemote / Win SMTC。**prototype 不描述**。

**user 鎖屏期間行為**：

- Recording 繼續（systemd `--user` service 跑著）
- chunk write 繼續
- libnotify interrupted（若觸發）通常**仍會 fire**，但 lock screen 上 user 看不到（KDE / GNOME 在 lock screen 不顯示 notification 預覽） → 解鎖後可見
- user 解鎖回 main window 仍見計時器繼續、waveform 跑著

---

## 螢幕 E — Finalizing

### Nielsen 5 對應

- Learnability：訊息字面直白（沿用 hero `focus.finalizing.asr` / `focus.finalizing.llm`）
- Errors：每條 ASR / LLM path 各有失敗訊息 + 重試按鈕；同 hero 邏輯

### 6 大資料狀態

per Linux mockup §E — 視覺 spec 在 mockup。互動同 hero iOS E（screen E section）— **不重抄**。Linux delta：

| 狀態 | Linux 互動 delta |
|---|---|
| 理想 | spinner + progress bar（CSS animated，**不用 GTK native spinner** per mockup） |
| 載入中 | 同上；tray icon 切 attention 變體（best-effort） |
| 局部 | inline `focus.partial.chunk_failed` Lucide `triangle-alert` 14px |
| 錯誤 | 所有 ASR 掛 → FOCUS-003 + `重試 ASR` + `先用空白 transcript 跑 LLM` 兩按鈕（沿用 hero）；**Linux 無 cloud fallback 選項**（per SPEC-44 信任邊界 — whisper.cpp on-device 唯一） |
| 極限 | 50min audio + 10 chunks 預期 ASR ~ 60-120s（whisper.cpp small on x86-64 AVX2）；CPU-only 無 AVX2 機器可能 > 3min → 觸發 `focus.finalizing.taking_longer` hint |

### Tap targets

per hero iOS E section — 沿用全部 tap target（`取消並先看逐字稿` / `重試 ASR` / `先用空白 transcript 跑 LLM`）。Linux delta：

- 螢幕 swipe-down：n/a（desktop 無 swipe）— 改用 ESC 鍵：v0.6.0 **disabled** (避免 user 誤觸 cancel finalize)；v0.7+ 可考慮 ESC 觸發 `取消並先看逐字稿`
- Finalizing 期間關 main window：**finalize 繼續跑**（systemd `--user` service） — 完成後 libnotify done 通知；user 重開 main window 看 history 或 deep link

### Timings

- ASR 各 chunk：whisper.cpp small on x86-64 AVX2 ~ 4-6s / 1min audio（M-series Mac 6-8s 為參考）
- 5 chunks 序列 ~ 20-30s（AVX2 / GPU offload）／ 60-180s（CPU-only 老機器）
- 無 AVX2 機器 fallback 走 whisper.cpp tiny model（精度降但速度回到可接受） — user 透過 settings 切換（v0.7+，v0.6.0 接受降級）

### Failure paths

per hero iOS E section — 沿用。Linux delta：

- **whisper.cpp binary missing / corrupted**：fallback prompt user 重灌 spectyn-mesh package（v0.6.0 顯示 toast `focus.err.asr_binary_missing` v0.7+ key，v0.6.0 fallback 用 `focus.err.no_takeaway`）
- **磁碟 IO 失敗**（chunk write fail）：同 hero iOS — toast + 切 Finalizing 保留已落地 chunk

---

## 螢幕 F — Done（Takeaway card + libnotify low）

### Nielsen 5 對應

- 沿用 hero iOS F — 不重抄

### 6 大資料狀態

| 狀態 | Linux 互動 |
|---|---|
| 理想 | takeaway 三段；libnotify low 通知 fire；tray icon 切回 idle mono |
| 空白（ASR 無語音） | 卡片顯「本次時段未偵測到語音，已為您記錄時長」（取 `focus.empty.no_speech`） + 「重錄這次」/「完成」雙按鈕；**libnotify 不發**（per Linux mockup §F Empty 變體） |
| 極限 | takeaway > 800 字 truncate + 「看完整摘要」CTA = `[看完整逐字稿]` 主按鈕（per hero F Limit invariant） |
| 錯誤 | 同 hero F — 全 ASR 掛 → takeaway = "(無 audio 可分析)" + 重跑 ASR 按鈕 |
| 局部 | 同 hero F — banner 引 `focus.partial.chunk_failed` |

### Tap targets

| Target | 動作 |
|---|---|
| `看完整逐字稿` button | label 取 `focus.done.view_full`；切 main window 內 transcript view（不是 separate window） |
| `新 session` button | label 取 `focus.done.new_session`；reset Focus tab Idle state |
| takeaway card truncated state → 「看完整摘要」CTA | 等同點 `[看完整逐字稿]` 主按鈕（per hero F Limit invariant），**不在原地展開** |
| **libnotify Done action「開啟」** | D-Bus `ActionInvoked` → spectyn deep link `spectyn://focus/done/<session_id>` → raise main window + 切 Focus tab + scroll 到該 session |
| **libnotify Done body click** | 同上 `default` action 觸發 |
| `空白變體 - 重錄這次` button | reset 並回 Focus tab Idle，保留 duration / goal tag |
| `空白變體 - 完成` button | 同 `新 session` |

### Entry animation

- 從 Finalizing → Done：webkit2gtk content cross-fade 250ms（不用 push transition — Linux 桌面慣例偏 fade）
- success icon scale-in：CSS `animation: scale-spring 400ms cubic-bezier(0.5, 1.5, 0.5, 1)`（CSS approximation of iOS spring）
- takeaway card fade-in + slide-up 12pt 持續 350ms ease-out（同 hero iOS）
- **無 haptic**（Linux 桌面無 haptic API）
- **libnotify done 通知** 同時 fire（D-Bus ~ 5-50ms round-trip）

### Failure paths

- **libnotify daemon 不在 / D-Bus 不可得**：通知不發；user 仍在 main window 看到 Done card（main window 為 canonical surface）
- **tray icon 切換失敗**（SNI service unregistered）：tray 不更新；接受視覺降級（main window Done card 為主要訊號）

---

## 跨螢幕互動

### Interruption Flow（per Linux wireframe §C'）

```
Recording ──[PipeWire node-removed / systemd PrepareForSleep / PA source-output-removed]──>
            Interrupted sub-state
              ↓
   (waveform 不凍結 / 計時不停 — per hero invariants desktop variant)
              ↓
   libnotify critical fire（urgency=critical, timeout=0）
   + main window inline banner（紅色頂部 banner，雙保險）
   + tray icon 切 attention 變體（best-effort）
              ↓
   [PipeWire node 回 / systemd suspend 取消 / user click action button]
              ↓
   30s 寬限內回 → Recording 繼續
   30s 寬限過 → 強制 finalize（interrupted=true 標記）→ Finalizing
```

### Back / 視窗 close 行為（Linux 桌面慣例）

| 螢幕 | 視窗 close button（X）行為 |
|---|---|
| Idle / Focus tab | 關 main window；無 confirm；下次重開仍在 Focus tab（per SPEC-45 §10.2 state restore） |
| Recording | 關 main window 不停 Recording（systemd service 跑著）；tray icon 仍可見；user 從 tray 或 libnotify 控制 |
| Finalizing | 關 main window 不取消 finalize；user 重開 main window 看 Done 卡 / 或從 libnotify done 通知打開 |
| Done | 關 main window；session 已存 events |

**ESC 鍵行為**：

| 螢幕 | ESC |
|---|---|
| A / Idle | n/a |
| B' device-error | 關卡片回 Idle |
| C Recording | n/a（不接 ESC — 避免誤觸 stop） |
| C' Interrupted libnotify | n/a（OS 處理） |
| E Finalizing | v0.6.0 disabled（避免誤觸 cancel）；v0.7+ 可考慮觸發 `取消並先看逐字稿` |
| F Done | 關 main window（同 close button） |

### Wayland portal vs X11 — 視窗 raise 行為

per Linux wireframe §跨 OS 對映 `Wayland portal 多半拒絕` — prototype 接受降級：

| 來源 | X11 raise | Wayland raise |
|---|---|---|
| Tray click → raise main window | `wmctrl -a spectyn-mesh` or D-Bus | KWin DBus (KDE) / sway IPC / GNOME Shell extension only — 可能失敗 |
| libnotify action → raise main window | 同上 | 同上 — fallback：通知本身在前景時 user 自然會看到 |
| deep link `spectyn://focus/done/...` → raise | 同上 | 同上 |

**降級策略**：raise call 失敗時 — spectyn 不報錯（避免錯誤 cascade），仰賴 user 自己切窗（task bar / Activities / Super 鍵 / wofi 等）

### Sway / i3 tiling 下 main window 行為

- main window 預設被 tile（與其他 windows 平分螢幕） — spectyn **不抗拒**
- user 想 float main window：自行寫 sway rule `for_window [app_id="spectyn-mesh"] floating enable`（per SPEC-45 §3.2 NG3 — user 自接）
- focus session 期間 main window 被 tile → Recording 仍正常跑（不依賴 window size）；waveform 自動 scale（CSS responsive layout）
- chip 浮動小視窗（v0.7+ feature）— 不在 focus flow 內，per Linux wireframe §invariants `spectyn-mesh-chip` WM_CLASS 不適用 focus

### Global shortcut `⌘⇧F` — 不承諾（per SPEC-45 §3.2 NG3）

- v0.6.0 **不註冊 global shortcut** — Wayland 多半拒絕、X11 XGrabKey 可但跟其他 app 衝突風險
- 教學文件指引 user 自接：
  - **KDE**：System Settings → Shortcuts → Custom Shortcuts → 新增 `spectyn-mesh-app --focus` 對應 `Meta+Shift+F`
  - **GNOME**：Settings → Keyboard → View and Customize Shortcuts → Custom shortcut
  - **Sway**：在 `~/.config/sway/config` 加 `bindsym $mod+Shift+f exec spectyn-mesh-app --focus`
  - **i3**：類似 sway
  - **Xfce**：Settings Manager → Keyboard → Application Shortcuts
- v0.7+ 評估用 D-Bus org.freedesktop.portal.GlobalShortcuts portal（XDG portal spec） — 但目前實作支援度低

### Theme switch（GTK dark/light follow）

per Linux wireframe §invariants + Linux mockup §design token — 5s 內切：

- spectyn 監聽 `org.freedesktop.appearance.color-scheme` D-Bus signal（XDG portal） + `gsettings monitor org.gnome.desktop.interface color-scheme`（GNOME） + KDE `kdeglobals` file watch
- 偵測到變化 → webkit2gtk 注入 CSS class（body 加 `.theme-light` / `.theme-dark`） → CSS variable 切換 → render 1 frame ~ 16ms
- **webkit2gtk < 2.40 fallback light**（不爆畫面）

---

## 通用 Empty / Maximum / Error — 互動補充

per Linux mockup §通用狀態 — 視覺 spec 全在 mockup。Prototype 鎖互動：

### Empty state（Focus tab 內 history 區無 session）

- 文案取 `focus.empty.history`（同 hero）
- 「前往 Focus」button：tap → scroll focus radio panel into view（同 tab 內，不切 tab）；webkit2gtk smooth scroll 300ms ease-out；無 haptic

### Empty state（ASR 無語音 — F 卡片變體）

- 文案取 `focus.empty.no_speech`
- 「重錄這次」button：reset state 回 Focus tab Idle，保留 duration / goal tag
- 「完成」button：同 `新 session`
- **libnotify 不發**（per Linux mockup §F Empty 變體）

### Maximum state（custom duration > 180）

- input 失焦 → CSS shake animation 80ms × 3 = 240ms total + GTK style class `error` 加紅框 → 1s 後 class 移除
- input 強制 clamp 回 180
- 顯示 `focus.limit.max_duration_hint` inline hint
- 下限 5min 同樣處理

### Maximum state（chunk count ≥ 100）

- 純態切換 `99+`，無轉場動畫（per mockup invariant）
- 數字區塊 min-width 48px 鎖死（同 hero iOS C 規範）

### Error state（global toast — libnotify 或 main window inline）

- 觸發時機：任何全螢幕級 error（B' device-error / interrupted critical / disk full / etc.）
- libnotify 路徑：urgency 對應（critical for interrupted / normal for disk full） + body click 救濟
- main window inline 路徑：webkit2gtk render banner（紅 / 黃色依嚴重度）；user 可手動關閉 ✕；6s 後 auto-dismiss（disk full / 一般 error） / 0 timeout 不消失（interrupted critical）
- 多個 error 排隊顯示，不疊加

---

## SUS（System Usability Scale）題目對齊

10 題 SUS — Linux 預期評分（基於 best-effort 平台特性，比 iOS / mac hero 略低）：

| 題目（簡寫） | 預期評分（5-point） | Linux-specific 設計依據 / 風險 |
|---|---|---|
| 1. 想常用 | 3–4 | main window canonical + tray bonus；風險：GNOME 無 tray 情境 user 感受割裂 |
| 2. 不必要的複雜 | 2–3 | DE detection / Wayland vs X11 對 user 透明；風險：global shortcut 不承諾 user 可能困惑 |
| 3. 容易使用 | 3–4 | 無 mic perm gate 省一步；風險：B' device-error 觸發時的 DE detection 命中率 |
| 4. 需要技術支援 | 2–3 | 多數 distro 開箱即用；風險：sway / i3 user 需自接 rules |
| 5. 各功能整合度高 | 3–4 | Focus tab + tray + libnotify 三通道；風險：通道間一致性需 compositor 配合 |
| 6. 太多不一致 | 2–3 | 跨 DE 行為差異（tray 可見性 / libnotify actions 支援）；風險：user 在不同 distro 體驗有落差 |
| 7. 多數人能很快學會 | 3–4 | 無 onboarding 即可開始；風險：tray attention 變體在 GNOME 看不見 |
| 8. 使用不靈活 | 2–3 | global shortcut user 自接；custom duration clamp 180；風險：advanced user 期待更多 |
| 9. 使用上有信心 | 3–4 | 多通道 fallback（main window / tray / libnotify）；風險：headless / libnotify 不可得時感覺「不知有沒有」 |
| 10. 學了很多才能用 | 1–2 | 第一次點 start 就會 |

**目標 SUS：60–75 範圍**（預期實測中位數 68，比 iOS hero 72 略低 — best-effort 平台特性）。**若 < 60，優先檢討**：

1. tray icon attention 變體在 GNOME 不可見的二級救濟（main window title prefix 是否夠 visible）
2. libnotify actions 在 dunst / mako 不支援時的 main window banner 救濟有效性
3. B' device-error 螢幕的 DE detection 命中率 + fallback 文案清晰度
4. Wayland portal 拒絕 raise window 時 user 的挫折感

---

## 開放問題（prototype 層面，剩餘）

1. **systemd `--user` service interrupt 行為**：user 手動 `systemctl --user stop spectyn-mesh` 期間 Recording 是否該 graceful finalize？目前傾向「graceful finalize（flush chunks）後 service exit」— 但若 user 期待立即停則需 SIGTERM handler。v0.7+ 鎖。
2. **Wayland XDG portal `org.freedesktop.portal.GlobalShortcuts` 評估**：v0.7+ 是否接？目前實作支援度低（GNOME 46+ / KDE 6+）— 接的話需 fallback chain。傾向 v0.7+ 試 portal 路徑、不行就教 user 自接。
3. **whisper.cpp 進度回報 → tray attention 切換 timing**：ASR 跑到 50% 時 tray 是否該切回 idle 還是維持 attention？目前傾向「整個 Finalizing 都 attention，Done 後切回 idle」— 但 Finalizing 可能 > 60s 太久 attention 讓 user 焦慮。
4. **webkit2gtk version detection fallback**：< 2.40 強制 light mode — 是否在 main window 角落加 hint「您的 webkit2gtk 版本較舊，部分視覺效果降級」？傾向不加（避免噪音）。
5. **D-Bus `org.freedesktop.appearance` portal vs gsettings 雙路徑** theme detection：哪個優先？傾向先試 portal（更標準）— fail 再 fallback gsettings / kdeglobals。
6. **libnotify done 通知 multiple sessions 短時間連發**：user 連續做 3 個 25min session 結束時 3 個 done 通知會堆疊（KDE 通知群組）— 是否該限制 throttle？傾向不限（user 主動完成 session 應該收到反饋）。

---

## 易用性測試準備

### 7 個 user task — Linux 特化版

| # | Task | 測項 | 6-state 覆蓋 |
|---|---|---|---|
| 1 | **首次使用 + Empty state**：「請開啟 spectyn-mesh，看一下 history 分頁無資料的樣子，再回 Focus 分頁開始 25 分鐘錄音」 | first-time flow + 無 mic perm prompt（vs mobile）+ history empty | **Empty**（history）+ Ideal（done） |
| 2 | **Tray 控制**：「錄音中請從系統 tray 圖示停止錄音」 | SNI tray 可見性 + right-click menu + Stop ≤ 2 操作 | Ideal（Recording → Done） |
| 3 | **GNOME 無 tray 救濟**（GNOME 機器）：「錄音中請只用 main window 停止」 | main window canonical 認知 + title prefix 可見性 | Ideal |
| 4 | **Interrupted（mic 被搶）**：「錄音中請打開另一個錄音 app（如 Audacity）搶 mic，觀察 spectyn 反應」 | libnotify critical + main window banner + 30s 寬限 + action button | **Error**（中斷態） |
| 5 | **B' Device-error**（USB mic 拔掉模擬）：「錄音前拔掉 USB mic，請點 start」 | B' 螢幕 + DE-aware 打開音訊設定 | **Error**（device-error） |
| 6 | **Done flow + libnotify**：「錄完看 takeaway，然後最小化 main window — 看 tray icon 變化跟 libnotify 通知」 | done card + libnotify low + tray idle 切回 | Ideal |
| 7 | **Maximum + Partial**：「請設 180 分鐘自訂 timer 立即停止；假設第 3 個 chunk ASR 失敗（測試環境注入），請從 Finalizing / Done 卡讀懂發生什麼」 | duration clamp + chunk 99+ + partial inline + truncated 視覺 | **Limit + Partial + Error** |

### Sampling

- 目標 5–7 user（per Nielsen「5 個 user 找 80% 問題」）
- **DE 覆蓋（關鍵）**：至少 GNOME × 2 + KDE × 2 + Sway / i3 × 1 + Xfce × 1（避免單一 DE 偏見）
- 角色：3 Linux 重度（含 tiling WM）+ 2 桌面用戶（GNOME / KDE）+ 2 OSS contributor
- 環境：Ubuntu 24.04 GNOME / Fedora KDE / Arch Sway / Debian Xfce（涵蓋 webkit2gtk 版本差異）

### 觀察重點

- **A / Focus tab**：user 是否預期 mic perm prompt？無 prompt 是否引起困惑？
- **B' Device-error**：DE-aware 打開音訊設定按鈕是否帶到對的 panel？命中率？
- **C Recording**：tray icon 變色是否被 user 注意（特別 GNOME 無 tray 機器）？main window title prefix 是否補足？
- **C' Interrupted**：libnotify critical 通知是否 user 看到？action button 是否被點？dunst / mako user 是否從 main window banner 補足？
- **E Finalizing**：whisper.cpp 在 user 機器跑多久？AVX2 vs CPU-only 落差是否觸發 `taking_longer` hint？
- **F Done**：libnotify low 通知是否被 user 看到（多半瞄一眼就過）？tray 切回 idle 是否被注意？

### 紀錄方式

- 螢幕錄影（OBS / kazam，user 同意後）
- think-aloud protocol
- 結束後 SUS 問卷 + 5 題開放問題（最讚 / 最差 / 困惑點 / 期望加 / 期望砍）
- **DE-specific 補充題**：「您的 DE 是 ___，tray / libnotify / global shortcut 哪些有用 / 沒用？」

---

## 下一步

→ 拉 5–7 user（DE 多樣性涵蓋）跑 usability test → 收 SUS 分數 + 觀察紀錄 → 回頭修 Linux Wireframe / Mockup / Prototype 對應點
→ 若 Linux prototype 經 usability 驗證 OK，與 iOS hero + mac / Android / Win prototype 一起進入 v0.6.0 ship freeze
→ 共用 task / observation 規格（per hero iOS prototype 「下一步」）— 跨平台 SUS 分數對照表可在 v0.7+ 補
