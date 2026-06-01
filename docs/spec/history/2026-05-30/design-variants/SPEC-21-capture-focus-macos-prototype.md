# SPEC-21 Capture Focus — macOS Prototype（原型）

> **Stage 3/3** · [線框稿（macOS）](./SPEC-21-capture-focus-macos-wireframe.md) → [視覺稿（macOS）](./SPEC-21-capture-focus-macos-mockup.md) → 原型（macOS）
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: macOS only。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-prototype.md](./SPEC-21-capture-focus-prototype.md)），本檔只描 macOS-specific 互動行為 — `⌘⇧F` 全域 shortcut / NSStatusItem dropdown timing / TCC prompt async handling / MPNowPlayingInfoCenter remote command / multi-monitor sheet spawn / Notification Center banner click sequence / VoiceOver focus order / Bartender fallback。共用 Nielsen 5 / 6 state / SUS / FSM 鏈接 hero prototype，不重抄。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-40-PLATFORM-macOS-foundations`](../specs/v060-deep-spec/SPEC-40-PLATFORM-macOS-foundations.md) · [`SPEC-41-PLATFORM-macOS-screens-flows`](../specs/v060-deep-spec/SPEC-41-PLATFORM-macOS-screens-flows.md)
> **這份的工作範圍**：把 macOS Mockup 變「可操作」 — 每個 tap target 點下去發生什麼、NSAnimation duration / NSHapticFeedbackManager pattern / async OS callback timing、失敗如何重試。為 multi-monitor + Stage Manager + lid close edge case 準備 walkthrough script + SUS 對齊。

## 為什麼 macOS 要獨立 prototype

iOS hero prototype（488 行）的互動模型建在 `UIViewController push transition` + `UIImpactFeedbackGenerator` + `AVAudioSession.recordPermission` 之上。macOS 互動模型完全不同：

1. **`⌘⇧F` 全域 shortcut** — 從任意 app（含 fullscreen）觸發；註冊 `MASShortcut` / `KeyboardShortcuts` framework，shortcut 衝突的 fallback wizard 是 macOS-only 互動
2. **NSStatusItem dropdown timing** — `NSPopover.show(relativeTo:)` 預設動畫 200ms，但 phantom 要區分 idle / recording 兩種 dropdown 內容，row sequence 不同
3. **TCC mic prompt 不可控** — `AVCaptureDevice.requestAccess(for: .audio)` callback 來時間 OS 排程（100ms–5s 不等），sheet 的 disabled state 要會等
4. **MPNowPlayingInfoCenter** 在 macOS 上行為 ≠ iOS — lock-screen 上 nowPlaying 顯示 + AirPods 中鍵 remote command 走相同 API，但 menu bar app 還要處理 NSStatusItem icon 同步切換
5. **Multi-monitor sheet spawn** — `NSWindow.beginSheet(_:completionHandler:)` 要決定 parent window 在哪個 monitor，per SPEC-41 G3 spawn 規則
6. **Notification Center banner click** — `UNUserNotificationCenter` delegate `didReceive` 要做 deep-link 還原 + 主 window activation sequence
7. **VoiceOver focus order** — `NSAccessibility.focusedUIElement` chain 在 sheet / dropdown / window 三層 presentation 各自獨立
8. **Bartender / Hidden Bar 偵測** — `NSStatusItem.button.window?.isVisible` 輪詢，每 60s check；icon 被擠出 menu bar overflow 區後要 fallback

→ 這 8 點全在 hero prototype 之外，獨立寫一份，工程師讀完直接知道怎麼接 SwiftUI + AppKit + Tauri macOS shell。

## 共用內容鏈接（不重抄）

| 共用區塊 | 鏈接 |
|---|---|
| Nielsen 5 易用性原則總攬 | hero prototype §17-27（同義照搬，macOS 補在各螢幕節） |
| 6 大資料狀態速查 + i18n 鏈接 | hero prototype §29-40 + macOS mockup §293-303 |
| 9-state FSM 主骨架 | hero prototype §40-62（macOS 共用，sub-state 差異本檔補） |
| SUS 10 題目對齊 | hero prototype §412-432（macOS 數字本檔末段補） |
| Walkthrough 紀錄方式 | hero prototype §476-481（macOS Stage Manager + multi-monitor 變體） |

## macOS 入口 timing 概觀（v0.6.0 ship 三條路徑）

| 入口 | 觸發 timing | 視覺進場 | 對應螢幕 |
|---|---|---|---|
| `⌘⇧F` 全域 shortcut | NSEvent global monitor 收到，~20ms 內 dispatch | NSWindow sheet 250ms slide-down ease-out（macOS 內建 `beginSheet` 動畫） | A |
| NSStatusItem dropdown → 「⏱ 開始焦點時段… ⌘⇧F」row | click status item → popover 200ms ease-out 出現 → click row → 50ms haptic + popover dismiss 100ms → sheet 250ms | dropdown 先收 → sheet 開（兩段共 ~350ms） | A |
| Main settings window Focus 區 → 按鈕 | 0ms 內 button press visual → sheet 250ms attach 到 settings window 為 parent | sheet 滑入（focus_steal=true 阻塞 settings window 操作） | A |

**Reduce Motion 開啟**：三條路徑都跳過動畫（sheet 直接 appear，popover 直接 visible），per SPEC-41 §12.2。

---

## 螢幕 A — Focus Start Sheet

### Nielsen 5 對應（macOS-specific）

- **Learnability**：sheet 標題直白「開始焦點時段」（取 mockup `focus.macos.sheet_title`）；duration radio 三檔 + tag input + trust badge 一頁看完，無 onboarding；`⌘⇧F` 走 `KeyboardShortcuts` framework 註冊，第一次用 user 從 menu bar dropdown 看到 row 旁有 `⌘⇧F` 提示自然學會
- **Efficiency**：`⌘⇧F` 從任何 app 直接呼出，**不用切 app context**（iOS 必開 app）；Enter 鍵直接觸發 [start]、Esc 取消 — keyboard-only workflow 一條 path 跑完
- **Memorability**：上次 duration 持久化於 `@AppStorage("focus.last_duration_min")`（同 iOS），第二次開 sheet 同一 radio 預選；tag input placeholder 取上次 tag（per mockup §60-65）
- **Errors**：TCC 未授 → [start] disabled + inline hint「需開麥克風權限 [open settings]」（per mockup §78），不會錯到「按了沒事」；shortcut 衝突 → wizard 提示自訂
- **Satisfaction**：trust badge 點下展開 `TrustExplainerView`（modal sheet 内疊）— 不會切走 user context

### 6 大資料狀態

| 狀態 | UI 表現（互動行為） |
|---|---|
| 理想 | radio 三檔可選；tag input 可輸入；trust badge 顯示；[start] enabled |
| 空白 | n/a（sheet 不該空） |
| 極限 | custom radio 選中時 num input clamp 5–180；超過 → input shake 240ms（10pt × 3 cycles × 80ms）+ NSHapticFeedbackManager `.alignment` pattern（macOS 對等 iOS `warning`）+ inline `focus.limit.max_duration_hint`；tag input ≥ 64 char → 不再接受輸入 + caret blink 停一拍 |
| 錯誤 | TCC 未授 → [start] disabled (`overlay-disabled-40`) + inline hint phantom-warning「需開麥克風權限 [open settings]」；麥克風硬體不存在（無 input device） → [start] disabled + `focus.err.no_mic` toast |
| 局部 | n/a（A 螢幕無 partial 概念） |
| 載入中 | first-time TCC prompt 等 user 回應 → [start] 替換 16pt spinner（per mockup §79）+ 整 sheet 進 `requesting` 微 state、其他控件 disabled |

### Tap targets（每個按下去發生什麼）

| Target | 動作 / Timing |
|---|---|
| `⌘⇧F` global shortcut | `KeyboardShortcuts.onKeyDown(for: .focusStart)` callback 觸發；20ms 內 dispatch；sheet 250ms slide-down；若已在 Recording state → ignore（per FSM Idle-only entry）+ NSStatusItem dropdown blink 一下提示 |
| 點 NSStatusItem | NSPopover.show(relativeTo:) 200ms ease-out；箭頭尖端對 status item center；無 haptic（macOS menu bar click 慣例無觸覺反饋） |
| Dropdown 「⏱ 開始焦點時段… ⌘⇧F」row | press 8ms 視覺回饋（bg `phantom-primary @ 24%`）→ release → popover 100ms dismiss → sheet 250ms attach；總共 ~350ms；focus 自動跳 sheet 第一 radio |
| Dropdown 「⚙ 設定…」row | 同上 timing 但 push `MainSettingsWindow`（720×640pt）非 sheet；NSWindow `makeKeyAndOrderFront` 預設 fade-in 100ms |
| Sheet radio `○ 25 分鐘 Pomodoro` | NSButton click → 0ms 內 selected dot 視覺切換；`@AppStorage("focus.last_duration_min")` 更新；無 haptic（macOS radio 慣例無觸覺） |
| Sheet radio `◉ 自訂 [30] 分鐘` | 同上 + custom NSTextField 自動 focus（caret blink）；按↑↓ 鍵 stepper +/-5；超界觸發 Limit state |
| Sheet tag input | NSTextField focus 0ms；輸入 64 char limit reached → caret 停 80ms 提示 |
| Sheet `[取消]` | 250ms slide-up dismiss；無 haptic；focus 還 parent window；無 confirm（sheet 無未存資料） |
| Sheet `[開始]` (first time, TCC 未授) | (1) 0ms button press visual；(2) 觸發 `AVCaptureDevice.requestAccess(for: .audio)`；(3) sheet 進 `requesting` 微 state — [start] 替換 spinner、其他 control disabled；(4) 等 OS callback（async，100ms–5s）；(5) callback 來 → granted 走 C / denied 走 B' |
| Sheet `[開始]` (TCC 已授) | (1) 0ms press visual；(2) NSHapticFeedbackManager `.generic` pattern（對等 iOS `medium`）；(3) AudioRecorder.open + isRecording=true；(4) sheet 250ms slide-up dismiss；(5) NSStatusItem icon 200ms 內切 `mic.fill` + 紅點 overlay；(6) 不 push 任何 window — user 回到原 app context |
| Trust badge tap | 0ms expand inline panel（per mockup §60-65 設計），不開新 window；ESC 收合；無 haptic |
| Sheet 右上 `[✕]` | 同 `[取消]` |
| Esc 鍵 | 同 `[取消]`（per SPEC-41 §7.4 esc_dismisses=true） |
| Enter 鍵 | 同 `[開始]`（NSButton.isDefault=true）|

### Animations / Timings

- Sheet 進場：NSWindow `beginSheet` 預設 250ms ease-out slide-down（macOS Sonoma 14.x / Sequoia 15.x 一致），Reduce Motion 跳過
- Sheet 退場：250ms ease-in slide-up；focus 還 parent
- Radio selected dot 切換：0ms（macOS 慣例 radio 無動畫）
- TextField focus ring：100ms fade-in（macOS 內建）
- `requesting` 微 state spinner：60°/s 等速旋轉，不變速
- TCC prompt 等待中無 sheet 動畫（凍結，等 callback）

### Failure paths（macOS-specific）

- **`⌘⇧F` shortcut 註冊衝突**（user 已綁此鍵給 Xcode / Raycast / etc）：first-launch wizard 偵測到 `KeyboardShortcuts.System.isReserved` 或 register fail → 跳 onboarding modal「⌘⇧F 已被 _ 占用，請重綁或繼續用 menu bar 觸發」+ NSAlert with `[Change Shortcut]` / `[Skip for now]` button（per SPEC-40 §3.1 G5 caveat）
- **TCC denied 後再開 sheet**：[start] 永遠 disabled + hint「[打開系統設定]」inline；click hint → `NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!)`；user 回 phantom 後 sheet 仍開、但需 user 重 click [start] 觸發 re-check（**TCC 授權變更不會主動 broadcast，要靠 sheet open / status item click trigger re-check**）
- **麥克風硬體不存在**（如 Mac Studio 無內建 mic 且未接外接）：`AVCaptureDevice.default(for: .audio) == nil` → [start] disabled + toast `focus.err.no_mic`（FOCUS-002）；不 trigger TCC prompt
- **Sheet parent window 被 close 中觸發 ⌘⇧F**：fallback 用 NSStatusItem 所在 monitor center spawn 一個 invisible utility window 當 parent（per SPEC-41 G3 multi-monitor rule）

### Walkthrough script（usability test：「請從任何 app 用快捷鍵開始 25 分鐘 focus」）

1. user 在 Safari / VSCode 等 app 按 `⌘⇧F` → 預期 sheet 250ms 滑下覆蓋當前 window
2. **觀察點 1**：user 是否 grasp「sheet 浮在當前 app 上，但 menu bar 仍可切」？若 30%+ user 試圖切 menu bar 卻被卡，要考慮 sheet 開時是否該 fade overlay 強調 modal 性
3. user 預期 25 min 已選 → 按 Enter / click [開始]
4. 首次：TCC prompt 跳，user 按 OK
5. **觀察點 2**：sheet dismiss 後 user 看不到 Recording UI（NSStatusItem 變色是唯一線索）→ 是否會問「有在錄嗎」？若 50%+ user 反問，要強化 first-time toast 引導「看 menu bar 紅點」

---

## 螢幕 B — TCC Microphone Prompt（OS 渲染）

不可自訂版面。phantom 唯一控字串：

```
Info.plist NSMicrophoneUsageDescription =
  "Phantom Mesh 在你開始焦點時段時錄音，全程在本機 ASR 轉寫，不上傳雲端。"
