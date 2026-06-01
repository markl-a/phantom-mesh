# SPEC-21 Capture Focus — Android Mockup（視覺稿）

> **階段 2/3** · [線框稿（Android）](./SPEC-21-capture-focus-android-wireframe.md) → 視覺稿 → [原型（待補）]
> **狀態（Status）**: draft v0.1 · **最後更新（Last updated）**: 2026-05-27
> **範圍（Scope）**: 僅限 Android — Material 3 / Material Symbols / Glance widget（Glance 小工具）/ Quick Settings tile（快速設定圖磚）視覺規格。**hero（主打）平台是 iOS**（見 [SPEC-21-capture-focus-mockup.md](./SPEC-21-capture-focus-mockup.md) §iOS hero + §Android section L298）— 本檔擴展 Android section（Android 章節）為完整視覺稿，**不重抄 hero**。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-34-PLATFORM-Android-screens-flows`](../specs/v060-deep-spec/SPEC-34-PLATFORM-Android-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 為什麼 Android 有獨立 mockup

hero mockup §Android（L298-340）只列「跟 iOS deltas」13 行 high-level。實際 Android 視覺需鎖：
1. **Material 3 tonal palette** — colors.xml + themes.xml 從 SPEC-02 design tokens 映射
2. **Dynamic Color 預設關**（per SPEC-34 §30(B)）— 但 settings 有 toggle，要畫兩種 mode
3. **Quick Settings tile icon** — Material 規範：mono single-color，24dp container，啟用態有 accent
4. **Glance widget chip palette** — 1×4 / 2×4 / 4×4 三 size，每個 chip 視覺
5. **FG-service notification 樣式** — Material You vs phantom brand 取捨
6. **MIUI 引導 dialog**（per SPEC-34 §30(F)）— button label / dismiss state 視覺
7. **TalkBack contentDescription** — 每元件可朗讀字串（不是視覺但屬 mockup spec）

## Design token 對映（per SPEC-02 + SPEC-34 §30(B)）

| Material 3 attribute | phantom token | 用途 |
|---|---|---|
| `colorPrimary` | `phantom-primary` (#8ab4f8) | 主動作按鈕、Quick-tile 啟用態 |
| `colorOnPrimary` | `phantom-bg` (#0f0f1a) | primary 上的文字 |
| `colorSurface` | `phantom-card` (#1a1a2e) | card / sheet 背景 |
| `colorSurfaceVariant` | `phantom-border` (#2a2a3e) | divider / chip bg |
| `colorError` | `phantom-danger` (#dc3545) | Stop / denied icon |
| `colorTertiary` | `phantom-warning` (#ff9800) | recording 中色 |
| `Typography.bodyLarge` | type ramp `body-lg` 16sp / 500 | 主按鈕 label |
| `Typography.displayMedium` | `display` 48sp / 700 | 計時器 |
| ripple opacity | `overlay-ripple-24` (24%) | 按鈕 pressed |

**Dynamic Color (Material You)**：v0.6.0 預設**關**，preserve phantom brand consistency；Settings → "Use system color" toggle 開啟後 user 桌布抽色覆蓋 `colorPrimary` 等。Toggle 視覺：Material Switch 元件。

## Material Symbols Rounded（per Icon 對照矩陣，per hero mockup §56）

繼承 hero mockup icon 矩陣（12 functions × 3 sets），Android column 用 Material Symbols Rounded：
- PTT / mic — `mic`
- mic-off (denied / paused) — `mic_off`
- play — `play_arrow`
- pause — `pause`
- stop — `stop`
- folder (chunk count) — `folder`
- check (success) — `check_circle`
- settings — `settings`
- back — `arrow_back`（Material 慣例，不用 chevron）
- warning — `warning`
- wifi-off — `wifi_off`
- timer — `timer`

## Android 共用文案 keys（per hero mockup, +1 new）

繼承 hero mockup §75-128 全部 i18n keys。Android 新增：

| Key | zh-TW | en |
|---|---|---|
<!-- removed Android-specific no_speech key; use hero mockup共用 `focus.empty.no_speech` instead (per Stage 2 review consensus) -->

## 螢幕 A — Focus Idle（in Capture tab，per SPEC-34 IA）

版型同 iOS A（duration picker + PTT + Timer + trust badge）— per hero mockup §iOS A L138-156。Android delta（mockup-level）：

- **bottom nav**: 4 tabs（Home / Coach / **Capture** / Settings）— Focus 是 Capture tab 內的子 surface
- **nav bar 高 56dp**（iOS 44pt）
- **status bar**: Recording 中 tint `colorTertiary` (`phantom-warning`)
- **按鈕 press**: Material ripple `overlay-ripple-24` (NOT 8% bg lighten)
- **system bar handling**: 螢幕 edge-to-edge，content padding 避開 status bar + nav bar
- **TalkBack labels**:
  - PTT: "按住說話，錄音中可放開"
  - Timer: "開始 {min} 分鐘計時錄音"
  - Trust badge: "本地加密，本機 ASR，不上雲端"

## 螢幕 B1 / B2 — 權限提示（Permission Prompts）

由 OS（作業系統）渲染，不可自訂版面。phantom 只能設定 manifest（資訊清單）`permission_request_text`：
- `RECORD_AUDIO` Permission rationale: `focus.perm.denied_reassure`（per hero mockup）
- `POST_NOTIFICATIONS` rationale: "通知欄顯示錄音狀態，方便您快速停止"

## 螢幕 B' — Denied 卡（覆蓋 Idle，per hero mockup §iOS B'）

視覺等同 iOS B' 但：
- icon: `mic_off`（Material fill）48dp `colorError`
- 「打開設定」button: Material `FilledButton` 48dp 高 + ripple
- deep link target: Android `APPLICATION_DETAILS_SETTINGS`

## 螢幕 C — 錄音中（Recording，Material 3 版型）

版型同 iOS C — per hero mockup §iOS C L168-178。Android delta：
- waveform: 32 bars，每 bar 寬 4dp × gap 2dp，高 0-100dp dynamic，color `colorTertiary`
- 計時器 text style: `Typography.displayMedium` (48sp/700) bold `colorTertiary`
- Pause/Stop button: Material `OutlinedButton` (Pause) + `FilledTonalButton` (Stop, color `colorError`) × `pause`/`stop` Material icon
- chunk count: Material Chip element + `folder` icon

### B2 略過時的降級提示列（degraded UI bar，Android 獨有）

```
┌──────────────────────────────────┐  height 32dp
│ ⓘ 沒給通知權限也可錄...             │  bg colorSurface, body-sm muted
└──────────────────────────────────┘
```

`focus.android.notif_optional` i18n key，dismiss action: swipe right 或 tap × icon。

## 螢幕 D — 前景服務通知（FG-service Notification，取代 iOS 鎖定畫面）

Material 通知（notification）規格：

```
┌──────────────────────────────────┐
│ [phantom-mono-icon-24] Phantom Mesh
│ Focus · 05:23 / 25:00
│
│ ┌──────┐
│ │ STOP │  ← single action button, Material 規範: capital
│ └──────┘
└──────────────────────────────────┘
```

- **Channel**: `focus_session`，IMPORTANCE_LOW（無音效、無震動，常駐通知欄）
- **Style**: `NotificationCompat.Builder` + `setOngoing(true)`（不可滑掉）
- **smallIcon**: `R.drawable.ic_phantom_mono`（24dp, mono, alpha 100%）
- **color**: `colorPrimary`（影響 icon tint）
- **contentTitle**: i18n key `app.name`
- **contentText**: 動態，per FSM state
  - Recording: "Focus · {elapsed} / {total}"
  - Finalizing/Transcribing: `focus.finalizing.asr`
- **action**: `Stop`（label `focus.btn.stop`）

## 螢幕 C' — 中斷子狀態（Interrupted sub-state）

視覺等同 iOS C' — waveform（波形）凍結（顏色 `phantom-muted`）+ 中斷訊息。Android delta（差異）：通知欄文字同步切換為「電話中已暫停」（取 `focus.interrupted.phone`）。

## 螢幕 E — 收尾中（Finalizing）

版型同 iOS E — per hero mockup §iOS E L246-264。Android delta：
- spinner: Material `CircularProgressIndicator` 32dp `colorPrimary`
- progress bar: Material `LinearProgressIndicator` height 4dp
- **同時更新 FG-service notification** 內容到 `focus.finalizing.asr`（user 看通知欄即知進度）

## 螢幕 F — 完成（Done，Takeaway card 重點摘要卡）

版型同 iOS F — per hero mockup §iOS F L266-294。Android delta：
- card: Material `Card` (filled) elevation 1dp + radius 12dp
- success icon: `check_circle` 64dp `colorPrimary` （不用 `phantom-success` — 因為 Material 3 沒 success color tier，用 `colorPrimary` 統一）
- **新狀態：ASR 無語音（Empty 變體）**:
  ```
  本次時段未偵測到語音
  ─────────────
  已為您記錄時長：25 分鐘
  
  [重錄這次]  [完成]
  ```
  取 `focus.empty.no_speech`。`[重錄]` 跳回 A Idle 同 mode；`[完成]` 寫 events row + 切 History。

## Quick Settings tile（快速設定圖磚）視覺（per SPEC-34 §146 G5）

Android Quick Settings 規範：

| 狀態 | 圖示 | 標籤 | 副標題 |
|---|---|---|---|
| **未啟用（Inactive）** | `mic` outlined 24dp `colorOnSurface` | "Phantom 焦點" | "1 tap 啟 25min" |
| **啟用中（Active）**（25min 倒數中） | `mic` filled 24dp `colorPrimary` | "Phantom 焦點" | "{elapsed}/25:00"（每 1 min 更新） |
| **已暫停（Paused）** | `mic_off` 24dp `colorTertiary` | "Phantom 焦點" | "已暫停" |

圖磚點擊（Tile click）行為：
- Inactive（未啟用） → 啟動時段（broadcast 廣播 → MeshNodeService）
- Active（啟用中） → 開啟 App 到 Focus 啟用畫面（不直接停止，防誤觸，per SPEC-34）

## Glance widget（v0.6.0 ship per SPEC-34，主用 SPEC-22 chip）

**注意**：Glance widget 主要服務 SPEC-22 habit chip palette，不直接服務 SPEC-21 focus。本檔不列 widget 視覺（屬 SPEC-22 mockup 範圍），僅備註：focus session 啟動後 widget 顯示「focus 中」inactive state（chip tap 仍 work，視覺 muted）— per SPEC-22 mockup（如有）。

## MIUI 引導 dialog（per SPEC-34 §30(F)）

僅在 `is_miui=true` + service 啟動失敗時跳。視覺：

```
┌──────────────────────────────────┐
│ 小米手機需要額外授權              │  title 20sp / 500
│                                  │
│ Phantom Mesh 需要「自啟動」+      │  body 14sp / 400
│ 「省電白名單」才能在背景持續錄音。│
│                                  │
│ ┌──────┐ ┌──────┐ ┌──────┐    │
│ │自啟動│ │省電  │ │不再提示│   │  Material `TextButton` × 3
│ └──────┘ └──────┘ └──────┘    │
└──────────────────────────────────┘
```

「不再提示」勾選後 → DataStore `miui_guide_dont_show_again=true`，永不再跳。

## Cross-platform invariants 對齊（per hero mockup §555）

繼承全部 hero invariants（trust badge 文字 / Stop danger color / 計時器顏色 / PTT × Timer 互斥 / takeaway card 尺寸 / Notification body 截字 / etc）。Android 額外：

- **Material Card elevation 1dp**（不是 macOS shadow，Material 規範）
- **Material ripple 24% opacity** 全按鈕共用
- **TalkBack label 必填**（per SPEC-34 G7 + WCAG 2.2 AA）
- **Notification ongoing flag 必開**（防 user 誤滑掉錄音中通知）
- **MIUI dialog dismiss state 持久化 via DataStore**

## 6 大資料狀態 — Android Mockup 視覺對映

| 狀態 | 視覺 |
|---|---|
| 理想 | F Done card 完整 |
| **空白（History）** | History tab 內 mono SVG illustration 192dp + `focus.empty.history` + 「前往 Focus」button（Material `FilledButton`） |
| **空白（ASR 無語音）** | F 安撫文案 + 「重錄這次」/「完成」雙 button（新增變體，per Agy R1 catch） |
| 極限 | C chunk `99+` chip / F takeaway > 800 字截斷（per hero mockup invariant，本檔不重述）|
| 錯誤 | B' Denied 卡 `colorError` icon + open-settings CTA |
| 局部 | E `focus.partial.chunk_failed` inline，`colorTertiary` 文字 |
| 載入中 | E spinner + progress bar，同時通知欄文字更新 |

## 開放問題（mockup 層面）

1. **計時器 text color recording 中**：用 `colorTertiary` 是否跟 Material 3 「Tertiary 是 accent 色」慣例衝突？或改 `colorPrimary`？提案：`colorTertiary`（per hero mockup 統一 warning 系）
2. **Quick Settings tile 啟用態 vs inactive**：active 用 filled icon + primary color；vs Inactive 用 outlined icon + onSurface — 對比是否足夠？需 device 測。
3. **Glance widget focus 中是否變灰** vs 完全隱藏？提案：變灰但仍可點（user 切 mode 自由）。

## 下一步

→ 進 [Android Prototype（待補）] 鎖定每個 tap target 行為、Material ripple timing、TalkBack focus order、FG-service 通知互動 sequence。
