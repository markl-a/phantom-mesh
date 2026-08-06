# SPEC-21 Capture Focus — Android Wireframe（線框稿）

> **Stage 1/3** · 線框稿 → [視覺稿（待補）] → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Android only。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-wireframe.md](./SPEC-21-capture-focus-wireframe.md) iOS 段），本檔只列 Android **deltas**，共用結構不重抄。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-33-PLATFORM-Android-foundations`](../specs/v060-deep-spec/SPEC-33-PLATFORM-Android-foundations.md) · [`SPEC-34-PLATFORM-Android-screens-flows`](../specs/v060-deep-spec/SPEC-34-PLATFORM-Android-screens-flows.md)
> **這份的工作範圍**：Android-specific layout & flow — 3 perm gates / FG-service / WorkManager / Material Symbols。共用 FSM 跟 iOS 同（見 hero wireframe §通用 session 狀態鏈），本檔不重抄。

## 為什麼 Android 有獨立 wireframe

iOS hero wireframe 的 Android 段只 ~35 行，作為「同 iOS X」的快速 reference。實際 Android：
1. **3 perm gates**（RECORD_AUDIO / POST_NOTIFICATIONS / FOREGROUND_SERVICE_MICROPHONE）— iOS 只 1 個
2. **FG-service notification** 是常駐通知欄不是鎖屏卡 — UX 不同
3. **WorkManager** 處理 app 被殺後的 ASR 完成 — iOS 沒這個 API
4. **Material Symbols + ripple effect** — 跟 iOS SF Symbols / native press 動畫不同
5. **Quick-tile v0.7+** + **app shortcut**（long press launcher icon）— Android 獨有

→ 這 5 點值得獨立 frame 級描述，不要塞在 iOS 段。

## 入口點（per SPEC-34）

| 進入點 | v0.6.0 | v0.7+ | Source |
|---|---|---|---|
| App drawer icon | ✅ | ✅ | SPEC-34 standard |
| **Quick Settings tile** | ✅ | ✅ | **SPEC-34 §146 G5（一鍵啟 25min focus session 是 v0.6.0 goal）** |
| **Glance widget**（chip palette，跨 feature 共用）| ✅ | ✅ | SPEC-34 §30(E) — 主要服務 SPEC-22 habit chip，focus 不直接用 |
| App shortcut（long-press launcher icon → "Start focus 25min"）| ❌ | ✅ | 延後 |
| In-app Focus tab（per Capture tab, bottom nav）| ✅ | ✅ | SPEC-34 §30(A) |
| Wear OS companion | ❌ | ❌ | v0.8+ |

**v0.6.0 ship 3 個**：app drawer + Capture tab → Focus + Quick Settings tile（per SPEC-34 G5）。

## 螢幕 A — Focus Idle（同 iOS A，省略 ASCII）

版型結構同 iOS A（duration picker + PTT button + Timer button + trust badge）。只列 Android delta：

- **nav bar 高 56dp**（iOS 44pt）
- **icon library**: Material Symbols Rounded（不是 SF Symbols）— per mockup icon 對照矩陣
- **按鈕 press 樣式**: Material ripple（color = `overlay-ripple-24`）— 不是 iOS 8% bg lighten
- **status bar tint**: Recording 中染色（具體 token 在 mockup）

## 螢幕 B1 / B2 — 3 個 Perm Gates

```
[A. Idle] tap PTT / Timer
       │
       ▼
[B1. RECORD_AUDIO runtime perm]    ← Android 6+ runtime
       ├── allow → ↓
       └── deny  → [B'. Denied 卡] (覆蓋 Idle, 同 iOS B')
                  
       ▼
[B2. POST_NOTIFICATIONS perm]      ← Android 13+, OPTIONAL
       ├── allow → ↓
       └── skip  → [Idle 頂部加提示 bar — 仍可錄，但無 shade UI]
       
(FOREGROUND_SERVICE_MICROPHONE     ← Android 14+ manifest-granted, no runtime prompt;
 manifest 必加 <foreground-service android:foregroundServiceType="microphone" />
 startForeground() 必傳 FOREGROUND_SERVICE_TYPE_MICROPHONE — 否則 OS 直接 throw SecurityException)
       
       ▼
