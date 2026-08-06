# Mac 手動測試 Playbook — 親手跑全 151 條的人類版

> **配套**：自動覆蓋率以 [`docs/test-cases/COVERAGE-MAP-mac.md`](../test-cases/COVERAGE-MAP-mac.md)（canonical 覆蓋率地圖）為準、
> machine-readable case DB 見 [`docs/test-cases/mac.md`](../test-cases/mac.md)。本檔是其中 ❌ manual 那組的人類 run-sheet。
>
> **誰跑**: operator 親手、不是 AI、不是 CI。
> **何時跑**: ship 前 / 大改 / 月度 sanity / 出 bug 後 regression
> **預估時長**: 完整一輪 90-120 min (含等待 / install / build)
> **怎麼紀錄**: 印出本檔、每條打 ✓/✗、失敗的寫 `[issue: <github-issue-or-stderr>]` 在右邊
> **跑兩遍**: 第一遍找 bug、修完再跑第二遍 confirm

---

## 跑前準備（5 min）

- [ ] mac 已 update 到最新 macOS（系統設定 → 一般 → 軟體更新）
- [ ] Apple Silicon (Apple → 關於 → 含 "M1/M2/M3/M4" 字樣)。Intel mac 不在本 playbook 範圍
- [ ] Terminal 已給 Full Disk Access（系統設定 → 隱私權與安全性 → 完整磁碟取用權 → 加入 Terminal）
- [ ] 網路通（`ping spectynmesh.com` 回 200）
- [ ] Disk space ≥ 2 GB
- [ ] **設定資料根隔離 `SPECTYN_HOME`**（spec'd data-root override，讓本輪測試不污染真實家目錄）：
  ```bash
  export SPECTYN_HOME="$(mktemp -d)/.spectyn-mesh"   # 或固定一個 sandbox 路徑
  echo "SPECTYN_HOME=$SPECTYN_HOME"
  ```
  > ⚠️ **drift 註**：`SPECTYN_HOME` 是 SPEC'd data-root override，但 **as-built 尚未被 home-resolver honor**（`core/src/config.rs:380-381` 仍走 `dirs::home_dir().join(".spectyn-mesh")`；`core/src/tui.rs:8927/8930` 明標「needs SPECTYN_HOME resolver」為 tracked follow-up）。在 #322 home-resolver 統一前，本檔以 `${SPECTYN_HOME:-$HOME/.spectyn-mesh}` 引用資料根：設了變數即用 sandbox，未設則 fallback 到實機現行路徑 `~/.spectyn-mesh`。#322 落地後此 fallback 可移除。
- [ ] 備份現有資料根（如果有，且未設 `SPECTYN_HOME`）：
  ```bash
  : "${SPECTYN_HOME:=$HOME/.spectyn-mesh}"
  mv "$SPECTYN_HOME" "$SPECTYN_HOME.bak-$(date +%s)" 2>/dev/null || true
  ```
- [ ] 準備一個記事 app（Notes / TextEdit）紀錄問題

---

## 章節索引

```
§1  CUJ-01 install → 第一筆 habit       (15 min)
§2  CUJ-02 daily capture loop           (20 min)
§3  CUJ-03 cross-device sync            (20 min, 需 2 台機 or 同帳號 broker)
§4  CUJ-04 degraded states              (20 min)
§5  CUJ-05 export + uninstall           (10 min)
§6  Mac platform 專屬                   (15 min)
§7  跑完後清理                          (5 min)
```

---

## §1. CUJ-01: install → 第一筆 habit (15 min)

### 1.1 線上 install（5 min）

| Step | 命令 / 動作 | 預期看到 | ✓/✗ | Notes |
|---|---|---|---|---|
| 1.1.1 | 開 Terminal | prompt 顯示 `<user>@<host>` | | |
| 1.1.2 | `curl -fsSL https://spectynmesh.com/install.sh \| sh` | 看到 install banner + download URL + "=== installed ===" | | |
| 1.1.3 | `which spectyn` | 印 `~/.local/bin/spectyn` | | |
| 1.1.4 | `spectyn --version` | 印 `spectyn 0.6.0 ...`（容忍 `0.6.0[-rc.N]` 後綴） | | |
| 1.1.5 | `ls -la "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/"` | 看到 `bin/` + `events.jsonl` + 可能其他 | | |

