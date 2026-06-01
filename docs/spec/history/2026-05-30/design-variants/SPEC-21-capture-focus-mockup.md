# SPEC-21 Capture Focus — Mockup（視覺稿）

> **Stage 2/3** of the user-flow chain · [線框稿（Wireframe）](./SPEC-21-capture-focus-wireframe.md) → 視覺稿（Mockup）→ [原型（Prototype）](./SPEC-21-capture-focus-prototype.md)
> **Status**: draft v0.3 **(sign-off after R8)**（R1 ship → R2 同步 wireframe → R3 修 Codex regressions → R4 加 stop_finalize key + iOS C Limit + 同步 wireframe dropdown → R5 拆 Limit CTA key + 修 Empty state OoS3 一致性 + invariants 措辭 → R6 收尾：6-state 表 Limit key 補拆 + wireframe header changelog + PTT/Timer 措辭跨檔同步 + Web breakpoint 內文加 pointer + 開放問題 #4 closed → R7 metadata：開放問題 #4 改回 numbered strikethrough + 補 R6 changelog 段 → R8 sign-off：4/5 reviewer 同意進 Prototype，1 nit 順手清）· **Last updated**: 2026-05-27
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) §10 · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)
> **這份的工作範圍**：把 Wireframe 的 `[placeholder]` 變實 — 顏色取 token、字體尺寸取 SPEC-02 type ramp、終版文案、icon 指定、元件 padding/radius/min-height、各視覺 state（idle/hover/pressed/disabled/loading）。互動 timing / 手勢偵測仍歸 Prototype。

## Design token 速查（取自 `app/tailwind.config.js`）

| Token | Hex | 用途 |
|---|---|---|
| `phantom-bg` | `#0f0f1a` | screen background |
| `phantom-card` | `#1a1a2e` | card / sheet / modal surface |
| `phantom-border` | `#2a2a3e` | divider / outline |
| `phantom-primary` | `#8ab4f8` | primary action (Start / Stop) |
| `phantom-secondary` | `#bb86fc` | secondary action (Pause) |
| `phantom-success` | `#4caf50` | done state / trust badge accent |
| `phantom-warning` | `#ff9800` | recording state (icon, waveform) |
| `phantom-danger` | `#dc3545` | stop / destructive |
| `phantom-text` | `#e0e0e0` | primary text |
| `phantom-muted` | `#8888aa` | secondary text / placeholder |

### Overlay opacity（從散落值收成 token）

| Token | 數值 | 用途 |
|---|---|---|
| `overlay-recording-16` | `phantom-warning @ 16%` | iOS recording 時 nav bar tint |
| `overlay-web-warn-20` | `phantom-warning @ 20%` | Web caveat banner（idle） |
| `overlay-ripple-24` | `phantom-primary @ 24%` | Android Material ripple |
| `overlay-web-warn-30` | `phantom-warning @ 30%` | Web caveat banner（recording 強化版） |
| `overlay-error-16` | `phantom-danger @ 16%` | Web C' upload-failed 背景 |
| `overlay-disabled-40` | element opacity 40% | PTT/Timer disabled 全用此（不換色，只壓透明度） |
| `overlay-denied-72` | `phantom-bg @ 72%` | iOS B'（Denied）遮罩 |

> Light-mode 對映：**TBD**。SPEC-02 §7 承諾覆蓋兩模式，本檔目前僅描述 dark-mode（主要 dogfood 配色）；亮色 token 表會在下版 SPEC-21 mockup 補（屆時 light/dark 兩欄並列）。

## 字級（type ramp，per SPEC-02 §7）

| Token | Size / Weight | 用途 |
|---|---|---|
| `display` | 48px / 700 | 計時器大數字 |
| `display-sm` | 32px / 700 | 桌面 menu bar dropdown 計時器 |
| `title` | 24px / 600 | 螢幕標題 |
| `title-sm` | 18px / 600 | section header |
| `body-lg` | 16px / 500 | 主要按鈕文字 |
| `body` | 14px / 400 | 一般文字 |
| `body-sm` | 13px / 400 | tooltip / hint |
| `caption` | 12px / 400 | trust badge / footer |

## Icon 規範

- **iOS / macOS**: SF Symbols 5（內建）
- **Android**: Material Symbols Rounded（內建）
- **Win / Linux / Web**: Lucide icons（已在 `MobileDashboard.tsx` 等使用）

### Icon 對照矩陣（12 functions × 3 sets）