[C. Recording — 同 iOS C]
```

### B' Denied 行為（同 iOS）

- RECORD_AUDIO 拒絕後：Idle 覆蓋遮罩卡片，含 mic-denied icon + 安撫文 + 「打開設定」按鈕
- Deep link: `Intent("android.settings.APPLICATION_DETAILS_SETTINGS")` → 系統 app info 頁面

### B2 skip 後 degraded UI

```
[A. Idle 變體]
┌────────────────────────────┐
│ ⓘ 沒給通知權限也可錄，       │  ← bg spectyn-card, body-sm muted, 32dp 高, 可滑掉
│   但通知欄不會顯示控制       │
├────────────────────────────┤
│ ...Idle 原內容...           │
└────────────────────────────┘
```

文案取 `focus.android.notif_optional`。錄音照常 work，但 shade UI 不顯示。

## 螢幕 C — Recording（同 iOS C ASCII + Android deltas）

版型同 iOS C（計時器 / waveform / pause-stop / chunk count / trust badge）。Delta：

- **計時器位置同 iOS**，數字色用 mockup token
- **waveform** 一樣 32 bars
- **沒有鎖屏卡（iOS D）** — Android 用 FG-service 通知取代，見下方螢幕 D

### Android 獨有：FG-service Notification（取代 iOS lock-screen）

```
[D. FG-service notification in shade]
┌──────────────────────────────────┐
│ [app icon mono] Spectyn Mesh     │
│ Focus · 05:23 / 25:00            │
│                                  │
│ [stop] (only action button)      │
└──────────────────────────────────┘
```

- **常駐通知欄**（persistent: true, low priority, no sound）
- **單一 action: stop only**（不擴 pause，per wireframe SPEC-21 hero）
- 點通知 body → 開回 spectyn Focus 螢幕
- 點 stop → 等同 app 內 ⏹ 停止

**B2 skip 情境下**：此通知不顯示（user 仍可開 app 操作）。

## 螢幕 C' — Interrupted sub-state

OS interrupt 來源（Android 上的觸發點，跟 iOS 不同）：
- 來電（AudioManager AUDIOFOCUS_LOSS）
- 其他 app 抓 mic（同上）
- 系統 sleep / Doze mode（Android 6+ 省電模式）
- 藍牙耳機切換（mic source 換到 BT mic）

UX 同 iOS C'（waveform 凍結 + interrupted 訊息 + 寬限）。文案取 `focus.interrupted.phone` / `focus.interrupted.mic_grabbed`。寬限數字 per wireframe FSM。

## 螢幕 E / F — Finalizing / Done（同 iOS 但 + 通知更新）

機制同 iOS：E phase 1 (Transcribing) + phase 2 (SummaryGen) → F Done takeaway card。

**Android delta**：app 進背景時 FG-service notification text 跟著 FSM 更新：
- `Recording`: "Focus · 05:23 / 25:00"
- `Finalizing/Transcribing`: "整理逐字稿 (2/5)…"（取 `focus.finalizing.asr`）
- `Done`（通知短暫切到，3s 後 dismiss）: "完成 · 25 分鐘 · 5 chunks"（取 `focus.done.title`）
- **ASR 全靜音情境（Empty）**: takeaway 卡片顯示安撫文「本次時段未偵測到語音，已為您記錄時長」+ session 仍寫 events（user opt-in re-record）

## 螢幕 B' — Denied（覆蓋 Idle）

iOS 的 B' 變體；Android 版差別：
- **deep link**: `Intent("android.settings.APPLICATION_DETAILS_SETTINGS")` → 系統 app info 頁面
- **B2 拒絕**（POST_NOTIFICATIONS）和 **B1 拒絕**（RECORD_AUDIO）不同處理：
  - B1 deny → 全螢幕 B' Denied 卡（阻止錄音）
  - B2 deny → 仍可錄，只是 Idle 頂部加 degraded UI bar（不阻止）

## Android 獨有 — WorkManager 接管（best-effort）

iOS 沒有 WorkManager 對等品。Android 場景：

```
[Recording 中 → app 被 OS killed]
       │
       ▼
[已落地 audio chunks 留在 disk]
       │
       ▼
[WorkManager 排 Expedited ASR job]   ← session stop 時排 1 個聚合 job（per Android Doze 限制，
       │                                Expedited job 才能在省電模式下短時間執行）
       ▼
[user 下次開 app]
       │
       ▼
[直接看到 Done card 含 takeaway]
       OR