**❌ 失敗時**：
- 1.1.2 卡住 → 檢查網路 + spectynmesh.com 是否 alive
- 1.1.4 "command not found" → 重新開 Terminal 視窗讓 PATH 更新

### 1.2 First-run 環境 (3 min)

| Step | 命令 | 預期 | ✓/✗ |
|---|---|---|---|
| 1.2.1 | `spectyn habit water --qty 250` | exit 0、stdout 含 streak | |
| 1.2.2 | `ls "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/identity.key"` | 檔案存在、64 bytes | |
| 1.2.3 | `ls "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/events.sqlite"` | 檔案存在 | |
| 1.2.4 | `find "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/events" -mindepth 1 -maxdepth 1 -type d -print` | 至少 1 個 event 目錄 | |
| 1.2.5 | `sqlite3 "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/events.sqlite" ".tables"` | 含 `fts5_events` (FTS5 as-built；`events` 主表為 v0.7.0+ planned) | |

### 1.3 完整第一條 habit (5 min)

| Step | 命令 | 預期 | ✓/✗ |
|---|---|---|---|
| 1.3.1 | `spectyn habit palette list` | 印 12 列 starter palette (預期含 "水" "咖啡") | |
| 1.3.2 | `spectyn habit coffee --qty 1` | exit 0、stdout 含 streak | |
| 1.3.3 | `spectyn habit streak --chip water` | 回 streak ≥ 1 | |
| 1.3.4 | `spectyn habit streak --chip coffee` | 回 streak ≥ 1 | |
| 1.3.5 | `spectyn habit "讀完 SICP ch3"` (freetext) | exit 0、event 寫入 | |
| 1.3.6 | `find "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/events" -mindepth 1 -maxdepth 1 -type d -print` | 至少 3 個 event 目錄（1.2.1 + 1.3.2 + 1.3.5） | |

### 1.4 SLO 量測（manual）

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 1.4.1 | 開 stopwatch、從 1.1.2 開始計時、到 1.3.6 結束 | 全程 ≤ 90 sec (SLO target) | _time:_ ___ sec |

**FAIL accepted reasons**: 第一次 install download 慢、coach review LLM key 還沒設

---

## §2. CUJ-02: daily capture loop (20 min)

### 2.1 食物 capture (5 min)

> ⚠️ 需 LLM key (e.g. GEMINI_API_KEY). 沒設的話跳過、留 ✗ 標 "no LLM"

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 2.1.1 | `cp <你的早餐照.jpg> /tmp/breakfast.jpg` | 檔案存在 | |
| 2.1.2 | `spectyn food --image /tmp/breakfast.jpg` | exit 0、印分析結果 | |
| 2.1.3 | `ls "${SPECTYN_HOME:-$HOME/.spectyn-mesh}"/events/*/modality_image.json` | 至少 1 個檔 | |
| 2.1.4 | `head -c 30 "${SPECTYN_HOME:-$HOME/.spectyn-mesh}"/events/*/modality_image.json \| xxd \| head -1` | 開頭含 "age-encryption.org/v1" magic (P4 加密驗證) | |

### 2.2 Focus session (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 2.2.1 | `spectyn focus start --duration 60` | 開始錄 audio | |
| 2.2.2 | 等 60s 或 Ctrl+C 結束 | exit 0、印 takeaway | |
| 2.2.3 | `ls "${SPECTYN_HOME:-$HOME/.spectyn-mesh}"/events/*/modality_audio.*` | 至少 1 個 (audio blob 加密) | |
| 2.2.4 | 麥克風權限提示彈出（首次） | 系統權限對話框、選「允許」 | |