```

### Tap targets

| Target | 動作 / Timing |
|---|---|
| `Don't Allow` | OS 寫入 TCC.db 為 denied；callback `granted=false`；sheet 退出 `requesting` 微 state → 進 B' Denied 覆蓋 |
| `OK` | OS 寫入 TCC.db 為 granted；callback `granted=true`；sheet 自動觸發 [開始] 流程；NSStatusItem icon 切 `mic.fill` |

### Async timing 處理（macOS-specific 重點）

- `AVCaptureDevice.requestAccess(for: .audio)` callback 來時間：實測 macOS Sonoma 14.x ~150-300ms / Sequoia 15.x ~200-500ms / 偶發 ≥ 2s（user 切走又切回）
- sheet 進 `requesting` 微 state 等待中：**不允許 user 操作其他 control**（spinner 顯示）；timeout 上限 10s — 超時顯示「OS 沒回應，請重試」+ [重試] button（觸發 `AVCaptureDevice.authorizationStatus(for: .audio)` 同步 check，多半已被 user 在 prompt 上 dismiss）
- 若 user 切到別 app 又切回（TCC prompt 仍掛著） → sheet 保持 `requesting` 不變、等 callback

---

## 螢幕 B' — TCC Denied（覆蓋 Sheet）

per mockup §107-123 卡片視覺。互動 spec：