[「上次有未完成 session」prompt（per wireframe FSM NG4）]
```

**Best-effort 而非 guarantee**：WorkManager 受 Android Doze / Battery Saver / OEM custom（MIUI / EMUI）等限制可能 defer 執行；用 Expedited Job + `setExpedited()` 提升優先級但仍非 100% 保證。session 結束時 user 若立刻開 app 就會等到 finalize；長時間不開 app（> 1 天）+ 低電量 + Doze 啟動 → 可能延遲到下次充電。

**FGS 啟動限制（Android 9/10/12+）**：Quick Settings tile 觸發 focus session 時 — TileService 從 system UI 啟動 FGS 屬 user-initiated，符合 Android 12+ FGS launch restriction。背景 mDNS / 通知觸發若要啟 FGS 必須走 PendingIntent → user-visible activity（v0.7+ 限制）。

## 入口架構決議（per SPEC-34 §30(A) IA + SPEC-21 hero invariants）

| 元素 | Android 對映 |
|---|---|
| **Bottom-nav 4 tabs**（per SPEC-34 + SPEC-03 sitemap）| **Home / Coach / Capture / Settings**（Focus 入口在 Capture tab 內，非獨立 tab）|
| **History tab 位置**（SPEC-21 hero lock）| 在 Capture tab 的 Focus 子畫面內顯示 history list 區段（不是 bottom-nav 獨立 tab — Android 4 tab IA 已被 SPEC-34 鎖死，hero 那條「bottom-nav 右一」只適用 iOS） |
| **System back button** | Kotlin `OnBackPressedDispatcher` 攔截 → emit Tauri event `system.back` → React Router `navigate(-1)`；history 空時放行給系統退出（per SPEC-34 §30(C)） |
| PTT × Timer 互斥 | 同 iOS hero invariant — Idle 時雙鈕並存、PTT 按住 Timer disabled、Timer 跑中切到 C 螢幕（PTT 從版面消失，邏輯保證） |

## Cross-platform invariants 對齊（per wireframe hero）

繼承全部 hero invariants（trust badge / Stop ≤ 2 操作 / waveform / chunk count / 計時器顏色）。Android 額外：

- **FG-service notification 必須 persistent**（不是 dismissable banner）
- **B2 skip 後降級 UI bar** 必須顯示（不能靜默）
- **POST_NOTIFICATIONS 拒絕不阻止錄音** — 只影響 shade UI
- **POST_NOTIFICATIONS recovery 入口**（per R1 Codex catch）— B2 skip 後 settings 內提供「重新授權通知」按鈕；點按 → 同樣 deep-link 進 `APPLICATION_DETAILS_SETTINGS`；user 開回通知權限後下次 Recording 自動顯示 FG-service shade UI（不需重啟 app）
- **TalkBack contentDescription 必填**（per SPEC-34 §148 G7 + WCAG 2.2 AA） — 所有 button / icon 元件都要

## 6 大資料狀態 — Android 對映表

| 狀態 | Android 螢幕 / 場景 | 對應 i18n key / mockup |
|---|---|---|
| **理想（Ideal）** | F Done takeaway card 完整 | `focus.done.title` per mockup §561 |
| **空白（Empty）** | History tab in Capture（無 session）/ ASR 無語音（session 跑完無 transcript）| `focus.empty.history` / 新加（Android 獨）安撫文 |
| **極限（Limit）** | C chunk 99+ / F takeaway > 800 字截斷 | `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` per mockup §563 |
| **錯誤（Error）** | B' Denied 卡 / interrupted | `focus.perm.denied` / `focus.interrupted.*` |
| **局部（Partial）** | E Finalizing inline `focus.partial.chunk_failed` | per mockup §565 |
| **載入中（Loading）** | E Finalizing + FG-service 通知文字更新 | `focus.finalizing.asr` + 通知 channel 字串更新 |

## 已決（per SPEC-34 lock + R1 review）

1. ~~Quick-tile 一鍵 25min vs 開 app~~ → **已決**：一鍵 25min 啟（per SPEC-34 §146 G5 = `system.tile.focus_start`）
2. ~~WorkManager ASR job 排程時機~~ → **已決**：session stop 時排 1 個 Expedited 聚合 job（per FGS Doze 限制 + 單 job 簡化）
3. ~~POST_NOTIFICATIONS skip UI 樣式~~ → **已決**：Idle 頂部 32dp 提示 bar，可滑掉（per `focus.android.notif_optional`）

## 開放問題（Android 層面，剩餘）

1. **Wear OS companion** (v0.8+) 是否值得做？SPEC-33 沒列。等 iOS hero usability test 結果再說。
2. **MIUI / EMUI / OneUI custom Doze 行為**：WorkManager Expedited 仍可能被 OEM 限制。需 device farm 跑測。

→ 互動 timing / 手勢 / haptic 細節歸 Android prototype（待補）。

## 下一步

→ 進 [Android Mockup（待補）] 決定 Material You dynamic color / icon ID / 終版文案。