| 角色 | SF Symbols (iOS/macOS) | Material Symbols (Android) | Lucide (Win/Linux/Web) |
|---|---|---|---|
| 麥克風 / PTT | `mic.fill` | `mic` | `mic` |
| 麥克風關閉 / paused / denied | `mic.slash` | `mic_off` | `mic-off` |
| 麥克風拒絕（denied 卡） | `mic.slash.circle.fill` | `mic_off`（含 fill state） | `mic-off`（含 fill state） |
| 播放 / 開始計時 | `play.fill` | `play_arrow` | `play` |
| 暫停 | `pause.fill` | `pause` | `pause` |
| 停止 | `stop.fill` | `stop` | `square`（filled square 慣例） |
| chunk 計數（資料夾） | `folder.fill` | `folder` | `folder` |
| 完成（success） | `checkmark.circle.fill` | `check_circle` | `check-circle` |
| 設定 | `gearshape` | `settings` | `settings` |
| 返回 | `chevron.left` | `arrow_back`（Material 慣例） | `arrow-left` |
| 警告 / 中斷 | `exclamationmark.triangle` | `warning` | `triangle-alert` |
| 上傳失敗 / 離線 | `wifi.slash` | `wifi_off` | `wifi-off` |

實作端取對應平台值，缺一律 fallback Lucide SVG bundled with app。

## 共用文案（zh-TW / en）

| Key | zh-TW | en |
|---|---|---|
| `focus.title` | 焦點時段 | Focus |
| `focus.duration.25` | 25 分鐘 | 25 min |
| `focus.duration.50` | 50 分鐘 | 50 min |
| `focus.duration.custom` | 自訂 | Custom |
| `focus.duration.unit` | 分鐘 | min |
| `focus.btn.ptt` | 按住說話 | Hold to talk |
| `focus.btn.start_timer` | 開始計時錄音 | Start timer |
| `focus.btn.pause` | 暫停 | Pause |
| `focus.btn.resume` | 繼續 | Resume |
| `focus.btn.stop` | 停止 | Stop |
| `focus.btn.stop_finalize` | 停止並收工 | Stop & finalize |
| `focus.btn.cancel` | 取消 | Cancel |
| `focus.btn.start` | 開始 | Start |
| `focus.label.goal_tag` | 目標標籤（選填） | Goal tag (optional) |
| `focus.trust_badge` | 🔒 本地加密 · 麥克風 ASR | 🔒 Encrypted on-device · local ASR |
| `focus.chunk_landed` | 已落地 chunk: {n} | Chunks saved: {n} |
| `focus.finalizing.asr` | 整理逐字稿 ({i}/{n}) | Transcribing ({i}/{n}) |
| `focus.finalizing.llm` | 產生 takeaway 中 | Generating takeaway… |
| `focus.done.title` | 完成 · {min} 分鐘 · {n} chunks | Done · {min} min · {n} chunks |
| `focus.done.view_full` | 看完整逐字稿 | View full transcript |
| `focus.done.new_session` | 新 session | New session |
| `focus.web.caveat` | ⚠ 瀏覽器模式：請保持本頁開啟，切走會中斷錄音 | ⚠ Browser mode: keep this tab open or recording stops |
| `focus.web.upload_failed` | 上傳到 host 失敗 | Upload to host failed |
| `focus.web.retry` | 重試 | Retry |
| `focus.web.save_offline` | 暫存到瀏覽器 | Save offline |
| `focus.perm.denied` | 需要麥克風才能 focus 錄音 | Microphone permission required |
| `focus.perm.denied_reassure` | 我們不會上傳音訊到雲端，ASR 也跑在本機 | We never upload audio to the cloud; ASR runs locally |
| `focus.perm.open_settings` | 打開設定 | Open settings |
| `focus.interrupted.phone` | 電話中已暫停 | Paused during call |
| `focus.interrupted.mic_grabbed` | 麥克風被其他 app 使用，已暫停 | Mic taken by another app; paused |
| `focus.interrupted.resume_hint` | 30 秒內回復將自動繼續 | Auto-resume if returned within 30s |
| `focus.android.notif_optional` | 沒給通知權限也可錄，但通知欄不會顯示控制 | Recording still works without notification permission; shade control unavailable |
| `focus.empty.history` | 還沒 focus session — 開始第一段就會顯示在這 | No focus sessions yet — start one to see it here |
| `focus.limit.chunk_overflow` | 99+ | 99+ |
| `focus.limit.takeaway_truncated_hint` | （摘要過長已截斷） | (truncated) |
| `focus.limit.view_full_takeaway` | 看完整摘要 | View full takeaway |
| `focus.partial.chunk_failed` | 轉文字失敗 (chunk {i}/{n})，已跳過 | Transcribe failed (chunk {i}/{n}); skipped |
| `focus.err.no_mic` | 找不到麥克風裝置 | No microphone available |
| `focus.limit.max_duration_hint` | 最長 180 分鐘 | Max 180 min |
| `focus.btn.retry_asr` | 重試 ASR | Retry ASR |
| `focus.btn.use_empty_transcript` | 先用空白 transcript 跑 LLM | Generate with empty transcript |
| `focus.btn.cancel_show_transcript` | 取消並先看逐字稿 | Cancel & view transcript |
| `focus.btn.retry_summary` | 重跑摘要 | Retry summary |
| `focus.confirm.leave_recording_msg` | 離開會停止錄音 | Leaving will stop recording |
| `focus.confirm.leave_recording_stop` | 停止並離開 | Stop & leave |
| `focus.err.disk_full` | 儲存空間不足，請清理後再錄 | Storage full; free up space to keep recording |
| `focus.finalizing.taking_longer` | 比預期久… | Taking longer than expected… |
| `focus.empty.go_to_focus` | 前往 Focus | Go to Focus |
| `focus.err.no_takeaway` | 未產出摘要 — 可重跑 ASR | No takeaway — re-run ASR available |
| `focus.empty.no_takeaway` | 本次 session 沒有摘要，可手動重跑 | No takeaway for this session — manual retry available |
| `focus.empty.no_speech` | 本次時段未偵測到語音，已為您記錄時長 | No speech detected; time still logged |
| `focus.web.perm_settings_hint` | 請從瀏覽器設定恢復麥克風權限 | Re-enable mic permission from browser settings |
| `focus.web.offline_pending` | 已暫存 {n} 段，連線恢復後自動上傳 | {n} chunks queued; auto-upload when reconnected |
| `focus.web.quota_exceeded` | 瀏覽器儲存空間已滿，請手動上傳 | Browser storage full; manual upload required |
| `focus.web.offline_unload_warn` | 還有上傳中段落，現在關閉可能遺失 | Pending upload — closing may lose data |
| `focus.btn.review` | 開啟回顧 | Open review |
| `focus.desktop.interrupt_notif_title` | Phantom Mesh 焦點時段中斷 | Phantom Mesh focus interrupted |
| `focus.desktop.interrupt_notif_action` | 開啟並停止 | Open & stop |