### Tap targets

| Target | 動作 / Timing |
|---|---|
| `[打開系統設定]` | `NSWorkspace.shared.open(URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")!)`；macOS Sonoma+ 會直接深連到 Microphone 區段；無 confirm；無 haptic（OS 切 app 過渡接管） |
| `[取消]` | 250ms fade-out 卡片；sheet 仍開但 [start] 仍 disabled；user 可再按 [取消] 關 sheet |
| 卡片外 / Esc | Esc 同 `[取消]`；卡片外 click 無效（modal） |

### Recovery sequence（macOS-specific）

1. user 點 [打開系統設定] → System Settings.app 開啟（macOS 控制 transition，~500ms）
2. user 在 Privacy & Security → Microphone 把 phantom-mesh toggle 打開
3. user 切回 Phantom Mesh（cmd+tab / dock click）
4. sheet 在前 → user 重按 [開始]（**sheet 不會自動偵測權限變更**，因 TCC 變更不 broadcast；我們 fallback：每次 sheet `viewDidAppear` / status item click 都 sync check `AVCaptureDevice.authorizationStatus`，若狀態變 → 更新 [start] enable）
5. 若 user 按 [開始] 此時權限已開 → 直接走 C（不再彈 prompt）

### Failure paths

- user 在 System Settings 把 toggle 打開但不切回 phantom → 我們無從得知；持續 disabled 直到 user 回來
- user 把 toggle 開了又關（折騰中） → 每次回 sheet sync check，UI 反映最後狀態

---

## 螢幕 C — Recording（無 dedicated window；NSStatusItem + dropdown）

**核心差別 vs iOS**：macOS 沒有 Recording 主畫面 — Recording 中 sheet 已 dismiss，user 工作 context（Safari / IDE / Figma）保留焦點。Recording 的唯一視覺錨點是 **NSStatusItem 變色 + 點開 dropdown 看詳情**。

### Nielsen 5 對應（macOS-specific）

