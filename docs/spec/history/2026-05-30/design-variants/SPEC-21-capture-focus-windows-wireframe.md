# SPEC-21 Capture Focus — Windows Wireframe（線框稿）

> **Stage 1/3** · 線框稿 → [視覺稿（待補）] → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Windows only。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-wireframe.md](./SPEC-21-capture-focus-wireframe.md) iOS 段），本檔只列 Windows **deltas**，共用結構不重抄。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-42-PLATFORM-Windows-foundations`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43-PLATFORM-Windows-screens-flows`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)
> **這份的工作範圍**：Windows-specific layout & flow — system tray dropdown / ActionCenter toast / 真窗 sheet（非 popover）/ 全域熱鍵 fallback chain / Focus Assist 互動。共用 FSM 跟 iOS 同（見 hero wireframe §通用 session 狀態鏈），本檔不重抄。

## 為什麼 Windows 有獨立 wireframe

iOS hero wireframe 的 Windows 段只 ~35 行，作為「同 macOS X」的快速 reference。實際 Windows：
1. **沒 transient-popover shell affordance** — Start sheet 必須是 real window（macOS 是 NSPanel sheet、Windows 是 NSWindow-equivalent 真窗）
2. **System tray 在右下角而非 menu bar 右上**（macOS NSStatusItem）— 視覺位置 + 右鍵語意都不同
3. **ActionCenter toast persists 到 user dismiss**（macOS Notification Center banner 自動消）— Win 用戶常 miss 一閃即逝的通知，persistent 是有意設計
4. **預設不註冊 global shortcut**（macOS hero 預設 `⌘⇧F`）— 避撞 Windows enterprise app（`Ctrl+Shift+F` 撞 Outlook find folder）
5. **Focus Assist / Quiet hours** 會折疊 toast — 需 `scenario="urgent"` 穿透；macOS 沒對等機制

→ 這 5 點值得獨立 frame 級描述，不要塞在 hero macOS 段。

## 入口點（per SPEC-43 §10.1 + §8.5）

| 進入點 | v0.6.0 | v0.7+ | Source |
|---|---|---|---|
| Main window `[Focus tab]` sidebar | ✅ | ✅ | SPEC-43 §10.1 S10（最可靠 canonical surface） |
| **System tray right-click → "Start Focus..."** | ✅ | ✅ | **SPEC-43 §8.2 item 6（tray dropdown 鎖定順序第 6 項）** |
| `Win+Shift+F` global hotkey（user opt-in） | ❌ | ✅ | **SPEC-43 §8.5（v0.6.0 不預設註冊；Settings → Hotkeys user 可開啟）** |
| `Ctrl+Alt+F` fallback hotkey | ❌ | ✅ | SPEC-43 §8.5 fallback 1（primary 撞時自動退） |
| Deep-link `phantom-mesh://focus/start` | ✅ | ✅ | SPEC-43 §10.1 S10 entry "deep-link" |
| **SystemMediaTransportControls (鎖屏控制)** | ❌ | ✅ | **v0.7+ 評估（SMTC API；v0.6.0 NG — Tauri 2 binding 未穩）** |
| Win 11 Widgets board | ❌ | ❌ | SPEC-43 §3.3 OoS1 |

**v0.6.0 ship 2 個**：main window Focus tab + tray menu「Start Focus...」（per SPEC-43 §8.2）。global hotkey 預設 OFF（避撞 enterprise app，per Alt-C 決策）— user 想要可至 Settings → Hotkeys 手動 enable。

## 螢幕 A — Focus Idle / Start Window（真窗 sheet，非 popover）

