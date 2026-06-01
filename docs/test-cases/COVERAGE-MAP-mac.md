# Mac 全場景測試覆蓋率地圖 (2026-05-31)

> 對照 `docs/test-cases/mac.md` 的 136 條 MAC-* case vs `core/tests/` 實際存在的
> 自動化測試。產出「✅ 已覆蓋 / 🟡 partial / ⬜ 未覆蓋 / 🔴 有 bug」分類，供決定
> 下一步打哪。本檔是 workflows 文件（explore → plan → 漸進覆蓋）的 explore 產物。

## 怎麼讀這張地圖

- **✅ 已覆蓋** = 有對應的綠色 `cargo test`（unit / integ / e2e），最近驗過。
- **🟡 partial** = 邏輯有 code、但測試只覆蓋一部分（或只有 wire-shape unit test，
  缺端到端 integ）。
- **⬜ 未覆蓋** = 完全沒有自動化測試（多為 manual=❌ 或還沒寫）。
- **🔴 有 bug** = 已知缺陷，未修。

## 總覽（136 條）

| 分類 | 條數 | 佔比 |
|---|---:|---:|
| ✅ 已覆蓋（自動化綠） | ~24 | 18% |
| 🟡 partial | ~18 | 13% |
| ⬜ 未覆蓋 | ~88 | 65% |
| 🔴 有 bug | 1 (Bug A) | 1% |
| ⏰ cron / manual-only（非自動化可達） | ~26 | — |

> 數字是估計（mac.md 狀態欄人工標記 + 本次交叉核對）；自動化可達的約 94 條中，
> ✅+🟡 約 42 條，⬜ 約 52 條。

---

## §1 CUJ-01 install → 第一筆 habit（35 條）

| 子群 | 狀態 | 備註 |
|---|---|---|
| 1.A install.sh 抓+解（12） | ⬜ 大多未覆蓋 | manual / 需網路；`v4_e2e_desktop_macos.rs` 摸到 S3 wizard 邊緣 |
| 1.B codesign / ad-hoc（5） | 🟡 | `codesign_macos.rs` 存在，覆蓋部分 |
| 1.C first-run 初始化（10） | 🟡 | `identity_init_outcome_integration.rs` 蓋 identity 初始化；agents.toml seed 未測 |
| 1.D 第一條 habit（8） | ✅ 部分 | `cuj02_daily_habit_subset.rs` 蓋 palette CRUD + checkin（與 1.D 重疊） |

**缺口重點**：install.sh 端到端（下載→解壓→第一次跑）零自動化覆蓋，純靠 manual。

## §2 CUJ-02 daily capture loop（44 條）

| 子群 | 狀態 | 對應測試 |
|---|---|---|
| 2.A food photo（8） | 🟡 | `capture_food_wire` unit tests（多 `#[ignore]` env-dependent）；無 integ |
| 2.B focus audio（7） | 🟡 | `capture_focus_wire` + `life_node::focus_session` unit；無 integ |
| 2.C habit chip+freetext（10） | ✅ | `cuj02_daily_habit_subset.rs`（CRUD + freetext + 錯誤目錄） |
| 2.D coach engine+delivery（10） | ✅ **本次新增** | `cuj04_coach_review_llm.rs`（COA-001 happy / COA-001c）+ `coach_delivery_wire` unit |
| 2.E provider fallback（9） | ✅ 部分 **本次新增** | PROV-006 → `cuj04_stats_only_fallback.rs`；fallback 中段 → `cuj04_coach_review_llm.rs` |

**本次 session 把 2.D / 2.E 從 🔴/⬜ 推到 ✅**：coach review 的 happy / 部分-fallback /
全-fail-degrade / lint-reject 四條路徑都有 hermetic wiremock 測試。

## §3 CUJ-03 cross-device sync（15 條）

| 子群 | 狀態 | 對應 |
|---|---|---|
| 3.A broker login + token（5） | 🟡 | `spec15_broker_vault_e2ee_regression.rs` 蓋 vault e2ee；login flow 部分 |
| 3.B cluster join + mDNS（5） | 🟡 | `cluster_heartbeat_selection.rs` + `mdns_wire` unit |
| 3.C sealed event sync（5） | 🟡 | `life_node_e004_encryption_e2e.rs` 蓋加密；跨裝置 sync SLO（SYN-003 5s）未量 |

