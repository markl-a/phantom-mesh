# SPEC-21 Capture Focus — Web Mockup（視覺稿）

> **Stage 2/3** · [線框稿（Web）](./SPEC-21-capture-focus-web-wireframe.md) → 視覺稿 → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Web / mobile-web only — Lucide icons / 兩斷點版型 / caveat banner / C' upload-failed sub-state 視覺規格。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-mockup.md](./SPEC-21-capture-focus-mockup.md) §iOS hero + §Web section L473-543），本檔擴展 §Web 為完整視覺稿，**不重抄 hero**。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md) · [`SPEC-17-PROTOCOL-tauri-bridge`](../specs/v060-deep-spec/SPEC-17-PROTOCOL-tauri-bridge.md)（C' upload queue）

## 為什麼 Web 有獨立 mockup

hero mockup §Web（L473-543）只列「跟 iOS deltas + breakpoint 切換 + A2 ASCII + C' ASCII」~70 行 high-level。實際 Web 視覺需鎖：
1. **兩個 layout 視覺值對映**（mobile-web A1 / desktop-web A2）— Tailwind responsive classes 各自 token
2. **Caveat banner 兩段顏色階梯**（idle `overlay-web-warn-20` 細條 / recording `overlay-web-warn-30` 加深）
3. **C' upload-failed 的雙救濟按鈕**視覺（retry × save-offline 形狀、色階、優先順序）
4. **新增 i18n keys 的視覺對映**（hero R6/R7/R8 + Stage 1 fix 加進共用 keys 表的 6 個 Web key）
5. **Lucide icon ID 映射**（不是 SF Symbols / Material Symbols — 跟 Win/Linux 同 lib，但用法不同）
6. **沒對應視覺**清單：lock-screen / FG-service / system tray / global shortcut（純 tab UI）
7. **Web Worker chunk timer 註記**（per Agy R1 architectural catch）— 視覺不變但 implementation note 必標

## Design token 對映（per hero mockup §design token 速查 + overlay 表）

繼承 hero `spectyn-*` 全部 token + 字級 type ramp。Web 額外使用 overlay tier：

| Overlay token | 數值 | Web 用途 |
|---|---|---|
| `overlay-web-warn-20` | `spectyn-warning @ 20%` | A1 / A2 caveat banner（idle）背景 |
| `overlay-web-warn-30` | `spectyn-warning @ 30%` | C Recording caveat banner（加深變體）背景 |
| `overlay-error-16` | `spectyn-danger @ 16%` | C' upload-failed 卡片背景 |
| `overlay-disabled-40` | element opacity 40% | A1 PTT 按住期間 Timer disabled |

實作端 Tailwind class：`bg-[var(--overlay-web-warn-20)]` 或 `bg-warning/20`（依 tailwind.config.js 命名收斂）。Light-mode 對映同 hero mockup §35 — **TBD 下版補**。

## Lucide icon 對映（per hero mockup §56 icon 矩陣 Web column）

繼承 hero icon 矩陣 Web column，本檔列 SPEC-21 用到的 8 個：

| 角色 | Lucide ID | 用途 |
|---|---|---|
| 麥克風 / PTT | `mic` | A1 PTT 大鈕 icon |
| 麥克風關閉 | `mic-off` | B' Denied 卡 / paused 變體 |
| 播放 / 開始計時 | `play` | A1/A2 Start timer 按鈕 |
| 暫停 | `pause` | C Recording Pause 按鈕 |
| 停止 | `square` | C Recording Stop 按鈕（filled square 慣例） |
| chunk 計數 | `folder` | C 段 chunk count badge |
| 警告 / 中斷 | `triangle-alert` | Caveat banner 起首 icon |
| 上傳失敗 / 離線 | `wifi-off` | C' upload-failed 起首 icon |

實作端：`lucide-react` 套件已在 `MobileDashboard.tsx` 用，import `{ Mic, MicOff, Play, Pause, Square, Folder, TriangleAlert, WifiOff } from 'lucide-react'`。