- **Learnability**：first-time 完成 sheet [開始] 後跳 toast「正在錄音 — 看 menu bar 紅點」（持續 4s），引導 user 認識 NSStatusItem；之後不再顯示
- **Efficiency**：Stop ≤ 2 操作（click status item + click [⏹ 停止並收工]）— per cross-platform invariant；`⌘⇧F` 二次按下不再開新 session（per FSM Idle-only entry，已 Recording 則彈 toast「焦點時段進行中」）
- **Memorability**：NSStatusItem 永遠在右上（per SPEC-40 G4 24/7），位置不變
- **Errors**：被 Zoom 搶 mic → AVAudioSession interrupt → Interrupted sub-state + Notification Center banner（per mockup §232）必發
- **Satisfaction**：dropdown 內 mini waveform 60pt height 即時跳 — user 點開就看到「真的在錄」

### 6 大資料狀態

| 狀態 | UI 表現 |
|---|---|
| 理想 | NSStatusItem `mic.fill` + 紅點；dropdown 內計時器 + mini waveform + chunk count；[停止] + [暫停] 兩 row |
| 空白 | n/a |
| 極限 | chunk ≥ 100 → dropdown row 文字「📁 已落地 chunk: 99+」（`focus.limit.chunk_overflow`）；計時器 50:00/50:00 達標 → 自動 stop 切 Finalizing，dropdown 內最後一禎顯閃紅 1s 後切 |
| 錯誤 | mic 被 Zoom/FaceTime 搶 → AVAudioSession interrupt → C' 變體 + Notification Center banner 必發；lid close → IOPMSystemPowerStateChange + Interrupted sub-state |
| 局部 | n/a（per chunk 失敗在 Finalizing 顯示） |
| 載入中 | n/a |

### Tap targets

| Target | 動作 / Timing |
|---|---|
| NSStatusItem click（recording 中） | NSPopover 200ms ease-out 展開 dropdown；dropdown 內容切 Recording 變體（per mockup §168-189）；user 工作 app 失去 key window 但 **不關閉**（per SPEC-41 G7 transient popover focus_steal=false）|
| Dropdown `[⏹ 停止並收工]` row | label 取 `focus.btn.stop_finalize`（desktop 用長版，per mockup §181）。(1) 8ms press visual（bg `phantom-danger @ 24%`）；(2) NSHapticFeedbackManager `.levelChange` pattern（對等 iOS `heavy`）；(3) AudioRecorder.close + flush；(4) popover 100ms dismiss；(5) NSStatusItem icon 切 Finalizing 變體（旋轉 dot overlay）；(6) D toast in-app HUD 出現 |
| Dropdown `[⏸ 暫停]` row | label 取 `focus.btn.pause`。(1) press visual；(2) NSHapticFeedbackManager `.alignment`；(3) AVAudioSession 設 inactive；(4) NSStatusItem icon 切 `mic.slash` paused 變體；(5) dropdown row 文字 [⏸ 暫停] 變 [▶ 繼續]（label `focus.btn.resume`）；(6) 計時器停 + waveform 凍結（per mockup §142 paused icon spec） |
| Dropdown `[▶ 繼續]` row | 反向：session 重啟、waveform 重跑、計時繼續、icon 切回 `mic.fill` 紅點、row 變回 [⏸ 暫停]；haptic `.alignment` |
| Dropdown 計時器 / waveform 區（純資訊） | 不可 click；tap 無動作 |
| Dropdown chunk count `📁 已落地 chunk: {n}` | tap 無動作（純資訊）；99 → 100 切顯 `99+` 不刷新動畫（per mockup §300 min-width 鎖死） |
| Dropdown 外 click | popover 100ms ease-out dismiss；user app 重 key window |
| Esc 鍵（dropdown 開時） | 同 dropdown 外 click |
| NSStatusItem right-click | 不支援（macOS 慣例 status item 左鍵 = popover，右鍵 = context menu 但 phantom v0.6 不做） |

### Lock-screen / AirPods remote command（macOS-specific）

macOS 沒 iOS-style lock-screen，但有：
1. **macOS lock-screen（screensaver 起來 / Cmd+Ctrl+Q）** — `MPNowPlayingInfoCenter.default()` 在 macOS 14+ 會在 lock-screen 上顯 nowPlaying 卡片（同 iOS API，但 macOS 渲染版面）
2. **AirPods 中鍵 / Touch ID 鍵盤 media key** — 觸發 `MPRemoteCommandCenter` callback

註冊 spec：
```swift
let center = MPRemoteCommandCenter.shared()
center.pauseCommand.addTarget { _ in handlePause(); return .success }
center.playCommand.addTarget { _ in handleResume(); return .success }
center.stopCommand.addTarget { _ in handleStop(); return .success }
```

| Target | 動作 |
|---|---|
| Lock-screen `pause` icon | 等同 dropdown `[⏸ 暫停]`（remote command）；NSStatusItem icon 同步切；解鎖回原 app context |
| Lock-screen `play` icon（暫停狀態） | 等同 `[▶ 繼續]` |
| Lock-screen `stop` icon | 等同 `[⏹ 停止並收工]`；觸發 finalize；haptic 不可控（OS 處理） |
| AirPods 中鍵單按 | 等同 `pauseCommand`（macOS 預設 mapping） |
| AirPods 中鍵長按 | 預設 Siri，不攔截 |
| 鍵盤 Touch ID media key (F8) | 等同 `playPauseToggle`（macOS 預設） |

### Animations / Timings

- NSStatusItem icon 切換：**0ms 直接換 NSImage**（per mockup §148 Apple HIG 慣例 — menu bar 動畫干擾）
- 紅點 overlay 出現：跟 icon 同 frame 換上（無單獨 fade）
- NSPopover 展開：200ms ease-out（macOS 內建）；Reduce Motion 跳過
- NSPopover 收起：100ms ease-in
- Waveform refresh：60fps；audio buffer 1024 samples 一禎；柱數 / 高度 / 色 token per mockup §174
- 計時器數字：每秒 update，無 animation
- chunk +1 flash：dropdown 已開時，chunk row 數字 200ms scale 1→1.1→1 spring（damping 0.6）

### Failure paths（macOS-specific）

