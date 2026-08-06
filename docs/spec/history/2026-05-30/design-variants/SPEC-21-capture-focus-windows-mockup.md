# SPEC-21 Capture Focus — Windows Mockup（視覺稿）

> **Stage 2/3** · [線框稿（Windows）](./SPEC-21-capture-focus-windows-wireframe.md) → 視覺稿 → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Windows only — Fluent design token / Lucide icon / Win 11 toast XML / tray icon 配色終值 / 終版文案。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-mockup.md](./SPEC-21-capture-focus-mockup.md) §iOS hero + §Windows section L399-447）— 本檔擴展 Windows section 為完整視覺稿，**不重抄 hero**。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-42-PLATFORM-Windows-foundations`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43-PLATFORM-Windows-screens-flows`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 為什麼 Windows 有獨立 mockup

hero mockup §Windows（L399-447）只列「跟 macOS deltas」49 行 high-level。實際 Windows 視覺需鎖：
1. **Tray icon Recording 配色終值** — wireframe §開放問題 #3 留給本檔拍板（綠 vs 橘 — 兩個 source of truth 衝突，必須收斂）
2. **Tray context menu 視覺結構**（Recording 期間 Stop 提到首項、其他 disabled）— hero mockup §417 只給 label，未列 visual states / icon spec
3. **Start window** 真窗（NSWindow-equivalent）480×320px + 預設 OS chrome — hero mockup §403 一句帶過
4. **ActionCenter toast** 完整 XML 樣板 + Done / Interrupted 兩變體 scenario / audio 屬性
5. **Lucide icons** 對映表（hero mockup §52 列 Win/Linux/Web 共用 Lucide，但未列 SPEC-21 Windows 終值）
6. **Focus Assist 折疊** 後的視覺退化路徑（in-app banner fallback）
7. **Narrator AutomationName** — 每元件可朗讀字串（per SPEC-43 §12.2 a11y）

## Tray icon Recording 配色 — 終值拍板（橘）

wireframe §開放問題 #3 留給本檔拍板。兩個 source of truth 衝突：

| Source | 配色 | 語意 |
|---|---|---|
| SPEC-43 §8.1 | **綠點**（`spectyn-tray-working.ico`） | cluster active task 的 working state（與 N peers running 同色） |
| hero mockup §412 | **spectyn-warning 飽和橘** | 「請小心、正在錄音」warning 語意 |

**本檔決議：採 spectyn-warning 橘**。理由：

1. **跨平台 invariant 對齊** — macOS NSStatusItem Recording 是「`mic.fill` 模板 + **紅點 overlay**」（per hero mockup §357），其紅點明確是「正在錄」warning 而非 cluster state。Windows 採橘維持「Recording = warning 系」跨桌面平台一致（mac 紅 / Win 橘 / Linux 橘）
2. **避免 cluster state 混淆** — SPEC-43 §8.1 綠點 working 是給「Coach 跑 task / Quick Log 排隊」這類**背景非 user-blocking task**；Focus 錄音是 **user 主動進行的 mic 採集**，semantic 不同
3. **計時器配色一致** — hero mockup §548 cross-platform invariant：「Recording 中計時器 = `spectyn-warning`」。tray icon 跟主視窗計時器同色（橘）視覺連續、user mental model 一致
4. **Recording 中 tray icon 跟主視窗形成 ambient 提示** — user 漏看主視窗、瞄一眼右下角橘色 mic icon 立刻知「我還在錄」；綠點容易跟 Slack / Discord 「online」綠混淆

→ **本檔覆寫 SPEC-43 §8.1 line 637 Recording 配色為橘**；SPEC-43 §8.1 綠點仍適用於「cluster active task」非 Focus 情境。建議 SPEC-43 下版同步：把 §8.1 第 5 列「**focus-recording: spectyn-tray-focus.ico（spectyn-warning 橘）**」加進 state matrix，與 working 並列。

## Design token 對映（per SPEC-02 + hero mockup §10）

繼承 hero mockup design token 速查（L10-22）+ overlay opacity 表（L26-34）。Windows 額外用 token：

| Token | Hex | Windows 用途 |
|---|---|---|
| `spectyn-bg` | `#0f0f1a` | Start window 內容區背景（OS title bar 不染） |
| `spectyn-card` | `#1a1a2e` | duration picker chip / button group bg |
| `spectyn-warning` | `#ff9800` | **Tray icon Recording 配色（拍板）** + 計時器 + waveform |
| `spectyn-danger` | `#dc3545` | Stop button bg + Tray icon Error state |
| `spectyn-muted` | `#8888aa` | Tray icon Idle + secondary text + Paused state |
| `spectyn-primary` | `#8ab4f8` | Start button bg + toast action button |