## Web 新增 i18n keys（視覺對映）

hero mockup §75-136 共用 keys 表已加進 6 個 Web 新 key（R6/R7/R8 + Stage 1 fix）。本檔列**每個 key 的視覺出處**：

| Key | 出現於 | 視覺位置 |
|---|---|---|
| `focus.web.caveat` | A1 / A2 / C | 全程頂部 caveat banner 文字 |
| `focus.web.upload_failed` | C' | C' 卡片 title（body-lg, `spectyn-danger`） |
| `focus.web.retry` | C' | Retry 按鈕 label |
| `focus.web.save_offline` | C' | Save-offline 按鈕 label |
| `focus.web.perm_settings_hint` | B' | Denied 卡 in-card 步驟提示（取代 deep link — Web 不可實現） |
| `focus.web.offline_pending` | C' save-offline 模式後續 | Caveat banner 文案切換版本（「已暫存 {n} 段...」） |
| `focus.web.quota_exceeded` | C / C' | IndexedDB quota 滿時 toast，強制停錄音 |
| `focus.web.offline_unload_warn` | tab 關閉時 | browser `beforeunload` 原生 dialog message |

## 螢幕 A1 — Idle（mobile-web，`< 768px` 或 `pointer: coarse`）

版型跟 iOS A 99% 一致（per hero mockup §iOS A L138-156）。Web delta（mockup-level）：

- **容器** max-width 480px 居中，左右 padding 16px
- **頂部 caveat banner**：全寬 36px 高（見下節）
- **PTT 大鈕**：圓 96×96px，bg `spectyn-primary` 邊框 2px，icon `mic` 32px center；按住變 bg `overlay-ripple-24`（同 Android ripple opacity，但用 CSS `:active` 而非 Material ripple）
- **duration picker**：3 顆 chip（25 / 50 / custom）28px 高、radius 14px、bg `spectyn-card`、selected 加邊框 1px `spectyn-primary`
- **Timer 副按鈕**：full-width 48px 高、bg `spectyn-card`、icon `play` + label `focus.btn.start_timer`
- **trust badge**：底部 caption `focus.trust_badge`，`spectyn-muted`
- **bottom nav**：4 tabs（同 iOS bottom-nav 結構，per hero invariants History tab lock 表），56px 高，bg `spectyn-card`

## 螢幕 A2 — Idle（desktop-web，`≥ 768px` 且 `pointer: fine`）

ASCII 已在 hero mockup §491-509 列。本檔列**視覺值補充**：

- **容器** max-width 640px 居中（跟 macOS main-window 對齊），左 sidebar 220px（History tab 等其他入口，per hero invariants）
- **計時器顯示** `display` 48px / 700 / `spectyn-muted`，centered
- **radio rows** 3 行：每行 32px 高、left padding 16px，radio circle 16px + label `body-lg`；selected 圓填 `spectyn-primary`
- **Start 按鈕** 320×48px centered，bg `spectyn-primary`、text `spectyn-bg`、icon `play` 20px + label `body-lg`；hover bg 加 10% 亮度、active 加 `overlay-ripple-24`
- **無 PTT 按鈕**：桌機鍵盤情境不適合 press-and-hold（hero R6 已決）→ **PTT × Timer 互斥 invariant 在 A2 不適用**（per hero mockup §551，僅 A1 適用）
- **trust badge**：caption `focus.trust_badge`，centered，`spectyn-muted`

## Caveat banner（A1 / A2 / C 共用，全程置頂）

兩段顏色階梯：

```
[Idle 變體 — A1 / A2]
┌───────────────────────────────────────────────┐  height 36px, full-width
│ ⚠ 瀏覽器模式：請保持本頁開啟，切走會中斷錄音    │  bg overlay-web-warn-20
│                                               │  border-bottom 1px spectyn-warning
└───────────────────────────────────────────────┘  text body-sm, spectyn-warning, padding 8px 16px
                                                    icon: triangle-alert 16px, gap 8px

[Recording 變體 — C]
┌───────────────────────────────────────────────┐  height 36px, full-width
│ ⚠ 瀏覽器模式：請保持本頁開啟，切走會中斷錄音    │  bg overlay-web-warn-30（加深）
└───────────────────────────────────────────────┘  其他同上
```