---

## iOS — Mockup

### 螢幕 A — Idle

```
┌────────────────────────────┐  bg: phantom-bg
│ ◀ 返回    焦點時段    ⚙ 設定 │  nav 44pt, title: title-sm
│                            │
│      ◯  00:00 / 25:00      │  clock-face: 240×240pt dim circle stroke 2pt phantom-border
│                            │  text: display, phantom-muted (idle)
│                            │
│  ┌─────────┐ ┌──┐ ┌──────┐│  pill buttons 32pt height, radius 16pt
│  │ 25 分鐘 │ │50│ │ 自訂 ││  selected: bg phantom-primary, text phantom-bg
│  └─────────┘ └──┘ └──────┘│  unselected: bg phantom-card, text phantom-text
│                            │
│ ┌────────────────────────┐ │  PTT btn: full-width minus 16pt margin,
│ │   ⏺  按住說話           │ │  height 96pt, radius 24pt
│ └────────────────────────┘ │  bg phantom-card, border 2pt phantom-primary
│  (large primary)           │  icon: SF "mic.fill" 32pt
│                            │  text: body-lg, phantom-primary
│ ─── 或 ───                  │  divider with caption text, phantom-muted
│                            │
│ ┌────────────────────────┐ │  Timer-start btn: same width, height 56pt, radius 12pt
│ │  ▶  開始計時錄音        │ │  bg phantom-primary, text phantom-bg, body-lg/600
│ └────────────────────────┘ │  icon: SF "play.fill" 20pt
│                            │
│ 🔒 本地加密 · 麥克風 ASR    │  trust badge: caption, phantom-muted, centered
└────────────────────────────┘
```

**Visual states**：
- 25/50/custom pill: idle (card bg) / selected (primary bg, bg text) / pressed (primary @ 80% opacity)
- PTT button: idle / pressed (border thickens to 3pt, bg phantom-card lighten 8%) / disabled (40% opacity, when perm denied)
- Timer button: idle / pressed (primary @ 80% opacity) / disabled

**Icons**：
- back chevron — SF `chevron.left` 18pt phantom-text
- settings — SF `gearshape` 18pt phantom-text
- PTT — SF `mic.fill` 32pt phantom-primary
- Timer start — SF `play.fill` 20pt phantom-bg

### 螢幕 C — Recording (Timer mode)