```
┌──────────────────────────────────────────────┐
│ Start Focus Session                  [_][□][X]│   ← OS chrome：標題列 + 三鈕（per SPEC-43 §10.6）
├──────────────────────────────────────────────┤
│                                              │
│              00:00 / 25:00                   │   ← elapsed/total display
│                                              │
│  Duration:  ( 25 ) min   [ 15 | 25 | 50 ]   │   ← duration picker（per SPEC-43 §10.6）
│                                              │
│              ┌──────────────────┐            │
│              │  ▶  開始計時錄音  │            │   ← Start btn (取 `focus.btn.start_timer`)
│              └──────────────────┘            │
│                                              │
│              🔒 本地加密 · 麥克風 ASR          │   ← trust badge (取 `focus.trust_badge`)
└──────────────────────────────────────────────┘
```

**Windows delta vs macOS A**：
- **真窗（NSWindow-equivalent）** 480×320px、置中 — **不是 sheet/popover**，因 Windows 沒 transient-popover shell affordance（per hero wireframe §Windows 重點）
- **不做 Mica acrylic 半透明**（SPEC-43 §3.2 NG2 — Tauri 2 對 Mica 支援 v0.5.x 未穩，留 v0.7+）
- **沒有 PTT 按鈕**（桌機鍵盤情境，per hero macOS / Windows 共識）
- **title bar 用系統預設**（不自訂；per mockup §394）
- **standard window controls**：min / max / close 三鈕（macOS 是左上紅黃綠，Windows 是右上 X）
- **Tab key 預設進所有 focusable element**（per SPEC-43 §14 — Windows 對 keyboard-first user 最友善；macOS 預設 Tab 不進 button、要 user 開「Full Keyboard Access」）
- **Win+Enter 直接 Start**（accelerator，per SPEC-43 §12.2 鍵盤導覽完整對照）
- **Escape 關閉視窗**（cancel）

## 螢幕 B — Permission（OS 不彈 dialog）

Windows runtime permission model — RegisterHotKey 與 mic capture 都**不彈 dialog**（per SPEC-43 §15）。
- **Mic 權限**：Win 10/11 Settings → Privacy → Microphone 已預設允許桌面 app；首次錄音若 user 在系統設定關掉 → mic capture fail 拋 `WASAPI_ERROR_DEVICE_DISABLED` → 走螢幕 B' 變體（不畫獨立 frame）
- **Toast 通知權限**：Settings → Notifications → Phantom Mesh enabled（預設開）；user 關掉 → `R.windows.toast_emit_fail`（per SPEC-43 §9.4） → 退化到 in-app banner
- **Global hotkey**：不需 user 授權 — 直接 `RegisterHotKey` API；衝突走 §8.5 fallback chain（不是權限問題）

→ 因 Windows 沒對等 iOS B perm gate 流程，本檔不畫獨立 B frame；異常路徑歸 B' 變體。

### B' Mic disabled 變體（覆蓋 Idle）

Idle 上覆蓋遮罩卡片：
- `R.windows.mic_disabled_by_system` 訊息文案
- 「打開設定」按鈕 → 深連結 `ms-settings:privacy-microphone`（Win 10/11 deep-link scheme）
- 「重試」按鈕（user 開完設定回來不用重啟 app）

## 螢幕 C — Recording（主視窗版型同 macOS C + tray icon 變色）

主視窗版型同 macOS C（計時器 / waveform / pause-stop / chunk count / trust badge）。Windows delta：

- **Tray icon state 切換**（per SPEC-43 §8.1 + mockup §404）：
  - Idle: `phantom-tray-idle.ico`（Lucide `mic` 16×16 phantom-muted）
  - **Recording: `phantom-tray-working.ico`（綠點）**（mockup 用 phantom-warning 飽和橘；本 wireframe 採 SPEC-43 §8.1 「working」semantics — focus 屬 active task，icon green）
  - Paused: `phantom-tray-idle.ico` + Lucide `mic-off` overlay
  - Error: `phantom-tray-error.ico`（紅點；mic 被搶 / 系統 interrupt）
- **State 切換 debounce 1 秒**（per SPEC-43 §8.1）— 避免 chunk 邊界閃爍
- **沒有鎖屏卡（iOS D / macOS lock screen）** — v0.6.0 Windows 不做 SMTC（per 入口表 v0.7+），鎖屏期間錄音持續但無 OS-level 控制介面