**視覺差異唯一**：bg opacity 20% → 30%。文字 / icon / 高度 / 字級全部一致。

**不可 dismissible**（per hero R6 已決 + 開放問題 #4 closed）— 視覺上不該出現 × icon。

**save-offline 模式下文案切**：`focus.web.caveat` → `focus.web.offline_pending`（「已暫存 {n} 段...」），bg 跟著切回 `overlay-web-warn-20`（warn 但非加深，表示「錄音 OK 但有暫存」）。

## 螢幕 B — getUserMedia Permission Prompt

**Browser native dialog — 不可自訂**（per Wireframe §103-106）。視覺由 Chrome / Safari / Firefox 各自決定，spectyn 完全無 control。

開發者僅能控制：
- **pre-permission education**（觸發前的 Idle 安撫文 `focus.perm.denied_reassure`）— A1 / A2 trust badge 已涵蓋
- **trigger timing**（user 主動 tap PTT / Start timer 才觸發，不在 page load）

## 螢幕 B' — Denied 卡（覆蓋 Idle）

視覺等同 iOS B'（per hero mockup §iOS B'），但：
- **icon** `mic-off` Lucide 48px `spectyn-danger`
- **主文** `focus.perm.denied` body-lg `spectyn-text`
- **安撫文** `focus.perm.denied_reassure` body `spectyn-muted`
- **「打開設定」deep link Web 不可實現** → 改顯示 in-card 步驟提示，文案取 `focus.web.perm_settings_hint`：
  ```
  ┌──────────────────────────────────┐
  │  [mic-off icon 48px spectyn-danger] │
  │                                  │
  │  需要麥克風才能 focus 錄音         │  body-lg
  │  我們不會上傳音訊到雲端...           │  body, spectyn-muted
  │                                  │
  │  ┌──────────────────────────┐    │  bg spectyn-card, padding 12px
  │  │ 請從瀏覽器設定恢復麥克風權限 │    │  body-sm, spectyn-muted
  │  └──────────────────────────┘    │  （取 `focus.web.perm_settings_hint`）
  └──────────────────────────────────┘
  ```
- **不畫 deep-link button**（Web 不開放） — 區別於 iOS / Android / desktop 的「打開設定」CTA

## 螢幕 C — Recording with Caveat Banner

版型同 iOS C（per hero mockup §iOS C L168-178）+ A1 / A2 容器尺寸。Web delta：

- **caveat banner 從 idle 細條 → recording 加深**：bg `overlay-web-warn-20` → `overlay-web-warn-30`（per R6 已決）
- **waveform** 32 bars × 4px wide × gap 2px，高 0-100px dynamic，color `spectyn-warning`
- **計時器** `display` 48px / 700 / `spectyn-warning`（recording 中色）
- **Pause / Stop** 兩鈕並列：Pause 96×40px `spectyn-card` + `pause` icon / Stop 96×40px `spectyn-danger` + `square` icon
- **chunk count chip** 右下角，bg `spectyn-card`、radius 12px、icon `folder` + 數字
- **沒 D 鎖屏卡**（browser 不給 lock-screen API）— 視覺空缺
- **沒 FG-service notification / system tray**（純 tab 內 UI）— 視覺空缺

**Web Worker chunk timer 註記**（per Wireframe §136 + Agy R1）：
- **視覺層面無變化** — waveform / timer / chunk count 全部相同
- **但 implementation 必須**：chunk 切割 setInterval 跑在 Web Worker（不是 main thread），否則 tab 切到背景 main thread 被 throttle 到 1Hz → 5min chunk 切點失準 → 錄音長度失真
- **退化視覺 OK**：tab 切背景時 waveform 掉幀 acceptable，**chunk 切割 / POST 不可掉**

