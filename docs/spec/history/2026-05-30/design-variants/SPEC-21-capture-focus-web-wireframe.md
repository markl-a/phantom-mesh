# SPEC-21 Capture Focus — Web Wireframe（線框稿）

> **Stage 1/3** · 線框稿 → [視覺稿（見 hero mockup §Web 段）](./SPEC-21-capture-focus-mockup.md) → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Web / mobile-web only。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-wireframe.md](./SPEC-21-capture-focus-wireframe.md) iOS 段），本檔只列 Web **deltas**，共用結構不重抄。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-17-PROTOCOL-tauri-bridge`](../specs/v060-deep-spec/SPEC-17-PROTOCOL-tauri-bridge.md)（C' upload queue 細節）· [`SPEC-15-PROTOCOL-broker-vault-sync`](../specs/v060-deep-spec/SPEC-15-PROTOCOL-broker-vault-sync.md)（host 不可達策略）
> **這份的工作範圍**：Web-specific layout & flow — breakpoint 切換 / caveat banner / C' upload-failed sub-state / browser perm prompt / 沒鎖屏沒 tray 的限制。共用 FSM 跟 iOS 同（見 hero wireframe §通用 session 狀態鏈），本檔不重抄。

## 為什麼 Web 有獨立 wireframe

hero wireframe 的 Web 段只 ~30 行，作為「degraded fallback」的快速 reference。實際 Web：

1. **沒專屬 platform-spec**（沒有 SPEC-50 之類）— hero wireframe + hero mockup §Web 是唯一 source of truth
2. **Breakpoint 切換**（`< 768px` 或 `pointer: coarse` → mobile / `≥ 768px` 且 `pointer: fine` → desktop）— 同一份 React 元件兩個 layout
3. **Caveat banner 全程置頂**（idle 細條 / recording 加深）— hero R6 已決
4. **C' upload-failed sub-state**：host 不可達時的暫存 + 重試 UI（mobile / desktop / Linux / Android / iOS 都沒這變體）
5. **degraded scope 很大**：tab 切到背景錄音不停，但 **chunk 切割計時必須走 Web Worker**（main thread setInterval 被 throttle 到 1Hz 會破壞 5min chunk 切點）/ 沒 lock-screen / 沒 FG-service / 沒 system tray / getUserMedia prompt 不可自訂

→ 這 5 點值得獨立 frame 級描述，不要塞在 hero §Web 段。

## 入口點

| 進入點 | v0.6.0 | v0.7+ | Source |
|---|---|---|---|
| Browser 開 `https://<host-tailscale-name>:7878/` | ✅ | ✅ | hero wireframe §Web |
| Bookmark / PWA add-to-homescreen（mobile-web） | ✅ | ✅ | browser-native，無 install flow |
| QR code 從 main window 掃進手機 web | ❌ | ✅ | 延後（serve UI 工具列） |
| 桌機推 sharable URL（如 IM 訊息） | ✅ | ✅ | host 必須在同 Tailscale tailnet |
| In-tab Focus view（per breakpoint top-nav / sidebar） | ✅ | ✅ | hero wireframe §Web |

**v0.6.0 ship**：直接 URL + in-tab Focus 入口。

## Breakpoint 切換（per hero wireframe §Web + hero mockup §Web）

```
[Browser 載入] → 量測 viewport width + pointer
       │
       ├── < 768px OR pointer: coarse → [mobile-web layout (A1)]
       └── ≥ 768px AND pointer: fine  → [desktop-web layout (A2)]
```

CSS media query：`@media (min-width: 768px) and (pointer: fine)`。同一份 React 元件 conditional render（不另切 page / 不換 route）。

**pointer 條件解 iPad 盲區**：iPad 寬度 > 768px 但純觸控（`pointer: coarse`），不該掉 PTT — desktop-web 版型只在「大螢幕 + 真實滑鼠 / 觸控板」時啟用。

| Layout | 視覺特徵 | 對應 mockup |
|---|---|---|
| **mobile-web (A1)** | PTT 大鈕 + duration picker + Timer 副選 | mockup §A1 |
| **desktop-web (A2)** | Timer-only（無 PTT — 鍵盤情境不合）+ duration radio rows | mockup §A2 |

## 螢幕 A1 — Idle（mobile-web，< 768px）

版型結構同 iOS A（duration picker + PTT button + Timer button + trust badge），但全程頂部多一條 caveat banner。Delta：