### Windows 獨有：Tray context menu（Recording 期間）

```
┌──────────────────────────────────────────────┐
│ Phantom Mesh · Focus 05:23 / 25:00          │   ← header（灰、不可點、動態；per SPEC-43 §8.2 item 1）
├──────────────────────────────────────────────┤
│ ⏹ Stop & finalize                Ctrl+Shift+S│   ← **Recording 期間最高優先（與 wireframe / mockup invariant 同步）**
│ ⏸ Pause                                      │
├──────────────────────────────────────────────┤
│ Open Phantom Mesh                  Ctrl+O    │
│ Settings...                                  │
└──────────────────────────────────────────────┘
```

**delta vs SPEC-43 §10.2 default tray dropdown**：Recording 中 tray menu **動態 rebuild** — header 文字切到 `focus.tray.header_recording`、Stop 提到首項、其他 capability 項（Quick Log / Start Focus...）灰階 disabled（避撞）。Stop 點下去走 phantom serve `focus_stop` Tauri command，等同 app 內 ⏹。

### Tray icon hover tooltip

`"Focus 05:23 / 25:00 — right-click for controls"`（取 `focus.tray.tooltip_recording`） — 同步主視窗計時器；i18n key per SPEC-05。

## 螢幕 C' — Interrupted sub-state

Windows OS interrupt 來源（跟 iOS 不同）：
- 其他 app 抓 mic（WASAPI exclusive mode；常見 Discord / Teams 切到 PTT）
- 系統 sleep（S3/S4）/ Modern Standby
- 藍牙耳機切換（mic source 換到 BT mic、AudioEndpointVolume API event）
- **Focus Assist activate 期間**（per SPEC-43 §15 — Focus Assist 不阻止錄音、但會折疊 toast）

UX 同 macOS C'：**desktop 無專屬 UI 變體**（waveform 不凍結、計時不停，per hero §Cross-platform invariants line 349）— Windows interrupt 多源自 mic 被搶、狀態不明顯。

**強制系統通知**（per hero invariant line 350）：Recording 中若 OS interrupt 觸發 + 主視窗非 active focus → 必發 ActionCenter toast（見下方螢幕 D'）。

## 螢幕 D — Done notification（ActionCenter toast）

```
┌─────────────────────────────────────────────┐
│  ◯  Phantom Mesh                            │   ← AppLogo（per mockup §423）
│      Focus 25 min · takeaway ready          │   ← title (i18n: `focus.done.title`)
│      第一行 takeaway 取 60 字（row 2 限制）  │   ← body line 1（取 takeaway 60 字截斷）
│                                             │
│      [ 開啟回顧 ]                            │   ← action button → activate main window + route /focus/:id
└─────────────────────────────────────────────┘
```

**Windows delta vs macOS Notification banner**：
- **Persists 到 user dismiss**（macOS NC banner ~5s 自動消）— per hero §Windows 重點 line 263「ActionCenter 通知 persists 比 mac NC banner 友善」
- **進 Action Center 歷史**（user 漏看可回查；macOS NC 也進歷史但 Win 11 ActionCenter UI 更可見）
- **AUMID-anchored**：`com.phantom-mesh.app`（per SPEC-43 §7.1.3 + SPEC-42 §8.5）— MSI 安裝時 shortcut metadata 註冊
- **deep-link launch**：`phantom-mesh://focus/<session_id>` → cold-launch 主視窗 → route to Focus tab takeaway card（per SPEC-43 §9.3 `coach_review_open` 同樣機制）
- **scenario="default"**：被 Focus Assist 折疊可接受（done 不是 urgent）
- **body row 2 ≤ 60 字**（per mockup §547 「Notification body 截字上限」cross-platform 統一）— 比 macOS NC 80 字嚴格

## 螢幕 D' — Interrupted toast（系統強制觸發）

Recording 中 OS interrupt + 主視窗非 active focus 必發（per hero invariant + mockup §433）：

