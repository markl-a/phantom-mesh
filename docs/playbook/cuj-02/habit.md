# CUJ-02 daily-loop · habit subset — manual playbook

> **Parent CUJ**: [`docs/cuj/02-daily-capture-loop.md`](../../cuj/02-daily-capture-loop.md)
> **Flow**: [`docs/flow/cuj-02/habit.md`](../../flow/cuj-02/habit.md)
> **Underlying spec**: [`docs/superpowers/specs/v060-deep-spec/SPEC-22-SYSTEM-capture-habit.md`](../../superpowers/specs/v060-deep-spec/SPEC-22-SYSTEM-capture-habit.md)
> **Automated tests**: `core/tests/cuj02_daily_habit_subset.rs`
>
> 此 playbook 是 CUJ-02 daily loop 內 **habit 子集** 的「使用者親手走一遍會看到什麼」
> 逐條 checklist。預估時長 15-20 min (不含等 broker sync)。每跑完一條打 ✓ / ✗。
> 失敗的 step 寫 issue 連此檔 line number。
>
> 完整 CUJ-02 playbook (含 food + focus + coach review) 後續會在
> `docs/playbook/cuj-02/index.md` 編列、本檔僅是 habit subset。

---

## 前置條件

- [ ] spectyn CLI 已 build 並 in PATH（`spectyn --version` 回 ≥ 0.6.0）
- [ ] 使用乾淨測試帳號 OR `--data-dir=/tmp/spec22-test` 不污染 prod
- [ ] 已有 starter palette（`spectyn habit palette list` 回 12 個預設 chip）

---

## Test 1：基本 log 一筆 + 看到 streak

```bash
# CLI 入口
spectyn habit water --qty 250
```

- [ ] 輸出包含 `streak=1` 或 `streak` 字樣（不應 panic）
- [ ] 輸出 < 100ms（手感即時、不卡）
- [ ] `spectyn habit streak --chip water` 回顯 current_streak=1

## Test 2：palette CRUD

```bash
spectyn habit palette list                           # 應印 12 條
spectyn habit palette add --id meditation --zh 冥想 --en Meditation --unit min --qty 5
spectyn habit palette list                           # 應變 13 條... 等等
```

- [ ] 第二個 add 應 **失敗**（palette 上限 12）── 期望錯誤 `ERR-22-PALETTE-FULL` 或同義訊息
- [ ] 若上述 add 成功（與 spec G1 違反），記 BUG
- [ ] `spectyn habit palette remove --id water` → 再 add `meditation` 應成功
- [ ] `spectyn habit palette reorder --id meditation --to 0` → list 第一條變 meditation

## Test 3：freetext fallback (G5)

```bash
spectyn habit "讀完 SICP ch3"
```

- [ ] 不應 panic／回非空成功訊息
- [ ] 內部 chip_id 應為 `freetext` + free_text 原樣存（用 `spectyn habit list-events --limit 1` 或 sqlite 檢查）

## Test 4：跨時區 streak 不追溯（G3 risk）

> 需要兩台機（或同機改時區）。若只 mac → 用 `TZ=Asia/Taipei spectyn ...` 跟 `TZ=America/New_York spectyn ...` 模擬。

```bash
TZ=Asia/Taipei spectyn habit coffee --qty 1        # 假設台北凌晨 1am log
TZ=America/New_York spectyn habit coffee --qty 1   # 模擬飛 NY 後再 log（同一 chip）
spectyn habit streak --chip coffee
```

- [ ] streak 不應因為 user 改時區而追溯改舊 event 的歸屬日
- [ ] 新 event 應以「當下系統時區」歸日

## Test 5：plaintext boundary (G7)

```bash
spectyn habit "跟某人吵架"  # user 自己寫敏感內容（責任 user 自負）
sqlite3 ~/.spectyn-mesh/events.sqlite \
  "SELECT metadata_json FROM events ORDER BY ts_ms DESC LIMIT 1"
```

- [ ] `metadata_json` 應該是 **age 加密 base64 / hex blob**、不是純 JSON 「跟某人吵架」字串
- [ ] `events_fts` 的 summary column 應該有 truncated text（最多 40 字）
- [ ] chip_palette 表存的是 chip_label「水」「咖啡」── plaintext 沒關係（公開資訊）

## Test 6：CLI / shortcut 對等（G8）

> 需要 desktop UI 在跑（`spectyn-mesh-app`）

- [ ] 按 Cmd+Shift+H（mac）/ Ctrl+Shift+H → 出現 chip popover
- [ ] popover 點「水」→ 輸入 250 → ✓
- [ ] CLI `spectyn habit list-events --limit 2` → 兩筆 metadata 結構應一致（除 source 欄位 desktop vs cli）

## Test 7：mobile widget 3-tap（G2）

> iOS / Android only — 需要 ayaneo / iOS sim 跑

- [ ] iPhone 鎖屏 → 解鎖（tap 1）
- [ ] Home screen 有 Spectyn Habit Widget → tap chip「水」（tap 2）
- [ ] 預設 250ml quick action 出現 → 點 ✓（tap 3）= 寫入
- [ ] 主 app 沒被打開（背景寫入）

## Test 8：跨裝置同步（G4）

> 需要 broker live + 兩台已配 broker token 的裝置

- [ ] 手機 log 一筆「咖啡」
- [ ] ≤ 5s 後 desktop `spectyn habit streak --chip coffee` 包含這筆
- [ ] 隔日早上 coach review (spectyn coach review) 提到此 chip pattern

---

## 退出條件 / 失敗如何 escalate

- 任何 Test 失敗 → 寫進 BROADCAST.md「SPEC-22 playbook fail @ Test N」+ 開 issue
- 全部 ✓ → 在 docs/status.md 旁邊寫一行：「SPEC-22 last manual pass: YYYY-MM-DD by <operator>」
- 期待 cadence：每次有改 capture_habit_wire.rs 或關聯 wire 都要重跑一次本 playbook

---

## 跑跑 automated 對齊

```bash
cargo test -p spectyn-mesh --test spec22_capture_habit_acceptance
```

- [ ] 全綠 → automated layer 沒退化
- [ ] 紅 → 跟手測比對哪邊較早抓到 → 補另一邊