- **mic 被 Zoom/FaceTime 搶**：`AVCaptureSession.runtimeErrorNotification` + `AVAudioSession` 等效 callback → 進 Interrupted sub-state；30s 寬限 per wireframe FSM；超時 finalize + `interrupted=true` 標記；**Notification Center banner 必發**（user 不在 phantom dropdown 開啟狀態時，per mockup §232）
- **lid close（外接顯示器以外的情境）**：macOS 觸發 sleep → `NSWorkspace.willSleepNotification` 觸發；如果 user 在 settings 開了「Recording 中防止電腦休眠」toggle → 走 `IOPMAssertionCreateWithName(kIOPMAssertionTypePreventUserIdleSystemSleep)`（per wireframe §212）保住；否則進 Interrupted → 30s 寬限超時 finalize
- **藍牙耳機切換**：AirPods 從 iPhone 切到 Mac → mic source 變 → `AVAudioEngine` 拋 routeChange notification → 我們重啟 audio session（200ms 內），通常 user 無感；若失敗 → Interrupted + banner
- **NSStatusItem 被 Bartender / Hidden Bar 隱藏 ≥ 60s** → 每 60s 輪詢 `statusItem.button?.window?.isVisible`；偵測到隱藏 → 發 `MACOS_HIDDEN_BAR_DETECTED` 系統通知（per SPEC-41 §11），banner 文字「Phantom Mesh 正在錄音但 menu bar icon 被隱藏 — 請打開 Bartender 設定讓 phantom-mesh 一直顯示」+ deep-link [打開設定]
- **macOS Sequoia 15+ Stage Manager 內**：NSPopover 仍 attach status item OK；user 切 Stage Manager group 時 popover 自動 dismiss（被 OS 收掉），重新點 status item 即可再開
- **存空間滿**（chunk encrypt 寫失敗）：toast `focus.err.disk_full` + 切 Finalizing（保留已落地 chunk）

### Walkthrough script（usability test：「錄音 30 秒後從 menu bar 暫停、繼續、停止」）

1. 從 sheet [開始] 完 → user 工作 app 仍前景，user 看到 NSStatusItem 變紅點
2. **觀察點 1**：user 是否能 30 秒內找到 NSStatusItem 並 click？若 30%+ user 找不到，重考慮 first-time 引導 toast
3. user click NSStatusItem → dropdown 展開
4. **觀察點 2**：user 是否認出 stop / pause 順序（stop 在 pause 上）？若混淆 → 加 row separator 或 visual hierarchy
5. user 按暫停 → NSStatusItem icon 切 `mic.slash` → 預期 user 看出 paused 狀態
6. **觀察點 3**：user 是否能從 menu bar icon 一眼分辨 recording / paused / idle 三態？若色盲 user 看不出 → 加形狀差異（已 spec：mic / mic.fill / mic.slash 三個 SF Symbol 形狀本身有別）
7. user 按繼續 → 按停止 → 切 Finalizing toast

---

## 螢幕 C' — Interrupted（強制 Notification Center banner）

### 觸發來源（macOS-specific）

- **mic 被 Zoom / FaceTime / 其他 app 搶**：`AVCaptureSession.wasInterrupted` + `AVAudioSession` 等效 callback
- **系統 sleep（lid close / Energy Saver / Cmd+Ctrl+Q）**：`NSWorkspace.willSleepNotification`
- **藍牙耳機切換**：`AVAudioEngine` routeChange notification（重試失敗時才進 Interrupted）

### Banner 互動（OS 渲染，phantom 設 UNNotificationContent）

per mockup §234-247 content spec。互動：

| Target | 動作 / Timing |
|---|---|
| Banner click（整 banner） | `UNUserNotificationCenter.delegate.userNotificationCenter(_:didReceive:withCompletionHandler:)` 觸發 `UNNotificationDefaultActionIdentifier`；handler 執行：(1) `NSApp.activate(ignoringOtherApps: true)` 切前景；(2) NSStatusItem dropdown 自動 show；(3) dropdown 內容是 Interrupted 變體（顯 `focus.interrupted.{reason}` + resume hint）；(4) 不開 sheet（user 已經在 session 中） |
| Banner [開啟並停止] action button | per mockup §244 `focus.desktop.interrupt_notif_action`。`UNNotificationAction` handler 執行：(1) `NSApp.activate`；(2) 觸發等同 `[⏹ 停止並收工]` 流程 → 切 Finalizing |
| Banner dismiss（swipe / ignore） | 留在 Notification Center history（≤ 5 條 throttle，per SPEC-41 §11）；session 繼續等 resume 或超時 finalize |

### Async timing

- AVCaptureSession.wasInterrupted callback → ~50ms 內 fire banner（搶 mic 場景）
- NSWorkspace.willSleepNotification → ~100ms 內 fire banner，但 banner 可能來不及顯示（系統已 sleep）— 醒來後 user 會在 Notification Center history 看到
- 30s 寬限後超時 finalize：發第二條 banner 「focus 已自動停止」+ takeaway ready（per mockup §240 reason 動態切）

### Failure paths

- **Do Not Disturb 開啟**：banner 不顯示，但仍寫 Notification Center history；session 仍 finalize；user 可從 history 看到（per SPEC-41 §11 throttle 規則仍 apply）
- **user 在 fullscreen app**（如 Final Cut）：macOS Sequoia banner 顯示但 sound 可能被壓；session 中斷邏輯不受影響

---

## 螢幕 D / E — Finalizing（in-app toast）+ Done（Notification Center banner）

### Nielsen 5 對應

- Learnability：D toast 文字直白 `focus.finalizing.asr`「整理逐字稿 (2/5)」+ `focus.finalizing.llm`「產生 takeaway 中…」
- Efficiency：D 是 HUD-style toast 出現在 NSStatusItem dropdown 內（user 點開可看）或 settings window 角落；非 Notification Center（因 Finalizing 短 < 30s）
- Memorability：D toast 樣式跟 Recording dropdown 一致（同 popover）
- Errors：兩條 path 各有失敗訊息 + 重試按鈕；FSM state 失敗 inline 顯示，不阻斷
- Satisfaction：Done 用 Notification Center banner 通知（system-level，user 在其他 app 也看得到）