- **容器** max-width 480px 居中
- **icon library**: Lucide（不是 SF Symbols / Material Symbols）— per mockup icon 對照矩陣
- **caveat banner**：A1 / A2 共用（見下）
- **getUserMedia prompt 是 browser native dialog**（B 階段），版面不可自訂

## 螢幕 A2 — Idle（desktop-web，≥ 768px 且 pointer: fine）

```
[A2. Focus Idle — desktop-web]
┌──────────────────────────────────────────┐
│ ⚠ [caveat-msg]                           │  ← 全程頂部 36px caveat banner（idle 細條）
├──────────────────────────────────────────┤
│                                          │
│         [elapsed / total clock]          │
│                                          │
│         ◯ [duration-opt-1]                │  ← radio rows（同 macOS Sheet）
│         ◯ [duration-opt-2]                │
│         ◉ [custom-num] [unit]            │
│                                          │
│         ┌──────────────────┐             │
│         │  [start-timer]   │             │  ← 中央單按鈕，無 PTT
│         └──────────────────┘             │
│                                          │
│         [trust-badge]                    │
└──────────────────────────────────────────┘
```

**A2 重點**：
- 去掉 PTT（桌機鍵盤情境不適合 press-and-hold）
- duration picker 改 radio rows（同 macOS Sheet）
- 沒有「並存雙鈕」→ **PTT × Timer 互斥 invariant 在 A2 不適用**（只 A1 適用）

## 螢幕 B — getUserMedia Permission Prompt

```
[A1 / A2] tap PTT or start-timer (first time)
       │
       ▼
[B. Browser native perm dialog — 不可自訂]
       ┌─────────────────────────────────┐
       │ "[origin] 想要使用你的麥克風"     │  ← Chrome / Safari / Firefox 各自版面
       │                                 │
       │  [封鎖]   [允許]                  │
       └─────────────────────────────────┘
       ├── allow → C
       └── deny  → [B'. Denied 卡] (覆蓋 Idle, 同 iOS B')
```

**Web 限制**：
- 版面 / 文案 / 按鈕順序全由 browser 控制，**開發者無權更動**
- 沒有「永久拒絕」vs「這次拒絕」的 API 區別 — 反映 `permissions.query()` state 為 `denied` 即視為拒絕
- HTTPS 強制：`http://` origin getUserMedia 直接 SecurityError（不顯示 prompt）。Tailscale 自簽 / mkcert 必須裝 root CA（per SPEC-15 host 不可達策略中的「user 必須先信任 host cert」前置條件）

### B' Denied 行為（同 iOS）

- getUserMedia 拒絕後：Idle 覆蓋遮罩卡片 + mic-denied icon + 安撫文 + 「打開設定」按鈕
- **「打開設定」deep link 在 Web 不可實現**（browser 不開放）→ 改顯示「點地址列鎖頭圖示 → 網站設定 → 麥克風」的步驟提示，文案取 `focus.web.perm_settings_hint`（**新增 key**，per 開放問題 #2）

## 螢幕 C — Recording with Caveat Banner

版型同 iOS C（計時器 / waveform / pause-stop / chunk count / trust badge），但全程頂部 caveat banner 顏色加深。

```
[C. Recording — with caveat banner]
┌──────────────────────────────┐
│ ⚠ [caveat-msg]               │  ← bg overlay-web-warn-30（加深，recording 變體）
├──────────────────────────────┤
│  [elapsed / total]           │
│  [waveform]                  │
│                              │
│  [pause]  [stop]             │
│                              │
│  [chunk-count]               │
│  [trust-badge]               │
└──────────────────────────────┘
```

**Web delta**：
- caveat banner 從 idle 細條 (`overlay-web-warn-20`) → recording 加深 (`overlay-web-warn-30`)，per mockup R6 已決
- **沒有 D 鎖屏卡**（browser 不給 lock-screen API）
- **沒有 FG-service notification / system tray**（純 tab 內 UI）
- tab 切到背景時 — getUserMedia stream 多數 browser 仍跑（不像 mobile safari background suspend 那麼狠），但 **page lifecycle `visibilitychange` → hidden 後 main thread 嚴重 throttle**：waveform render 掉幀（UX 退化但 acceptable）；**chunk 切割計時 / chunk POST 必須跑在 Web Worker**（main thread setInterval 被降到 1Hz 會破壞 5min chunk 切點 → 錄音長度失真，per Agy R1 architectural catch）。FSM 不變

