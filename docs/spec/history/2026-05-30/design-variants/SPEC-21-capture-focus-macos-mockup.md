# SPEC-21 Capture Focus — macOS Mockup（視覺稿）

> **階段 2/3** · [線框稿（macOS）](./SPEC-21-capture-focus-macos-wireframe.md) → 視覺稿 → [原型（待補）]
> **狀態**：draft v0.1 · **最後更新**：2026-05-27
> **範圍**：僅限 macOS — NSStatusItem icon 合成圖 / NSWindow sheet（工作表）/ NSPopover dropdown（下拉選單）/ TCC（透明、同意與控制權限機制）提示 / Notification Center（通知中心）banner（橫幅）視覺規格。**主打（hero）平台是 iOS**（見 [SPEC-21-capture-focus-mockup.md](./SPEC-21-capture-focus-mockup.md) §iOS hero + §macOS section L327-396）— 本檔將 macOS 段落擴展為完整視覺稿，**不重抄主打稿**。
> **規格文件**：[`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-41-PLATFORM-macOS-screens-flows`](../specs/v060-deep-spec/SPEC-41-PLATFORM-macOS-screens-flows.md) · [`SPEC-40-PLATFORM-macOS-foundations`](../specs/v060-deep-spec/SPEC-40-PLATFORM-macOS-foundations.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 為什麼 macOS 有獨立 mockup

主打視覺稿 §macOS（L327-396）只列「Start Sheet 框 + menu bar（選單列）icon 三態 + dropdown ASCII + Done/Interrupted banner schema + Takeaway sidebar（側邊欄）」共 ~70 行高層級（high-level）內容。實際 macOS 視覺需鎖定：
1. **NSStatusItem icon 三態 composite（合成）規格** — template image（範本圖像）相對於 紅點 overlay（疊加層）像素位置 + dark/light（深色/淺色）自動適應
2. **NSStatusItem dropdown 完整視覺** — popover 背景 / border（邊框）/ radius（圓角）/ row min-height（列最小高度）+ 變暗邊（per SPEC-41 §7.4 transient presentation 暫態呈現）
3. **NSWindow Sheet 視覺** — radius / hairline divider（髮絲分隔線）/ button row 對齊（focus_steal sheet 風格相對於 free window 不同）
4. **TCC permission prompt（權限提示）** — 作業系統（OS）渲染版面不可控，但 Info.plist `NSMicrophoneUsageDescription` 字串要鎖
5. **Notification Center banner layout（版面）** — icon / title / subtitle / body / action button 每塊 token + 截字邊界
6. **Interrupted banner 強制觸發** — 系統高優先級 sound=default 視覺差異
7. **VoiceOver labels（旁白標籤）**（per SPEC-41 §12.2）— 每元件 `NSAccessibilityLabel` 字串
8. **Dark mode（深色模式）跟隨系統，Light mode（淺色模式）不做**（per SPEC-02 §7 light token TBD，v0.6 範圍外）

→ 這 8 點值得獨立的視覺稿級描述，不要塞在主打稿 §macOS。

## Design token 對映（per SPEC-02 + SPEC-40 G4）

繼承主打視覺稿 §8-35 全部 token（`phantom-bg` / `phantom-card` / `phantom-border` / `phantom-primary` / `phantom-warning` / `phantom-danger` / `phantom-text` / `phantom-muted` / overlay 系列）— 本檔**不重新定義 hex（十六進位色碼）**。macOS 專屬對映：

| macOS 系統屬性 | phantom token | 用途 |
|---|---|---|
| NSWindow `backgroundColor` (sheet) | `phantom-card` | Sheet / popover 背景 |
| NSPopover `appearance` | `NSAppearanceNameDarkAqua` | 跟隨系統 dark mode（強制 dark v0.6） |
| NSStatusItem `button.image` | NSImage **template** `isTemplate=true` | menu bar icon 自動深淺色適應（per SPEC-40 G4） |
| NSMenuItem hover 背景 | `phantom-primary @ 16%` | dropdown row 滑過（取 `overlay-recording-16` 同 16% 系列） |
| NSMenuItem pressed 背景 | `phantom-primary @ 24%` | dropdown row 按下 |
| Notification Center 強調色 | `phantom-primary` | banner 上 action button tint（色調）（OS 渲染但 Info.plist 可指定 NSUserNotificationDefaultSoundName） |
| Window radius | 12pt | NSWindow / NSPopover 共用（per hero mockup §macOS L332） |
| Hairline divider | 0.5pt `phantom-border` | sheet / dropdown 分隔線（Retina 1 物理像素） |

> Light-mode 對映：**v0.6.0 範圍外（out of scope）**（per SPEC-02 §7 light token TBD）。實作端 `NSAppearance.current` 強制 `darkAqua`；使用者系統設為 Light 也吃 dark 配色 — 待 v0.7 補。

## SF Symbols 5 規範（per Icon 對照矩陣，per hero mockup §56）

繼承主打視覺稿 icon 矩陣 iOS/macOS 欄（column）全部值，macOS 專屬補充：

- **NSStatusItem icon 全用 template image**（單色 mono），`SF.symbol.withConfiguration(.preferringMonochrome)` 拿出單色 SVG → `NSImage(systemSymbolName:)` + `isTemplate=true`
- icon 尺寸：menu bar 用 SF point size **18pt**（Apple HIG（人機介面指南）menu bar 慣例 16-18pt；phantom 取 18pt 強對比）
- dropdown row icon 用 **16pt**（選單 row 慣例）
- Sheet 內 icon 用 **20pt**（情境式 button icon）
- Notification banner icon = app icon（NSApp.applicationIconImage），不自訂

## 螢幕 A — Focus Start Sheet（視覺稿，per SPEC-41 §10.4 S3）

```
┌──────────────────────────────────────┐  Sheet：480×320pt, bg phantom-card, radius 12pt
│ 開始焦點時段                  [✕]    │  title 24px/600 phantom-text, padding 20pt
│ ─────────────────────────────────── │  hairline 0.5pt phantom-border
│                                      │
│ 時長：                                │  title-sm 18px/600 phantom-text
│  ○ 25 分鐘 Pomodoro                   │  NSRadioButton：32pt height, label body 14px
│  ○ 50 分鐘                             │  selected radio dot：phantom-primary 8pt
│  ◉ 自訂： [ 30 ] 分鐘                  │  custom input：NSTextField 60×28pt,
│                                      │   bg phantom-bg, radius 6pt, padding 8pt
│                                      │
│ 目標標籤（選填）：                      │  取 `focus.label.goal_tag`
│ ┌──────────────────────────────┐    │  NSTextField full-width, height 32pt,
│ │ deep_work, spec_writing      │    │   bg phantom-bg, radius 6pt, padding 8pt
│ └──────────────────────────────┘    │   placeholder `輸入標籤…` phantom-muted
│                                      │
│ 🔒 本地加密 · 麥克風 ASR               │  caption 12px phantom-muted, 取 `focus.trust_badge`
│                                      │  （與全平台一字不差）
│      [ 取消 ]      [ 開始 ]          │  Cancel：96×32pt, bg transparent, text phantom-muted
│                                      │  Start：96×32pt, bg phantom-primary, text phantom-bg, body-lg
└──────────────────────────────────────┘
```

**Sheet 視覺狀態**：
- **idle（閒置）**：上述基準
- **hover Start（滑過開始）**：背景 `phantom-primary @ 90%`（提亮 10%）
- **pressed Start（按下開始）**：背景 `phantom-primary @ 80%` + 內縮 1pt
- **disabled Start（停用開始）**（TCC 未授權）：`overlay-disabled-40`（40% opacity 不透明度，不換色）+ 下方行內提示（inline hint）「需開麥克風權限 [open settings]」(caption phantom-warning)
- **loading（載入中）**：Start label 替換為 spinner（轉圈）16pt phantom-bg（避免視覺跳動）

**視覺備註**：
- Sheet 跟 parent window（父視窗）用 macOS 內建 `NSWindow.beginSheet` 滑入動畫；Reduce Motion（減少動態效果）開啟時跳過動畫（per SPEC-41 §12.2）
- title bar（標題列）不顯示系統 traffic light（紅綠燈按鈕）（sheet 模式預設無 close/min/max button；[✕] 為自繪 button 16×16pt phantom-muted hover→phantom-text）

**VoiceOver labels**（per SPEC-41 §12.2 NSAccessibilityLabel）：
- Sheet：「開始焦點時段對話框」role `AXSheet`
- 25 min radio：「25 分鐘 Pomodoro」role `AXRadioButton`
- Custom input：「自訂時長分鐘」role `AXTextField`
- Tag input：「目標標籤，選填」role `AXTextField`
- Start button：「開始焦點時段」role `AXButton`
- Cancel button：「取消」role `AXButton`

## 螢幕 B — TCC Microphone Prompt（系統渲染）

OS 渲染，不可自訂版面。phantom 只控制 Info.plist 字串：

| 欄位 | 值 |
|---|---|
| `NSMicrophoneUsageDescription` | 「Phantom Mesh 在你開始焦點時段時錄音，全程在本機 ASR（自動語音辨識）轉寫，不上傳雲端。」/ en: "Phantom Mesh records during focus sessions; ASR runs on-device, no cloud upload." |

**視覺備註**：
- OS prompt 視覺隨 macOS 版本（Sonoma 14.x / Sequoia 15.x 略有不同）— phantom 無權更動
- 拒絕後**不能再彈出** — 使用者須去 `System Settings → Privacy & Security → Microphone` 手動開啟（per SPEC-40 §15 TCC 11 條盤點）

## 螢幕 B' — TCC Denied（拒絕，覆蓋於 Sheet 上）

```
[B' on top of A sheet — overlay-denied-72 半透明遮罩]
┌────────────────────────────┐  Card：360×200pt, bg phantom-card, radius 12pt,
│                            │   shadow 0/8/24 rgba(0,0,0,0.4) (macOS native shadow)
│       [mic.slash.circle    │  SF Symbols `mic.slash.circle.fill` 48pt phantom-danger,
│        .fill 48pt 紅]      │   center top, margin-top 24pt
│                            │
│   需要麥克風才能 focus 錄音   │  title-sm 18px/600 phantom-text, 取 `focus.perm.denied`
│   我們不會上傳音訊到雲端，    │  body 14px phantom-muted, 取 `focus.perm.denied_reassure`
│   ASR 也跑在本機              │
│                            │
│  ┌────────────────────┐    │  Open Settings：240×32pt, bg phantom-primary,
│  │   打開系統設定        │    │   text phantom-bg, body-lg, 取 `focus.perm.open_settings`
│  └────────────────────┘    │
│         [取消]              │  TextButton 80×24pt, text phantom-muted
└────────────────────────────┘
```

**視覺狀態**：
- **idle（閒置）**：上述基準
- **hover Open Settings（滑過打開設定）**：背景 `phantom-primary @ 90%`
- **pressed Open Settings（按下打開設定）**：背景 `phantom-primary @ 80%`
- **clicked（點擊後）**：跳轉系統 `x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone`（NSWorkspace.open URL）— 視覺上 sheet 仍開啟，等使用者回來

**VoiceOver labels**：
- Card：「麥克風權限被拒絕」role `AXGroup`
- mic.slash icon：accessibilityLabel「麥克風關閉」role `AXImage`
- Open Settings button：「打開系統設定隱私頁」role `AXButton`

## NSStatusItem icon spec（三態 composite 合成，per SPEC-41 §10.2）

| 狀態 | SF Symbol | Token | NSImage composite | 視覺 |
|---|---|---|---|---|
| **Idle（閒置）** | `mic` (mono 單色, line-art 線稿) | phantom-text（dark mode → 淺灰 light gray） | `isTemplate=true` 單一 SVG | menu bar 上一個 18pt 單色 mic icon |
| **Recording（錄音中）** | `mic.fill` (solid 實心) | phantom-warning 主 icon + phantom-danger 紅點 | `mic.fill` 18pt + 紅點 6pt overlay 位於右上角 (x=12, y=2 從左上算起)，用 `NSImage` drawingHandler 合成 | 實心 mic + 右上紅點，遠看就知道在錄 |
| **Paused（暫停）** | `mic.slash` | phantom-muted（變暗 dim） | `isTemplate=true` 單一 SVG | 斜線 mic icon，比錄音中更暗 |
| **Finalizing（收工中）** | `mic.fill` + 旋轉 dot（點） | phantom-warning + animating（動畫中） | 取 `mic.fill` 18pt + 1.5pt 旋轉 dot overlay（細微 subtle，per Apple HIG menu bar 不該太顯眼） | 錄音結束 → spinner 過渡，使用者知道還沒收工 |

**視覺備註**：
- Template image (`isTemplate=true`) 讓 macOS 自動反相（invert）：淺色 menu bar 用 深色 icon，深色 menu bar 用 淺色 icon — **不要硬寫顏色，靠系統處理**（per SPEC-40 G4）
- 紅點 overlay **不能**用 template — 用寫死（hard-coded）的 `phantom-danger`（使用者視覺上需要警示色保持不變）
- icon 切換轉場（transition）：直接換圖，不做動畫（menu bar 動畫會干擾，per Apple HIG）

## NSStatusItem dropdown 完整視覺（per SPEC-41 §7.4 + hero mockup §macOS L360-368）

### Dropdown — Idle state（閒置狀態）

```
[click NSStatusItem]
        ↓
┌──────────────────────────┐  NSPopover：width 320pt（hero mockup L362 鎖），
│ Phantom Mesh             │   bg phantom-card, radius 12pt, border 0.5pt phantom-border
│ 焦點時段：未啟動           │   shadow 0/12/32 rgba(0,0,0,0.5)
│ ─────────────────────── │  hairline 0.5pt phantom-border, margin 8pt
│ ⏱  開始焦點時段… ⌘⇧F    │  NSMenuItem-like row：min-height 32pt, padding 12pt h / 8pt v
│ ⚙  設定…                  │   icon SF 16pt phantom-text + label body 14px
│ ─────────────────────── │   shortcut 灰 right-aligned body-sm phantom-muted
│ ⓘ  關於 Phantom Mesh      │
└──────────────────────────┘
```

### Dropdown — Recording state（錄音狀態，per hero mockup §macOS L360-368）

```
┌──────────────────────────┐  NSPopover：width 320pt
│ 🔴 Focus 05:23/25:00      │  body-lg phantom-warning + display-sm 32px/700 time
│                          │   padding 12pt
│ ▁▂▃▅▇▅▃▂                  │  mini waveform 60pt height, 24 bars × 4pt wide × 2pt gap,
│                          │   color phantom-warning, 即時更新
│ 📁 已落地 chunk: 3         │  body 14px phantom-muted, icon SF `folder.fill` 14pt
│ ─────────────────────── │  hairline
│ ⏹  停止並收工              │  NSMenuItem row：min-height 32pt, icon `stop.fill` 16pt
│                          │   phantom-danger + label body 14px phantom-text,
│                          │   取 `focus.btn.stop_finalize`（desktop 用「停止並收工」
│                          │   不同於 mobile `focus.btn.stop`「停止」— per hero mockup L371）
│ ⏸  暫停                    │  NSMenuItem row：min-height 32pt, icon `pause.fill` 16pt
│                          │   phantom-secondary + label body 14px phantom-text,
│                          │   取 `focus.btn.pause`
└──────────────────────────┘
```

**Row 順序鎖**：stop 在 pause 之上（per hero mockup L366 + macOS wireframe R4 — 錄音期間最高優先；與 Windows tray（系統匣）invariant 一致）

### Dropdown row 視覺狀態

| 狀態 | 背景 | text | icon |
|---|---|---|---|
| **idle（閒置）** | transparent（透明） | phantom-text | 依角色上色（tinted per role） |
| **hover（滑過）** | `phantom-primary @ 16%` | phantom-text | 不變（unchanged） |
| **pressed（按下）** | `phantom-primary @ 24%` | phantom-text | 不變（unchanged） |
| **disabled（停用）** | transparent（透明） | `overlay-disabled-40` (40% opacity) | 同 opacity 不透明度 |
| **destructive hover（破壞性滑過）**（Stop row） | `phantom-danger @ 16%` | phantom-text | phantom-danger |
| **destructive pressed（破壞性按下）**（Stop row） | `phantom-danger @ 24%` | phantom-text | phantom-danger |

**VoiceOver labels**（per SPEC-41 §12.2）：
- NSPopover：role `AXPopover`，accessibilityLabel「Phantom Mesh 焦點選單」
- Recording row：「焦點時段中，已錄 5 分 23 秒，共 25 分鐘」role `AXStaticText`
- Stop row：「停止並收工焦點時段」role `AXMenuItem`
- Pause row：「暫停焦點時段」role `AXMenuItem`

**呈現規則（Presentation rules）**（per SPEC-41 §7.4 transient popover 暫態彈出視窗）：
- `focus_steal=false`（不搶使用者當前 app 焦點）
- `esc_dismisses=true`（按 Esc 關閉）
- 點擊外部自動關閉（dismiss）
- 附著（attached）於 NSStatusItem，箭頭尖端對齊 status item 中心

## Notification Center banner — Done（完成，per hero mockup §macOS L373-379）

OS 渲染，不可自訂版面。phantom 設定 `UNUserNotificationCenter` content（內容）：

| 欄位 | 值 | Token / 限制 |
|---|---|---|
| `icon` | NSApp.applicationIconImage | phantom 單色 icon @ 60pt（OS 渲染） |
| `title` | `"Phantom Mesh"` | OS 取 18px/600 |
| `subtitle` | `"Focus 25 min · takeaway ready"` | OS 取 13px/400 |
| `body` | 第一行 takeaway（取 80 字截斷） | OS 取 13px/400, 最多 2 行（per cross-platform invariant L552） |
| `sound` | none（Done 非緊急） | — |
| `categoryIdentifier` | `focus.done` | 無 action button（點擊整個 banner 開啟 app） |
| click action（點擊動作） | 開啟主視窗 Focus 分頁 | `UNNotificationDefaultActionIdentifier` handler |

**視覺備註**：
- banner 出現位置：螢幕右上角（使用者系統設定可改）— phantom 無權更動
- banner 自動消失時機（timing）由 OS 控制（Persistent 持續 相對於 Banner 橫幅 模式看使用者設定）
- Notification Center 累積 history（歷史）：phantom 最多累積 ≤ 5 條（per SPEC-41 §11 通知 throttle 節流）

## Notification Center banner — Interrupted（系統強制觸發）

錄音中 OS 中斷（interrupt）（mic 被佔用 / sleep 睡眠 / BT 藍牙切換）+ 主視窗非 active focus（作用中焦點）時必發。phantom 設定：

| 欄位 | 值 | Token / 限制 |
|---|---|---|
| `icon` | NSApp.applicationIconImage | phantom 單色 icon |
| `title` | 取 `focus.desktop.interrupt_notif_title`「Phantom Mesh 焦點時段中斷」 | — |
| `subtitle` | `"5:23 / 25:00 · {reason}"` 動態 reason（原因） | reason 取自 `focus.interrupted.mic_grabbed` / `focus.interrupted.phone` / 新增 sleep / BT 變體 |
| `body` | 取 `focus.interrupted.resume_hint`「30 秒內回復將自動繼續」 | 跨平台一字不差 |
| `sound` | **default（預設）**（高優先級） | per hero mockup L388 — OS 中斷必發聲 |
| `categoryIdentifier` | `focus.interrupted` | 含 action button |
| action button | 取 `focus.desktop.interrupt_notif_action`「開啟並停止」 | UNNotificationAction title |
| click action（點擊動作） | 開啟 Phantom Mesh + NSStatusItem dropdown 作用中 | deep-link（深層連結）handler |

**與 Done 的視覺差異**：sound = default（Done 無聲）+ 含 action button（Done 無 button，點擊整個 banner）

## 螢幕 F — Main window Focus tab Takeaway card（per hero mockup §macOS L390-396）

```
┌─────── sidebar 220pt ─────── main 640pt ───────────────┐  NSWindow: 720×640pt default
│ 最近 10 個 session         ┌──────────────────────────┐│   bg phantom-bg, title bar 系統預設
│ ─────────────────         │ ✓ 完成 · 25 分鐘 · 5 chunks ││  Sidebar：bg phantom-card,
│ 🟢 今 14:30 deep_work     │                          ││   border-right 0.5pt phantom-border
│    25min                  │  [takeaway 三段內容]       ││  Card：bg phantom-card,
│ ⚪ 今 11:00 spec_writing   │  ...                      ││   radius 16pt, padding 20pt,
│    50min                  │                          ││   shadow 0/4/12 rgba(0,0,0,0.3)
│ ...                       │  ┌────────┐ ┌──────────┐ ││  Success icon: SF
│                           │  │看完整稿 │ │新 session ││   `checkmark.circle.fill` 32pt
│                           │  └────────┘ └──────────┘ ││   phantom-success
│ [+ 新焦點時段]              └──────────────────────────┘│  CTA buttons：min-height 32pt,
│                                                       │   radius 8pt
└───────────────────────────────────────────────────────┘
```

**視覺狀態**：
- Sidebar row idle（閒置）：透明背景
- Sidebar row hover（滑過）：背景 `phantom-primary @ 16%`
- Sidebar row selected（選取）：背景 `phantom-primary @ 24%` + 左側 border 3pt phantom-primary
- 「看完整稿」button：背景 `phantom-card` border 1pt phantom-border, text phantom-text body-lg
- 「新 session」button：背景 `phantom-primary` text phantom-bg body-lg

**VoiceOver labels**：
- Sidebar：「最近 10 個焦點時段，列表」role `AXList`
- 每 row：「{date} {tag} {duration}」role `AXButton`
- Card：「焦點時段成果摘要」role `AXGroup`
- Success icon：accessibilityLabel「已完成」role `AXImage`

## 跨平台不變量（Cross-platform invariants）對齊（per hero mockup §555）

繼承全部主打稿不變量（trust badge 信任徽章文字 / Stop danger color 停止危險色 / 計時器顏色 / Notification body 截字 80 字 / takeaway card 尺寸 / 等等）。macOS 額外：

- **NSStatusItem icon 是錄音中唯一視覺錨點** — 24/7 不消失（per SPEC-40 G4 / SPEC-41 G5）；template image 自動適應深色/淺色 menu bar
- **Dropdown row 順序 stop 在 pause 之上**（per hero mockup L366 + 與 Windows tray invariant 一致）
- **Sheet 相對於 popover 相對於 window 三層呈現嚴格區分**：Start = sheet (focus_steal)，dropdown = popover (transient)，Takeaway = window (independent)
- **Notification Center banner sound=default 僅限 Interrupted**（Done 無聲）— per hero mockup L388 高優先級相對於低優先級
- **VoiceOver labels 必填**（per SPEC-41 §12.2 + WCAG 2.2 AA（網頁內容無障礙指引）G10）— sheet / popover / dropdown row / icon 全要
- **Hairline divider 0.5pt**（Retina 1 物理像素，macOS 慣例）— 不是 1pt（會看起來太粗）
- **Window radius 12pt**（macOS Big Sur 後內建的系統 radius）— sheet / popover 共用
- **Reduce Motion（減少動態效果）** 開啟 → sheet 滑入動畫關閉（per SPEC-41 §12.2）

## 6 大資料狀態 — macOS 視覺稿視覺對映

| 狀態 | 視覺 |
|---|---|
| **理想（Ideal）** | F 主視窗 Takeaway card 完整三段 + sidebar 含 10 個 session history + 雙 CTA（行動呼籲）button |
| **空白（Empty - history 歷史）** | F window sidebar 空：單色 SVG 插圖 96pt phantom-muted + `focus.empty.history` + 「+ 新焦點時段」button (per hero invariant L557 桌面 sidebar history) |
| **空白（Empty - ASR 無語音）** | F card 內安撫文 `focus.empty.no_speech` + 「重錄這次」/「完成」雙 button（與 Android 視覺稿 §150 變體一致） |
| **極限（Limit）** | Dropdown chunk count `99+`（`focus.limit.chunk_overflow`）chip（標籤）min-width 鎖死避免閃動 / F takeaway > 800 字截斷顯示「看完整摘要」(`focus.limit.view_full_takeaway`) |
| **錯誤（Error）** | B' TCC denied 遮罩（overlay-denied-72 + mic.slash.circle.fill 48pt phantom-danger + open-settings CTA）/ Notification Center Interrupted banner 帶 sound=default |
| **局部（Partial）** | E phase 2 部分 chunk ASR 失敗 — 應用程式內（in-app）HUD（抬頭顯示）toast（提示訊息）`focus.partial.chunk_failed` phantom-warning + 後續 takeaway 內以行內方式標示「(chunk 3/5 skipped)」 |
| **載入中（Loading）** | NSStatusItem icon 切換為 Finalizing 旋轉 dot overlay + dropdown 內 row spinner 16pt + 文字更新 `focus.finalizing.asr` |

## 已決（per macOS wireframe §225 + hero mockup R8 sign-off 定案）

1. ~~Sheet 背景用 phantom-card 還是系統 vibrancy（半透明材質）?~~ → **已決**：`phantom-card` 純色（vibrancy 在 dark mode 對比不夠 + 品牌一致性（brand consistency）優先）
2. ~~NSStatusItem icon 錄音中用 fill 還是加紅點?~~ → **已決**：`mic.fill` + 紅點 6pt overlay（per hero mockup §macOS L356 — 雙重視覺提示，色盲安全）
3. ~~Dropdown width（寬度）~~ → **已決**：320pt（per hero mockup §macOS L362 — 容納 mini waveform（迷你波形）60pt 高 + chunk count + 雙 row CTA）
4. ~~Notification Done 是否要 sound?~~ → **已決**：無聲（Done 非緊急；只有 Interrupted 才 sound=default per hero invariant L558）
5. ~~Light mode 對映~~ → **延後至 v0.7（defer）**（per SPEC-02 §7 light token TBD，本檔僅 dark）

## 開放問題（視覺稿層面，剩餘）

1. **NSStatusItem Paused 用 `mic.slash` 還是 `mic.fill` + amber（琥珀色）點?**（共用 hero §開放 Q3，L577）— 提案：`mic.slash` 清楚但跟「stopped 已停止」混淆；待原型（prototype）測使用者識別率
2. **Finalizing icon 旋轉 dot 動畫尺寸** — 1.5pt 會不會在 Retina 上太小看不到？需實機（device）測試（Apple HIG menu bar 不該太顯眼，但要看得到）
3. **NSPopover dropdown 在 Stage Manager（幕前調度）group（群組）中的行為** — 同 macOS wireframe §開放 Q4；視覺上 popover 仍附著於 status item 沒問題，但 Stage Manager 切換 group 時 popover 是否會被擠掉尚未測試
4. **Reduce Transparency（減少透明度）開啟時 popover 視覺** — `phantom-card` 純色沒問題，但 shadow（陰影）可能被系統壓掉，需驗證對比是否足夠

> 移到原型（Prototype）階段的問題：sheet 滑入 timing / NSStatusItem icon transition timing / dropdown auto-dismiss（自動關閉）delay / Notification banner action button 點按 → app 啟用（activate）順序

## 下一步

→ 進入 [macOS Prototype（待補）] 鎖定 `⌘⇧F` 全域 shortcut（快捷鍵）註冊衝突的 fallback（後備方案）/ NSStatusItem icon transition timing / popover dismiss 互動 / TCC 拒絕後的 deep-link 復原（recovery）順序 / 多螢幕（multi-monitor）+ Stage Manager 邊界案例（edge case）。