### 2.3 Habit 進階測試 (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 2.3.1 | `for i in $(seq 1 13); do spectyn habit palette add --id "t$i" --zh t$i --en t$i; done` | 第 13 次 fail + "PALETTE-FULL" | |
| 2.3.2 | `spectyn habit palette list \| wc -l` | 12（不超） | |
| 2.3.3 | `spectyn habit palette remove --id t1; spectyn habit palette add --id meditation --zh 冥想 --en Meditation` | 第 2 條 add 應成功 | |
| 2.3.4 | `spectyn habit palette reorder --id meditation --to 0` | exit 0、list 第 1 條是 meditation | |
| 2.3.5 | `spectyn habit grid --chip water --weeks 4` | ASCII grid 7×4 印出 | |

### 2.4 Coach review (5 min)

> ⚠️ 需 LLM key 才能跑 LLM 模式。沒 key 應退化 stats-only (CUJ-04 OFF-002)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 2.4.1 | `spectyn coach review --date $(date -v-1d +%Y-%m-%d)` | exit 0、印 markdown | |
| 2.4.2 | output 含「水」or「咖啡」or「冥想」 | 至少一個提到 | |
| 2.4.3 | `spectyn coach schedule install` | exit 0、`~/Library/LaunchAgents/ai.spectynmesh.coach.plist` 存在 | |
| 2.4.4 | （可選）等到隔天 7:00 看 launchd 是否自動跑 | `${SPECTYN_HOME:-$HOME/.spectyn-mesh}/reviews/<date>.md` 自動產 | |

---

## §3. CUJ-03: cross-device sync (20 min)

> 需要：另一台 mac OR iOS 裝置 OR 同帳號 broker

### 3.1 broker login (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 3.1.1 | `spectyn login` | 開瀏覽器 OAuth | |
| 3.1.2 | 登入 (Google / Apple) | 回 callback | |
| 3.1.3 | `cat "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/broker.json" \| jq '.token \| length'` | token 長度 > 0 | |
| 3.1.4 | `spectyn cluster status` | 顯示節點列表 | |

### 3.2 mac+mac sync (10 min)

> 需 2 台 Apple Silicon mac、同 broker 帳號

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 3.2.1 | mac A: `spectyn habit water --qty 250` | exit 0 | |
| 3.2.2 | mac B (同帳號): `spectyn habit streak --chip water` (≤ 5s 後) | 看到 mac A 那筆 | _delay: ___ s_ |
| 3.2.3 | SLO: 5s 內看得到 | ≤ 5s | |

### 3.3 mac+iPhone sync (5 min, optional)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 3.3.1 | iPhone spectyn app 同帳號 login | 收 token | |
| 3.3.2 | iPhone widget tap 「水」 | 顯示 logged | |
| 3.3.3 | mac terminal `spectyn habit streak --chip water` (≤ 30s 後) | 看到 iPhone 那筆 | |

---

## §4. CUJ-04: degraded states (20 min)

### 4.1 Offline mode (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 4.1.1 | 關 wifi (Cmd+Space → 控制中心 → wifi off) | 無網 | |
| 4.1.2 | `spectyn habit water --qty 250` | exit 0、event 仍寫本地 sqlite | |
| 4.1.3 | `spectyn coach review --date today` | 退化 stats-only 模式 (no LLM) | |
| 4.1.4 | 開 wifi back on | 網回 | |
| 4.1.5 | `spectyn cluster status` | broker 接得到、queue drain | |

### 4.2 沒 LLM key (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 4.2.1 | `mv "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/agents.toml" /tmp/agents.toml.bak` | 移開 config | |
| 4.2.2 | `spectyn habit water --qty 250` | exit 0 (capture 不靠 LLM) | |
| 4.2.3 | `spectyn coach review --date today` | 印「設定 → providers」hint | |
| 4.2.4 | `mv /tmp/agents.toml.bak "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/agents.toml"` | 復原 | |

### 4.3 Identity.key 損毀 (5 min, ⚠ 會破壞 events)

