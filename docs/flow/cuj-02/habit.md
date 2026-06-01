# CUJ-02 daily-loop · habit subset — user flow drill-down

> **Parent CUJ**: [`docs/cuj/02-daily-capture-loop.md`](../../cuj/02-daily-capture-loop.md)
> **Underlying spec**: [`docs/superpowers/specs/v060-deep-spec/SPEC-22-SYSTEM-capture-habit.md`](../../superpowers/specs/v060-deep-spec/SPEC-22-SYSTEM-capture-habit.md)
> **Playbook**: [`docs/playbook/cuj-02/habit.md`](../../playbook/cuj-02/habit.md)
> **Tests**: `core/tests/cuj02_daily_habit_subset.rs`
>
> 此檔是 CUJ-02 daily capture loop 中**habit subset 的 user flow drill-down** ──
> 描述 user 用 widget / CLI / desktop 三個入口 log habit 會看到什麼、做什麼。
> 行為層 (不寫 px)。每個 sub-flow 7±3 步、跨 5 OS 共用一份。
>
> 完整 CUJ-02 (含 food + focus + coach review) 見 parent CUJ doc。

---

## Flow A：第一次 log 一個習慣（widget / CLI / desktop 三入口都收同一份 event）

```
1. user 想 log「喝水 250ml」
2. 從以下三個入口擇一：
   ├─ iOS/Android: home 解鎖 → habit widget → 點「水」chip → 預設 250 → tap ✓
   ├─ Desktop:      按 Cmd+Shift+H (mac) / Ctrl+Shift+H (win/linux) → 跳 chip popover → 點「水」
   └─ CLI:          phantom habit water --qty 250
3. 系統把 metadata（含 qty=250, unit=ml）走 age 加密 → 寫入 sqlite events 表
4. summary（純文字「水」）寫進 events_fts 為了搜尋
5. 回傳當下 streak（如「水 streak=5」）到使用者
6. （5 秒內）broker vault 同步、其他裝置看得到此筆
7. 完成 ── 全程 ≤ 3 tap 或 1 CLI command
```

**Platform divergence**: iOS widget = WidgetKit medium 2x4 grid；Android = Glance widget；Desktop popover = Tauri tray window；CLI 跨所有 OS 一致。

**期望覆蓋**: G2 (mobile 3-tap) + G7 (plaintext boundary) + G8 (CLI parity)

---

## Flow B：管理 chip palette（新增 / 刪除 / 重排）

```
1. user 想加一個自訂 chip「冥想」
2. 從入口：CLI `phantom habit palette add --id meditation --zh 冥想 --en Meditation --unit min --qty 5`
   OR Desktop: menu bar → Settings → Habits → "+ Add chip"
3. 系統驗證：palette 目前 < 12 個 ?
   ├─ 是 → 加入、回傳新 chip
   └─ 否 → 拒絕 + 錯誤 ERR-22-PALETTE-FULL「palette 上限 12，請先移除一個」
4. 同樣驗 chip_id 格式 [a-z0-9_]{1,32}、不重複
5. palette 寫入 sqlite chip_palette 表 (plaintext，因為 chip_label 公開無 PII)
6. 各裝置 ≤5s 同步、widget 自動更新可選 chip
7. 完成
```

**期望覆蓋**: G1 (palette CRUD + 上下限驗證) + G7 (plaintext boundary)

---

## Flow C：查看 streak（連續打卡天數）

```
1. user 想看「我咖啡連續幾天」
2. 入口：CLI `phantom habit streak --chip coffee`
   OR Desktop: menu bar → 點 phantom icon → 看 streak panel
3. 系統執行 §8.3 演算法：
   - 查最近 30 天 habit kind events、filter chip_id=coffee
   - 按 user-local-timezone 切日、每日有 >= 1 筆 = 連續
   - 中斷一天 → streak 歸 0 重算
4. 回傳 StreakResult { chip_id, current_streak, longest_streak, last_logged_at }
5. CLI 印 table、UI 顯示 30 天 grid heatmap (deep green = 多次 / light = 1 次 / gray = 0)
6. 完成 (p50 < 10 ms per chip)
```

**期望覆蓋**: G3 (streak algo) + G6 (perf budget)

---

## Flow D：free-text fallback（不在 palette 內的習慣）

```
1. user 想 log「今天讀完 SICP ch3」── 沒有對應 chip
2. 入口：CLI `phantom habit "讀完 SICP ch3"`
   OR widget「+其他」打字
3. 系統判斷：input 不在 chip_palette → chip_id = "freetext"
4. metadata = { chip_id: "freetext", free_text: "讀完 SICP ch3" }
5. 走 age 加密 → 寫入 events 表（同 Flow A）
6. summary = "freetext: 讀完 SICP ch3"（截斷 40 字）→ events_fts
7. coach.agent 之後用 LLM 解析（v0.7.0+）── v0.6.0 只負責 raw text 存好
```

**期望覆蓋**: G5 (freetext) + G7 (free_text plaintext 邊界 — user 自行負責)

---

## Flow E：跨裝置同步（手機 log → 桌機 coach review）

```
1. user 早上 iPhone widget 點「水」三次
2. iPhone HabitModule → EventStore::append（本地加密 + sqlite）
3. ≤5s 內 broker vault sync (SPEC-15) 把 sealed events 推上 phantommesh.io
4. Desktop on poll cycle pull → unseal → 寫進本地 events 表
5. （隔日早上 7:00）coach.agent 跑 daily review、讀 last 24h events
6. review 內容提到「你今早喝水 750ml、最近 5 天 streak」
7. user 桌機看到 review = JS3 達成
```

**期望覆蓋**: G4 (cross-device sync) — 需要 broker live + 2 device

---

## 失敗路徑 (key edges)

| 情境 | 期望反應 |
|---|---|
| palette = 12，想加 13 個 | ERR-22-PALETTE-FULL，UI/CLI 擋 |
| chip_id 含空白 / 大寫 / 特殊字 | ERR-22-INVALID-SLUG |
| streak 跨時區（user 飛 GMT+8 → GMT-5）| 既存 event 歸屬日不變、新 event 以當下時區計（G3 註） |
| chip_id 已存在 | ERR-22-DUPLICATE |
| record_checkin 給不存在 chip_id | ERR-22-UNKNOWN-CHIP（除非 chip_id="freetext"）|

---

## Coverage map

| Flow | Verifies G[X] | Test ID |
|---|---|---|
| A | G2, G7, G8 | T-habit-widget-3-tap (mobile) / T-habit-shortcut-cli-parity / T-habit-plaintext-boundary |
| B | G1, G7 | T-habit-palette-crud |
| C | G3, G6 | T-habit-streak-algo, T-habit-streak-tz-change, T-habit-perf-streak |
| D | G5, G7 | T-habit-freetext-fallback |
| E | G4 | T-habit-cross-device-sync |