```
┌─────────────────────────────────────────────┐
│  ◯  Phantom Mesh                            │
│      `focus.desktop.interrupt_notif_title`   │   ← title i18n key
│      5:23 / 25:00 · mic 被佔用              │   ← body line 1（dynamic per interrupt 來源）
│      `focus.interrupted.resume_hint`         │   ← body line 2 i18n key
│                                             │
│      [ `focus.desktop.interrupt_notif_action` ]│   ← action → activate main window + invoke stop
└─────────────────────────────────────────────┘
```

**delta vs D**：
- **scenario="urgent"**（per mockup §442）— 穿透 Focus Assist 折疊，避免重要狀態被靜音
- **audio: default**（不是 silent）— interrupt 是異常需 user attention
- 文案三 key 與 macOS / Linux 共用（cross-platform invariant per mockup §553）

## 螢幕 E / F — Finalizing / Done（同 macOS + tray icon 同步 + toast 觸發）

機制同 macOS：E phase 1 (Transcribing) + phase 2 (SummaryGen) → F Done takeaway card 落在 main window Focus tab。

**Windows delta**：
- **Tray icon 同步**：`Recording` → `Finalizing`（同綠點，hover tooltip 變「整理逐字稿...」）→ `Done`（綠點 3 秒後回 idle）
- **Tray menu header 同步**：`focus.finalizing.asr` 訊息（i18n key per hero）
- **D toast 在 Done 那刻 emit**（per Flow 2 sequence，per SPEC-43 §6.3）— 即使主視窗 active 也彈一次（與 macOS focus-suppressed banner 行為不同，Win 不抑制）
- **ASR 全靜音情境（Empty）**：takeaway card 顯示安撫文「本次時段未偵測到語音，已為您記錄時長」+ session 仍寫 events（user opt-in re-record）— **不發 toast**（避免空通知打擾）

## 入口架構決議（per SPEC-43 §10.1 IA + SPEC-21 hero invariants）

| 元素 | Windows 對映 |
|---|---|
| **主視窗 IA**（per SPEC-43 + SPEC-03 sitemap）| Main window 左側 **220px sidebar** 含 Focus tab（per mockup §552 cross-platform lock）|
| **History tab 位置**（per hero invariant lock）| main window 左 sidebar 內的 Focus tab 子畫面 history list 區段（**不是獨立視窗**） |
| **System back** | Windows 沒對應 system back gesture；Alt+Left（browser-history convention）映 React Router `navigate(-1)`；Escape 關閉模態（per SPEC-43 §12.2） |
| **PTT × Timer 互斥** | n/a — Windows Idle 沒 PTT 鈕（per hero macOS / Windows 共識，桌機鍵盤情境）。「Timer 跑中切到 C 螢幕」邏輯保證、無視覺切換需求 |
| **Tray menu 動態 rebuild** | Recording 期間 menu items reorder（Stop 提到首項）+ Quick Log / Start Focus 灰階 disabled（避撞） |
| **Hotkey accelerator 衝突** | per SPEC-43 §8.5 fallback chain：primary fail → `Ctrl+Alt+F` → user capture mode；Settings → Hotkeys tab 顯示警示（per Flow 5 sequence） |

## Cross-platform invariants 對齊（per hero wireframe）

繼承全部 hero invariants（trust badge / Stop ≤ 2 操作 / waveform / chunk count / 計時器顏色 / **desktop 中斷強制系統通知** per line 350）。Windows 額外：

- **Tray icon 必常駐**（不可隱藏；user 想關只能 Settings → General quit phantom serve）— 是 cluster status 唯一 ambient indicator
- **Recording 期間 tray menu 第一項必為 Stop**（per SPEC-43 §8.2 鎖定順序 + mockup §417 「Stop & finalize 最高優先，與 wireframe 同步」）
- **ActionCenter toast 必 AUMID-anchored**（無 AUMID → `R.windows.toast_emit_fail`，per SPEC-42 §8.5）
- **Tray dropdown render < 150ms p95**（per SPEC-43 G1） — wireframe 層保證 menu rebuild 不阻塞 UI thread
- **Toast emit < 500ms p95**（per SPEC-43 G2） — phantom serve 背景 emit 不依賴 webview alive
- **Narrator AutomationName 必填**（per SPEC-43 §12.2 + WCAG 2.2 AA） — 所有 button / icon / tray menu item 都要

