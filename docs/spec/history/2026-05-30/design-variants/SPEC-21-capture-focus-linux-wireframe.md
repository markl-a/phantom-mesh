# SPEC-21 Capture Focus — Linux Wireframe（線框稿）

> **Stage 1/3** · 線框稿 → [視覺稿（待補）] → [原型（待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-27
> **Scope**: Linux only。**hero 平台是 iOS**（見 [SPEC-21-capture-focus-wireframe.md](./SPEC-21-capture-focus-wireframe.md) iOS 段），本檔只列 Linux **deltas**，共用結構不重抄。Linux 是 best-effort 平台（DE 碎片化 — GNOME / KDE / Xfce / tiling WM 行為各異），寫法比 mac/win 更務實 — 預設「能 work 就好」而非「追求完美」。
> **Spec**: [`SPEC-21-SYSTEM-capture-focus`](../specs/v060-deep-spec/SPEC-21-SYSTEM-capture-focus.md) · [`SPEC-44-PLATFORM-Linux-foundations`](../specs/v060-deep-spec/SPEC-44-PLATFORM-Linux-foundations.md) · [`SPEC-45-PLATFORM-Linux-screens-flows`](../specs/v060-deep-spec/SPEC-45-PLATFORM-Linux-screens-flows.md)
> **這份的工作範圍**：Linux-specific layout & flow — DE 碎片化 fallback / SNI tray / libnotify / X11 vs Wayland / 無 mic perm gate / 不承諾 lock-screen / 不承諾 global shortcut。共用 FSM 跟 iOS 同（見 hero wireframe §通用 session 狀態鏈），本檔不重抄。

## 為什麼 Linux 有獨立 wireframe

hero wireframe 的 Linux 段只 ~25 行 best-effort 條列。實際 Linux：
1. **DE 碎片化**（GNOME 無預設 tray / KDE / Xfce 有 / Sway tile-by-default） — 沒有「Linux desktop」單一 UX，要 matrix 化
2. **無 OS-level mic permission gate**（PulseAudio / PipeWire 不像 iOS/macOS/Android 那樣 prompt） — perm flow 比 mobile 簡單但有 device-error 路徑
3. **X11 vs Wayland 全域熱鍵雙路徑**（X11 XGrabKey OK / Wayland compositor 多半拒絕） — `⌘⇧F` 不承諾
4. **libnotify urgency 三級**（low / normal / critical）跟 mac NC banner / Win ActionCenter 行為對映不同
5. **whisper.cpp 是唯一 on-device ASR 路徑** — 不提供 cloud fallback（per SPEC-44 信任邊界）

→ 這 5 點值得獨立 frame 級描述，不要塞在 hero 段。

## 入口點（per SPEC-45 §10.2）

| 進入點 | v0.6.0 | v0.7+ | Source |
|---|---|---|---|
| Main window Focus tab（canonical surface） | ✅ | ✅ | SPEC-45 §10.2.6 main window |
| Tray dropdown → "Focus 開始"（SNI item） | best-effort | ✅ | SPEC-45 §10.2.1（GNOME 需 `appindicatorsupport` extension） |
| `.desktop` action（dock 右鍵 → "Focus 開始"） | ✅ | ✅ | SPEC-45 §10.3 `[Desktop Action FocusStart]` |
| CLI `phantom-mesh-app --focus` | ✅ | ✅ | SPEC-45 §10.3 Exec flag |
| **`⌘⇧F` global shortcut** | ❌ | best-effort | **SPEC-45 §3.2 NG3**：不承諾、user 自接 wmctrl/KWin shortcut |
| `phantom://focus/start` deep link | ✅ | ✅ | SPEC-44 §8.3 `x-scheme-handler/phantom` |
| Wear OS / mobile companion | ❌ | ❌ | n/a |

**v0.6.0 ship 4 個可靠路徑**：main window + .desktop action + CLI + deep link。Tray 跟 global shortcut 是 best-effort（DE 相依）。

## 螢幕 A — Start Window（同 hero macOS A 內容、Linux 框架差異）

hero 已定 ASCII（duration radio + goal tag input + trust badge + cancel/start）。Linux delta：

- **視窗類型**：GTK CSD（client-side decorations）真窗，**不是 macOS sheet**（Linux 無 sheet 慣例）
- **WM_CLASS**: `phantom-mesh`（per SPEC-45 §7.1 `LinuxScreenSpec`） — 讓 tiling WM rule 認得
- **預設大小**: 480×320px，min 400×280px
- **decorations: true**（GTK CSD per SPEC-45 §3.2 NG4 — 不自家畫 titlebar）
- **theme follow**: `prefers-color-scheme` media query（per SPEC-45 G4） — GTK 切深淺色 5s 內跟著切；webkit2gtk < 2.40 fallback light
- **icon set**: Lucide SVG bundled（per mockup icon 對照矩陣） — 不用 SF / Material
- **focus tab 為 canonical surface**：main window 內 Focus tab 跟此 sheet 共用元件（per hero invariant），sheet 是「快速啟動」、tab 是「完整視圖（含 history list）」

## 螢幕 B — Mic Permission（**Linux 無 OS gate**）

Linux 跟 iOS / Android / macOS 都不同：**PulseAudio / PipeWire 不會 OS-level prompt** ask 使用者「允許 mic 嗎」。第一次抓 mic 直接成功（assume user 桌面已 setup audio）。所以：

```
[A. Start window] tap Start
       │
       ▼
(無 perm prompt — 直接走到 C；Linux 無 B 對等螢幕)
       │
       ▼
[C. Recording]
```

**Device-error 路徑**（取代 perm denied）：

```
[A. Start window] tap Start
       │
       ▼
[B'. No-mic error 卡（覆蓋 Start window 或主畫面）]
┌────────────────────────────┐
│  [mic-off icon]            │  Lucide mic-off, phantom-danger
│                            │
│  找不到麥克風裝置           │  title（取 `focus.err.no_mic`）
│                            │
│  請檢查系統音訊設定         │  body, phantom-muted
│                            │
│ ┌────────────────────────┐ │  Open-settings btn → exec `pavucontrol` 或
│ │  打開音訊設定           │ │  `kcmshell5 kcm_pulseaudio`（依 DE 偵測）
│ └────────────────────────┘ │
└────────────────────────────┘
```

- **觸發**：core 抓 `cpal::default_input_device()` 回 `None` 或 PipeWire `open_capture_stream` fail
- **不阻擋 main window 其他功能**（chip / coach / settings 仍可用）
- **deep link 不固定**：偵測 DE 用 `XDG_CURRENT_DESKTOP` env → GNOME 開 `gnome-control-center sound` / KDE 開 `kcmshell5 kcm_pulseaudio` / Xfce 開 `pavucontrol` / 其他 fallback `xdg-open` audio settings URI；測不到任何 audio control panel 時 disable 該按鈕、只顯示文字提示

## 螢幕 C — Recording（同 hero C ASCII + Linux deltas）

版型同 hero（計時器 / waveform / pause-stop / chunk count / trust badge）。Delta：

- **icon set**: Lucide（mic / pause / square stop / folder）
- **theme**: 跟桌面 dark/light 走 — recording accent `phantom-warning` 不變
- **tray icon 同步狀態**（best-effort，DE 相依）：
  - idle: Lucide `mic` 16×16 phantom-muted
  - recording: Lucide `mic` 16×16 phantom-warning
  - paused: Lucide `mic-off` 16×16 phantom-muted
- **沒有鎖屏卡**（per hero NG6 + SPEC-45 §12.1） — Linux 無「lock-screen now-playing」統一介面（loginctl 不對等 macOS MPRemote / Win SMTC）
- **沒有 FG-service 通知對等品**（systemd `--user` phantom.service 在 background 跑、不是 Android FGS 概念）

## 螢幕 C-tray — Tray right-click menu（best-effort，DE 相依）

```
[Tray right-click menu (KDE / Xfce / GNOME+appindicator)]
┌──────────────────────────┐
│ 🔴 Focus 05:23/25:00     │  disabled row（status display）
│ ──                        │
│ ⏹ 停止並收工              │  Recording 期間最高優先（取 `focus.btn.stop_finalize`）
│ ⏸ 暫停                    │  取 `focus.btn.pause`
│ ──                        │
│ Open Phantom Mesh         │
└──────────────────────────┘
```

- **跟 mac / Win tray 結構同**（per hero macOS C / Windows C ASCII，invariants 鎖 stop 在 pause 上方）
- **GNOME 無預設 tray 情境**：tray menu 不存在 → user 從 main window Focus tab 內按鈕停（per SPEC-45 §6.4 onboarding step 4 已提示「請裝 appindicatorsupport」）
- **Sway / i3wm 情境**：tray 行為依 swaybar / i3bar 配置，本檔不畫專屬 frame；CLI fallback `phantom-mesh-app --focus-stop`（v0.7+ 確認）

## 螢幕 C' — Interrupted sub-state（desktop 無專屬 UI 變體）

per hero invariants「desktop 無專屬 UI 變體（waveform 不凍結、計時不停）」+ 「desktop 中斷強制系統通知」：

Linux interrupt 觸發點（跟 mac / Win 不同）：
- mic 被搶（PipeWire `node-removed` event 或 PulseAudio `source-output-removed`）
- 系統 sleep / suspend（systemd-logind `PrepareForSleep` D-Bus signal）
- 藍牙耳機切換（PipeWire profile change → mic source 切換）

**強制 libnotify 通知**（per hero invariants「desktop 中斷強制系統通知」）：

```
[Linux libnotify (D-Bus org.freedesktop.Notifications) — interrupted]
┌─────────────────────────────────────┐
│ [icon mono]  Phantom Mesh 焦點時段中斷│  summary 取 `focus.desktop.interrupt_notif_title`
│                                     │
│ 5:23 / 25:00 · mic 被佔用            │  body line 1（依 interrupt 來源動態填）
│ 30 秒內回復將自動繼續                │  body line 2（取 `focus.interrupted.resume_hint`）
│                                     │
│ [ 開啟並停止 ]                       │  action button（取 `focus.desktop.interrupt_notif_action`）
└─────────────────────────────────────┘
```

- **urgency = `critical`**（per mockup §463 + hero invariants — 不讓 dunst / mako compositor 折疊）
- **timeout = 0**（不自動消失，等 user 處理）
- **action button 支援度看 compositor**：KDE / GNOME Shell OK；sway + mako / hyprland + dunst 看 user 自家 mako/dunst 設定 — 若 actions= 不顯示，user 仍可點 body 開回 main window
- **app 進背景時 tray icon 同步切到 attention 變體**（best-effort — SNI `NeedsAttention` status，per SPEC-45 §7.1 `SniStatus`）

## 螢幕 D / E / F — Finalizing / Done

- **D（lock-screen）不存在**（per hero NG6） — Linux 無對等品
- **E Finalizing** 同 hero E（spinner + progress + partial inline）— icon 用 Lucide
- **F Done takeaway card** 同 hero F — 落在 main window Focus tab；libnotify 同時 fire「focus session done」通知（urgency=normal、timeout 默認），點通知 → `phantom://focus/done/<session_id>` deep link → 跳回 main window 該 session

### libnotify Done notification 結構（per SPEC-45 §10.2.13 + mockup §452）

```
[Linux libnotify — focus done]
┌─────────────────────────────────────┐
│ [icon mono]  Focus complete          │  summary（en：`focus.done.title` 縮短版）
│ 25 min · 5 chunks · takeaway ready  │  body（取 `focus.done.title` 模板）
│                                     │
│ [ 開啟 ]                             │  action → main window Focus tab 該 session
└─────────────────────────────────────┘
```

- **urgency = `low`**（per mockup §457）— 不打斷 user
- **PII 不放 body**（per SPEC-45 §12.1）— takeaway 第一行**不**塞 notification body（避免 lock-screen 預覽外洩 personal observation）
- **ASR 全靜音情境（Empty）**：通知不發；main window F 卡片顯示安撫文「本次時段未偵測到語音，已為您記錄時長」+ session 仍寫 events

## Linux 獨有 — DE / display server fallback matrix

per SPEC-45 §14 跨環境差異矩陣，capture-focus 子集：

| 行為 | GNOME（Mutter） | KDE Plasma（KWin） | Xfce（xfwm4） | Sway / i3 | 本檔處理 |
|---|---|---|---|---|---|
| Tray 顯示 | 需 `appindicatorsupport` | 預設可見 | 預設可見 | swaybar 配置 | tray 是 bonus，main window 為 canonical |
| 全域熱鍵 `⌘⇧F` | Mutter portal 嚴 | KWin portal 中 | xfwm4 走 xdotool（X11 only） | sway `bindsym` 手動 | v0.6.0 不承諾、教 user 自接（per NG） |
| libnotify actions | GNOME Shell OK | Plasma OK | xfce4-notifyd 部分 | mako / dunst 看設定 | actions 失敗時 user 仍可點 body |
| Interrupt notification urgency=critical | 顯示 banner 直到處理 | popup 持續顯示 | persistent | mako/dunst 看 config | 兩條救濟（body click + actions） |
| Audio device prompt | 無 | 無 | 無 | 無 | Linux 全平台無 mic perm gate |
| Lock-screen 控制 | 無對等 | 無對等 | 無對等 | 無對等 | NG6 — 不承諾 |

## 跨 OS 對映（per SPEC-45 §14.3 + hero invariants）

| 行為 | macOS（SPEC-41） | Windows（SPEC-43） | **Linux（本檔）** | 備註 |
|---|---|---|---|---|
| 啟動 sheet | NSWindow sheet | 真窗 modal | GTK CSD 真窗 | Linux 無 sheet 慣例 |
| Tray | menubar dropdown（NSMenu，永遠在） | Shell_NotifyIcon（永遠在） | SNI dropdown（GNOME 可能不可見） | Linux 唯一可能不可見 |
| Mic perm gate | TCC 一次性 prompt | 隱式 grant | 無 gate（PA/PW 不 prompt） | Linux 簡化 |
| Lock-screen | MPNowPlayingInfoCenter | 無（用 Action Center toast 取代） | 無對等 | mac 獨有 |
| Interrupted notification | NSUserNotification banner | ActionCenter toast `urgent` | libnotify `critical` | i18n key 跨平台一字不差 |
| Global shortcut `⌘⇧F` | NSEvent monitor（user opt-in） | RegisterHotKey（user opt-in） | X11 XGrabKey 可 / Wayland 多半拒 | Linux 唯一可能被拒 |
| ASR 路徑 | whisper.cpp on-device | whisper.cpp on-device | whisper.cpp on-device | 三家一致、cloud fallback 違反信任邊界 |

## Cross-platform invariants 對齊（per hero wireframe）

繼承全部 hero invariants（trust badge / Stop ≤ 2 操作 / waveform / chunk count / 計時器顏色 / desktop interrupt notification 強制觸發）。Linux 額外：

- **Main window Focus tab 為 canonical surface**（tray 是 bonus，可能不存在）
- **無 mic perm gate** — 但 **device-error 螢幕 B' 必須存在**（取代 perm denied）
- **libnotify body 不含 PII**（per SPEC-45 §12.1 + STRIDE Information Disclosure 緩解）
- **theme follow desktop dark/light**（5s 內切，per SPEC-45 G4）— webkit2gtk < 2.40 降級 light
- **WM_CLASS / `StartupWMClass=phantom-mesh`** 必須設（tiling WM rule 認得）
- **`phantom-mesh-chip` WM_CLASS 不適用 focus**（focus 走 main window，不浮動小視窗）

## 6 大資料狀態 — Linux 對映表

| 狀態 | Linux 螢幕 / 場景 | 對應 i18n key / mockup |
|---|---|---|
| **理想（Ideal）** | F Done takeaway card（main window Focus tab） + libnotify done 通知 | `focus.done.title` per mockup §452 |
| **空白（Empty）** | main window Focus tab 內 history 區（無 session） / ASR 全靜音 session | `focus.empty.history` / 「本次時段未偵測到語音，已為您記錄時長」 |
| **極限（Limit）** | C chunk 99+ / F takeaway > 800 字截斷 | `focus.limit.chunk_overflow` / `focus.limit.takeaway_truncated_hint` per mockup §563 |
| **錯誤（Error）** | B' No-mic device error / interrupted libnotify critical / `LINUX-SCREEN-LIBNOTIFY-NO-SERVICE` headless server | `focus.err.no_mic` / `focus.interrupted.*` |
| **局部（Partial）** | E Finalizing inline `focus.partial.chunk_failed` | per mockup §565 |
| **載入中（Loading）** | E Finalizing + tray icon 同步 attention 變體（best-effort） | `focus.finalizing.asr` + SNI `NeedsAttention` |

## 已決（per SPEC-45 lock + hero invariants）

1. ~~Tray vs main window canonical~~ → **已決**：main window Focus tab 為 canonical，tray 是 bonus（per SPEC-45 §3.2 NG1/NG2 + GNOME tray 不保證可見）
2. ~~Global shortcut `⌘⇧F` 支援~~ → **已決**：v0.6.0 不承諾、教 user 自接 wmctrl/KWin bindsym（per SPEC-45 §3.2 NG3 + Wayland portal 不保證 grant）
3. ~~Mic permission gate~~ → **已決**：Linux 無 OS gate、直接走到 Recording；fail 走 B' device-error 螢幕（取 `focus.err.no_mic`）
4. ~~Lock-screen 控制~~ → **已決**：不承諾（per hero NG6 + SPEC-45 §12.1 — Linux 無 MPRemote / SMTC 對等品）
5. ~~Interrupted notification urgency~~ → **已決**：`critical` + timeout 0（per mockup §463）
6. ~~ASR 路徑~~ → **已決**：whisper.cpp on-device 唯一（per SPEC-44 信任邊界、cloud fallback 違反 P3 local-first）
7. ~~libnotify body PII~~ → **已決**：body 不含 takeaway / chip 內容（per SPEC-45 §12.1 STRIDE Information Disclosure）

## 開放問題（Linux 層面，剩餘）

1. **Sway / i3wm tiling 下 main window 被 tile**：focus session 期間 main window 可能被 tile 化（被當作普通 window 排版）。是否在 `.desktop` 加 `StartupWMClass=phantom-mesh-focus` 子類別讓 user 寫 sway rule float？提案：暫不細分，沿用主 WM_CLASS。
2. **`gnome-control-center` deep link DE-specific 偵測 fallback**：B' device-error 卡上「打開音訊設定」按鈕在罕見 DE（如 LXQt / Budgie）測不到對應 binary 時，是否只顯示文字提示還是 fallback `xdg-open`？提案：fallback `xdg-open audio:///`（若 URI scheme handler 存在）→ 不存在則 disable 按鈕。
3. **libnotify `critical` urgency 在 dunst / mako 不支援 actions** 時的二級救濟：點 body 是否可帶 D-Bus signal 回 phantom？目前仰賴 `default` action — 部分 compositor 不 fire。Mitigation：tray icon 同步切 attention 變體 + main window 加 prompt（v0.7+ 補完整鏈）。
4. **whisper.cpp CPU-only fallback on old hardware**：若 user 機器無 AVX2 / GPU offload，whisper.cpp tiny model 跑 5min audio 可能 > 3min — 是否提示「ASR 比預期久」？提案：takes_longer hint 已在 `focus.finalizing.taking_longer` key，主鏈 reused 即可、不畫 Linux 專屬 frame。

→ 互動 timing / 手勢 / sway rule docs 細節歸 Linux prototype（待補）。

## 下一步

→ 進 [Linux Mockup（待補）] 決定 GTK CSD titlebar / 終版文案 / Lucide icon ID / DE-aware fallback 細節。