### D — Finalizing toast（in-app HUD）

per wireframe §157-180。互動：

| Target | 動作 |
|---|---|
| NSStatusItem click（Finalizing 中） | dropdown 展開，內容是 Finalizing 變體：頂部 spinner row「整理逐字稿 (2/5)」+ 下方「[取消並先看逐字稿]」row（caption 字級，小字降低誤觸） |
| Dropdown `[取消並先看逐字稿]` row | label 取 `focus.btn.cancel_show_transcript`。中斷 LLM call；用 stitched transcript 寫 row（takeaway = ""）；切 E → F 直接打開 Takeaway window |
| Dropdown `[重試 ASR]` row（錯誤態） | label 取 `focus.btn.retry_asr`。重跑失敗 chunk；若 user 開了 cloud fallback → Groq Whisper |
| Dropdown `[先用空白 transcript 跑 LLM]` row（全 ASR 掛時） | label 取 `focus.btn.use_empty_transcript`。LLM 跑空字串 → takeaway = "(無 audio 可分析)" |

### E → F — Done banner + Takeaway window

per mockup §213-230。互動：

| Target | 動作 / Timing |
|---|---|
| Done banner click（整 banner） | (1) `UNNotificationDefaultActionIdentifier` handler；(2) `NSApp.activate(ignoringOtherApps: true)` ~100ms 切前景；(3) F Takeaway NSWindow 開啟（`makeKeyAndOrderFront`），spawn 在 NSStatusItem 所在 monitor center（per SPEC-41 G3 + §8 state machine `follow_menu_bar_icon`）；(4) F window 進場 fade-in 100ms；(5) NSStatusItem icon 切回 idle `mic` mono |
| Done banner [dismiss] | 留在 Notification Center history（≤ 5 條 throttle）；user 後續從 history click 也走相同 handler |
| F Window 不開 banner（user 在 phantom 主 window active 時） | per hero invariant L350 desktop interrupted 才強制 banner；Done 只在主 window 非 active 時發 banner，main window active → 直接 F window switch tab focus 動 + in-app micro toast「takeaway 已完成」 |

### F window — Takeaway tap targets

per mockup §249-278。互動：

| Target | 動作 / Timing |
|---|---|
| `[看完整稿]` button | label 取 `focus.done.view_full`。push `TranscriptView` 在同 window 內（不是新 window），切換動畫 200ms cross-fade |
| `[新 session]` button | label 取 `focus.done.new_session`。(1) F window 不關；(2) 觸發 ⌘⇧F 等效 → A sheet 開啟 attach 到 F window 為 parent；250ms slide-down |
| `[+ 新焦點時段]` sidebar 底部 button | 同 `[新 session]` |
| Sidebar 任一 history row | click → 0ms 換 main 區內容為該 session 的 takeaway；row bg 切 selected state；keyboard ↑↓ 切換亦支援 |
| F window 關閉（`⌘W` 或 close button） | NSWindow close；位置 size persist 到 `macos_window_state.json`（per SPEC-41 §8 state machine） |
| 截斷 takeaway 點「看完整摘要」CTA | 取 `focus.limit.view_full_takeaway`。同 `[看完整稿]` 流程（不在原地展開，per hero prototype §329） |

### Animations / Timings

- D toast 進場：popover 展開 200ms（Recording dropdown 內 row 切換）
- E → F transition：banner click → window 開 100ms fade-in；success icon 從 0→1 scale spring (damping 0.6, response 0.4)
- F window 切 history row：bg color transition 100ms ease-out
- Success haptic：NSHapticFeedbackManager `.levelChange` 雙短脈衝（對等 iOS `success` 雙震）— **但 macOS 觸覺反饋只在 Force Touch trackpad / Magic Mouse 上有效**；user 用一般 USB mouse 收不到，所以視覺回饋（icon scale spring）一定要在

### Failure paths

- **Multi-monitor F window 拔螢幕後變孤兒**：偵測 `NSScreen.screens` change → 自動搬到 primary monitor center（per SPEC-41 §18.1）
- **Notification 被 DND 壓**：F window 不彈出（user 不會被打斷）；user 從 NSStatusItem dropdown 看「上次 takeaway」row 點開可達
- **takeaway 是空字串**（user 取消 LLM 或 LLM 全掛）：F window 顯示 `focus.empty.no_takeaway` + `[重跑摘要]` button（取 `focus.btn.retry_summary`）

---

## 跨螢幕互動

### Interruption Flow（macOS 變體）

```
Recording ──[AVCaptureSession.wasInterrupted / willSleep / routeChange-fail]──> Interrupted sub-state
              ↓
   NSStatusItem icon 切 `mic.slash`（paused 變體）+ 計時暫停 + waveform 凍結
              ↓
   Notification Center banner fire（sound=default + action button）— 強制
              ↓
   [.ended received within wireframe FSM 寬限 30s]
              ↓
   AVCaptureSession 重啟 → Recording 繼續；NSStatusItem 切回 `mic.fill`
              ↓
   [.ended NOT received within 30s]
              ↓
   強制 finalize（interrupted=true 標記）→ D toast → E → F window
```

### Multi-monitor spawn 行為總覽

per SPEC-41 G3 + wireframe §183-192：

