# SPEC-20 Capture Food — Windows Wireframe（線框稿）

> **Stage 1/3** · 線框稿（wireframe，低保真版型骨架）→ [視覺稿（mockup，待補）] → [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only。本檔描述「拍餐點 → AI 抽食物 + 估熱量」在 **Windows 桌面**的版型與流程。SPEC-20 hero 平台是行動端（iOS / Android 相機），**桌面沒有手機相機**，所以 Windows 不是「列 deltas」而是**獨立 capture 來源模型**（見下）。
> **Spec**: [`SPEC-20-SYSTEM-capture-food`](../specs/v060-deep-spec/SPEC-20-SYSTEM-capture-food.md) · [`SPEC-42-PLATFORM-Windows-foundations`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43-PLATFORM-Windows-screens-flows`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)
> **這份的工作範圍**：Windows-specific capture 來源（webcam / 檔案 / 剪貼簿）、真窗（real window，非 transient popover）版型、system tray（系統匣）「Quick Log Meal」入口、analyzing 樂觀 UI、結果卡片 + 修正。共用加密 / storage / vision fallback 邏輯與 hero 同（SPEC-20 §1，Rust core 處理），本檔不重抄。

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar（支柱）** | **P2 多模態理解（multimodal understanding）** — `P2.food` capability（SPEC-01 §8.2）；cross-cut **P4 加密為先（encryption-first）**（image + 分析走 SPEC-13 age 加密）+ **P3 進化網（evolve mesh）**（coach 隔日讀 `food.meal` event） |
| **Source spec** | SPEC-20-SYSTEM-capture-food |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 1/3 wireframe |

## 為什麼 Windows food capture 跟行動端結構不同

SPEC-20 §1 的 hero 流程是「手機相機 → `PHPickerViewController`（iOS 圖片選擇器）/ `ActivityResultLauncher`（Android 拍照啟動器）→ 720p 壓縮 → `food_capture(blob)`」。Windows 桌面有 4 個結構差異：

1. **沒有手機後鏡頭** — 桌機要嘛接 **webcam（網路攝影機）**，要嘛使用者**已經有一張餐點照片檔**。capture 來源從「單一相機」變成「**多來源選擇器**」。
2. **沒有 transient-popover shell affordance（暫態浮層外殼能力）** — Start sheet 必須是 **real window（真窗）**（per SPEC-43，同 SPEC-21 Windows wireframe §螢幕 A 結論）。
3. **剪貼簿是一等公民** — Windows 使用者常 `Win+Shift+S`（內建截圖）或從瀏覽器複製圖片；`Ctrl+V` 貼上應直接成為 capture 來源（行動端沒這習慣）。
4. **system tray 在右下角**（macOS 是右上 menu bar）— 右鍵語意 + 視覺位置都不同；「Quick Log Meal」掛 tray dropdown。

→ 因此 Windows food capture 用**獨立 capture-source frame**，不沿用行動端「快門即拍」單步模型。

## 縮寫對照表

> - **webcam（網路攝影機）**：桌機外接 / 內建鏡頭，透過 Windows `MediaCapture` API 取單張 frame
> - **real window（真窗）**：有標題列 + 最小/最大/關閉三鈕的 OS 視窗，非暫態浮層
> - **system tray（系統匣）**：Windows 右下角通知區圖示
> - **optimistic UI（樂觀介面）**：LLM 還沒回前先顯示「分析中」骨架，不卡住使用者
> - **BYOM（Bring Your Own Model，自帶模型金鑰）**：雲端 vision 用使用者自己的 API key，預設不啟用
> - **vision LLM（視覺大型語言模型）**：能看圖的 LLM；SPEC-20 fallback chain = Claude 3.7 Sonnet → GPT-4o → Gemini 2.5 Pro → Haiku 3.5
> - **ASR / EXIF / FSM**：本檔未用，見 SPEC-21

## 入口點（per SPEC-43 §8 + §10）

| 進入點 | v0.6.0 | v0.7+ | 說明 |
|---|---|---|---|
| Main window `[Life / Food tab]` sidebar | ✅ | ✅ | canonical surface — 最可靠入口 |
| **System tray right-click → "Quick Log Meal..."** | ✅ | ✅ | tray dropdown capability 項（per SPEC-43 §8.2，與 SPEC-21 "Start Focus..." 同層） |
| `Ctrl+V` 貼上圖片（main window 有焦點時） | ✅ | ✅ | 剪貼簿截圖 / 複製圖直接成 capture 來源 |
| Deep-link `phantom-mesh://food/capture` | ✅ | ✅ | 跨機 dispatch / 外部觸發 |
| `Win+Shift+M` global hotkey（使用者 opt-in） | ❌ | ✅ | 預設 OFF（避撞 enterprise app，同 SPEC-21 §8.5 決策）；Settings → Hotkeys 手動開 |
| 拖放圖片到主視窗 | ✅ | ✅ | drag-drop 任一處主視窗 → 進 capture 來源 = 該檔 |

**v0.6.0 ship 3 個**：main window Food tab + tray「Quick Log Meal...」+ `Ctrl+V` 貼上。global hotkey 預設 OFF。

> **Deep-link 白名單註**：`food/` path prefix **待加入 SPEC-43 §12.1 deep-link host 白名單**（目前 STRIDE anti-spoofing 只放行 `coach/` `cluster/` `settings/`）。同 SPEC-21 `focus/` 的情況 — 屬 SPEC-43 §12.1 系統性缺口，需補 `food/` `focus/` `habit/` 三個 capture prefix。

## 螢幕 A — Capture Source Picker（真窗，非 popover）

```
┌────────────────────────────────────────────────┐
│ Quick Log Meal                       [_][o][X]  │  ← OS chrome 標題列 + 三鈕（per SPEC-43 §10.6）
├────────────────────────────────────────────────┤
│                                                │
│   選擇餐點照片來源：                            │
│                                                │
│   ┌────────────┐  ┌────────────┐  ┌──────────┐ │
│   │  [camera]  │  │  [folder]  │  │ [clip]   │ │
│   │  Webcam    │  │  選檔案    │  │ 貼上     │ │  ← 3 來源（取 food.btn.source_{webcam,file,paste}）
│   │  拍一張    │  │  Browse... │  │ Ctrl+V   │ │
│   └────────────┘  └────────────┘  └──────────┘ │
│                                                │
│   ...或把圖片拖放到這個視窗任一處              │  ← drag-drop hint（取 food.hint.drag_drop）
│                                                │
│   [LOCK] 本地加密 · 雲端 vision 為 BYOM 選用    │  ← trust badge（取 food.trust_badge）
└────────────────────────────────────────────────┘
```

**Windows delta / 設計重點**：
- **真窗 520×360px、置中** — 不是 sheet/popover（Windows 無暫態浮層 affordance）
- **3 個來源磚** + drag-drop 區（4 種 capture 路徑）；webcam 無裝置時該磚灰階 disabled + tooltip `food.tooltip.no_webcam`
- **不做 Mica acrylic 半透明**（同 SPEC-43 §3.2 NG — Tauri 2 Mica binding 未穩，留 v0.7+）
- **trust badge 明示雲端揭露**：vision 上雲是 BYOM opt-in（per SPEC-20 §1 隱私段），預設只本地加密落地、不自動上雲分析
- **Tab 鍵走遍 3 磚 + Browse + Cancel**（per SPEC-43 §14 keyboard-first）
- **Escape = 取消關窗**；**Ctrl+V 任何時候**直接吃剪貼簿圖 → 跳 analyzing

### A' Webcam 子狀態（選 Webcam 後覆蓋）

```
┌────────────────────────────────────────────────┐
│ Quick Log Meal · Webcam              [_][o][X]  │
├────────────────────────────────────────────────┤
│        ┌──────────────────────────┐            │
│        │   (live webcam preview)   │            │  ← MediaCapture live frame
│        └──────────────────────────┘            │
│              ┌──────────────┐                  │
│              │  [O] 拍照     │                  │  ← capture frame（取 food.btn.shutter）
│              └──────────────┘                  │
│   [< 換來源]                                   │
└────────────────────────────────────────────────┘
```
- webcam 權限：Win 10/11 Settings → Privacy → Camera 預設允許桌面 app；關閉時 capture fail 拋 `FOOD_CAMERA_DISABLED`（**建議新增至 SPEC-20 §11.1**；目前 spec 只有 `FOOD_ANALYSIS_FAILED` / `FOOD_BLOB_TOO_LARGE` / `FOOD_DECRYPT_FAILED`，webcam 是桌面特有來源故需補此碼）→ 覆蓋遮罩卡 + 「打開設定」深連結 `ms-settings:privacy-webcam` + 「重試」

## 螢幕 B — Analyzing（樂觀 UI，8 秒 budget）

拿到 image bytes 後立即進 B（不等使用者）：客戶端壓 720p/80%（< 200ms）→ 加密落地 → vision LLM 分析。

```
┌────────────────────────────────────────────────┐
│ Quick Log Meal · Analyzing           [_][o][X]  │
├────────────────────────────────────────────────┤
│        ┌──────────────────────────┐            │
│        │   (compressed thumbnail)  │            │  ← 已壓縮縮圖（已加密落地）
│        └──────────────────────────┘            │
│                                                │
│   [spinner] 分析中... 估計食物與熱量            │  ← analyzing skeleton（取 food.status.analyzing）
│   [====------] ~8s                             │  ← 進度示意（非精確）
│                                                │
│   [LOCK] 已本地加密落地（你關掉也會記錄）       │  ← 安心文案（取 food.status.persisted_safe）
└────────────────────────────────────────────────┘
```

**設計重點**：
- **event 在分析前就已寫入**（per SPEC-20 §1 + G6）— 即使 LLM 超時 / 全 fail，`food.meal` row 已落地（`status="analysis_failed"`、`items=[]`），使用者可事後補
- **8 秒 analyze budget**（SPEC-20 §3 G1 30 秒總預算的子段）— 超時切 optimistic：直接進 C 顯示「分析較久，稍後更新」狀態，不阻塞關窗
- fallback chain 對使用者**不可見**（Rust core 內部跑 Claude→GPT→Gemini→Haiku）；只在 4 全 fail 時於 C 顯示 `FOOD_ANALYSIS_FAILED` 文案

## 螢幕 C — Result Card（食物清單 + 熱量 + 修正）

```
┌────────────────────────────────────────────────┐
│ Meal logged · 12:34                  [_][o][X]  │
├────────────────────────────────────────────────┤
│   ┌────────┐   雞胸肉沙拉                       │
│   │ (thumb)│   ~ 320 kcal       [confidence ●●●○]│  ← item row（name / est_calories / confidence）
│   └────────┘   糙米飯 1 碗                       │
│               ~ 220 kcal        [confidence ●●○○]│
│               ───────────────                   │
│               合計 ~ 540 kcal                    │  ← total（取 food.result.total）
│                                                │
│   [ 修正清單 ]   [ 看不準？重分析 ]   [ 完成 ]   │  ← correct / re-analyze / done
│                                                │
│   [LOCK] 只有你的 identity.key 能解開這張照片    │  ← trust badge 重申
└────────────────────────────────────────────────┘
```

**設計重點**：
- **每項顯示 confidence（信心度）4 格點**；任一項 `name="unknown"` + `confidence<0.5` 時該 row 標「看不清楚，請手動補」（per SPEC-20 §3 G5 不 hallucinate 紀律）
- **「修正清單」** → inline 編輯 item name / kcal → append 一筆 correction event（不改原 row，符 P4 append-only）
- **「完成」** 關窗、tray icon 回 idle、main window Food tab 列表頭插這筆
- 30 秒總預算（G1）：A 拍/選（即時）+ B 壓縮加密（<1s）+ analyze（≤8s p50）+ C 渲染 → p50 遠在 30s 內

### C' Analysis-failed 變體

4 provider 全 fail（或無 BYOM key 且未開雲端）：
- 卡片顯示 `food.result.analysis_failed`：「照片已安全記錄，但這次沒分析出食物」
- 「手動輸入」按鈕 → 直接進修正清單空白 row
- 「重分析」→ 重跑 fallback chain（image 已加密落地，不用重拍）

## Windows 獨有：Tray context menu（capture 期間）

```
┌────────────────────────────────────────────────┐
│ Phantom Mesh · Life                            │  ← header（灰、不可點；per SPEC-43 §8.2 item 1）
├────────────────────────────────────────────────┤
│ [camera] Quick Log Meal...          Ctrl+Shift+M│  ← capability 項（global hotkey 影子，v0.7+ 才註冊）
│ [focus]  Start Focus...                         │  ← SPEC-21 sibling（並列）
├────────────────────────────────────────────────┤
│ Open Phantom Mesh                   Ctrl+O      │
│ Settings...                                     │
└────────────────────────────────────────────────┘
```

- analyzing 期間 tray icon 切 `phantom-tray-working.ico`（綠點，per SPEC-43 §8.1 working semantics）；done 後 debounce 1 秒回 idle
- 與 SPEC-21 focus 共用同一 tray dropdown — capture capabilities 並列，互不灰化（食物 capture 是瞬時、非 long-running session，不像 focus 錄音會 rebuild menu）

## 待補（下一 pipeline stage）

- **Stage 2 mockup**：實際配色（design tokens SPEC-02）、文案最終版（i18n key SPEC-05）、confidence 點視覺、error 卡視覺
- **Stage 3 prototype**：HTML/Tauri-component sketch — capture source picker + analyzing skeleton + result card 三態切換
- **hero wireframe（SPEC-20-capture-food-wireframe.md）尚未存在** — 若之後補 iOS/Android hero，本檔可改為只列 Windows deltas（同 SPEC-21 模式）
