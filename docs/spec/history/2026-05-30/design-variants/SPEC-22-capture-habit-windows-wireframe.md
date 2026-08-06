# SPEC-22 Capture Habit — Windows Wireframe（線框稿）

> **Stage 1/3** · 線框稿（wireframe，低保真版型骨架）→ [視覺稿（mockup，待補）] → [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only。SPEC-22 hero 是行動端 home-screen widget（主畫面小工具，iOS WidgetKit / Android Glance）；**Windows 桌面沒有 home-screen widget**，所以快速打卡主面改成 **System Tray（系統匣）dropdown chip palette（下拉標籤盤）**。本檔描述桌面的 chip 快速 log（紀錄）+ streak（連續天數）熱力圖 + chip palette 管理。
> **Spec**: [`SPEC-22-SYSTEM-capture-habit`](../specs/v060-deep-spec/SPEC-22-SYSTEM-capture-habit.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P2 多模態理解** — text + behavior 子模態（「Image + audio + text + behavior context — all in」BIG-GOAL §P2 line 62）；cross-cut P3 進化網（coach 週 review 讀 habit）、P4 加密為先（metadata 走 SPEC-13 age） |
| **Source spec** | SPEC-22-SYSTEM-capture-habit |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 1/3 wireframe |

## 為什麼 Windows habit capture 跟行動端結構不同

SPEC-22 §1 hero 是「home-screen widget 上 6–12 個 chip、≤ 3 tap 完成 log」。Windows 桌面有 3 個差異：
1. **沒有 home-screen widget** — Win 11 Widgets board v0.6.0 OoS（範圍外，同 SPEC-43 §3.3）。桌面最快 surface 是 **System Tray dropdown**：右下角圖示右鍵 → chip 格子 → 點一下 = log。
2. **chip grid 面 = 輕量 custom popup（自繪浮層），不是 Win32 原生 context menu** — 原生 `TrackPopupMenu` 點擊即關，無法「連點多 chip」；故用 frameless（無邊框）always-on-top 小視窗 anchor 到 tray icon，點擊不關、失焦或 Esc 才關。視覺做成 menu 樣但行為是 popup window。
3. **鍵盤 + 滑鼠並用** — chip 可數字鍵快選（`1`–`9` 對前 9 個 chip）；行動端純 tap。

→ Windows 快速打卡 = tray chip grid；管理 + streak = main window Habit tab（分頁）。

## 縮寫對照表

> - **chip（標籤）**：一個可點的習慣按鈕（水 / 咖啡 / 運動 ...），點一下記一筆
> - **chip palette（標籤盤）**：使用者自訂的 chip 集合，存 `chip_palette` sqlite 表
> - **streak（連續天數）**：連續 N 天每天至少 1 筆同 chip event 的天數
> - **heatmap（熱力圖）**：用色塊深淺表每天打卡量的網格
> - **System Tray（系統匣）**：Windows 右下角通知區
> - **qty stepper（數量微調器）**：+/- 調整數量的小控件（如水 250ml）
> - **FTS5 / NLP / WidgetKit / Glance**：本檔未用，見 SPEC-22 §1

## 入口點（per SPEC-43 §8 + §10）

| 進入點 | v0.6.0 | v0.7+ | 說明 |
|---|---|---|---|
| **System tray right-click → chip grid** | ✅ | ✅ | 桌面最快打卡面（取代行動端 widget）；點 chip 即 log |
| Main window `[Life / Habit tab]` | ✅ | ✅ | chip 管理 + streak heatmap + free-text + 30 天統計 |
| CLI `spectyn habit "water 250"` / `spectyn habit streak` | ✅ | ✅ | per SPEC-22 §1 (c)；終端機 user 最快 |
| `Win+Shift+H` global hotkey → tray chip grid 彈出 | ❌ | ✅ | 預設 OFF（避撞 enterprise app，同 SPEC-21 §8.5 決策）；Settings → Hotkeys 開 |
| Deep-link `spectyn-mesh://habit/log?chip={id}` | ✅ | ✅ | 跨機 / 外部觸發單一 chip |
| Win 11 Widgets board | ❌ | ❌ | OoS（SPEC-43 §3.3） |

**v0.6.0 ship 3 個**：tray chip grid + main window Habit tab + CLI。global hotkey 預設 OFF。

> **Deep-link 白名單註**：`habit/` path prefix **待加入 SPEC-43 §12.1 deep-link host 白名單**（目前只放行 `coach/` `cluster/` `settings/`）— 同 SPEC-20/21 capture prefix 的系統性缺口。

## 螢幕 A — System Tray chip grid（tray-anchored custom popup，非原生 menu）

右鍵 tray icon → dropdown 頂部塞 chip grid（3×4 = 12 chip）：

```
+-------------------------------------------+
| Spectyn Mesh . Habit                      |  ← header（灰、不可點）
+-------------------------------------------+
|  [1 水]   [2 咖啡] [3 運動] [4 冥想]      |  ← chip grid 3 列 x 4，數字鍵快選
|  [5 讀書] [6 走路] [7 戒菸] [8 戒酒]      |
|  [9 深呼吸][伸展] [寫日記] [早睡]         |
+-------------------------------------------+
|  free-text: [____________________] Enter  |  ← 自由打字 fallback（取 habit.input.free_text）
+-------------------------------------------+
|  Open Habit tab...            Ctrl+Shift+H |
|  Open Spectyn Mesh             Ctrl+O      |
+-------------------------------------------+
```

**Windows delta / 設計重點**：
- 點任一 chip → 立刻寫一筆 `EventKind::Habit`、chip 閃一下 confirm（取 `habit.feedback.logged`）、dropdown **不自動關**（連點多 chip：水 + 咖啡 + 冥想一次記完）
- **需數量的 chip（水 250ml）**：點 chip → inline 展開 qty stepper（`[- 250 +] ml`）→ Enter 確認；不需數量的（運動 / 冥想）直接記 1
- **數字鍵 `1`–`9`** 對前 9 chip 快選（鍵盤 user 不用滑鼠）
- chip grid 由 `chip_palette` 表動態 render，順序 = user 拖拉序（管理在 main window）
- dropdown 是**輕量 custom popup window**（frameless always-on-top，非 Win32 原生 context menu — 原生 menu 點擊即關、塞不了會連點的 grid）；anchor 到 tray icon、失焦/Esc 才關

## 螢幕 B — Main window Habit tab（管理 + streak）

```
+--------------------------------------------------+
| Habit                                            |  ← tab header
+--------------------------------------------------+
|  今日: 水 x3 . 咖啡 x1 . 冥想 x1                 |  ← 今日 summary
|                                                  |
|  水         streak 12 天  [##  ## # ##]  30d:48  |  ← 每 chip 一列：streak + 7d heatmap + 30d count
|  冥想       streak  5 天  [# #  #  ## ]  30d:18  |
|  運動       streak  0 天  [    #   #   ]  30d: 9  |
|  ...                                             |
|                                                  |
|  [+ 新增 chip]   [管理 palette]   [打卡...]      |  ← 管理入口
+--------------------------------------------------+
```

- **7d heatmap**：7 個方塊（週一→週日），色深表當天打卡量（0=空 / 1=淺 / >=3=深）；ASCII 線框用 `#` 密度示意
- **streak 數字**：連續天數（SPEC-22 lenient 演算法：任一天 >=1 event 即算）
- **30d count**：30 天總筆數
- 「打卡...」→ 開 A 同款 chip grid（真窗版，給沒裝 tray 習慣的 user）
- sqlite 結構化 query 即時算（< 10ms，per SPEC-22 §1）

## 螢幕 C — Free-text 快速 log

tray 或 main window 的 free-text 框：
- 打字「跑步 30 分」→ Enter → 寫 `EventKind::Habit`，`free_text="跑步 30 分"`、`summary` 同字供 FTS5 搜尋
- **v0.6.0 不解析數量/單位 NLP**（SPEC-22 OoS2）— 先存 raw text，coach（SPEC-23）日後用 LLM 解析
- 若 free_text 開頭 match 某 chip label（如「水 250」）→ 提示「要記到『水』chip 嗎？」(可 Enter 確認綁定 chip_id)

## 螢幕 D — Chip palette 管理（真窗）

```
+--------------------------------------------------+
| 管理習慣 chip                       [_][o][X]    |
+--------------------------------------------------+
|  拖拉重排 . 點 x 刪除 . [+ 新增]                 |
|  [::] 水         [x]                             |  ← [::] = 拖拉把手
|  [::] 咖啡       [x]                             |
|  [::] 運動       [x]                             |
|  ...                                             |
|  [+ 新增 chip]  名稱:[______] 需數量? [ ] 單位:[__]|
+--------------------------------------------------+
```
- 拖拉重排 → 更新 `chip_palette` 表 order 欄；刪除 → soft-delete（保留歷史 event）
- 新增：名稱 + 是否需數量 + 單位（如 ml / 分 / 杯）
- **v0.6.0 不跨裝置同步 chip_palette**（SPEC-22 OoS1 — broker vault 只同步 SPEC-15 vault_items）；每台機獨立 palette

## 失敗 / 邊界（per SPEC-04 error catalog）

- chip_palette 表讀失敗 → tray grid 退化為「只有 free-text 框 + 預設 12 chip hardcode fallback」
- event 寫入失敗（磁碟滿）→ chip 閃紅 + toast；不靜默吞
- 同一秒連點同 chip 多下 → 各記一筆（lenient 設計，不 debounce — 喝兩口水算兩筆合理）

## 待補（下一 pipeline stage）

- **Stage 2 mockup**：chip grid 配色（design token SPEC-02）、heatmap 色階終值、qty stepper 視覺、終版文案（i18n SPEC-05）、Narrator a11y
- **Stage 3 prototype**：tray chip grid + heatmap + palette 編輯 互動腳本 + HTML 草圖
- **hero wireframe（SPEC-22-capture-habit-wireframe.md）尚未存在** — 之後補行動端 hero 可改本檔為 deltas