```
┌────────────────────────────┐  bg: phantom-bg
│             05:23 / 25:00  │  display/600, phantom-warning (recording accent)
│                            │
│  ▁▂▃▅▇▇▇▅▃▂▁              │  waveform: 120pt height, 32 bars, phantom-warning
│                            │
│ ┌──────┐    ┌──────────┐  │  Pause: 120×56pt, radius 12pt, bg phantom-card,
│ │ ⏸暫停│    │ ⏹ 停止    │  │   icon phantom-secondary, text phantom-text
│ └──────┘    └──────────┘  │  Stop: 120×56pt, radius 12pt, bg phantom-danger,
│                            │   text phantom-text, icon SF "stop.fill" 20pt
│ 📁 已落地 chunk: 3          │  body, phantom-muted, icon: SF "folder.fill" 14pt
│                            │  **Limit 變體**：chunk ≥ 100 顯示 `99+` (取 `focus.limit.chunk_overflow`)；
│                            │   數字區塊 min-width 48pt 鎖死，避免 3→99+ 跳寬時右側 icon 閃爍
│ 🔒 本地加密                  │  caption, phantom-muted
└────────────────────────────┘
```

**Recording 中 nav bar 改色**：頂部 nav bar tint 從 `phantom-bg` → `overlay-recording-16`（`phantom-warning @ 16%`），作為「我在錄」的全螢幕提醒。

### 螢幕 B' — Denied（覆蓋在 Idle 上的遮罩卡）

```
┌────────────────────────────┐  bg: overlay-denied-72（phantom-bg @ 72% on Idle 之上）
│                            │
│      [mic-denied-icon]     │  SF "mic.slash.circle.fill" 64pt, phantom-danger
│                            │
│  需要麥克風才能 focus 錄音   │  title, phantom-text, centered
│                            │
│  我們不會上傳音訊到雲端，    │  body, phantom-muted, centered, line-height 1.5
│  ASR 也跑在本機。            │
│                            │
│ ┌────────────────────────┐ │  Open-settings btn: 240×48pt centered, radius 12pt
│ │     打開設定            │ │  bg phantom-primary, text phantom-bg, body-lg/600
│ └────────────────────────┘ │  icon: SF "gearshape" 18pt phantom-bg
│                            │
└────────────────────────────┘
```

**Visual states**: Idle 螢幕仍可見背景但 PTT 跟 Timer button 都套 `overlay-disabled-40`（40% opacity，無 hover/pressed）。Open-settings 按鈕 pressed 狀態 `phantom-primary @ 80%`。

### 螢幕 C' — Interrupted（Recording 變體，waveform 凍結）

```
┌────────────────────────────┐  bg: phantom-bg
│             05:23 / 25:00  │  display, phantom-muted (從 warning 退色到 muted)
│                            │
│  ▁▂▃▅▇▇▇▅▃▂▁              │  waveform: phantom-muted bars（凍結，無 update）
│                            │
│  電話中已暫停                │  body-lg, phantom-warning, centered
│  30 秒內回復將自動繼續        │  body-sm, phantom-muted, centered
│                            │
│ ┌──────┐    ┌──────────┐  │  Pause: 等同 C，但 disabled 樣式
│ │ ⏸暫停│    │ ⏹ 停止    │  │  Stop: 等同 C，仍 enabled
│ └──────┘    └──────────┘  │
│ 📁 已落地 chunk: 3          │  保持
│ 🔒 本地加密                  │
└────────────────────────────┘
```

**Visual states**: OS resume 回 C 或超時切 E（Finalizing 標 `interrupted=true`）— 30s 寬限與超時邏輯規格在 wireframe FSM 鎖，本檔不重複數字。

### 螢幕 D — Lock-screen (rendered by iOS)

不可自訂版面。phantom 只能設：
- `title` = "Phantom Mesh"
- `artist` = "Focus · 05:23"
- artwork = app icon mono variant 512×512pt
- playback controls = `pause` + `stop` (no skip)

### 螢幕 E — Finalizing（過渡螢幕）

```
┌────────────────────────────┐  bg: phantom-bg
│                            │
│        [spinner-32]        │  centered, 32pt spinner, phantom-warning stroke
│                            │
│   整理逐字稿 (2/5)          │  body-lg, phantom-text, centered
│   ████████░░ 40%           │  progress bar 240×4pt, fg phantom-warning, bg phantom-border
│   ⚠ 轉文字失敗 (chunk 3/5)，│  body-sm, phantom-warning, centered（partial — 僅在某 chunk ASR fail 時顯示，inline 於 progress 下方）
│      已跳過                  │
│                            │
│   產生 takeaway 中…         │  body, phantom-muted (pending — 灰，等 ASR 完才亮色)
│                            │
│   取消並先看逐字稿           │  caption, phantom-muted, underline, tap target
└────────────────────────────┘
```

**Visual states**: 兩個 phase 並列 — phase 1 (ASR) 跑時 progress 走、訊息一在 phantom-text、訊息二在 phantom-muted；phase 2 (LLM) 接手時訊息二切到 phantom-text。Partial 訊息（`focus.partial.chunk_failed`）僅在 chunk ASR fail 時 inline 出現。「取消並先看逐字稿」action 行為由 [Prototype](./SPEC-21-capture-focus-prototype.md) 層鎖定，本檔僅定視覺。

### 螢幕 F — Done (Takeaway card)