> 跑前先 `spectyn backup --to /tmp/bak.tar.gz`，跑後 restore

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 4.3.1 | `spectyn backup --to /tmp/bak.tar.gz` | tar.gz 產出 | |
| 4.3.2 | `echo "garbage" > "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/identity.key"` | 損毀 | |
| 4.3.3 | `spectyn habit water --qty 250` | exit 非 0 + "IdentityKeyMissing" 或同義 | |
| 4.3.4 | `H="${SPECTYN_HOME:-$HOME/.spectyn-mesh}"; rm -rf "$H"; tar -xzf /tmp/bak.tar.gz -C "$(dirname "$H")"` | 復原 | |
| 4.3.5 | `spectyn habit streak --chip water` | 之前 event 仍讀得到 | |

### 4.4 Sqlite 損毀 (5 min, ✅ Sip Recovery shipped)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 4.4.1 | `spectyn backup --to /tmp/pre.tar.gz` | tar.gz 產出 | |
| 4.4.2 | `echo "garbage" > "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/events.sqlite"` | 損毀 | |
| 4.4.3 | `spectyn habit water --qty 250` | exit 0 + stderr 含 "rotated" + 新建 fresh sqlite | |
| 4.4.4 | `ls "${SPECTYN_HOME:-$HOME/.spectyn-mesh}"/events.sqlite.corrupt-*` | 損毀檔被搬到 .corrupt-<ts> | |
| 4.4.5 | `H="${SPECTYN_HOME:-$HOME/.spectyn-mesh}"; rm -rf "$H"; tar -xzf /tmp/pre.tar.gz -C "$(dirname "$H")"` | 復原 | |

---

## §5. CUJ-05: export + uninstall (10 min)

### 5.1 Export 三種模式 (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 5.1.1 | `spectyn data export --format json > /tmp/events.json; jq '.events \| length' /tmp/events.json` | ≥ 1 | |
| 5.1.2 | `spectyn data export --format md > /tmp/events.md; head /tmp/events.md` | markdown 起頭 | |
| 5.1.3 | `spectyn backup --to /tmp/full.tar.gz` | tar.gz 產出 | |
| 5.1.4 | `tar tzf /tmp/full.tar.gz \| head -10` | 列檔含 identity.key + events.sqlite + events/ | |
| 5.1.5 | restore test: `mkdir /tmp/restore-test; tar -xzf /tmp/full.tar.gz -C /tmp/restore-test; ls /tmp/restore-test/.spectyn-mesh/` | 看到 identity.key + ... | |

### 5.2 Delete (5 min, ⚠ 會刪 events)

> 跑前已 backup（5.1.3）

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 5.2.1 | `spectyn data delete --all --yes` | exit 0、events_dir 清空 | |
| 5.2.2 | `ls "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/events/"` | 空 | |
| 5.2.3 | `ls "${SPECTYN_HOME:-$HOME/.spectyn-mesh}/identity.key"` | 仍存在（CUJ-05 DEL-001a 證明 scope 對） | |
| 5.2.4 | （optional broker DELETE 等 task #140 ship）| skip | n/a |
| 5.2.5 | `H="${SPECTYN_HOME:-$HOME/.spectyn-mesh}"; rm -rf "$H"; tar -xzf /tmp/full.tar.gz -C "$(dirname "$H")"; spectyn habit streak --chip water` | 從 backup 復原成功、看得到原 streak | |

---

## §6. Mac platform 專屬 (15 min)

### 6.1 LaunchAgent (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 6.1.1 | `spectyn service install` | LaunchAgent .plist 寫入 | |
| 6.1.2 | `launchctl list ai.spectynmesh.serve` | 顯示 service entry | |
| 6.1.3 | `spectyn service status` | 印 active | |
| 6.1.4 | `spectyn service uninstall` | .plist 刪、service 停 | |

### 6.2 macOS xattr / TCC (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 6.2.1 | `ls -la@ ~/.local/bin/spectyn` | 行尾可能有 @ (provenance xattr) 但執行 OK | |
| 6.2.2 | `~/.local/bin/spectyn --version` | 跑 + 不被 Gatekeeper 擋 | |
| 6.2.3 | `xattr ~/.local/bin/spectyn` | 印 xattr 列 | |