## 6 大資料狀態 — Windows 對映表

| 狀態 | Windows 螢幕 / 場景 | 對應 i18n key / mockup |
|---|---|---|
| **理想（Ideal）** | F Done takeaway card 完整 + D ActionCenter toast persists | `focus.done.title` per mockup §561 + §423 toast |
| **空白（Empty）** | History list in Focus tab sidebar（無 session）/ ASR 無語音（session 跑完無 transcript，**不發 toast**）| `focus.empty.history` 共用 / 新加（Windows 獨）安撫文 |
| **極限（Limit）** | C chunk 99+ / F takeaway > 800 字截斷 / toast body row 2 > 60 字截斷 | `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` per mockup §563 + 60 字 toast 上限 per mockup §547 |
| **錯誤（Error）** | B' Mic disabled / D' Interrupted toast / `R.windows.toast_emit_fail` 退化到 in-app banner | `focus.perm.denied` / `focus.interrupted.*` / `focus.desktop.interrupt_notif_*` per mockup §433 |
| **局部（Partial）** | E Finalizing inline `focus.partial.chunk_failed` | per mockup §565 |
| **載入中（Loading）** | E Finalizing + tray icon working green + tray menu header 動態更新 | `focus.finalizing.asr` + tray header 字串更新 |

## 已決（per SPEC-43 lock + Alt-C 決策）

1. ~~Sheet vs 真窗~~ → **已決**：真窗（NSWindow-equivalent；Windows 沒 transient-popover shell affordance，per hero §Windows 重點 line 261）
2. ~~Global shortcut 預設註冊~~ → **已決**：v0.6.0 不預設註冊（避撞 enterprise app，per SPEC-43 §17 Alt-C — `Ctrl+Shift+F` 撞 Outlook find folder；`Win+Shift+F` 留給 v0.7+ user opt-in）
3. ~~Tray menu Stop 位置~~ → **已決**：Recording 期間提到首項（per SPEC-43 §8.2 + mockup §417 「Recording 期間最高優先」）
4. ~~Toast persistent vs 自動消失~~ → **已決**：persistent（per hero §Windows 重點 line 263 + AUMID + Action Center 歷史機制）
5. ~~Mica acrylic 半透明~~ → **已決**：v0.6.0 不做（SPEC-43 §3.2 NG2，留 v0.7+；Tauri 2 binding 未穩 + node-a black flash 踩坑）

## 開放問題（Windows 層面，剩餘）

1. **SystemMediaTransportControls（SMTC）鎖屏控制**（v0.7+）：值得做嗎？SPEC-42 / SPEC-43 都沒列。等 iOS hero usability test 結果 + Tauri 2 SMTC binding 穩定再評估。Linux 也沒對等（per hero §Linux line 296 「不承諾鎖屏控制」）— 桌面三平台一起留 v0.7+。
2. **Focus Assist 折疊行為**：Done toast 用 `scenario="default"` 可接受被 Focus Assist 折疊（user 之後到 Action Center 補看）；但 user research 是否顯示「miss 一次就失約」？需 5-user UX session 量測。
3. **Tray icon Recording 配色**：SPEC-43 §8.1 「working」= 綠點（與 cluster active task 同），但 mockup §404 用 `phantom-warning` 飽和橘（暗示「請小心、正在錄」）— 採哪個語意？本 wireframe 暫採 SPEC-43 綠點，留 mockup 階段確認終值。

→ 互動 timing / 鍵盤焦點 cycle / 動效 token 細節歸 Windows prototype（待補）。

## 下一步

→ 進 [Windows Mockup（待補）] 決定 Fluent design token / icon ID / 終版文案 / tray icon 配色（綠 vs 橘）終值。