## 螢幕 C' — Upload Failed Sub-state（Web 獨有）

ASCII 已在 hero mockup §524-538 列。本檔列**視覺值補充 + 雙救濟按鈕優先順序**：

- **卡片背景** `overlay-error-16`（`spectyn-danger @ 16%`）— 全寬覆蓋 C Recording 容器
- **title** `focus.web.upload_failed`，body-lg `spectyn-danger`，centered，icon `wifi-off` 20px gap 8px
- **waveform 凍結** — 灰色 `spectyn-muted`，停止繪製（視覺上明顯區別於 C 的 `spectyn-warning` 動態）
- **Retry button**（96×40px）：bg `spectyn-danger`、text `spectyn-text`、**主要救濟**（user 期望「先試重連」） — 放左、視覺權重高
- **Save-offline button**（144×40px）：bg `spectyn-card`、text `spectyn-text`、**次要救濟**（fallback path） — 放右、視覺權重低
- **trust badge** 仍在底部 caption — 提醒「就算 upload 失敗，本地仍加密」

**雙救濟並列不能刪一邊**（per Wireframe invariants「C' 不可阻止繼續錄音」）— 視覺上一定要兩鈕並列、不能藏進 menu。

**retry 連續失敗 N 次後**：自動切到 save-offline 模式，C' 視覺收回 → 回 C，caveat banner 文案切 `focus.web.offline_pending`（per Wireframe §165 + 開放問題 #4 提案）。

## 螢幕 E / F — Finalizing / Done

機制同 iOS E / F（per hero mockup §iOS E L246-264 + §iOS F L266-294）。Web delta：

- **ASR / LLM 全跑 host**（spectyn-serve 那台），browser 只 render — 視覺上跟 iOS 完全一樣（spinner + progress bar + 文案）
- **E spinner**：用 CSS animation 或 Lucide `loader-2` + `animate-spin` Tailwind class，32px `spectyn-primary`
- **F success icon**：Lucide `check-circle` 64px `spectyn-success`
- **沒「app 被殺」概念** — tab 關了就斷，browser native `beforeunload` dialog 顯示 `focus.web.offline_unload_warn`，不可自訂版面 / 按鈕順序（瀏覽器規範）

## IndexedDB quota 滿（Empty/Limit 邊界）

per Wireframe 開放問題 #3 提案：**強制停 + 顯示 `focus.web.quota_exceeded`**（不 silent drop oldest，因傷信任）。視覺：

- **error toast** 全寬 48px 高，bg `spectyn-danger @ 95%`，text `spectyn-text`，body-lg
- **文案** `focus.web.quota_exceeded`：「瀏覽器儲存空間已滿，請手動上傳」
- **行為**：停止錄音 + 切回 A1 / A2 Idle + toast 持續顯示直到 user dismiss

## Cross-platform invariants 對齊（per hero mockup §555 + Wireframe §195-203）

繼承全部 hero invariants（trust badge 文字 / Stop danger color / 計時器顏色 / 按鈕尺寸 / takeaway card / chunk count chip）。Web 額外：

- **Caveat banner 全程置頂** — idle / recording / save-offline 三段文案 + 顏色切換，**不可 dismissible**
- **C' upload-failed 不可阻止繼續錄音** — 雙救濟按鈕一定要並列出現
- **getUserMedia prompt 不可自訂** — 但 prompt 前的 pre-permission education（trust-badge + Idle 安撫文）必填
- **HTTPS 強制** — 沒 cert 不會走到 Focus tab，視覺空缺由 onboarding 接管
- **沒 lock-screen / FG-service / system tray / global shortcut** — 視覺空缺清單，per Wireframe §202
- **Recording 中切 tab 不阻止** — 視覺上無遮罩無警告，但 caveat banner 已預警

## 6 大資料狀態 — Web Mockup 視覺對映