### 6.3 Spotlight (5 min)

| Step | 動作 | 預期 | ✓/✗ |
|---|---|---|---|
| 6.3.1 | `mdfind "kMDItemFSName == 'events.sqlite'"` | 不出現（或 metadata 不洩明文） | |
| 6.3.2 | Spotlight (Cmd+Space) search 「streak」 | 找不到本地 events 內容 | |

---

## §7. 跑完後清理 (5 min)

| Step | 動作 | ✓/✗ |
|---|---|---|
| 7.1 | 回復原本資料根（未設 `SPECTYN_HOME` 時）：`H="${SPECTYN_HOME:-$HOME/.spectyn-mesh}"; mv "$H".bak-* "$H" 2>/dev/null \|\| true`。若用 sandbox `SPECTYN_HOME`，改為 `rm -rf "$SPECTYN_HOME"`（真實家目錄全程未被污染） | |
| 7.2 | 砍掉 /tmp 測試檔：`rm -rf /tmp/{bak,pre,full,events,restore-test,breakfast}.* /tmp/{bak,pre,full,events,restore-test,breakfast}` | |
| 7.3 | （可選）`brew install / cleanup` | |
| 7.4 | 統計：填本檔總 ✓/✗ + 寫一行 summary | |

---

## §8. 結果紀錄表

```
跑完日期：           __________
總共題目：           151
過 (✓):             ____
失敗 (✗):           ____
跳過 (n/a):         ____
最高 SLO 違反：     __________________
最重要 issue (top 3):
  1. __________________________________________________
  2. __________________________________________________
  3. __________________________________________________
```

把上面表格存到 `docs/manual-runs/<date>.md` + commit。

---

## §9. 失敗如何 escalate

1. 寫進 `docs/known-issues.md` (one-liner)
2. 開 GitHub issue + 貼 stderr + screenshot + 本檔對應 step ID
3. 在 BROADCAST.md 留訊息給其他 cluster session
4. 嚴重 fail (data loss / P4 leak) → user 直接修不等其他

---

## §10. 跑兩遍策略

```
第一遍 (60-90 min):
  跑全部 §1-§7、邊跑邊紀錄
  失敗的留在表上、不修
  跑完看 §8 統計

修 bug 階段 (依失敗數):
  按 §8 top 3 一個個修
  每修一個 commit + push
  AI auto-triage 跑 cargo test 確認

第二遍 (60-90 min):
  針對 §8 失敗那組重跑（minimum）
  全綠才算 ship-ready
  
若第二遍仍有 fail → 再修、跑第三遍
```

---

## §11. 跟自動化的關係

本檔是 **人類版**、跟 `docs/test-cases/mac.md` 自動版互補：

- 自動版（v2）有 `Auto` 欄、CLI runner 跑 ✅ 那組
- 本檔（manual）跑 ❌ 那組（TUI 渲染、麥克風權限、Spotlight 索引、launchd 1 天 trigger 等）
- AI QA walk（dev-process-v2.md §2.4B）跑兩者 union 一輪、發現問題 + 自修 + escalate

人類 + AI + automation 三件套互相補。

---

## §12. ship-ready 的客觀標準

```
✅ ship-ready 必過：
   §1.1-§1.3 全條 ✓
   §5.1 export 全條 ✓
   §6.1 LaunchAgent CRUD ✓
   §4.4 sqlite recovery ✓
   §1.4 SLO ≤ 90s

🟡 ship-acceptable 部分過：
   §2 capture (任 1 模態 ✓ 即可、剩可 noon 後修)
   §3 cross-device (1 機證明 ≤ 30s 即 ok、5s 是 stretch)
   §4 degraded 7 條中 ≥ 5 條 ✓

🔴 ship-blocker 一條都不能 fail：
   §1.1.4 spectyn --version
   §1.2.1 第一條 habit 寫入
   §5.2.5 reinstall 可復原（沒備份 → 全雪盤）
   §4.4.3 sqlite recovery 不掉資料
```
