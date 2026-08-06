# Mac 全場景測試覆蓋率地圖 (2026-05-31) — canonical

> 對照 `docs/test-cases/mac.md` 的 138 條 MAC-* case vs `core/tests/` 實際存在的
> 自動化測試。產出「✅ 已覆蓋 / 🟡 partial / ⬜ 未覆蓋 / 🔴 有 bug」分類，供決定
> 下一步打哪。本檔是 workflows 文件（explore → plan → 漸進覆蓋）的 explore 產物。
>
> **此檔是 Mac 5-CUJ 測試的單一 canonical 覆蓋率地圖。** 配套三件：
> - **case DB（machine-readable）**：[`docs/test-cases/mac.md`](mac.md) ── 138 條 MAC-* 的 `cmd`/`expected`/`last_run` 欄，CLI runner 直接讀。
> - **人類 run-sheet**：[`docs/manual-playbook/mac.md`](../manual-playbook/mac.md) ── ship 前親手跑兩遍的 ✓/✗ 表。
> - **ship readiness**：§ Ship readiness（本檔下方，由舊 `docs/oss-ship/mac.md` 收斂而來）── BIG-GOAL 對映、SPEC 反查、build→R2→install 管線、5-gate noon-ship checklist。
>
> 另一份不同範疇的清單：[`docs/integration/2026-05-30-mac-app-cli-test-playbook.md`](../integration/2026-05-30-mac-app-cli-test-playbook.md)（99 條 **app+CLI/serve/browser** 功能級盤點，S1–S7）── 不是 5-CUJ 覆蓋表，互補不重疊。

## 怎麼讀這張地圖

- **✅ 已覆蓋** = 有對應的綠色 `cargo test`（unit / integ / e2e），最近驗過。
- **🟡 partial** = 邏輯有 code、但測試只覆蓋一部分（或只有 wire-shape unit test，
  缺端到端 integ）。
- **⬜ 未覆蓋** = 完全沒有自動化測試（多為 manual=❌ 或還沒寫）。
- **🔴 有 bug** = 已知缺陷，未修。

## 總覽（138 條）

| 分類 | 條數 | 佔比 |
|---|---:|---:|
| ✅/🟢 已覆蓋 / 已驗 | 59 | 43% |
| 🟡 partial | 18 | 13% |
| ⬜ 未覆蓋 | 54 | 39% |
| 🔴 重要缺 | 7 | 5% |
| （其中 ❌ manual-only 27 + ⏰ cron 3 = 30 條非自動化可達） | | |

> 2026-06-11 機械重數（`grep -c '^| MAC-'` = 138；狀態欄依首個 emoji 分類）。
> 先前版本標 136 條是表頭小計脫節（2.D 實為 11 條、CUJ-05 實為 13 條）；
> mac.md §9 同步修正。

---

## §1 CUJ-01 install → 第一筆 habit（35 條）

| 子群 | 狀態 | 備註 |
|---|---|---|
| 1.A install.sh 抓+解（12） | ⬜ 大多未覆蓋 | manual / 需網路；`v4_e2e_desktop_macos.rs` 摸到 S3 wizard 邊緣 |
| 1.B codesign / ad-hoc（5） | 🟡 | `codesign_macos.rs` 存在，覆蓋部分 |
| 1.C first-run 初始化（10） | 🟡 | `identity_init_outcome_integration.rs` 蓋 identity 初始化；agents.toml seed 未測 |
| 1.D 第一條 habit（8） | ✅ 部分 | `cuj02_daily_habit_subset.rs` 蓋 palette CRUD + checkin（與 1.D 重疊） |

**缺口重點**：install.sh 端到端（下載→解壓→第一次跑）零自動化覆蓋，純靠 manual。

## §2 CUJ-02 daily capture loop（45 條）