**Fluent design token 對映**：本檔**不採 Fluent native palette**（如 SystemAccentColor）— 維持 spectyn brand consistency（同 Android 不採 Material You）。Windows 11 user 若想要系統色，v0.7+ 評估 Settings 加「Use system accent」toggle，預設關。

**Mica acrylic 半透明**：**v0.6.0 不做**（per wireframe §開放問題 #5 已決 + SPEC-43 §3.2 NG2 — Tauri 2 binding 未穩 + node-a black flash 踩坑）。Start window 採實心 `spectyn-bg` 背景。

## 字級（per SPEC-02 type ramp）

繼承 hero mockup §37-49 字級表。Windows 字體：

- **主視窗 / Start window**：`Segoe UI Variable`（Win 11 預設）→ fallback `Segoe UI`（Win 10）→ fallback `Tahoma`
- **計時器大數字**：`Segoe UI Variable Display`（Win 11 才有）→ fallback `Segoe UI Variable` → fallback 系統 sans-serif
- **Tray menu**：**用系統預設**（不自訂；Windows menu 字體跟著 system theme，user 設「使用較大字型」會生效）
- **Toast**：**OS-rendered**，spectyn 不控字體（XML 只控文字內容）

## Lucide icon 對映（per hero mockup §56 Icon 對照矩陣 — Lucide column）

繼承 hero mockup §58-71 全表。Windows 用 Lucide column（已在 `MobileDashboard.tsx` / `app/src/components/*` 使用）。Windows 特化：

| 角色 | Lucide icon | Windows 用途 / 尺寸 |
|---|---|---|
| 麥克風 / Recording | `mic` | Tray icon 16×16（Recording state） + Start window PTT n/a |
| 麥克風關閉 / Paused | `mic-off` | Tray icon overlay（Paused state） + B' denied 卡 64×64 |
| 停止 | `square`（filled） | Stop button icon 16×16 + Tray menu Stop row |
| 暫停 | `pause` | Tray menu Pause row + 主視窗 Pause button icon |
| 播放 / 開始 | `play` | Start button icon 16×16 |
| 設定 | `settings` | Tray menu Settings row + B' 「打開設定」按鈕 |
| 警告 / 中斷 | `triangle-alert` | D' Interrupted toast icon + Focus Assist fallback banner |
| 完成 | `check-circle` | F Done takeaway card icon 24×24 + D toast AppLogo overlay |
| 開啟 | `external-link` | D toast action button icon（next to 「開啟回顧」） |
| 折疊 | `chevron-down` | Start window duration picker dropdown（v0.7+ custom mode） |

**Icon 來源**：所有 Lucide SVG bundled with app（`app/src/icons/lucide/*.svg`），**不依賴系統 icon font**（避 Win 10 fallback 不一致）。Tray icon `.ico` 檔由 Lucide SVG 預先 render 成 16×16 + 32×32 多 frame（DPI scaling per SPEC-43 §OoS3）。

## Windows 共用文案 keys（per hero mockup, +3 new）

繼承 hero mockup §75-135 全部 i18n keys。Windows 新增：

| Key | zh-TW | en |
|---|---|---|
| `focus.tray.header_idle` | Spectyn Mesh · {peers} 節點 · 待機 | Spectyn Mesh · {peers} peers · idle |
| `focus.tray.header_recording` | Spectyn Mesh · Focus {elapsed} / {total} | Spectyn Mesh · Focus {elapsed} / {total} |
| `focus.tray.tooltip_recording` | Focus {elapsed} / {total} — 右鍵開控制 | Focus {elapsed} / {total} — right-click for controls |
| `focus.windows.mic_disabled_by_system` | 系統已停用麥克風，請至設定開啟 | Microphone disabled in Windows settings |
| `focus.windows.focus_assist_fallback` | Focus Assist 開啟中，通知改在 app 內顯示 | Focus Assist active; notifications shown in-app |

## 螢幕 A — Start Window（真窗 480×320px）