| Window 類 | Spawn 行為 | Prototype 互動細節 |
|---|---|---|
| A. Focus start sheet | `follow_parent_window` | `⌘⇧F` 純 global trigger 無 parent → NSStatusItem 所在 monitor center；從 settings window 觸發 → attach settings window 同 monitor |
| C dropdown (NSPopover) | `attached_to_status_item` | 永遠跟 NSStatusItem；用 `NSPopover.show(relativeTo: button.bounds, of: button, preferredEdge: .minY)` |
| F. Takeaway window | `follow_menu_bar_icon` | 開啟時 spawn NSStatusItem monitor center；user 拖到別 monitor 後 size persist per-monitor（per `macos_window_state.json`） |
| Notification banner | OS 控制 | user 系統設定決定（通常 NSStatusItem 同 monitor） |

### VoiceOver focus order

per SPEC-41 §12.2 + mockup §85-91 / §131-134 / §200-205 / §274-278。

- **Sheet open**：VO focus 自動跳 sheet title「開始焦點時段對話框」→ Tab/VO+→ duration radio group → tag input → trust badge → [取消] → [開始]
- **Dropdown open**（Idle）：VO focus 跳「Phantom Mesh 焦點選單」→ 「開始焦點時段… ⌘⇧F」→ 「設定…」→ 「關於 Phantom Mesh」
- **Dropdown open**（Recording）：VO focus 跳「焦點時段中，已錄 _ 分 _ 秒」→ 「停止並收工焦點時段」→ 「暫停焦點時段」
- **F window open**：VO focus 跳 sidebar「最近 10 個焦點時段，列表」→ Tab/VO+→ main card「焦點時段成果摘要」→「看完整稿」→「新 session」

### Back / Esc / ⌘W 行為

| 螢幕 | 操作 |
|---|---|
| Sheet A | Esc 關 sheet；⌘W 同 Esc；無 confirm |
| Dropdown C | Esc 關 popover；click outside 同；session 仍 recording 不變 |
| Interrupted banner | Esc 不 apply（OS 控制）；只能 click action / swipe / wait timeout |
| F window | ⌘W 關 window；size persist；user 可從 NSStatusItem → 設定 → Focus tab 重開 |

### App 進背景 / 回前景（macOS-specific）

- Recording → 背景（user cmd+tab 切 Safari）：phantom 仍跑、NSStatusItem 仍紅點、recording continues；無 UI change（per macOS multi-app 慣例）
- Finalizing → 背景：whisper.cpp 本機跑、不需 background task；user 切回 phantom 主 window 可見 D toast；切走時 Done banner 接管通知
- F window 關閉但 phantom app 還跑：NSStatusItem 仍 idle 顯示；user 重 click → dropdown「上次 takeaway」row 可重開 F window

---

## 通用 Empty / Maximum / Error — 互動補充（視覺 spec 全在 mockup）

### Empty state（F window sidebar 首次 0 session）

- 文案取 `focus.empty.history`（「還沒有 focus session — 開始第一段就會顯示在這」）
- sidebar 中央 SVG illustration 96pt phantom-muted + 文字 + `[+ 新焦點時段]` button（取 `focus.empty.go_to_focus` 的 macOS 變體 — 直接觸發 ⌘⇧F 等效 sheet open）
- haptic 無

### Maximum state（custom duration 超 180）

- user 在 sheet custom radio input 輸入 200 → input blur 時 clamp 回 180 + shake animation（10pt × 3 cycles × 80ms 共 240ms）
- NSHapticFeedbackManager `.alignment` pattern（對等 iOS `warning`）
- inline hint phantom-warning「最長 180 分鐘」(`focus.limit.max_duration_hint`)
- 下限 5 min 同（clamp + shake + hint）

### Error state（global error toast / alert）

- 觸發時機：任何 sheet-level error（TCC denied / mic 不存在 / disk full）
- 視覺由 mockup invariant 定（dark mode dialog 或 inline toast）
- Auto-dismiss after 6s（toast） / NSAlert 由 user 點 OK 關閉
- 無 haptic（避免 error 連震）；多個 error 排隊不疊加

---

## SUS（System Usability Scale）macOS 預期分布

繼承 hero prototype §412-432 全部 10 題框架，macOS 預期分布差異：

| 題目（簡寫） | macOS 預期評分 | 設計依據 / 風險（macOS-specific） |
|---|---|---|
| 1. 想常用 | 4–5 | `⌘⇧F` 全域 shortcut 非常 sticky；風險：與 user 既有 shortcut 衝突 |
| 2. 不必要的複雜 | 1–2 | 三條入口（shortcut / status item / settings）但都到同 sheet；風險：first-time user 不知有 shortcut |
| 3. 容易使用 | 4–5 | sheet 直白；風險：NSStatusItem icon recording 中 user 可能找不到（Bartender 隱藏） |
| 4. 需要技術支援 | 1–2 | TCC 拒絕 → 系統設定 deep-link 直達；風險：user 不熟 macOS Privacy 區段 |
| 5. 各功能整合度高 | 4–5 | sheet / popover / window / banner 四層 presentation 各司其職 |
| 6. 太多不一致 | 1–3 | row sequence stop > pause 與 Windows tray 一致；風險：iOS user 切過來會找 PTT 找不到 |
| 7. 多數人能很快學會 | 4–5 | 全 keyboard-only 可完成 session；風險：mouse-only user 三條入口未必都嘗試 |
| 8. 使用不靈活 | 1–2 | shortcut 可自訂、多 monitor support、size persist；風險：custom duration 上限 180 |
| 9. 使用上有信心 | 4–5 | NSStatusItem 24/7 顯示給安全感；風險：Bartender 隱藏時失錨 |
| 10. 學了很多才能用 | 1–2 | `⌘⇧F` 一次就學會 |

**目標 SUS：72–85**（macOS 平均 +5 因 keyboard-driven workflow 對 desktop user 親和）。**若 < 72，優先檢討**：
1. NSStatusItem icon 三態（idle / recording / paused）user 識別率（特別色盲 user）
2. `⌘⇧F` shortcut 衝突 fallback wizard 友善度
3. Multi-monitor F window spawn 位置 user 預期是否符合
4. Bartender 隱藏 fallback 通知是否被 user 注意

