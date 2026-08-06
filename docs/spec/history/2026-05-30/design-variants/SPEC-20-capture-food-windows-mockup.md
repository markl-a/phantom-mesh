# SPEC-20 Capture Food — Windows Mockup（視覺稿）

> **Stage 2/3** · [線框稿（wireframe）](./SPEC-20-capture-food-windows-wireframe.md) → 視覺稿（mockup，高保真：配色 + icon + 終版文案 + a11y）→ [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — Fluent design token（設計變數）/ Lucide icon（圖示）/ Win 11 ActionCenter toast XML（通知中心快顯樣板）/ tray icon 配色終值 / 終版文案 / Narrator（朗讀器）AutomationName。沿用 SPEC-21 Windows mockup 的 token 速查與規範，不重抄；本檔鎖 food capture 特有視覺。
> **Spec**: [`SPEC-20-SYSTEM-capture-food`](../specs/v060-deep-spec/SPEC-20-SYSTEM-capture-food.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P2 多模態理解** `P2.food`；cross-cut P4 加密為先、P3 進化網 |
| **Source spec** | SPEC-20-SYSTEM-capture-food |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 2/3 mockup |

## 為什麼 Windows food capture 有獨立 mockup

wireframe 鎖了版型骨架，本檔鎖會影響實作的視覺終值：
1. **3 個 capture 來源磚的視覺 + 狀態**（enabled / hover / disabled-no-webcam）— wireframe 只給 label
2. **confidence（信心度）4 格點的配色語意** — 高 / 中 / 低信心三段色
3. **analyzing 樂觀骨架的動畫語意**（spinner + 不精確進度條 — 不可顯示假百分比）
4. **ActionCenter toast XML 樣板** — Meal-logged + analysis-failed 兩變體
5. **tray icon analyzing 配色**（瞬時 working，非 SPEC-21 的 long-running 錄音）
6. **trust badge 雲端揭露文案終值**（BYOM opt-in 法務語氣）
7. **Narrator AutomationName** — 每元件可朗讀字串

## Design token 對映（per SPEC-02，沿用 SPEC-21 mockup 速查）

| Token | Hex | food capture 用途 |
|---|---|---|
| `spectyn-bg` | `#0f0f1a` | Capture window 內容區背景（OS title bar 不染） |
| `spectyn-card` | `#1a1a2e` | 3 來源磚 / 結果卡 item row 背景 |
| `spectyn-primary` | `#8ab4f8` | 主按鈕（拍照 / 完成）+ toast action 鈕 |
| `spectyn-success` | `#81c995` | **analyzing tray icon（綠，working）** + 高信心 confidence 點 |
| `spectyn-warning` | `#ff9800` | 「看不準」低信心標記 + in-window analyzing spinner（非 tray） |
| `spectyn-danger` | `#f28b82` | FOOD_CAMERA_DISABLED / analysis-failed 卡 icon |
| `spectyn-muted` | `#6b6b80` | drag-drop hint、disabled 磚、confidence 空格點 |

### confidence 4 格點配色（食物特有，本檔拍板）

| 信心區間 | 實心點數 | 點配色 | 語意 |
|---|---|---|---|
| `>= 0.8` | ●●●● / ●●●○ | `spectyn-success #81c995` | 高信心，直接採用 |
| `0.5 - 0.8` | ●●○○ | `spectyn-primary #8ab4f8` | 中信心，建議檢視 |
| `< 0.5`（含 `unknown`） | ●○○○ | `spectyn-warning #ff9800` | 低信心，row 標「看不清楚，請手動補」（per SPEC-20 §3 G5 不 hallucinate 紀律） |

空格點一律 `spectyn-muted`。**不得**用紅色（`spectyn-danger`）表低信心 — 紅是 error 系，低信心不是錯誤。

## Lucide icon 對映（per SPEC-21 mockup §63 同源，food 特化）

| 角色 | Lucide icon | 用途 / 尺寸 |
|---|---|---|
| Webcam 來源 | `camera` | 來源磚 1，32×32 spectyn-primary |
| 檔案來源 | `folder-open` | 來源磚 2，32×32 |
| 剪貼簿來源 | `clipboard-paste` | 來源磚 3，32×32 |
| 拖放提示 | `image-plus` | drag-drop 區 hint，24×24 spectyn-muted |
| 分析中 | `loader`（旋轉） | analyzing spinner，20×20 spectyn-warning |
| 完成 | `check-circle` | result 卡 header + toast AppLogo overlay，24×24 |
| 低信心 / 警告 | `triangle-alert` | 低信心 item row + analysis-failed 卡，16×16 spectyn-warning |
| 加密 | `lock` | trust badge，14×14 |

Icon 全 bundled（`app/src/icons/lucide/*.svg`），不依賴系統 icon font。tray icon `.ico` 由 Lucide SVG 預 render 16×16 + 32×32 多 frame（DPI scaling per SPEC-43）。

## 文案 keys（per SPEC-05 i18n；繁中 + en 並列，+ 本檔新增）

| key | 繁中 | English |
|---|---|---|
| `food.window.title` | 記一餐 | Quick Log Meal |
| `food.btn.source_webcam` | 拍一張 | Take photo |
| `food.btn.source_file` | 選檔案 | Browse... |
| `food.btn.source_paste` | 貼上 (Ctrl+V) | Paste (Ctrl+V) |
| `food.hint.drag_drop` | ...或把圖片拖放到視窗任一處 | ...or drop an image anywhere here |
| `food.btn.shutter` | 拍照 | Capture |
| `food.status.analyzing` | 分析中... 估計食物與熱量 | Analyzing... estimating food + calories |
| `food.status.persisted_safe` | 已本地加密落地（你關掉也會記錄） | Saved encrypted locally (kept even if you close) |
| `food.result.total` | 合計 ~ {kcal} kcal | Total ~ {kcal} kcal |
| `food.result.low_confidence_row` | 看不清楚，請手動補 | Unclear — please correct |
| `food.result.analysis_failed` | 照片已安全記錄，但這次沒分析出食物 | Photo saved safely, but no food detected this time |
| `food.trust_badge` | 本地加密 · 雲端 vision 為 BYOM 選用 | Encrypted locally · cloud vision is BYOM opt-in |
| `food.tooltip.no_webcam` | 找不到攝影機 | No camera found |

## 螢幕 A — Capture Source Picker（真窗 520×360px）

```
+------------------------------------------------+
| Quick Log Meal               [_][o][X]         |   title bar：spectyn-bg 不染，OS 預設 chrome
+------------------------------------------------+
|  選擇餐點照片來源：                            |   text-secondary 14px
|                                                |
|  +-----------+  +-----------+  +-----------+    |   3 磚：spectyn-card bg, radius 8, 120x96
|  | (camera)  |  |(folder-o) |  | (clip)    |    |   icon 32px spectyn-primary 置中
|  |  拍一張   |  |  選檔案   |  | 貼上 ^V    |    |   label 13px text-primary
|  +-----------+  +-----------+  +-----------+    |
|                                                |
|  (image-plus) ...或把圖片拖放到視窗任一處      |   spectyn-muted 13px
|                                                |
|  (lock) 本地加密 · 雲端 vision 為 BYOM 選用     |   trust badge spectyn-muted 12px
+------------------------------------------------+
```

**磚狀態**：
- enabled：spectyn-card bg；hover → border 1px spectyn-primary + bg lighten 6%
- disabled（無 webcam）：opacity 0.4、icon/label spectyn-muted、tooltip `food.tooltip.no_webcam`、不可聚焦
- focus（Tab）：2px spectyn-primary outline（per SPEC-43 §14 focus ring）

**Narrator AutomationName**：
- 視窗：「記一餐視窗。選擇餐點照片來源。」
- 磚 1：「拍一張，使用攝影機。按鈕。」磚 2：「選檔案，瀏覽圖片。按鈕。」磚 3：「貼上剪貼簿圖片。按鈕。」
- 拖放區：「也可拖放圖片到此視窗。」

## 螢幕 B — Analyzing（樂觀骨架）

```
+------------------------------------------------+
| Quick Log Meal . Analyzing   [_][o][X]         |
+------------------------------------------------+
|        +------------------------+              |   已壓縮縮圖 720p（已加密落地）radius 8
|        |   (compressed thumb)   |              |
|        +------------------------+              |
|  (loader spin) 分析中... 估計食物與熱量        |   spinner spectyn-warning 20px + text 14px
|  [============------------]  約 8 秒           |   不精確進度（CSS 動畫掃動，非 % 數字）
|  (lock) 已本地加密落地（你關掉也會記錄）       |   spectyn-success 12px 安心文案
+------------------------------------------------+
```
- **進度條不可顯示假百分比** — 用 indeterminate 掃動動畫（Fluent ProgressBar indeterminate），因 LLM 回應時間不可預測；標「約 8 秒」是 budget 提示非保證
- 超 8 秒 budget → 切 optimistic：直接渲染 C 並標「分析較久，稍後更新」，不卡關窗

## 螢幕 C — Result Card（信心度視覺 + 修正）

```
+------------------------------------------------+
| (check-circle) Meal logged . 12:34  [_][o][X]  |
+------------------------------------------------+
|  +------+  雞胸肉沙拉                           |   item row：spectyn-card, radius 6
|  |thumb |  ~ 320 kcal          (****) ●●●●     |   confidence 高 → spectyn-success
|  +------+  糙米飯 1 碗                          |
|            ~ 220 kcal          (**--) ●●○○     |   中 → spectyn-primary
|            (triangle-alert) 看不清楚，請手動補  |   低/unknown → spectyn-warning row
|            -------------------                 |
|            合計 ~ 540 kcal                      |   total 16px bold
|                                                |
|  [ 修正清單 ]    [ 重分析 ]    [ 完成 ]         |   完成 = spectyn-primary 主鈕
|  (lock) 只有你的 identity.key 能解開這張照片    |   trust badge
+------------------------------------------------+
```
- 「修正清單」inline 編輯 → append correction event（不改原 row，P4 append-only）
- 「完成」→ 關窗、tray 回 idle、main window Food tab 列表頭插這筆

## 螢幕 D — ActionCenter toast（Win 11 XML 樣板）

Meal-logged toast（user 已關窗、analyze 完成時才發）：

```xml
<toast scenario="reminder" activationType="protocol"
       launch="spectyn-mesh://food/result?id={event_id}">
  <visual>
    <binding template="ToastGeneric">
      <text>已記錄一餐 ~ {kcal} kcal</text>
      <text>雞胸肉沙拉、糙米飯 . 點開看明細</text>
      <image placement="appLogoOverride" src="spectyn-tray-idle.png"/>
    </binding>
  </visual>
  <actions>
    <action content="開啟明細" arguments="spectyn-mesh://food/result?id={event_id}"/>
  </actions>
</toast>
```

analysis-failed 變體：`scenario="reminder"`、text 換 `food.result.analysis_failed`、action 換「手動輸入」→ `spectyn-mesh://food/correct?id={event_id}`。

- **toast 持續到 user dismiss**（per SPEC-43 — Win 用戶常 miss 一閃即逝通知）
- **不加 `<audio>`**（食物記錄非急事；避免打擾，對齊 P4「shame-free」操作原則）
- Focus Assist 折疊期間 → toast 退化為 in-app banner（per SPEC-43 §9.4 `R.windows.toast_emit_fail` 路徑）

## Tray icon state（food capture，瞬時）

| State | icon | 配色 | 說明 |
|---|---|---|---|
| Idle | `spectyn-tray-idle.ico` | spectyn-muted | 無 capture |
| **Analyzing** | `spectyn-tray-working.ico` | **spectyn-success 綠**（瞬時 ~8s） | per SPEC-43 §8.1 working semantics：food analyze 是**非 user-blocking 的瞬時背景 task**（不像 SPEC-21 focus 錄音是 user 主動長時 mic 採集，那才用橘 warning）。本檔與 wireframe 一致採綠，**不沿用 SPEC-21 橘**（兩者 task 性質不同）。 |
| Done | 回 idle（debounce 1s） | — | toast 已發 |
| Error | `spectyn-tray-error.ico` | spectyn-danger | camera disabled / 4-provider fail |

food capture 是**瞬時 task**（非 SPEC-21 錄音的 long-running session），所以 tray menu **不 rebuild**（不需把 Stop 提首項）。

## Cross-platform invariants 對齊

- confidence 點語意（高綠 / 中藍 / 低橘）跨 5 平台一致（iOS / Android / mac / Win / Linux 同 SPEC-02 token）
- 「event 在 analyze 前已落地」是 pipeline invariant（SPEC-20 §1），所有平台 toast/卡片都標 `food.status.persisted_safe`
- 雲端 vision = BYOM opt-in 的揭露文案跨平台同字（法務一致性）

## 已決（per wireframe + 本檔拍板）

- capture 來源 = webcam / file / paste / drag-drop 四路徑（wireframe）
- confidence 配色三段（本檔）；analyzing 用 indeterminate 進度（本檔）
- analyzing tray = spectyn-success 綠（本檔；per SPEC-43 §8.1 working=綠，food analyze 是非 blocking 瞬時背景 task，**不沿用** SPEC-21 focus 錄音的橘）
- toast 無 audio、持續到 dismiss（本檔）

## 開放問題（留 prototype / 後續）

1. webcam live preview 的 frame rate / 解析度上限 — 留 prototype 量測
2. 多餐合併（一張照片多盤）的 item 上限 UI — SPEC-20 §multi-item 待 mockup v0.2
3. correction event 的 undo 視窗 — 跨 SPEC-16 event append 語意，待 coach engine（SPEC-23）對齊

## 下一步

**Stage 3 prototype**：HTML / Tauri-component sketch — source picker + analyzing skeleton + result card 三態可切，confidence 點用真 CSS 渲染。