```
┌──────────────────────────────────────────────┐  ← OS chrome：title bar height 32px
│ Start Focus Session              [_][□][X]   │   bg: OS theme（不染 spectyn-bg）
├──────────────────────────────────────────────┤   ← divider 1px Win11 stroke
│                                              │  bg: spectyn-bg（內容區）
│                                              │
│              00:00 / 25:00                   │   display 48px/700, spectyn-muted (idle)
│                                              │
│  Duration:                                   │   body-lg 16px/500, spectyn-text, padding-left 24px
│  ┌────┐ ┌────┐ ┌────┐                       │   chip group: 3 × 64×32px, gap 8px
│  │ 15 │ │ 25 │ │ 50 │  min                   │   selected: bg spectyn-primary, text spectyn-bg, radius 6px
│  └────┘ └────┘ └────┘                       │   unselected: bg spectyn-card, text spectyn-text, border 1px spectyn-border
│                                              │
│              ┌──────────────────┐            │   Start btn: 240×40px, radius 6px
│              │  ▶  開始計時錄音  │            │   bg spectyn-primary, text spectyn-bg, body-lg/600
│              └──────────────────┘            │   icon: Lucide `play` 16px spectyn-bg
│                                              │
│         🔒 本地加密 · 麥克風 ASR                │   caption 12px/400, spectyn-muted, centered
└──────────────────────────────────────────────┘  ← window border：Win 11 1px stroke + 8px corner radius
```

**Visual states**：
- duration chip：idle (card bg) / hovered (bg lighten 8%) / selected (primary bg) / focused (2px spectyn-primary outline ring，鍵盤 navigation 必有)
- Start button：idle / hovered (primary @ 90%) / pressed (primary @ 80%) / focused (2px spectyn-primary outline ring inset 2px) / disabled (`overlay-disabled-40`)
- title bar：跟著 Win 11 dark/light theme 自動切（不自繪）

**Window 規格**：
- Size 480×320px 固定（min/max width 鎖死，user 不可 resize — 維持版型穩定）
- Position：開啟時置中於 user 當前 monitor（多顯示器抓 active focus monitor）
- Z-order：normal window（不 always-on-top）
- close action：等同 Cancel — 不寫 events、不啟動 session

**鍵盤導覽（per SPEC-43 §12.2）**：
- Tab order：15 chip → 25 chip → 50 chip → Start button → close button（最後）
- Win+Enter / Enter on Start：trigger Start
- Escape：close window（cancel）
- Alt+F4：close window（OS 規範）
- 1 / 2 / 3 數字鍵：直接選 15/25/50 chip（accelerator）

**Narrator AutomationName**：
- Title bar: "Start Focus Session" (auto from window title)
- 15/25/50 chip: "Duration {n} minutes, button" + AccessKey "1/2/3"
- Start button: "Start timer recording, button, Enter to activate"
- Trust badge: "Encrypted on device, local microphone ASR. Audio never uploaded."

## 螢幕 B' — Mic disabled 變體（覆蓋 Idle）

per wireframe §B' Mic disabled 變體。視覺：

```
┌──────────────────────────────────────────────┐  覆蓋層: bg spectyn-bg @ 92% on Start window
│                                              │
│         [mic-disabled-icon-64]               │  Lucide `mic-off` 64×64, spectyn-danger
│                                              │
│   系統已停用麥克風，請至設定開啟              │  body-lg 16px/500, spectyn-text, centered
│                                              │  取 `focus.windows.mic_disabled_by_system`
│   ┌──────────┐  ┌──────────┐                │  buttons: 各 96×32px, gap 12px, centered
│   │ 打開設定 │  │ 重試      │                │
│   └──────────┘  └──────────┘                │
│                                              │
└──────────────────────────────────────────────┘
```

- 「打開設定」button：bg spectyn-primary, text spectyn-bg, radius 6px, icon Lucide `settings` 16px → deep-link `ms-settings:privacy-microphone`
- 「重試」button：bg spectyn-card, text spectyn-text, border 1px spectyn-border, radius 6px → re-invoke `WASAPI` init

**Visual states**：背景 Start window chip / Start button 全套 `overlay-disabled-40`。Open settings 按鈕 pressed = primary @ 80%。

## 螢幕 C — Recording（主視窗 + tray icon 同步）

主視窗版型同 hero mockup macOS C（per L181-197）— 計時器 / waveform / Pause-Stop / chunk count / trust badge 全繼承。**Windows delta** 僅 tray icon 與 tray menu rebuild（見下）。

