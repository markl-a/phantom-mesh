# SPEC-21 Capture Focus — macOS Wireframe（線框稿）

> **Stage 1/3** · 線框稿 → [視覺稿（mockup §macOS）](./SPEC-21-capture-focus-mockup.md) → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: macOS only。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-wireframe.md](./SPEC-21-capture-focus-wireframe.md) iOS 段），本檔只列 macOS **deltas**，共用結構不重抄。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-40-PLATFORM-macOS-foundations`](../specs/v060-deep-spec/SPEC-40-PLATFORM-macOS-foundations.md) · [`SPEC-41-PLATFORM-macOS-screens-flows`](../specs/v060-deep-spec/SPEC-41-PLATFORM-macOS-screens-flows.md)
> **這份的工作範圍**：macOS-specific layout & flow — `⌘⇧F` global shortcut / NSStatusItem menu bar / NSPopover transient mode / NSWindow sheet / TCC mic prompt / multi-monitor / Notification Center。共用 FSM 跟 iOS 同（見 hero wireframe §通用 session 狀態鏈），本檔不重抄。

## 為什麼 macOS 有獨立 wireframe

iOS hero wireframe 的 macOS 段只 ~30 行（L170-200）只到「sheet + menu bar dropdown + Notification banner」骨架。實際 macOS：

1. **全域快捷鍵 `⌘⇧F`** — 從任意 app（含 fullscreen）觸發 focus start（per SPEC-40 G5、SPEC-41 G2）— iOS / Android 無此 API
2. **NSStatusItem 是 24/7 唯一視覺指標** — 跑 30 天不消失（per SPEC-40 G4 / SPEC-41 G5）— iOS 沒 menu bar
3. **TCC Microphone 是 macOS 獨有授權層** — 跟 iOS info.plist prompt 不同 UI、跟 Android RECORD_AUDIO runtime 不同流程
4. **Sheet vs popover vs window 三層 presentation 區分** — per SPEC-41 §10.1 12 screen taxonomy；mobile 沒這層抽象
5. **Multi-monitor spawn 規則** — settings / takeaway window 跟 NSStatusItem 所在 monitor（per SPEC-41 G3）— mobile 無此概念
6. **Notification Center banner** 取代 iOS lock-screen / Android FG-service notification — 系統渲染、不可自訂版面

→ 這 6 點值得獨立 frame 級描述，不要塞在 iOS 段。

## 入口點（per SPEC-41 §10.1 + SPEC-40 G5）

| 進入點 | v0.6.0 | v0.7+ | Source |
|---|---|---|---|
| **Global shortcut `⌘⇧F`** | ✅ | ✅ | **SPEC-40 G5 + SPEC-41 §10.4 S3 trigger** |
| **NSStatusItem dropdown → "Start focus session…"** | ✅ | ✅ | SPEC-41 §10.2 S1 row「⏱ Start focus session… ⌘⇧F」 |
| Main settings window Focus 區 | ✅ | ✅ | SPEC-41 §10.5 S4 General tab 入口（v0.6.0 直接 trigger S3 sheet）|
| Shortcuts.app 整合 | ❌ | ✅ | SPEC-41 OoS3 延後 |
| Spotlight 整合 | ❌ | ❌ | SPEC-41 OoS2，v0.8+ |
| Touch Bar | ❌ | ❌ | 不做（Apple 已棄） |

**v0.6.0 ship 3 個**：`⌘⇧F` global + menu bar dropdown + settings window 內按鈕。三條路徑都收斂到 **S3 focus start sheet**（per SPEC-41 §10.4）。

## 螢幕 A — Focus Start Sheet（per SPEC-41 §10.4 S3）

**Presentation**: NSWindow sheet（**focus_steal=true / esc_dismisses=true**，per SPEC-41 §7.4），attached to parent settings window 或當前 active window；非 popover（per SPEC-41 §10.4 + SPEC-41 §17 Alt 3 — 因要 duration + tag 兩個輸入，popover 太擠）。

```
┌──────────────────────────────────────┐  Sheet 480×320pt（per §7.4 default_size）
│ [title]                       [✕]    │  ← title「開始焦點 session」 i18n `focus.macos.sheet_title`
│ ──────────────────────────────────  │
│ [duration-label]                     │
│  ○ [opt-25]                          │  ← radio rows
│  ○ [opt-50]                          │
│  ◉ [opt-custom] [num] [unit]         │
│                                      │
│ [tag-label]                          │  ← per hero mockup §macOS L327-345
│  [text-input]                        │
│                                      │
│ [trust-badge]                        │  ← `focus.trust_badge`（per hero invariants）
│                                      │
│      [ cancel ]      [ start ]       │
└──────────────────────────────────────┘
```

**Wireframe 重點**:
- ASCII 內 `[label]` 為 placeholder — 終版字串走 i18n key（per hero mockup §macOS L327-345）
- Sheet 邊距：`focus_steal=true` 阻塞父 window 操作 — 但 menu bar / 其他 app 仍可切（per SPEC-41 §7.4）
- 首次 trigger 後若 **TCC Microphone 未授** → sheet 內 [start] 按鈕 disabled 並顯「需開麥克風權限 [open settings]」（per SPEC-41 §15 + SPEC-41 §10.4 edge case）
- 沒有 PTT（鍵盤輸入情境不適合 press-and-hold，per hero wireframe macOS §L218）

## 螢幕 B — TCC Microphone Prompt（系統渲染）

```
[trigger: 第一次 [start] 按下時]
        │
        ▼