```
┌────────────────────────────┐  bg: phantom-bg
│  ✓ 完成 · 25 分鐘 · 5 chunks│  title-sm, phantom-success, icon SF "checkmark.circle.fill"
│  ────────────────────       │  divider 1pt phantom-border
│                            │
│  ┌──────────────────────┐  │  takeaway card: bg phantom-card, radius 16pt, padding 20pt
│  │ 主要 ideas:           │  │   title-sm, phantom-text
│  │  • [line 1]          │  │   body, phantom-text, line-height 1.5
│  │  • [line 2]          │  │
│  │                      │  │
│  │ Action items:        │  │
│  │  • [...]             │  │
│  │                      │  │
│  │ 情緒 / 卡點:          │  │
│  │  • [...]             │  │
│  │ ┄┄┄┄┄┄┄ (fade) ┄┄┄┄┄┄│  │  takeaway > 800 字觸發截斷：
│  │ ...                   │  │   max-height 480pt, bottom 64pt fade-out
│  │ [看完整摘要] CTA      │  │   gradient overlay phantom-card → transparent；
│  └──────────────────────┘  │   底部 inline "看完整摘要" tap target（body-sm, phantom-primary, underline）
│                            │
│ ┌─────────────┐ ┌────────┐ │  view-full: 56pt height, bg phantom-card
│ │ 看完整逐字稿 │ │ 新 session│ │  new: 56pt, bg phantom-primary, text phantom-bg
│ └─────────────┘ └────────┘ │
└────────────────────────────┘
```

**Limit state — Done card 截斷**：takeaway 字數 > 800 時，卡片 max-height 480pt + 底部 64pt fade-out gradient（`phantom-card → transparent`）+ inline hint「（摘要過長已截斷）」（取 `focus.limit.takeaway_truncated_hint`）+ CTA「看完整摘要」（取 `focus.limit.view_full_takeaway`）。Tap CTA 等同點 `[看完整逐字稿]` 主按鈕。

---

## Android — Mockup

幾乎等同 iOS，差異列舉：

- 字體：Roboto Flex（替代 SF）；type ramp 同表
- icon 改用 Material Symbols Rounded（per Icon 對照矩陣）
- nav bar height 56dp（替代 iOS 44pt）
- ripple effect 內建：所有按鈕 pressed 狀態用 Material ripple（color = `overlay-ripple-24`）
- **B' / C' / E 視覺等同 iOS**，僅 icon 套用 Material Symbols Rounded：B' denied icon 用 `mic_off`（fill）、C' interrupted 訊息字級不變、E spinner 用 Material CircularProgressIndicator 樣式
- B2 (POST_NOTIFICATIONS) skip 後的 Idle 頂部多一條提示 bar：
  ```
  ⓘ 沒給通知權限也可錄，但通知欄不會顯示控制
  ```
  bg `phantom-card`（subtle info），body-sm `phantom-muted`，high 32dp，可滑掉
- FG-service notification（取代 iOS lock-screen）：
  - icon 小圖：app mono icon
  - title: "Phantom Mesh"
  - text: "Focus · 05:23"
  - actions: `stop` only（同 wireframe Android D；mockup 不擴增至 pause — pause 透過解鎖 app 操作）
  - persistent: true
  - low priority（不發聲、不震動）

---

## macOS — Mockup

### 螢幕 A — Start Sheet

```
┌──────────────────────────────────────┐  Sheet：480×320pt, bg phantom-card, radius 12pt
│ 開始焦點時段                  [✕]    │  title 24px/600 phantom-text
│ ────────────────────────────────    │  divider phantom-border
│                                      │
│ 時長：                                │  title-sm phantom-text
│  ○ 25 分鐘 Pomodoro                  │  radio rows: 32pt height
│  ○ 50 分鐘                            │  selected radio: phantom-primary
│  ◉ 自訂： [ 30 ] 分鐘                 │  custom input: 60×28pt, radius 6pt, bg phantom-bg
│                                      │
│ 目標標籤（選填）：                     │
│ ┌──────────────────────────────┐    │  text input: full-width, height 32pt,
│ │ deep_work, spec_writing      │    │   bg phantom-bg, radius 6pt, padding 8pt
│ └──────────────────────────────┘    │   placeholder "輸入標籤…" phantom-muted
│                                      │
│ 🔒 本地加密 · 麥克風 ASR              │  caption phantom-muted (取 `focus.trust_badge` — 一字不差)
│                                      │
│      [ 取消 ]      [ 開始 ]          │  Cancel: 96×32pt, bg transparent, text phantom-muted
│                                      │  Start: 96×32pt, bg phantom-primary, text phantom-bg
└──────────────────────────────────────┘
```

### Menu bar icon