| 子群 | 狀態 | 對應測試 |
|---|---|---|
| 2.A food photo（8） | 🟡 | FOOD-001 → `cuj02_capture_hermetic.rs`（hermetic：wiremock LLM + inlineData + 加密落地 + 讀回 + P4）；CLI→serve daemon hop 仍只有 key-gated e2e（`life_node_capture_e2e.rs`） |
| 2.B focus audio（7） | 🟡 | FOC-001 → `cuj02_capture_hermetic.rs`（hermetic：`life_node::focus_session` 全 lifecycle + age 加密讀回）；audio/ASR 子句 N/A（CLI 無 audio capture） |
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

## §5 CUJ-05 export + uninstall（13 條）

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
2. ~~**CUJ-02 2.A/2.B integ（food / focus capture）**~~ — ✅ 2026-06-11 已補
   `cuj02_capture_hermetic.rs`（FOOD-001 / FOC-001 hermetic）；殘餘缺口 =
   CLI→serve daemon hop（key-gated e2e only）+ focus audio/ASR（feature 未落地）。
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

---

# Ship readiness（由舊 `docs/oss-ship/mac.md` 收斂）

> Ship 目標：terminal 上 `curl install.sh | sh` 安裝、user 在 terminal 跑 `spectyn` 各 command。
> **Binary 物理檔、不開源 source**。本段保留 oss-ship 表獨有的 BIG-GOAL 對映、SPEC 反查、build→R2 管線與 5-gate ship checklist；
> 逐條 CUJ 狀態以上方覆蓋率地圖為準（單一真相）。

## SR-0. Anchor — BIG-GOAL 對映

| BIG-GOAL 層 | Mac 角色 |
|---|---|
| Pillar P1 跨裝置 mesh | mac 是 mesh 對等節點 (mDNS 自動發現 + RPC) |
| Pillar P2 多模態 | mac CLI 是 capture command 入口 (`spectyn food` / `focus` / `habit`) |
| Pillar P3 進化網 | mac 跑 coach + 技能庫 (skill extraction 主要在桌機) |
| Pillar P4 加密為先 | mac 是 identity.key 持有者 + events sqlite host (v0.6.0 範圍) |
| Track | Life + Work 共用 (mac 是 Work Track 主機型) |
| Anti-goal 不違反 | ✓ 非 IDE 綁定 / ✓ 非純雲端 / ✓ 走 CLI 非 GUI 優先 |

## SR-1. SPEC × Feature 反向追蹤 (Mac 涉及的 SPEC)

| SPEC | Mac 用到的功能 |
|---|---|
| SPEC-12 identity | first-run identity.key |
| SPEC-13 encryption | event 加密 (capture + sync) |
| SPEC-14 LLM providers | capture + fallback chain |
| SPEC-15 broker vault | cross-device sync |
| SPEC-16 event storage | events.sqlite (first-run + capture) |
| SPEC-20 capture-food | `spectyn food` |
| SPEC-21 capture-focus | `spectyn focus` |
| SPEC-22 capture-habit | `spectyn habit` + streak |
| SPEC-23 coach-engine | `spectyn coach review` |
| SPEC-24 coach-delivery | coach markdown 輸出 + telegram |
| SPEC-25 skill-extraction | (P3 技能庫、background) |
| SPEC-26 cluster-dispatch | cluster join |
| SPEC-28 30s-hello | 整個 onboarding |
| SPEC-40 macOS foundations | 啟動 + 系統整合 (LaunchAgent) |
| SPEC-41 macOS screens-flows | (本次 ship 主走 CLI、screen 不重點) |
| SPEC-50 broker-api | server-side endpoint |

## SR-2. Build pipeline → R2 → install