[B. macOS TCC system prompt]
┌────────────────────────────────┐
│ [app] would like to access     │  ← OS 渲染，不可自訂
│ the microphone.                │
│                                │
│ [usage-description-from-       │  ← Info.plist NSMicrophoneUsageDescription
│  Info.plist]                   │
│                                │
│  [Don't Allow]   [OK]          │
└────────────────────────────────┘
        │ OK → 進 C
        │ Don't Allow → B'
```

**Wireframe 重點**:
- 同 iOS B：系統渲染、無權更動版面
- 差別：macOS TCC 拒絕後**不能再彈** — user 須去 `System Settings → Privacy & Security → Microphone` 手動開啟（per SPEC-40 §15 TCC 11 條盤點，授權無法 bypass）
- Deep-link recovery: `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone`（per SPEC-40 §6.3 sequence diagram 同 pattern）

## 螢幕 B' — TCC Denied（覆蓋 Sheet）

```
[B' on top of A sheet — 半透明遮罩]
┌────────────────────────────┐
│  [mic-denied-icon]         │
│                            │
│  [denied-headline]         │  ← `focus.perm.denied_macos`（per hero invariants i18n）
│  [reassurance-copy]        │
│                            │
│  [open-settings-btn]       │  ← 點 → 系統 Privacy & Security
│  [cancel-btn]              │
└────────────────────────────┘
```

跟 iOS B' 同 frame 結構（per hero wireframe §iOS B'），只是 deep link target 不同：
- iOS: `UIApplication.openSettingsURLString`
- macOS: `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone`

## 螢幕 C — Recording（不畫 dedicated window；NSStatusItem 變色 + dropdown 展現）

**核心差別 vs iOS**：macOS 沒有「Recording 全螢幕主畫面」— Recording 中 sheet 自動 dismiss，**user 的工作 context（瀏覽器 / IDE）保留焦點**。Recording 的唯一視覺錨點是 **NSStatusItem 變色**（per SPEC-41 §10.2 icon state machine）。

```
[macOS menu bar 右上角]
                                          ┌──────────────────────┐
[●][●][●] ... [📶] [🔋] [🕐]  [🎙🔴]  →  │ Phantom Mesh         │  ← NSStatusItem dropdown
                                ↑         │ Focus 05:23/25:00    │   click 後展開
                          recording-icon  │ [waveform-mini]      │
                          + 紅點 overlay  │ ──                   │
                                          │ [stop-finalize-row]  │  ← Recording 中最高優先
                                          │ [pause-row]          │   per hero mockup §macOS L361
                                          └──────────────────────┘
```

**Wireframe 重點**:
- NSStatusItem icon：Idle `mic` mono → Recording `mic.fill` + 紅點 overlay（per SPEC-41 §10.2 + mockup L350）
- NSImage template image 自動深淺色適應（per SPEC-40 G4 / SPEC-41 G4）
- Dropdown presentation = `dropdown`（attached_to_status_item，**focus_steal=false / esc_dismisses=true**，per SPEC-41 §7.4）— **不搶 user 當前 app 焦點**（per SPEC-41 G7、J2、§6.4 F2 sequence — 同 chip popover 原則）
- Dropdown row 順序：**stop 在 pause 之上**（per hero wireframe §macOS L205 R4 fix + mockup §macOS L361 — Recording 期間最高優先；與 Windows tray invariant 一致）
- Dropdown row label 走 i18n：`focus.btn.stop_finalize`（desktop 用「停止並收工」/「Stop & finalize」）vs mobile `focus.btn.stop`（「停止」）— 不同字串（per mockup §macOS L366）
- 計時器、waveform mini、chunk count 仍 render — 只是塞進 dropdown 內 60pt height mini waveform（per mockup §macOS L358）

## 螢幕 C' — Interrupted（per hero invariant + SPEC-41 §interrupt notification）

OS interrupt 來源（macOS 上）：
- mic 被別 app 抓（如 Zoom / FaceTime 啟動）— AVAudioSession 等效
- 系統 sleep（lid close / Energy Saver 觸發）
- 藍牙耳機切換（AirPods H1/H2 chip mic source 切換）

**Desktop 無專屬 UI 變體**（per hero invariants L349）— waveform 不凍結、計時不停；改用**系統強制 Notification Center banner**（per hero invariants L350「Desktop 中斷強制系統通知」+ mockup §macOS L376-383）。

```
[C' macOS Notification Center banner（系統渲染，OS interrupt + 主視窗非 active focus 時必發）]
┌────────────────────────────────────┐
│ [app-icon]  Phantom Mesh           │  ← title `focus.desktop.interrupt_notif_title`
│ Focus 05:23 / 25:00 · [reason]     │  ← subtitle 動態（mic 被佔 / sleep / BT 切）
│ [resume-hint-body]                 │  ← `focus.interrupted.resume_hint`（跨平台一字不差）
│ [stop-action]                      │  ← `focus.desktop.interrupt_notif_action` deep-link
│                       sound: default│
└────────────────────────────────────┘
```

Banner 點擊 → 開回 Phantom Mesh 主 window 或 NSStatusItem dropdown active。

## 螢幕 D / E — Finalizing / Done（per hero + macOS Notification Center）

機制同 iOS：E phase 1 (Transcribing) + phase 2 (SummaryGen)。**macOS 沒鎖屏控制（iOS D 對等物）**— 改用：

```
[D. Toast（in-app HUD，非系統）— Finalizing]
        ↓ 完成
[E. macOS Notification Center banner — Done]
┌────────────────────────────────────┐
│ [app-icon]  Phantom Mesh           │  ← title「Phantom Mesh」（per mockup §macOS L368-374）
│ Focus 25 min · takeaway ready      │  ← subtitle
│ [takeaway-first-line-80-chars]     │  ← body 第一行 takeaway 取 80 字
│                                    │  click → open main window Focus tab
└────────────────────────────────────┘
        ↓ click banner
[F. Main window Focus tab — Takeaway card（per mockup §macOS L385-390）]
        ┌─────── sidebar 220pt ────── main 640pt ───────┐
        │ [history-list-recent-10]   [takeaway-card]    │
        │                            [duration+count]   │
        │                            [takeaway-body]    │
        │                            [view-full] [new]  │
        └───────────────────────────────────────────────┘
```

**Wireframe 重點**:
- D 是 in-app toast（HUD 樣式 menubar dropdown 內 / 或 settings window 角落），不走 Notification Center — 因 Finalizing 階段短（< 30s）且 user 可能還在 NSStatusItem dropdown
- E 是 Notification Center banner — 系統渲染、版面不可自訂（per SPEC-41 §10.13 對齊 mockup §macOS L368-374）
- F = `coach_review_reader` 等效 NSWindow（per SPEC-41 §7.4 default_size 720×640、follow_menu_bar_icon multi-monitor）— **不嵌 NSStatusItem dropdown**（per SPEC-41 §17 Alt 2「dropdown 暫態本質不適合長閱讀」）
- F 卡片結構同 iOS F，但加 left sidebar 220pt 顯最近 10 session history（per mockup §macOS L388-390）

## Multi-Monitor 行為（per SPEC-41 G3）

| Window 類 | Spawn 規則 |
|---|---|
| A. Focus start sheet | `follow_parent_window` — 跟 trigger 的 parent window（如 settings）同 monitor；若無 parent（純 `⌘⇧F` global trigger）→ NSStatusItem 所在 monitor |
| C dropdown | `attached_to_status_item` — 永遠跟 NSStatusItem |
| F. Takeaway window | `follow_menu_bar_icon` — 永遠 NSStatusItem 所在 monitor；user 拖到別 monitor 後 size persist per-monitor（per SPEC-41 §8 state machine + macos_window_state.json）|
| Notification Center banner | OS 控制，user 系統設定決定 |

**邊界 case**：拔掉螢幕後 takeaway window 變孤兒 → 自動搬到 primary monitor center（per SPEC-41 §18.1 risk + mitigation）。

## 入口架構決議（per SPEC-41 §6.1 IA + SPEC-21 hero invariants）

| 元素 | macOS 對映 |
|---|---|
| **Cluster IA 主結構**（per SPEC-41 12 screens）| 4-tab settings window（General / Cluster / Providers / Privacy）— Focus 不獨立 tab，從 menu bar dropdown 直 trigger sheet |
| **History 位置**（hero invariant：desktop sidebar）| 在 takeaway window F 左側 sidebar 220pt（per mockup §macOS L388-389）— 不在 settings window 內 |
| **System back equivalent** | `⌘W` 關 F window / Esc 關 A sheet（per SPEC-41 §12.2 a11y）|
| PTT × Timer 互斥 | **N/A（per hero invariants L348「mobile only」）** — macOS 無 PTT |

## Cross-platform invariants 對齊（per hero wireframe §invariants）

繼承全部 hero invariants（trust badge / Stop ≤ 2 操作 / waveform / chunk count / 計時器顏色）。macOS 額外：

- **NSStatusItem icon 是 Recording 中唯一視覺錨點** — 24/7 不消失（per SPEC-40 G4 / SPEC-41 G5 30 天測項）；Bartender / Hidden Bar 偵測到 button.visible=false ≥ 60s → 發 `MACOS_HIDDEN_BAR_DETECTED` 系統通知（per SPEC-41 §11）
- **Sheet vs popover vs dropdown 三層 presentation 嚴格分**（per SPEC-41 §7.4）：focus start = sheet（focus_steal）；NSStatusItem dropdown 與 chip popover = transient（不搶焦點）
- **Notification Center banner 必須 fire**（OS interrupt 時 + 主 window 非 active focus，per hero invariant L350）— sound = default（高優先級）
- **計時器、waveform、chunk count 都塞 NSStatusItem dropdown**（不開獨立 Recording 主畫面）— 但 Stop 操作仍 ≤ 2（dropdown 第一個 actionable row 就是 stop，per hero invariant Stop ≤ 2）
- **VoiceOver labels 必填**（per SPEC-41 §12.2 + WCAG 2.2 AA） — sheet / dropdown row / icon 全要
- **IOPMAssertion 防休眠**（Recording 期間 opt-in）— `IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleSystemSleep)`：user 蓋上螢幕 / 進入省電模式會 trigger interrupted → 30s timeout 後 finalize；opt-in 後 Recording 期間保持喚醒不進 sleep。Settings → General → "Recording 中防止電腦休眠" toggle，預設 off（per user 預期行為）；中斷率高的場景再 opt-in

## 6 大資料狀態 — macOS 對映表

| 狀態 | macOS 螢幕 / 場景 | 對應 i18n key / mockup |
|---|---|---|
| **理想（Ideal）** | F Takeaway window + sidebar 含 10 個 history session | `focus.done.title` per mockup §561 + §macOS L385 |
| **空白（Empty）** | F window sidebar 0 session（首次完成前）／ ASR 無語音時 takeaway 顯安撫文 | `focus.empty.history` / `focus.empty.no_voice_macos` |
| **極限（Limit）** | C dropdown chunk 99+ ／ F takeaway > 800 字截斷顯「查看完整」| `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` per mockup §563 |
| **錯誤（Error）** | B' TCC denied 覆蓋 sheet ／ C' interrupted Notification Center banner | `focus.perm.denied_macos` / `focus.interrupted.*` |
| **局部（Partial）** | E phase 2 部分 chunk ASR 失敗 — in-app toast + 後續 takeaway 標示 | `focus.partial.chunk_failed` per mockup §565 |
| **載入中（Loading）** | D Finalizing toast ／ NSStatusItem dropdown 文字更新（spinner row） | `focus.finalizing.asr` |

## 已決（per SPEC-41 §10 lock + hero wireframe R4）

1. ~~Sheet vs popover 之爭~~ → **已決**：sheet（per SPEC-41 §10.4 + §7.4 — 因要 duration + tag 兩輸入，popover 太擠）
2. ~~Recording 中 NSStatusItem dropdown stop / pause 順序~~ → **已決**：stop 在 pause 之上（per hero wireframe macOS R4 fix + mockup §macOS L361 — Recording 期間最高優先，與 Windows tray invariant 同步）
3. ~~Takeaway 顯示位置~~ → **已決**：獨立 NSWindow（F）含 sidebar + main card，不嵌 NSStatusItem dropdown（per SPEC-41 §17 Alt 2 — dropdown 暫態本質不適合長閱讀）
4. ~~Multi-monitor spawn 規則~~ → **已決**：follow NSStatusItem 所在 monitor + per-monitor size persist（per SPEC-41 G3 + §8 state machine）
5. ~~OS interrupt UX~~ → **已決**：強制 Notification Center banner（per hero invariant L350 + mockup §macOS L376-383）

## 開放問題（macOS 層面，剩餘）

1. **`⌘⇧F` 連按二下跳過 sheet 直接 25min fast-path** — 共用 hero §開放 Q1（per hero wireframe L362）；macOS impl 時若拿到 NSEvent double-tap pattern 後可單獨開
2. **Recording 中是否要強制 keep-awake**（防 lid close 觸發 sleep 中斷） — SPEC-40 沒列；可走 `IOPMAssertionCreateWithName(kIOPMAssertionTypeNoIdleSleep)` 但需 user opt-in（避免電池焦慮）
3. **`⌘⇧F` shortcut 衝突 fallback** — 若 user 已綁此鍵給別 app，第一次註冊 fail 時 wizard 提示自訂（per SPEC-40 §3.1 G5 caveat）— wizard UI 跟 SPEC-41 §10.5 General tab `[Change…]` 整合，但 fail UX flow 細節未鎖
4. **Stage Manager 內 NSStatusItem 行為** — SPEC-41 §18.2 Q5 預設不特殊處理，但 Recording 中 user 進 Stage Manager group 是否要持續顯倒數 → 未測，等 user feedback

→ 互動 timing / 動畫 / 系統 banner 行為細節歸 macOS prototype（待補）。

## 下一步

→ 進 [Mockup §macOS 段](./SPEC-21-capture-focus-mockup.md#macos--mockup)（已 ship，行 L322-390）決定 SF Symbols / 終版文案 / NSStatusItem icon composite 細節；prototype 階段測 multi-monitor + Stage Manager + lid close edge case。