- Idle: SF `mic` mono template 18pt
- Recording: SF `mic.fill` template + 紅點 overlay 6pt at top-right corner（NSImage composite）
- Paused: SF `mic.slash` template

### Menu bar dropdown — Recording

```
┌──────────────────────────┐  Width 320pt (per SPEC-41 §7.4)
│ 🔴 Focus 05:23/25:00      │  body-lg phantom-warning + display-sm time
│ ▁▂▃▅▇▅▃▂                   │  mini waveform 60pt height
│ ──                        │  divider phantom-border
│ ⏹ 停止並收工              │  row 32pt, icon + label (Recording 期間最高優先 — 與 Windows tray 一致)
│ ⏸ 暫停                    │  row 32pt
└──────────────────────────┘
```

> ASCII 內 label 字串為 zh-TW demo placeholder；實作端透過 `focus.btn.stop_finalize` / `focus.btn.pause` i18n key 動態帶入。注意 desktop menu 「停止並收工 / Stop & finalize」與 mobile 螢幕內按鈕「停止 / Stop」(`focus.btn.stop`) 是**不同字串**，desktop 用前者強調「結束 session」、mobile 用後者更短。Win tray + Linux tray menu 同此約定。

### Done — Notification banner（macOS Notification Center）

不可自訂版面。phantom 設：
- title: "Phantom Mesh"
- subtitle: "Focus 25 min · takeaway ready"
- body: 第一行 takeaway （取 80 字）
- click action → open main window Focus tab

### Interrupted — Notification banner（系統強制觸發）

Recording 中 OS interrupt 觸發、主視窗非 active focus 時必發。phantom 設：
- title: 取 `focus.desktop.interrupt_notif_title`
- subtitle: "5:23 / 25:00 · mic 被佔用 / 系統 sleep / 藍牙切換" （依 interrupt 來源動態填）
- body: 取 `focus.interrupted.resume_hint`（跨平台一字不差）
- actions: 取 `focus.desktop.interrupt_notif_action`
- sound: default（OS interrupt 屬高優先級 → 需聲音提醒）

### Main window Focus tab — Takeaway 卡片

跟 iOS F 一樣的卡片結構，差別：
- card width: 640pt 中央對齊（main window）
- 多一個 history sidebar（左側 220pt，顯示最近 10 個 session）
- top toolbar 多 "Export" 按鈕（v0.7+ feature placeholder）

---

## Windows — Mockup

幾乎等同 macOS，差異：

### 螢幕 A — Start Window（非 sheet）

- 真窗 NSWindow-equivalent，480×320pt
- title bar 用系統預設（不自訂）
- 其餘版面同 macOS Sheet 內容

### Tray icon

- Idle: Lucide `mic` 16×16, phantom-muted single-color
- Recording: Lucide `mic` 16×16 phantom-warning（飽和橘）
- Paused: Lucide `mic-off` 16×16 phantom-muted

### Tray context menu

Windows 標準 menu 樣式（system theme），phantom 只控 label：

```
Focus 05:23/25:00         (disabled row, italic)
─────────
⏹ Stop & finalize         (Recording 期間最高優先 — 與 wireframe 同步)
⏸ Pause
─────────
Open Phantom Mesh
```

### Done — ActionCenter toast

Win10/11 標準 toast：
- AppLogo: phantom mono icon
- title: "Phantom Mesh"
- body line 1: "Focus 25 min · takeaway ready"
- body line 2: 第一行 takeaway 取 60 字（toast row 2 限制）
- actions: `Open` button → opens main window
- audio: default

### Interrupted — ActionCenter toast（系統強制觸發）

Recording 中 OS interrupt + 主視窗非 active 時必發：
- AppLogo: phantom mono icon
- title: 取 `focus.desktop.interrupt_notif_title`
- body line 1: "5:23 / 25:00 · mic 被佔用"（依 interrupt 來源動態）
- body line 2: 取 `focus.interrupted.resume_hint`
- actions: 取 `focus.desktop.interrupt_notif_action` button → activate main window + invoke stop
- audio: default
- scenario: `urgent`（高優先級，避免被 Focus Assist 折疊；具體啟用方式由 prototype 驗證實作端 toast XML schema）

---

## Linux — Mockup

幾乎等同 Win，差異：

- 沒指定平台 icon 系統 → 用 Lucide SVG bundled with app
- Tray icon 透過 StatusNotifierItem (KDE/Plasma) 或 AppIndicator (GNOME extension users)；GNOME 預設沒 tray，本檔不畫 tray fallback
- Done notification 透過 `notify-send` / libnotify：
  - icon: phantom mono SVG
  - summary: "Focus complete"
  - body: "25 min · 5 chunks · takeaway ready"
  - urgency: low
  - timeout: 默認