---

## 開放問題（prototype 層面，macOS-specific）

1. **`⌘⇧F` 連按二下跳過 sheet 直接 25min fast-path**（共用 hero §開放 Q1）：NSEvent global monitor 拿 double-tap pattern 後可實作；user 預期符合度未測，傾向 v0.7+ 加（先確保 sheet 流程穩）
2. **Recording 中 IOPMAssertion keep-awake 是否預設 on**：目前預設 off（避免電池焦慮），user 在 settings opt-in；若 usability test 30%+ user 抱怨 lid close 中斷錄音 → 考慮預設 on（per wireframe §212）
3. **NSStatusItem Paused 用 `mic.slash` 還是 `mic.fill` + amber 點**：共用 mockup §313 開放 Q1；prototype 階段測 user 識別率（recording vs paused vs idle 三態）
4. **Finalizing icon 旋轉 dot 動畫尺寸**：1.5pt 在 Retina display 上是否看得到？device 測（共用 mockup §316 開放 Q2）
5. **Stage Manager 內 NSStatusItem 行為**：popover dismiss 是 OS 控制、phantom 無權；若 user 在 Stage Manager 頻繁切 group 導致 dropdown 被收 → 加 fallback「打開 main window 查看 session 狀態」hint？傾向不加（避免 over-engineering）
6. **AirPods 中鍵 single tap vs double tap mapping**：目前單按 = `pauseCommand`（macOS 預設）；雙按 / 三按是否要綁 stop / skip-chunk？v0.6 不做（不搶 iOS Music app 預設）
7. **VoiceOver 在 dropdown popover 自動 dismiss 後 focus 還哪**：iOS `UIAccessibility.post(.screenChanged, argument: nil)` 對等；macOS `NSAccessibility.post(element: app, notification: .focusedUIElementChanged)` 但 popover 收掉後 focus 還 user 前一個 app — 此跨 app focus 還原是否 work 需測

---

## 易用性測試準備（macOS 變體）

### 7 個 user task — 涵蓋 6 大資料狀態 + Nielsen 5 + macOS-specific

| # | Task | macOS 測項 | 6-state 覆蓋 |
|---|---|---|---|
| 1 | **首次使用 + 全域 shortcut**：「請在 Safari 開著任何網頁時用快捷鍵開始 25 分鐘 focus」 | `⌘⇧F` 註冊 + 第一次 TCC prompt + sheet from-anywhere flow | Loading（TCC wait） / Ideal（done） |
| 2 | **NSStatusItem dropdown 操控**：「在錄音中從 menu bar 暫停一次、繼續一次、停止」 | NSPopover 200ms timing + row sequence stop>pause 認知 | Ideal |
| 3 | **Multi-monitor**：「拖 Takeaway window 到第二顯示器後關閉、重開」 | F window per-monitor size persist + NSStatusItem follow | Ideal + 多 monitor edge |
| 4 | **OS interrupt**：「Zoom 開會時 mic 被搶，請從 Notification Center banner 看出發生什麼」 | AVCaptureSession interrupt + banner sound=default + action button | Error（中斷態） |
| 5 | **Done flow + Notification**：「focus 完成後從 Notification Center 點 banner 看 takeaway」 | UNNotificationCenter delegate + window spawn timing | Ideal |
| 6 | **Maximum state**：「請設定 200 min 自訂 timer」 | input clamp 回 180 + shake animation + NSHaptic alignment | Maximum |
| 7 | **TCC denied recovery**：「假設你已拒絕麥克風權限，從 sheet 一路到打開系統設定重授」 | NSWorkspace deep-link + sheet re-check 邏輯 | Error / Loading |

### Sampling

- 目標 5–7 user（per Nielsen 5 user 找 80% 問題）
- 角色：3 行動族 + 2 隱私意識 + 2 OSS contributor（per SPEC-21 §5 personas）
- 環境：MacBook Pro M3 14"（單螢幕）+ Mac Studio + 27" 外接（多 monitor）各跑一次

### 觀察重點（macOS-specific）

- **shortcut 衝突**：user 是否撞到 `⌘⇧F` 已被 Xcode / Raycast 占用？wizard fallback 友善度
- **NSStatusItem 識別**：first-time user 是否注意到 menu bar 紅點？Bartender / Hidden Bar 用戶失錨率
- **Stage Manager edge**：user 切 group 時 dropdown 被收是否引起困惑？
- **Multi-monitor spawn**：F window spawn 在 NSStatusItem monitor 是否符合 user 預期？拔螢幕後孤兒處理是否優雅？
- **TCC denied 救濟**：deep-link 到 System Settings → Microphone 是否一氣呵成？sync re-check 是否會被 user 注意

### 紀錄方式

- 螢幕錄影（user 同意後）+ external monitor 同步錄（multi-monitor task）
- think-aloud protocol（user 邊操作邊說）
- 結束後 SUS 問卷 + 5 題開放問題（最讚 / 最差 / 困惑點 / 期望加 / 期望砍）
- macOS-specific：問 user「你常用什麼 menu bar tool」（Bartender / Hidden Bar / Ice）→ 推測隱藏 risk

---

## 下一步

→ 拉 5 user 跑 macOS usability test（單螢幕 + 多 monitor 各跑）→ 收 SUS 分數 + 觀察紀錄 → 回頭修 macOS Wireframe / Mockup / Prototype 對應點
→ 對齊 SPEC-41 §10 catalog 12 screen 全部交付（A sheet / B TCC / B' denied / C dropdown / C' interrupted banner / D toast / E finalizing / F takeaway window）
→ Stage Manager / Bartender / lid close / Zoom 搶 mic 四條 edge case 跑 device 實測，記到 SPEC-40 §18 risk register