| 狀態 | 視覺 |
|---|---|
| **理想** | F Done takeaway card 完整（同 iOS，但用 Lucide `check-circle`）|
| **空白（History）** | History tab in-tab，mono SVG illustration 192px `spectyn-muted` + `focus.empty.history` + 「前往 Focus」`spectyn-primary` 按鈕 |
| **空白（ASR 無語音）** | F takeaway card 顯示 `focus.empty.no_speech`（per hero key）+「重錄這次」/「完成」雙按鈕（per Android mockup F empty variant pattern）|
| **極限** | C chunk `99+` chip（`focus.limit.chunk_overflow`）/ F takeaway > 800 字截斷 / **IndexedDB quota 滿**（Web 獨有 limit，`focus.web.quota_exceeded` 強制停 toast）|
| **錯誤** | B' Denied 卡（Lucide `mic-off` `spectyn-danger` icon + in-card `focus.web.perm_settings_hint`，**無 deep-link button**）/ **C' upload-failed**（Web 獨有，`overlay-error-16` 卡片 + retry × save-offline 雙救濟）|
| **局部** | E `focus.partial.chunk_failed` inline 訊息（同 iOS）/ save-offline 模式下 caveat banner 切 `focus.web.offline_pending`「上傳中 X 段，落地 Y 段」 |
| **載入中** | E spinner-32 + progress bar（同 iOS，但全部計算在 host） |

## 已決（per hero R6 / R7 / R8 + Wireframe 已決）

1. **Caveat banner 顏色階梯**：idle `overlay-web-warn-20` 細條 / recording `overlay-web-warn-30` 加深 / save-offline 回 `overlay-web-warn-20` 但文案切（per hero mockup §573 + 開放問題 #4 closed）
2. **Breakpoint 切點 pointer 條件**：`@media (min-width: 768px) and (pointer: fine)`，pointer 條件解 iPad 盲區（per hero §477 + Wireframe §42-44）
3. **C' 雙救濟並列**：retry 左 / save-offline 右，**不能藏進 menu**（per Wireframe invariants）
4. **History tab 入口位置**：跟 breakpoint 走，mobile-web 底 nav / desktop-web sidebar 220px（per hero mockup §557）
5. **B' Denied 卡無 deep-link button**：改顯示 in-card 步驟提示（per Wireframe §111）

## 開放問題（mockup 層面）

1. **C' Retry vs Save-offline 視覺權重**：目前提案 Retry 主色（`spectyn-danger` bg）/ Save-offline 次要（`spectyn-card` bg），但 user 真正常用路徑可能是 save-offline（host 真不可達時 retry 浪費時間）→ v0.7+ 看遙測數據再考慮對調
2. **save-offline 暫存 chunk 數顯示**：caveat banner 內 inline 數字 vs 額外角落 chip？提案：inline（caveat banner 文案已 cover）
3. **IndexedDB quota 滿 toast 是否該帶「下載已暫存」CTA**：v0.6.0 提案不帶（先強制停 + 提示，user 自己想辦法），v0.7+ 可加「下載 zip」action
4. **Lucide icon weight**：Lucide 預設 stroke-width 2，要不要在 Web focus 視覺統一改 1.5（更輕量、更接近 SF Symbols 視感）？提案：保留預設 2，先求跨平台一致
5. **dark-mode-only ship**：v0.6.0 只出 dark mode 視覺（per hero mockup §35），light mode token TBD 下版補

> 已收進已決的舊問題：
> - ~~Caveat banner 永遠在 vs 只在 recording~~ → 已決全程置頂、兩段顏色
> - ~~B' deep-link 改成什麼~~ → 已決 in-card `focus.web.perm_settings_hint` 提示

→ 互動 timing / retry backoff / IndexedDB queue schema / Web Worker chunk timer 細節歸 Web prototype + SPEC-17 tauri-bridge。

## 下一步

→ 進 [Web Prototype（待補）] 描述每個 tap target 點下去發生什麼、Web Worker 跟 main thread 通訊 sequence、retry backoff 曲線、IndexedDB queue flush 順序、`beforeunload` 提示 timing。