## 螢幕 C' — Upload Failed Sub-state（Web 獨有）

iOS / Android / desktop 都沒這變體。Web 場景：

```
[Recording 中 → host 不可達]
       │
       │ 觸發來源：
       │  - 網路斷
       │  - spectyn-serve 重啟 / crash
       │  - Tailscale 連線掉
       │  - cert 改變導致 fetch 失敗
       ▼
[C'. Upload Failed — Recording 變體]
┌──────────────────────────────┐
│ ⚠ [upload-failed-msg]        │  ← bg overlay-error-16，紅色變體
├──────────────────────────────┤
│  [elapsed / total]           │
│  [waveform-frozen]           │  ← 灰，停止繪製
│                              │
│  [retry-btn] [save-offline]  │  ← 兩條救濟並列
│                              │
│  [trust-badge]               │
└──────────────────────────────┘
       │
       ├── retry 成功 → 回 C（繼續 upload + recording）
       ├── retry 連續失敗 N 次 → 自動降到 save-offline 模式
       └── save-offline → 寫 IndexedDB queue → 繼續錄音，UI 標 "已暫存 X 段"
```

**Web 重點**：
- C' **不是停止錄音**（chunks 仍持續落 IndexedDB），只是 upload pipeline 中斷
- retry 次數 / backoff / queue 上限 / 過期清除 / 重連後 flush 順序 — **defer 到 SPEC-17 tauri-bridge / SPEC-15 broker-vault-sync** 決定
- save-offline 模式下 user 關 tab 前 fire `beforeunload` 提示「還有 X 段未上傳，關了會留在 browser」— 文案 `focus.web.offline_unload_warn`（**新增 key**）
- IndexedDB quota（典型 ~50MB-2GB / origin，視 browser）超出 → 強制停錄音 + 顯示 `focus.web.quota_exceeded`（**新增 key**），per 開放問題 #3

## 螢幕 E / F — Finalizing / Done（同 iOS）

機制同 iOS：E phase 1 (Transcribing) + phase 2 (SummaryGen) → F Done takeaway card。

**Web delta**：
- **ASR / chunk transport / LLM 全跑在 host**（spectyn-serve 那台），**不在 browser**
  - browser 只負責 `MediaRecorder` chunk → POST 給 host → 收 transcript/takeaway → render
  - whisper.cpp / Ollama 都在 host，跟 Web 無關
- **沒有「app 被殺」概念** — tab 關了就斷，沒 WorkManager 接管。tab 關前 fire `beforeunload` 提示「未 finalize 的 session 會丟」（per save-offline 同 key 但 mode 不同）
- E phase 進度顯示同 iOS（spinner + `focus.finalizing.asr` + `focus.finalizing.llm`）— 因 host 是真實 ASR 端

## 入口架構決議

| 元素 | Web 對映 |
|---|---|
| **In-tab nav**（per breakpoint） | mobile-web 底 nav（同 iOS bottom-nav 結構，4 tabs）/ desktop-web 左 sidebar 220pt（同 macOS / Windows main window） |
| **History tab 位置** | 跟 breakpoint 走 — mobile-web 底 nav 右一 / desktop-web sidebar 中（per hero invariants History tab lock 表 + mockup §552） |
| **System back（mobile-web）** | browser 原生 back（gesture / button）— Web 不攔 `popstate`，React Router 自處理 |
| **PTT × Timer 互斥**（mobile-web A1 only） | 同 iOS hero invariant — PTT 按住期間 Timer disabled；A2 desktop 沒有 PTT 故不適用（per mockup §546） |

## Cross-platform invariants 對齊（per wireframe hero）

繼承全部 hero invariants（trust badge / Stop ≤ 2 操作 / waveform / chunk count / 計時器顏色）。Web 額外：

- **Caveat banner 必須全程置頂**（idle / recording / C' 都在，只變顏色深淺）— 不能 dismissible（per mockup R6 已決）
- **C' upload-failed 不可阻止繼續錄音** — 必須給 save-offline 路徑，user 行為被 host 不可達綁架是 anti-pattern
- **getUserMedia prompt 不可自訂**（browser layer）— 但 prompt 前的 pre-permission education 仍要做（trust-badge + Idle 安撫文）
- **HTTPS 強制** — 沒 cert 直接掛在 onboarding，不會走到 Focus tab
- **沒 lock-screen / FG-service / system tray / global shortcut**（純 tab 內 UI）— hero §Web 「不承諾」清單延伸
- **Recording 中切 tab 不阻止**（getUserMedia 仍跑，但 throttle）— 文案安撫 user「最好別切，但切了不會立刻掛」