**A. 本機 build + 上傳 (繞 CI)**
```bash
cd core
cargo build --release --bin spectyn --target aarch64-apple-darwin
cargo build --release --bin spectyn --target x86_64-apple-darwin

# SHA256 sidecar
cd target
shasum -a 256 aarch64-apple-darwin/release/spectyn > spectyn-aarch64-apple-darwin.sha256
shasum -a 256 x86_64-apple-darwin/release/spectyn > spectyn-x86_64-apple-darwin.sha256

# Rename + upload to R2 (user 操作 wrangler)
cp aarch64-apple-darwin/release/spectyn /tmp/spectyn-aarch64-apple-darwin
cp x86_64-apple-darwin/release/spectyn /tmp/spectyn-x86_64-apple-darwin
wrangler r2 object put spectyn-binaries/spectyn-aarch64-apple-darwin --file /tmp/spectyn-aarch64-apple-darwin
wrangler r2 object put spectyn-binaries/spectyn-x86_64-apple-darwin --file /tmp/spectyn-x86_64-apple-darwin
wrangler r2 object put spectyn-binaries/spectyn-aarch64-apple-darwin.sha256 --file ...
wrangler r2 object put spectyn-binaries/spectyn-x86_64-apple-darwin.sha256 --file ...
```

**B. 確認 install.sh 認 mac 命名規約**
```bash
SPECTYN_INSTALL_DRY_RUN=1 sh scripts/install.sh
# 預期印: would download https://phantommesh.io/dist/spectyn-aarch64-apple-darwin
```
如果命名不對、改 install.sh map 表。

**C. 補 publish-spectyn-binary.yml mac target (永續方案)** — 加 `aarch64-apple-darwin` / `x86_64-apple-darwin`；需 macos-latest runner、燒 10× cost，先繞、之後再修。

## SR-3. 完整 ship checklist (5 道閘)

> 對應測試 ID 見上方覆蓋率地圖 §8 ship gate 對映 / `mac.md §8`。

- **Gate 1 — Mac binary 本機 build**：`cargo build --release --bin spectyn --target {aarch64,x86_64}-apple-darwin` 兩個 `--version` match `spectyn v?0\.6\.0(-rc\.[0-9]+)?`、`--help` ≥ 10 subcommand。
- **Gate 2 — 上傳 R2 + install.sh**：兩 binary + 兩 .sha256 上 R2、清乾淨 `~/.spectyn-mesh/`（先備份）後跑 `curl phantommesh.io/install | sh`、`~/.spectyn-mesh/bin/spectyn` 存在可執行、PATH 自動加入。
- **Gate 3 — CUJ-01 first habit**：`spectyn habit water --qty 250` 不報錯、`spectyn habit streak --chip water` ≥ 1、`events/<uuid>/` 目錄 ≥ 1（或 EventStore 讀回斷言）。
- **Gate 4 — CUJ-02 一個 capture**：`spectyn food --image <test.jpg>` 走 LLM、`spectyn coach review` 產 markdown。
- **Gate 5 — CUJ-03 mac+mac**：mac1 `spectyn login` → token、log 一筆 habit、mac2 同帳號 `spectyn habit streak` 看得到 ≤ 30s。

**5 道 gate 過 = ship ok**；Gate 5 fail 但 1–4 過 = 退化版 single-mac ship 也可接受。

## SR-4. 自動化測試補洞 backlog（ship 後做）

| Priority | Test | 估時 |
|---|---|---|
| P0 | cuj01 install smoke (shell test) | 1 day |
| P0 | cuj02 食/焦/coach 各補 integration test | 2 days |
| P0 | cuj03 cross-device sync e2e (broker mock + 兩 EventStore) | 2 days |
| P1 | cuj04 degraded 7 scenario | 3 days |
| P2 | cuj05 export+delete CLI MVP | 1 week (含 broker side) |

## SR-5. README 應更新處

1. Version badge: v0.5.0 → v0.6.0（內文已有但 badge 沒同步）
2. §安裝 加 mac 段：「mac 上開 terminal: `curl -fsSL https://phantommesh.io/install | sh`」
3. §已知限制：「CUJ-05 export+uninstall — 早期狀態、EU release pending」（現況見覆蓋率地圖 §5：CUJ-05 已是覆蓋率最高的 CUJ）
4. §測試現況：指向本覆蓋率地圖 + `docs/status.md`