### Tray icon state matrix（拍板覆寫 SPEC-43 §8.1）

| State | Icon | Tint | Hover tooltip | Narrator label |
|---|---|---|---|---|
| Idle | Lucide `mic` 16×16 | `spectyn-muted` (#8888aa) | "Spectyn Mesh · idle" | "Spectyn Mesh microphone, idle" |
| **Recording**（**橘 — 本檔拍板**） | Lucide `mic` 16×16 | **`spectyn-warning` (#ff9800)** | 取 `focus.tray.tooltip_recording` | "Spectyn Mesh recording, {elapsed} elapsed" |
| Paused | Lucide `mic-off` 16×16 | `spectyn-muted` | "Focus 已暫停 — 右鍵繼續" | "Spectyn Mesh paused, right-click to resume" |
| Error | Lucide `mic` 16×16 + 紅點 overlay 6×6 右下角 | `spectyn-danger` (#dc3545) | "麥克風錯誤 — 右鍵看選項" | "Spectyn Mesh microphone error" |

**State 切換需 debounce 防閃爍**（per SPEC-43 §8.1）— chunk 邊界連續切換時避免 icon flicker。具體 debounce timing 由 Prototype 鎖。

**Icon 檔**（per SPEC-43 §8.1 命名）：
- `spectyn-tray-idle.ico` — Idle
- **`spectyn-tray-focus.ico`**（新增） — Recording（橘）
- `spectyn-tray-paused.ico`（新增） — Paused
- `spectyn-tray-error.ico` — Error

每 ico 含 16×16 + 32×32 + 48×48 三 frame（DPI scaling per SPEC-43 §OoS3）。

### Tray context menu（Recording 期間，動態 rebuild）

per wireframe §92-103。視覺：

```
┌──────────────────────────────────────────────┐  Win 11 system menu style
│ Spectyn Mesh · Focus 05:23 / 25:00          │  header 灰 disabled, italic, body 14px
│                                              │  取 `focus.tray.header_recording`
├──────────────────────────────────────────────┤  divider 1px system-stroke
│ ⏹ Stop & finalize                Ctrl+Shift+S│  ← Recording 期間最高優先（hero invariant + Win tray rule）
│ ⏸ Pause                                      │     row 32px, icon 16px + label body, accelerator 右對齊
├──────────────────────────────────────────────┤
│ Open Spectyn Mesh                  Ctrl+O    │  row 32px
│ Settings...                                  │
└──────────────────────────────────────────────┘
```

**delta vs hero mockup §419-426**：
- **Stop 提到 Pause 上方**（per hero wireframe R4 + line 205 「stop & finalize 提到 pause 上方，recording 中最高優先」）— hero mockup §422 已對齊，本檔再 lock 一次
- **header 動態更新計時器**（取 `focus.tray.header_recording` i18n key）— rebuild 頻率與 debounce 由 Prototype 鎖
- **accelerator 右對齊**（Win 11 menu 慣例） — `Ctrl+Shift+S` 為 Stop 加速鍵（user 不必右鍵 menu，可主視窗 active 時直接按）
- **icon Lucide-from-SVG**：Stop = `square` (filled), Pause = `pause`，皆 16px 用 Tauri menu icon API 嵌入
- **disabled item 顏色**：跟系統 menu disabled item 一致（不自染）
- **Quick Log / Start Focus... 灰階 disabled**（避撞，per SPEC-43 §8.2 鎖定順序 + wireframe line 105）— 本檔示意省略，實作端 menu rebuild 時保持 row 但 disable

### Tray icon hover tooltip

取 `focus.tray.tooltip_recording`：`Focus 05:23 / 25:00 — 右鍵開控制`（zh-TW）/ `Focus 05:23 / 25:00 — right-click for controls`（en）。每 1s update。Win 11 tooltip max-width 不可控（OS render），文案長度 < 50 char 確保不被 trim。

## 螢幕 D — Done ActionCenter toast（XML 樣板）

per wireframe §125-141 + hero mockup §429-436。終版 toast XML：

```xml
<toast launch="spectyn-mesh://focus/{session_id}" scenario="default">
  <visual>
    <binding template="ToastGeneric">
      <text id="1">Spectyn Mesh</text>
      <text id="2">Focus 25 min · takeaway ready</text>
      <text id="3">{takeaway_line_1 truncated to 60 chars}</text>
      <image placement="appLogoOverride" hint-crop="circle"
             src="ms-appx:///assets/spectyn-mono-icon-44.png"/>
    </binding>
  </visual>
  <actions>
    <action content="開啟回顧" arguments="action=open"
            activationType="protocol"/>
  </actions>
  <audio src="ms-winsoundevent:Notification.Default"/>
</toast>
```

**視覺對映**：

```
┌─────────────────────────────────────────────┐  Win 11 toast，OS-rendered，spectyn 只控 XML
│  ◯  Spectyn Mesh                            │  AppLogo 44×44 circle crop, spectyn mono icon
│      Focus 25 min · takeaway ready          │  text id=2，第一行 (取 `focus.done.title` 變體)
│      第一行 takeaway 截 60 字…              │  text id=3，截字上限 60（per hero mockup §552）
│                                             │
│      ┌──────────────┐                       │  action button，OS style
│      │  開啟回顧     │                       │  label key `focus.btn.review`（**待補進 hero mockup 共用文案 — Stage 3 前**）
│      └──────────────┘                       │
└─────────────────────────────────────────────┘
```

**XML 屬性決議**：
- `launch="spectyn-mesh://focus/{session_id}"` — toast body click（非 button）的 fallback（per SPEC-43 §9.3 deep-link）
- `scenario="default"` — 可被 Focus Assist 折疊（Done 不是 urgent，user 之後到 Action Center 補看，per wireframe §開放問題 #2）
- `audio` = `Notification.Default` — 預設輕音，user 可在 Settings → Notifications 調靜音
- `activationType="protocol"` — 走 `spectyn-mesh://` deep-link 啟動主視窗（per SPEC-43 §9.3 同 coach review 機制）
- AUMID anchor：`com.spectyn-mesh.app`（per SPEC-42 §8.5，MSI 安裝時 shortcut metadata 註冊；無 AUMID → `R.windows.toast_emit_fail` 退化）

**截字規則**：takeaway 第一行 > 60 字時 trim 至 57 字 + "…" suffix（per hero mockup §552 cross-platform 統一）。**空白變體（ASR 無語音）：不發 toast**（避免空通知打擾，per wireframe §170 + hero mockup `focus.empty.no_speech`）。

## 螢幕 D' — Interrupted toast（系統強制觸發）

per wireframe §143-161 + hero mockup §438-447。XML 與 D 同結構，差異 attribute：

```xml
<toast launch="spectyn-mesh://focus/{session_id}/stop" scenario="urgent">
  <visual>
    <binding template="ToastGeneric">
      <text id="1">Spectyn Mesh 焦點時段中斷</text>      <!-- focus.desktop.interrupt_notif_title -->
      <text id="2">5:23 / 25:00 · mic 被佔用</text>      <!-- 動態 per interrupt 來源 -->
      <text id="3">30 秒內回復將自動繼續</text>           <!-- focus.interrupted.resume_hint -->
      <image placement="appLogoOverride" hint-crop="circle"
             src="ms-appx:///assets/spectyn-mono-icon-44.png"/>
    </binding>
  </visual>
  <actions>
    <action content="開啟並停止" arguments="action=stop"
            activationType="protocol"/>
  </actions>
  <audio src="ms-winsoundevent:Notification.Looping.Alarm2"
         loop="false"/>
</toast>
```

**delta vs D**：
- **`scenario="urgent"`** — 穿透 Focus Assist 折疊（per wireframe §159 + hero mockup §447 — 高優先級，避免重要狀態被靜音）
- **action button label** 取 `focus.desktop.interrupt_notif_action`「開啟並停止」 — 點下去 launch `spectyn-mesh://focus/{id}/stop` deep-link、activate 主視窗 + invoke stop
- **audio**：`Alarm2` 提示音（非預設輕音）— interrupt 是異常需 user attention
- **三 text 共用 i18n keys**（hero mockup §134-136 desktop interrupt 跨 mac/win/linux 一字不差）

**Focus Assist 折疊視覺處理**（per wireframe §117 + SPEC-43 §15）：
- Done toast（`scenario="default"`）被折疊 → user 之後到 Action Center（Win+N）補看，無 in-app fallback
- Interrupted toast（`scenario="urgent"`）正常穿透；若 user 在 Win 11 Settings 完全關掉 toast → 退化到主視窗頂部 banner（`focus.windows.focus_assist_fallback` 文案、icon `triangle-alert` 16px spectyn-warning、bg `overlay-recording-16`、可點關閉）

## 螢幕 E / F — Finalizing / Done（主視窗 + tray 同步）

主視窗版型同 hero mockup macOS / iOS E / F（per L251-296）。**Windows delta**：

- **Tray icon 同步**：Recording 橘 → Finalizing 橘（hover tooltip 變「整理逐字稿 ({i}/{n})」取 `focus.finalizing.asr`）→ Done 橘 3 秒後回 Idle muted
- **Tray menu header 同步**：Finalizing 中 header 取 `focus.finalizing.asr`；Done 中 header 切回 idle 變體
- **D toast 在 Done 那刻 emit**（per Flow 2 + SPEC-43 §6.3）— 即使主視窗 active 也彈一次（per wireframe §170 — Windows 不抑制 active focus 通知，與 macOS focus-suppressed banner 行為不同）
- **ASR 全靜音情境（Empty）**：takeaway card 顯示安撫文「本次時段未偵測到語音，已為您記錄時長」（取 `focus.empty.no_speech`）+ session 仍寫 events row — **不發 toast**（per wireframe §172）
- **F takeaway card 樣式**：bg `spectyn-card`, radius 12px（不是 hero mockup §278 iOS 16pt — Win 11 標準 corner 12px），padding 20px, max-width 640px（主視窗 sidebar 220px + 內容區）

## Padding / radius / spacing 規格

| 元件 | padding | radius | min-height / size |
|---|---|---|---|
| Start window 內容區 | 24px all sides | n/a | 480×320 fixed |
| Duration chip | 12px horizontal / 8px vertical | 6px | 64×32px |
| Start button | 24px horizontal | 6px | 240×40px |
| Stop / Pause button（主視窗 C） | 16px horizontal | 6px | 120×40px |
| Tray menu row | 8px vertical | 0（OS-rendered） | 32px |
| Tray context menu | n/a（OS-rendered） | 8px (Win 11) | min-width 280px |
| Toast | n/a（OS-rendered） | 8px (Win 11) | OS spec |
| Takeaway card (F) | 20px all sides | 12px | min-width 400px, max-width 640px |
| B' Mic disabled 覆蓋層 | 32px all sides | n/a | full window |

**Win 11 corner radius 慣例**：所有 spectyn-controlled element 採 6px（小元件）或 12px（card / dialog）— 與 Win 11 Fluent 規範 corner radius 對齊（system menu / toast OS 自帶 8px）。

## Cross-platform invariants 對齊（per hero mockup §546）

繼承全部 hero invariants（trust badge 文字 / Stop danger color / 計時器顏色 / takeaway card 尺寸 / Notification body 截字 / Interrupted 系統通知）。**Windows 額外**：

- **Tray icon Recording = `spectyn-warning` 橘**（本檔拍板，覆寫 SPEC-43 §8.1）— 與 macOS NSStatusItem 紅點 / Linux StatusNotifierItem 橘維持「Recording = warning 系」桌面三平台對齊
- **Tray context menu Recording 期間 Stop 必為首項**（per SPEC-43 §8.2 鎖定順序 + hero mockup §422 同步）
- **Toast body 第二行（id=3）≤ 60 字截斷**（per hero mockup §552 cross-platform 統一，比 macOS NC 80 字嚴格）
- **AUMID-anchored toast**（無 AUMID → `R.windows.toast_emit_fail` 退化到 in-app banner，per SPEC-42 §8.5）
- **Tray dropdown render < 150ms p95** + **Toast emit < 500ms p95**（per SPEC-43 G1 / G2）
- **Narrator AutomationName 必填**（per SPEC-43 §12.2 + WCAG 2.2 AA） — Start window 所有 chip / button / Tray menu item / Toast text 都要
- **Win 11 corner radius 6px / 12px**（per Fluent 規範） — 與 macOS 12pt / Linux 任意（GTK theme 控）並列為平台特化值

## 6 大資料狀態 — Windows Mockup 視覺對映

| 狀態 | Windows 螢幕 | 視覺表現 |
|---|---|---|
| **理想（Ideal）** | F Done takeaway card + D ActionCenter toast persists | 完整 takeaway 三段（主視窗 sidebar 220px + 內容 640px）+ toast `scenario="default"` 進 Action Center 歷史 |
| **空白（Empty — History）** | History list in main window left sidebar（無 session）| Lucide `inbox` SVG 64px spectyn-muted + `focus.empty.history` 文案 + "前往 Focus" button (bg spectyn-primary, 24×8px padding) |
| **空白（Empty — ASR 無語音）** | F 安撫文案 + 「重錄這次」/「完成」雙 button（per hero mockup §144 跨平台一致）| 取 `focus.empty.no_speech`；**不發 toast**（per wireframe §172） |
| **極限（Limit）** | C chunk `99+` chip / F takeaway > 800 字截斷 / toast text id=3 > 60 字截斷 | `focus.limit.chunk_overflow` + chunk 區塊 min-width 48px lock（per hero mockup §194） + `focus.limit.takeaway_truncated_hint` + `focus.limit.view_full_takeaway` CTA |
| **錯誤（Error）** | B' Mic disabled / D' Interrupted toast / Focus Assist fallback banner | B' 覆蓋層 spectyn-bg @ 92% + `mic-off` 64px spectyn-danger / D' toast `scenario="urgent"` + Alarm2 audio / fallback banner `triangle-alert` 16px + `overlay-recording-16` bg |
| **局部（Partial）** | E Finalizing inline `focus.partial.chunk_failed` | 同 hero mockup §260 — body-sm spectyn-warning inline 於 progress bar 下方 |
| **載入中（Loading）** | E Finalizing + tray icon 橘 + tray header 動態更新 | spinner 32px spectyn-warning stroke + progress bar 240×4px + tray menu header 取 `focus.finalizing.asr` 即時更新 |

## 已決（per wireframe lock + 本檔拍板）

1. ~~Tray icon Recording 配色（綠 vs 橘）~~ → **已決**：**spectyn-warning 橘**（本檔拍板，理由見 §「Tray icon Recording 配色 — 終值拍板」；覆寫 SPEC-43 §8.1 line 637，建議 SPEC-43 下版同步補「focus-recording」row）
2. ~~Start window 尺寸~~ → **已決**：480×320px 固定，user 不可 resize（per hero mockup §405 + SPEC-43 §10.6）
3. ~~Tray menu Stop 位置~~ → **已決**：Recording 期間提到 Pause 上方首項（per wireframe §99 + hero mockup §422 + hero wireframe R4 line 205）
4. ~~Toast persistent~~ → **已決**：`scenario="default"` Done + `scenario="urgent"` Interrupted；皆 persistent + AUMID-anchored（per wireframe §136-141 + SPEC-43 §9.3）
5. ~~Mica acrylic~~ → **已決**：v0.6.0 不做（per wireframe §開放問題 #5 + SPEC-43 §3.2 NG2）
6. ~~字體~~ → **已決**：Segoe UI Variable / Segoe UI fallback（Win 11 / Win 10）
7. ~~Corner radius~~ → **已決**：spectyn-controlled element 6px（小） / 12px（card） — 對齊 Win 11 Fluent 慣例

## 開放問題（mockup 層面，剩餘）

1. **Tray icon Paused state**：用 `mic-off` outlined 還是 `mic` + amber dot overlay？提案 `mic-off`（清楚），但跟 Idle 的 `mic` outlined 對比不夠強 — 需 node-a / node-b / node-a 三機 4K monitor 各跑一次 visual test 確認可辨識度。
2. **Focus Assist fallback banner 視覺**：本檔提案用 `triangle-alert` + `overlay-recording-16` bg 主視窗頂部 banner；但 user 已在 Focus Assist 期間（dnd 模式）— banner 是否還是該彈？或退化到下次 user 主動切回主視窗才顯示？需 5-user UX session 量測。
3. **Tray icon DPI scaling Recording 配色一致性**：16/32/48 三 frame 預 render spectyn-warning 橘是否在 200% scaling 機（node-a）與 100% scaling 機（node-b）肉眼一致？需 ico 檔產線測試。
4. **Start window 開啟動畫**：Win 11 預設 fade-in 100ms vs 即刻顯示？提案即刻顯示（user opt-in 從 tray menu 點下去希望立刻 ready）— 但屬互動動效，**移到 Prototype 開放問題**。

→ 互動 timing / 鍵盤焦點 cycle / Win 11 toast XML 行為驗證細節歸 Windows prototype（待補）。

## 下一步

→ 進 [Windows Prototype（待補）] 鎖每個 tap target 行為、Tray menu rebuild timing、Toast deep-link launch sequence、Focus Assist 偵測 + fallback banner 觸發時機、Narrator focus order 完整驗證。