- Interrupted notification（OS interrupt 強制觸發，同 mac/win 規格）：
  - icon: phantom mono SVG
  - summary: 取 `focus.desktop.interrupt_notif_title`
  - body: 取 `focus.interrupted.resume_hint`
  - urgency: `critical`（不讓 compositor 折疊）
  - timeout: 0（不自動消失）
  - actions: `default=` 取 `focus.desktop.interrupt_notif_action`（compositor 支援才有效，KDE/GNOME OK，sway/Hyprland 看 mako 設定）

---

## Web — Mockup

### Breakpoint 切換（per Wireframe）

| 條件 | 版型 | 主視覺 |
|---|---|---|
| `< 768px` 或 `pointer: coarse` | **mobile-web** | hero PTT 大鈕 + duration picker + Timer 次按 |
| `≥ 768px` 且 `pointer: fine` | **desktop-web** | Timer-only（無 PTT — 鍵盤情境不合）+ 較緊湊垂直排版 |

切點走 CSS media query `@media (min-width: 768px) and (pointer: fine)`；同一份 React 元件 conditional render，不另外切 page。**pointer 條件解 iPad 盲區**：iPad 寬度 > 768px 但純觸控（`pointer: coarse`），不該掉 PTT — 故 desktop 版型只在「大螢幕 + 真實滑鼠/觸控板」時啟用。

### 螢幕 A1 — Idle（mobile-web，< 768px）

跟 iOS A 99% 一致，差別：
- 容器 max-width 480px 居中
- 全程頂部多一條 caveat banner（見下）
- 主要按鈕用 Lucide icons（替 SF Symbols）

### 螢幕 A2 — Idle（desktop-web，≥ 768px）

```
┌──────────────────────────────────────────────┐  Caveat banner 同 A1
├──────────────────────────────────────────────┤
│                                              │
│              00:00 / 25:00                   │  display, phantom-muted, centered
│                                              │
│       ◯ 25 分鐘 Pomodoro                      │  radio rows: 32pt, 同 macOS Sheet 樣式
│       ◯ 50 分鐘                                │
│       ◉ 自訂： [ 30 ] 分鐘                     │
│                                              │
│        ┌────────────────────┐                │  Start btn: 320×48px centered
│        │  ▶  開始計時錄音    │                │  bg phantom-primary, text phantom-bg, body-lg
│        └────────────────────┘                │  label 取 `focus.btn.start_timer`
│                                              │
│        🔒 本地加密 · 麥克風 ASR                 │  caption, phantom-muted, centered
└──────────────────────────────────────────────┘
```

**設計差**：去掉 PTT（桌機環境鍵盤主），duration picker 改 radio rows（同 macOS Sheet）。

### Caveat banner（兩版共用，覆蓋全寬）

```
┌──────────────────────────────┐  Caveat banner: full-width, height 36px
│ ⚠ 瀏覽器模式：請保持本頁開啟  │  bg overlay-web-warn-20, border-bottom 1px phantom-warning
│   切走會中斷錄音              │  text body-sm, phantom-warning
└──────────────────────────────┘
```

Recording 中 banner 顏色強化（bg `overlay-web-warn-30`）。

### 螢幕 C' — 上傳失敗（host 不可達）

```
┌──────────────────────────────┐  bg: overlay-error-16
│ ⚠ 上傳到 host 失敗            │  body-lg, phantom-danger, centered
│  ▁▂▃▅▇▅▃▂                     │  waveform: phantom-muted（凍結）
│                              │
│ ┌────────┐  ┌──────────────┐│  Retry: 96×40px, bg phantom-danger, text phantom-text
│ │ 重試    │  │ 暫存到瀏覽器 ││  Save-offline: 144×40px, bg phantom-card, text phantom-text
│ └────────┘  └──────────────┘│
│ 🔒 本地加密                  │
└──────────────────────────────┘
```

**Visual states**: Retry 跑 → 切回 C；Save-offline 跑 → 寫 IndexedDB queue 後切 C，UI 標 "已暫存 X 段"（具體上限 / 過期由 SPEC-17 決定）。

### Browser perm prompt

不可自訂（browser native dialog）。

---

## Cross-platform invariants（mockup 層面）