## 6 大資料狀態 — Web 對映表

| 狀態 | Web 螢幕 / 場景 | 對應 i18n key / mockup |
|---|---|---|
| **理想（Ideal）** | F Done takeaway card 完整（同 iOS） | `focus.done.title` per mockup §561 |
| **空白（Empty）** | History tab in-tab（無 session）/ ASR 無語音 | `focus.empty.history` per mockup §562 |
| **極限（Limit）** | C chunk 99+ / F takeaway > 800 字截斷 / IndexedDB quota 滿（Web 獨有 limit）| `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` / `focus.web.quota_exceeded`（新增）per mockup §563 + 開放問題 #3 |
| **錯誤（Error）** | B' Denied 卡 / **C' upload-failed**（Web 獨有）| `focus.perm.denied` / `focus.web.upload_failed` + `focus.web.retry` + `focus.web.save_offline` per mockup §564 |
| **局部（Partial）** | E Finalizing inline 訊息（同 iOS）/ save-offline 模式下「上傳中 X 段，落地 Y 段」分流 | `focus.partial.chunk_failed` + `focus.web.offline_pending`（新增）per 開放問題 #4 |
| **載入中（Loading）** | E Finalizing spinner + 進度（同 iOS，但所有計算在 host） | `focus.finalizing.asr` + `focus.finalizing.llm` |

## 已決（per hero R6 / R7 + mockup R6）

1. ~~Caveat banner 永遠在 vs 只在 recording~~ → **已決**：全程頂部，idle `overlay-web-warn-20` 細條 / recording `overlay-web-warn-30` 加深（per hero wireframe 開放問題 #2 closed + mockup §573）
2. ~~History tab 位置 Web 對映~~ → **已決**：跟 breakpoint 走，mobile-web 底 nav / desktop-web sidebar（per hero invariants 表 + pointer 條件）
3. ~~Breakpoint 切點 pointer 條件~~ → **已決**：`@media (min-width: 768px) and (pointer: fine)`，pointer 條件解 iPad 盲區（per hero §Web 內文 + mockup §477）

## 開放問題（Web 層面）

1. **save-offline 上限與 retry 次數**：N 次失敗後降級的 N 值？IndexedDB queue 上限是時間（24h）還是容量（50MB）？ → defer SPEC-17 tauri-bridge
2. **B' Denied 設定步驟提示**：因 Web 不能 deep-link，需要 in-app 圖文步驟教 user 開麥克風權限。文案 `focus.web.perm_settings_hint` **未定**（要分 Chrome / Safari / Firefox 還是寫通用？）
3. **IndexedDB quota 滿時的 UX**：強制停 + 提示清舊資料？還是 silent drop oldest chunks？提案：強制停 + `focus.web.quota_exceeded`，因「靜默丟資料」傷信任
4. **save-offline 模式的 partial state UI**：「上傳中 X 段，落地 Y 段」要不要新 frame？目前提案：複用 C' frame，只變 caveat banner 文案到 `focus.web.offline_pending`
5. **PWA install / add-to-homescreen**：mobile-web 是否該主動 prompt `beforeinstallprompt`？提案：v0.6.0 不主動，v0.7+ 看 retention 數據再開

> 已收進 invariants 的舊問題：
> - ~~tab 切走是否強制停錄音~~ → invariants「Recording 中切 tab 不阻止」+ C 段 visibilitychange throttle 註記
> - ~~C' 是 frame 還是 toast~~ → wireframe C' 已畫為 Recording 變體 frame

→ 互動 timing / 重試 backoff / IndexedDB schema 細節歸 Web prototype + SPEC-17 tauri-bridge。

## 下一步

→ 進 [Web Mockup（合併在 hero mockup §Web 段）](./SPEC-21-capture-focus-mockup.md#web--mockup) 看視覺值（color / type / size / 終版文案 / Lucide icon ID）。Web 沒獨立 mockup 檔，因 hero mockup §Web 已涵蓋 A1 / A2 / C' 三組 ASCII + design tokens。
