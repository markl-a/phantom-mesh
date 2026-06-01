# SPEC-22 Capture Habit — Windows Mockup（視覺稿）

> **Stage 2/3** · [線框稿（wireframe）](./SPEC-22-capture-habit-windows-wireframe.md) → 視覺稿（mockup，配色 + icon + 終版文案 + a11y）→ [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — chip（標籤）grid 配色 + heatmap（熱力圖）色階 + qty stepper（數量微調器）視覺 + streak（連續天數）數字樣式 + 終版文案 + Narrator（朗讀器）AutomationName。沿用 SPEC-20/21 mockup 的 design token（設計變數）速查，不重抄。
> **Spec**: [`SPEC-22-SYSTEM-capture-habit`](../specs/v060-deep-spec/SPEC-22-SYSTEM-capture-habit.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P2 多模態理解**（text + behavior）；cross-cut P3 進化網、P4 加密為先 |
| **Source spec** | SPEC-22-SYSTEM-capture-habit |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 2/3 mockup |

## 為什麼 habit capture 有獨立 Windows mockup

wireframe 鎖了版型；本檔鎖影響實作的視覺終值：
1. **chip grid 配色 + log 後 confirm 動效**（閃一下 = 什麼顏色、多久）
2. **streak heatmap 色階終值**（0 / 1 / 2 / >=3 筆的 4 段色）
3. **qty stepper 視覺**（需數量 chip 展開的 +/- 控件）
4. **streak 數字情緒色**（連續天數越長越「熱」— 但不羞辱斷掉的）
5. **終版文案**（i18n key SPEC-05）+ **Narrator AutomationName**

## Design token 對映（per SPEC-02，沿用 SPEC-20/21 mockup 速查）

| Token | Hex | habit 用途 |
|---|---|---|
| `phantom-bg` | `#0f0f1a` | tray dropdown / Habit tab 背景 |
| `phantom-card` | `#1a1a2e` | chip 按鈕 bg、heatmap 空格 |
| `phantom-primary` | `#8ab4f8` | chip hover border、qty stepper +/- 鈕 |
| `phantom-success` | `#81c995` | log 後 chip confirm 閃光、heatmap 高密度格 |
| `phantom-muted` | `#6b6b80` | streak 0 天數字、heatmap 空格邊、free-text placeholder |

### heatmap 4 段色階（本檔拍板）

| 當天筆數 | 色 | 說明 |
|---|---|---|
| 0 | `phantom-card #1a1a2e`（空格，僅描邊 phantom-muted） | 沒打卡 |
| 1 | `phantom-success @ 35% opacity` | 有打卡 |
| 2 | `phantom-success @ 65%` | 打 2 筆 |
| >= 3 | `phantom-success @ 100% #81c995` | 高密度 |

→ 全綠色階（非紅→綠 diverging）— habit 是「做了就好」，**不存在「壞」的一天**（對齊 P4 shame-free 操作原則）。斷 streak 不染紅。

### streak 數字情緒（不羞辱斷線）

| streak | 數字色 | 附飾 |
|---|---|---|
| 0 天 | `phantom-muted`（灰） | 無（不畫紅 / 不畫破碎 icon — 斷了不羞辱） |
| 1–6 天 | `phantom-primary`（藍） | — |
| >= 7 天 | `phantom-success`（綠）+ Lucide `flame` 16px | 「火苗」獎勵連續 |

## Lucide icon 對映（per SPEC-20/21 mockup 同源）

| 角色 | Lucide icon | 用途 |
|---|---|---|
| 拖拉把手 | `grip-vertical` | palette 編輯重排，16px phantom-muted |
| 刪除 chip | `x` | palette 編輯，14px |
| 新增 chip | `plus` | 16px phantom-primary |
| streak 火苗 | `flame` | >=7 天獎勵，16px phantom-success |
| qty 減 / 加 | `minus` / `plus` | qty stepper，14px |
| 打卡 confirm | `check` | log 後 chip 上閃一下，16px phantom-success |

chip 本身**不配 icon**（純文字 label + 數字鍵角標）— 避免 12 個 emoji 在 CP950 locale render 不一致；user 自訂 chip 也不強迫選 icon。

## 文案 keys（per SPEC-05 i18n）

| key | 繁中 | English |
|---|---|---|
| `habit.tray.header` | Phantom Mesh · Habit | Phantom Mesh · Habit |
| `habit.feedback.logged` | 已記 {chip} {qty}{unit} | Logged {chip} {qty}{unit} |
| `habit.input.free_text` | 打字記錄（例：跑步 30 分） | Type to log (e.g. run 30 min) |
| `habit.streak.label` | streak {n} 天 | {n}-day streak |
| `habit.streak.zero` | 還沒開始 | not started |
| `habit.tab.today_summary` | 今日：{summary} | Today: {summary} |
| `habit.palette.add` | 新增 chip | Add chip |
| `habit.palette.needs_qty` | 需要數量 | Needs a quantity |
| `habit.confirm_bind_chip` | 要記到「{chip}」嗎？ | Log this under "{chip}"? |

## 螢幕 A — Tray chip grid（tray-anchored frameless custom popup，非原生 menu）

```
+-------------------------------------------+
| Phantom Mesh . Habit                      |   header phantom-muted 12px 不可點
+-------------------------------------------+
|  [1 水]  [2 咖啡] [3 運動] [4 冥想]       |   chip：phantom-card bg, radius 6, 數字角標 phantom-muted
|  [5 讀書][6 走路] [7 戒菸] [8 戒酒]       |   hover → border phantom-primary
|  [9深呼吸][伸展] [寫日記] [早睡]          |   click → 閃 phantom-success check 200ms
+-------------------------------------------+
|  打字記錄: [____________________]  Enter  |   placeholder phantom-muted 13px
+-------------------------------------------+
|  Open Habit tab...           Ctrl+Shift+H |
+-------------------------------------------+
```
- click chip → 寫 event → chip 上疊 `check` icon phantom-success 閃 200ms（取 `habit.feedback.logged`）→ dropdown **不關**（連點）
- 需數量 chip（水）→ click 後 inline 展開：`[- 250 +] ml  [OK]`（qty stepper，phantom-primary +/- 鈕）
- 數字鍵 `1`–`9` = 快選前 9 chip（角標數字提示）

**Narrator AutomationName**：
- chip：「{label}，習慣標籤，按 {n} 鍵快選。已連續 {streak} 天。按鈕。」
- log 後：`aria-live="polite"` 朗讀「已記 {chip}」

## 螢幕 B — Habit tab streak heatmap

```
+--------------------------------------------------+
| Habit                                            |
+--------------------------------------------------+
|  今日：水 x3 . 咖啡 x1 . 冥想 x1                 |   summary 14px
|                                                  |
|  水      (flame) streak 12   [.:#:.#:]  30d 48   |   >=7 天 → flame + 綠數字
|  冥想            streak  5    [#.:..#.]  30d 18   |   1-6 天 → 藍數字
|  運動            還沒開始     [.......]  30d  9   |   0 天 → 灰「還沒開始」(不染紅)
|                                                  |
|  [+ 新增 chip]  [管理 palette]  [打卡...]        |
+--------------------------------------------------+
```
- heatmap 7 格（週一→週日）4 段綠色階；`.` = 空格、`:` = 淺、`#` = 滿（ASCII 示意，實作用 opacity）
- streak >=7 顯 `flame` icon；=0 顯灰字「還沒開始」（`habit.streak.zero`）不羞辱

## 螢幕 D — qty stepper + palette 編輯視覺

qty stepper（chip 需數量時 inline）：
```
[ (minus) ]  250  [ (plus) ]  ml   [OK]
```
- `minus`/`plus` 鈕 phantom-primary、長按連續 +/-（step = chip 定義，預設 50）
- 數字框可直接打字；`ml` 單位 phantom-muted 不可改（chip 定義時鎖）

palette 編輯列：`(grip-vertical) {label}  (x)` — 拖拉重排、x soft-delete（保留歷史 event）

## ActionCenter toast（habit 通常不發 toast）

habit log 是**高頻瞬時**動作 — **預設不發 ActionCenter toast**（會洗版、違反 P4 shame-free 的「不打擾」）。例外：
- streak 達里程碑（7 / 30 / 100 天）→ 發一次慶祝 toast（`scenario="reminder"`，無音效）：「水 連續 30 天！」
- 其餘 log 只靠 tray chip 閃光 + Habit tab 即時更新，不 toast

## Cross-platform invariants 對齊

- heatmap 全綠色階（不 diverging、不染紅）跨 5 平台一致 — habit 無「壞的一天」
- streak lenient 演算法（任一天 >=1 event 即算）跨平台同（SPEC-22 §1）
- chip_palette 不跨裝置同步（SPEC-22 OoS1）— 各平台獨立 palette，視覺一致但資料不同步

## 已決（per wireframe + 本檔拍板）

- heatmap 4 段綠色階（本檔）；streak 情緒色三段 + flame >=7（本檔）
- chip 無 icon 純文字（避 CP950 emoji 不一致，本檔）
- habit log 預設不 toast，只里程碑慶祝（本檔）

## 開放問題（留 prototype / 後續）

1. qty stepper 長按加速曲線 — 留 prototype 調手感
2. >12 chip 時 tray grid 是否滾動 / 分頁 — 待 palette 上限決策
3. 里程碑 toast 的 streak 門檻是否 user 可調 — 待 Settings 設計

## 下一步

**Stage 3 prototype**：tray chip grid 連點 + qty stepper + heatmap 互動腳本 + HTML 草圖。