**缺口重點**：MAC-CUJ03-SYN-003（5s sync SLO）從沒量過 — §10 列為 P0。

## §4 CUJ-04 degraded states（16 條）

| Case | 狀態 | 對應 |
|---|---|---|
| OFF-002 / LLM-001 stats-only | ✅ **本次** | `cuj04_stats_only_fallback.rs` |
| PROV-006 全 fail | ✅ **本次** | 同上 |
| LLM-002 404 model skip | ✅ | 今天觸發過（手動） |
| LLM-004 no-key UX | ⬜ | 友善錯誤訊息未測 |
| LLM-003 capture 不靠 LLM | ⬜ | local-first capture 未獨立測 |
| DB-001 sqlite 壞檔 recovery | ✅ ship | #143（已 ship） |
| DB-002 first-run auto-create | ⬜ | |
| IK-001/002 identity 壞/空 fail-loud | 🟡 | `key_derivation` unit 蓋 corrupt/empty；CLI 端 exit code 未測 |
| IK-003 regen 後讀舊 event | ⬜ manual | |
| BRK-001/002 broker 401 mid-sync | 🟡 | mock 部分 |
| OFF-001/003 wifi 切換 | ⬜ manual | |
| DSK-001 disk full | ⬜ manual | |
| PERM-001 TCC 撤銷 | ✅ | 今天碰到（手動驗） |

## §5 CUJ-05 export + uninstall（12 條）

| 子群 | 狀態 | 對應 |
|---|---|---|
| backup --to | ✅ | `cuj05_backup_export.rs` |
| data delete --all (+--include-broker) | ✅ | `cuj05_delete_include_broker.rs` |
| identity import | ✅ | `cuj05_identity_import.rs`（5 條全綠） |

**CUJ-05 是覆蓋率最高的 CUJ** — 上個 session（#139–141）已補滿。

## §6–§7 平台專屬 + P4 invariant（14 條）

| 群 | 狀態 |
|---|---|
| PLAT (9, codesign/notarize/launchd/TCC) | 🟡 多 manual；`codesign_macos.rs` / `mlx_macos.rs` / `providers_macos.rs` 摸邊 |
| P4 cross-SPEC invariant (5) | 🟡 加密 invariant 有 `life_node_e004` + `spec15` 蓋 |

---

## 🔴 已知 bug（未修）

| ID | 描述 | 影響 |
|---|---|---|
| **MAC-CUJ02-PROV-009 (Bug A)** | TUI provider error 渲染 leak | release-blocker — demo 開 TUI 時 user 直接看到 |

> Bug A 是目前唯一明確標 🔴 的 release-blocker。本次 session 未碰（屬 TUI 渲染，
> 不在 #142 coach core 範圍）。

---

## 建議下一步優先序（給 user 決策）

依「自動化可達 × demo/開源價值 × 缺口大小」排序：

1. **🔴 Bug A（TUI provider error render leak）** — 唯一 release-blocker，user 直接受影響。
   需先複現（開 TUI 觸發 provider error），再修渲染。
2. **CUJ-02 2.A/2.B integ（food / focus capture）** — 有 wire code、缺端到端 integ；
   可仿 `cuj02_daily_habit_subset.rs` 補，hermetic 可達。
3. **CUJ-04 LLM-004 / LLM-003 / DB-002** — 小而完整的 degraded-path case，wiremock/TempDir 可測。
4. **CUJ-03 SYN-003（5s sync SLO）** — 需要兩節點，較重，但開源 mesh 賣點。
5. **CUJ-01 install.sh e2e** — 價值高（第一印象）但需網路 + 跨環境，自動化成本最高，可能留 manual playbook。

---

## 方法論（workflows 文件如何套用）

本檔遵循 code.claude.com/workflows 的 **explore → plan → code（TDD）** 流程：
- **explore**（本檔）：先盤點現況，不寫 code，產出對照地圖。
- **plan**：user 從上面優先序挑一條 CUJ，定義「完成 = 哪些 case 變綠」。
- **code（TDD）**：先寫 hermetic 測試（wiremock / TempDir / 注入 env），紅 → 實作 → 綠 →
  驗證（真 `cargo test` exit code，不被 pipe 遮蔽）→ commit（含 Claude co-author trailer）。

#142 這次就是這條流程的範例：先補 `cuj04_*` 測試紅 → wire `run_daily_review` → 綠 →
順手修 3 個既有壞檔 → commit。