- **計時器顏色**：idle = phantom-muted，recording = phantom-warning，interrupted = phantom-muted，done = phantom-success
- **Stop 按鈕**：永遠 phantom-danger 背景；mobile 螢幕內 ≥ 96pt 寬置於最右；desktop tray/menu 內列第一項（横向 toolbar 取最右、直向選單取最上）
- **PTT button**（mobile only）：永遠 ≥ 96pt 高，永遠 phantom-primary 邊框
- **PTT × Timer 互斥 disabled 視覺**：**僅適用同畫面雙按鈕共存場景**（iOS A Idle / mobile-web A1 Idle）。PTT 按住期間 Timer 按鈕套 `overlay-disabled-40`（40% opacity，無 hover/pressed）；反向 Timer 計時觸發時，**畫面已切到 C Recording、PTT 按鈕從版面消失**，因此「Timer 跑中 PTT disable」僅是邏輯保證、視覺上無從顯示
- **Notification body 截字上限（跨平台統一）**：macOS NC banner ≤ 80 字 / Windows ActionCenter row 2 ≤ 60 字 / Linux notify-send body 不截（distro 自處理）
- **Trust badge 文字**：所有平台一字不差（取 `focus.trust_badge` key）
- **Takeaway card 尺寸**：永遠 phantom-card 背景 + radius 16pt + padding 20pt；mobile max-width 480pt；desktop main-window 640pt；可滾、可截斷（見 F Limit state）
- **Error toast**：phantom-danger 背景 95% opacity，48pt 高，全寬；toast 自動消失行為由 Prototype 鎖（mockup 只定靜態樣式）
- **Error background overlay**（非 toast 的 error 區塊背景，例：Web C' upload-failed）：`overlay-error-16`（`phantom-danger @ 16%`）
- **History tab 入口位置**（per Wireframe lock）：iOS bottom-nav 右一 tab / **Android Capture tab 內 Focus 子畫面**（per SPEC-34 4-tab IA Home/Coach/Capture/Settings）/ macOS/Windows/Linux main window 左 sidebar 220pt / Web 跟 breakpoint（mobile-web 底 nav / desktop-web sidebar）
- **Interrupted 系統通知**：Recording + 主視窗非 active focus 時觸發；三平台 layout 已列（macOS Notification banner / Windows ActionCenter toast `urgent` / Linux notify-send `critical`），共用 i18n keys `focus.desktop.interrupt_notif_*` + `focus.interrupted.resume_hint`（body 一字不差）

## 6 大資料狀態 — Mockup 視覺對映表

對齊 Tom Liou Medium 文「UI 設計師必須定義」的 6 種狀態，每平台都該覆蓋。

| 狀態 | 螢幕 | 視覺表現 |
|---|---|---|
| **理想（Ideal）** | F Done | 完整 takeaway 三段 + success icon + view-full / new-session 雙按鈕 |
| **空白（Empty）** | History tab（無 session） | **本檔僅列規格**：mono SVG illustration 192pt phantom-muted + `focus.empty.history` 文案 + "前往 Focus" 按鈕。ASCII frame **defer 到平台 catalog**（SPEC-31 iOS / SPEC-34 Android / SPEC-41 macOS / SPEC-43 Windows / SPEC-45 Linux）— 因 wireframe §OoS3 已標 History tab 出本 spec 範圍 |
| **極限（Limit）** | C Recording / F Done | chunk count `99+`（`focus.limit.chunk_overflow`）/ takeaway 長度 > 800 字觸發截斷：hint `focus.limit.takeaway_truncated_hint` + CTA `focus.limit.view_full_takeaway` |
| **錯誤（Error）** | B' / Web C' | B' Denied 遮罩（phantom-danger icon + open-settings CTA）/ Web upload-failed（overlay-error-16 + retry / save-offline 雙按鈕）/ 全平台 error toast 規格 |
| **局部（Partial）** | E Finalizing | 「轉文字失敗 (chunk 3/5)，已跳過」inline 訊息 phantom-warning，繼續 stitch 其他 chunks |
| **載入中（Loading）** | E Finalizing | spinner-32 + progress bar 40%（ASR）+ pending 訊息（phantom-muted → 切到 phantom-text 表示進度推進）|

## 開放問題（mockup 層面）

1. **計時器數字顏色**：recording 期間應該全程 `phantom-warning`（高警示）還是只在「過半」後變色？目前提案：全程 warning。
2. **Lock-screen artwork**（iOS）：用 mono app icon 還是當下 waveform 截圖？mono 簡單、waveform 有「正在錄」直覺。提案：mono。
3. **macOS dropdown 暫停顏色**：暫停中 menu bar icon 用 `mic.slash` 還是 `mic.fill` + amber 點？提案：`mic.slash` 清楚但跟「停了」混淆。
4. ~~**Web caveat banner 永遠在 vs 只在 recording**~~ → **已決（R6）**：全程頂部 idle 細條 (`overlay-web-warn-20`) / recording 加深 (`overlay-web-warn-30`)（見 Web 段正文 + design token 子表）。編號保留以便後續 round 引用。

> 移到 Prototype 的問題：~~PTT idle pulse 動畫~~ → 屬互動動效，已在 Prototype 開放問題 #2。

→ 互動 timing、手勢、失敗路徑去 Prototype 解。

## 下一步

→ 進 [Prototype](./SPEC-21-capture-focus-prototype.md) 描述每個 tap target 點下去發生什麼、動畫 / haptic / timing、失敗如何重試。
