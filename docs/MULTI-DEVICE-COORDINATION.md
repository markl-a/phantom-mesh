# 多裝置協調協議（Multi-Device Coordination Protocol）

> **狀態 2026-05-28**：v2 取代 v1（下方 Rules 1-9）。
> v2 = phantom-coord skill + SPEC-80 + SPEC-81 多 CLI 編排（multi-CLI orchestration）出貨。
> v1 = 歷史性的 platform/<os> 分支模型（Rules 1-9）。下方各節
> 保留作為存檔價值；**操作指引以 v2 為準**。

## v2 — phantom-coord skill 模型（自 2026-05-27 起為正式版本）

**唯一來源連結：**
- [`MASTER.md`](superpowers/MASTER.md) §9 4-Machine 並行開發模式（進入點）
- [`ARCH-EXECUTION-ENTITIES.md`](superpowers/ARCH-EXECUTION-ENTITIES.md) §4 Stack C phantom-coord 詳圖
- [`SPEC-80-INFRA-mode-b-collab-dev`](superpowers/specs/v060-deep-spec/SPEC-80-INFRA-mode-b-collab-dev.md) — git-based message bus（基於 git 的訊息匯流排）設計
- [`SPEC-81-INFRA-multi-cli-orchestration`](superpowers/specs/v060-deep-spec/SPEC-81-INFRA-multi-cli-orchestration.md) — Gateway（閘道）routing（路由）6 個 backend CLI（後端命令列工具）+ Phase A→D self-dev arc（自我開發歷程）
- [`configs/README.md`](../configs/README.md) — 每台機器拷哪份 agents.toml

**3 條核心紀律（取代 v1 Rules 1-9 大部分）**：

1. **1 canonical repo, 1 main branch（單一正式倉庫、單一主分支）** ── 4 台機器各自 clone，main 只接 merge commit / fast-forward
2. **每台只 push `wip/<own-host>/<task-uuid>`** ── 不准動別 host 的分支；別 host 的 wip 視為 read-only（唯讀）
3. **Per-task brief + frontmatter（每任務簡報 + 前置中繼資料）** ── `.ai-shared/queue/<host>/<uuid>.brief.md` 含 `scope_files / depends_on / preferred_tool / budget_usd`，phantom-coord follower（跟隨者）自動 claim（認領）+ verify（驗證）+ done（完成）

**4 個 host 角色（依 SPEC-81 + `cluster_nodes.md`）**：

| Host | Platform 工作 | Config | Bootstrap |
|---|---|---|---|
| **mac** (this) | Apple 相關 + Rust core + spec authority（規格權威） | `configs/agents.coordinator.toml` | ✅ cron 5/27 已裝 |
| **node-a** ThinkPad **Windows 11** + WSL2 | **Windows 主力**: MSI / codesign / .NET + WSL Linux + heavy Rust + overnight | `configs/agents.coordinator.toml` | `follower-windows.ps1 -InstallScheduledTask` (admin PS) |
| **node-b** Windows | Android 主力: APK / AVD + Win 副手 | `configs/agents.worker.toml` | `follower-windows.ps1 -InstallScheduledTask` |
| **node-d** Windows | Win 副手 + demo | `configs/agents.worker.toml` | 同 node-b |

**iPhone / iPad / Android = runtime peer（執行期對等節點）**（裝 phantom-mesh app、加入 cluster、收 task；**不寫 code**）。Mobile execution model（行動裝置執行模型）詳見 SPEC-30 §6.5 / SPEC-33 §6.5（embedded-core-no-serve 決議）。

**衝突解決自動化（不靠人手協調）**：

| 場景 | 自動處理 |
|---|---|
| 兩 task 想改同檔 | SPEC-81 P2 `scope_files` lock → 後到 task 等 |
| Task A `depends_on: [B]`，B 未完 | P2 dep check → A 拒 claim |
| 兩 host 同時 `git push main` | atomic race（原子競爭）→ 輸的 rebase 重推 |
| Host 死了帶走 in-progress | 5 min heartbeat（心跳）過期 → `reclaim-dead-host.sh` 撿回 |
| Tool 預算用爆 | P3 budget gate（預算閘門）→ 自動 fallback 到下個 tool |

**Per-machine bootstrap 一行**：
```bash
curl -sSL https://raw.githubusercontent.com/markl-a/phantom-mesh/main/scripts/ai/coord/bootstrap-remote.sh \
  | bash -s -- --host <node-a|node-b|node-d|mac> [--leader]
```

**Per-task dispatch（手動 / 替代 cron）**：
```bash
bash scripts/ai/coord/dispatch-router.sh .ai-shared/queue/<host>/<uuid>.brief.md
```

---

## v1（存檔，2026 年 5 月）— 原始的 platform/<os> 分支模型

> 以下 Rules 1-9 是 5/1 寫的最早協議、保留作 audit。實際運作以上面 v2 為準。
> v1 中提到的 `platform/<os>` 分支、`agents.base.toml` split、`phantom-core-lock.json` 
> 都被 v2 phantom-coord 模型取代。`phantom doctor --mesh` v1 提到的概念在 v2 透過
> `.ai-shared/heartbeat/<host>-last.txt` 達成同樣目的。

---

**本文件是任何 Claude / Cursor / 人類 session（工作階段）在一台以上機器
並行開發 phantom-mesh 時的正式工作協定。** 每個 session — Mac、node-a Windows、
Linux 機器、iOS、Android — 都先讀這份。在全新機器上開啟新 session 嗎？**從這裡開始。**

目標：**讓 5 個平台同時開發，而不讓程式碼庫或部署的 mesh（網狀網路）碎裂。**
Phantom Mesh 的設計讓二進位檔可以自我修改；那唯有在跨機器工作的人類 + agent
都同意一套協調協議時才安全。本文件就是那份協定。

---

## TL;DR — 六條規則

1. **單一正式倉庫、單一 main 分支。** Session 在
   `platform/<os>` 分支上工作。跨平台 / `core/` 編輯走一條
   共享 integration branch（整合分支）再 PR 進 main。
2. **每個 session 都有 scope（範圍）。** 它擁有一條分支、擁有特定路徑，
   其他一切只讀，永不靜默編輯別 session 的
   領地。
3. **二進位檔散佈經由 GitHub release tag，而非本地 cargo
   build。** Tag 觸發跨平台 CI matrix（持續整合矩陣）；每台機器拉
   自己的 release artifact（發行產物）。本地 build 僅供開發用。
4. **設定拆分：base（已提交）+ local（每機器）。**
   `agents.base.toml` 放在倉庫裡。`~/.phantom-mesh/local.toml` 存放 node
   name + cluster secret（叢集密鑰），永不提交。
5. **Wire-protocol（線路協議）版本管理是明確的。** 每個 RPC 回應都帶
   `wire_version`；過時的 peer（對等節點）以清楚的錯誤拒絕不相容的鄰居，
   而非靜默握手失敗。
6. **驗證是每日的、自動化的、一個命令。**
   `phantom doctor --mesh` 是「5 台現在是否都互相通訊」的綠燈/紅燈。

本文件其餘部分逐條說明每條規則、給出衝突
解決流程，並列出尚未實作的部分。

---

## Rule 1 — 分支與合併協議

```
main  ────────────────────────────────────────────────────▶ (protected)
   ▲   ▲                ▲                  ▲           ▲
   │   │                │                  │           │
   │   └─ phase1-r1-foundations ── shared integration branch (current
   │                                "what's everyone working on" trunk)
   │
   ├─ platform/macos     ── Mac session work: core/, app/src-tauri/{macos,ios}/
   ├─ platform/windows   ── node-a session work: scripts/build-windows.sh, app/src-tauri/win/
   │                        ALIAS: feat/windows is currently used by node-a as the
   │                        live working branch (commit f670ab1+). Treated as
   │                        equivalent to platform/windows during the v0.1.0
   │                        freeze. Renames to platform/windows on 5/15 unfreeze
   │                        per SPEC-FREEZE-V1 §13 cleanup.
   ├─ platform/linux     ── Linux box work: scripts/build-linux.sh, systemd templates
   │                        Currently no live branch — Oracle Cloud A1 session
   │                        will create it per SESSION-ONBOARDING.md §3.2.
   ├─ platform/ios       ── iOS-specific Tauri work (lives on Mac)
   └─ platform/android   ── Android Tauri work + phantom-mobile (lives on node-a)
                            ALIAS: feat/android is currently node-a's live
                            working branch (commit 963c3fe). Same equivalence
                            + rename schedule as feat/windows.
```

### 每日流程

1. **Session 開始：** 在本 session 的 `platform/*` 分支上執行
   `git fetch && git rebase origin/main`。若有衝突：見 Rule 7。
2. **Session 進行中：** 自由 commit 到你的 `platform/*` 分支。
   每 30-60 分鐘 push 一次，讓其他 session 看到你的活動。
3. **Session 結束：** 若你動到自己擁有路徑以外的任何東西，
   從 `platform/<os>` → `phase1-r1-foundations` 開一個 PR。標上
   `multi-session` 讓其他 session 看到。
4. **每週：** 在所有 5 個平台 build 都 CI 綠燈後，
   `phase1-r1-foundations` → `main` PR。**只有 main 才是 release tag 的來源。**

### Commit 訊息的 scope 標籤

每個 commit 訊息以一個 scope 前綴開頭：
```
[mac]    fix(tui): cursor position with combining diacritics
[win]    feat(scheduled-task): bootout race fix in restart logic
[core]   fix(mesh): wire_version field on /rpc/ping
[shared] docs: update CO-EVOLUTION roadmap for Phase 4 land
```

`[core]` 和 `[shared]` commit 在 PR review 時會吸引額外注意，
因為它們影響每個 session。

---

## Rule 2 — Scope 紀律

每個 session 被賦予 3 層權限。

| Session | 擁有（可自由編輯） | 讀取（僅作 context） | 協調（須先公告） |
|---|---|---|---|
| **node-c (Mac)** | `core/`（預設）、`app/src-tauri/`、`templates/`、`docs/`、`.github/workflows/`、`scripts/build-mac.sh`、`app/src-tauri/ios/` | 一切 | `core/src/mesh.rs`、`core/src/serve.rs::rpc_*`、`core/src/keys.rs`（其他人依賴） |
| **node-a Win** | `scripts/build-windows.sh`、`.github/workflows/release-windows.yml`、`app/src-tauri/android/`、`app/src-tauri/gen/android/` 底下任何東西 | 一切 | `core/` 預設唯讀 |
| **Linux** | `scripts/build-linux.sh`、`templates/phantom-mesh.service.tmpl`（新增）、`dist/linux*` | 一切 | `core/` 預設唯讀 |
| **iOS** | `app/src-tauri/ios/`、signing config（簽章設定）檔案 | 一切 | 無 — 跑在 Mac 上，隔離到僅 iOS 的路徑 |
| **Android** | `phantom-mobile/` 倉庫、`app/src-tauri/android/`（當 node-a 也跑 Android session 時） | 一切 | 無 |

**「協調」** 意指：在編輯那些路徑之一前，先在
`EVOLVE-GOALS.md` 留個註記或開一個 draft PR（草稿 PR）。不要靜默 push `mesh.rs` —
它會破壞每個其他 session 的 `core-sha`。

### 「擁有」真正的意思

若 session A 擁有某路徑，session B 會看到那裡的變更出現在 main 但
**不得編輯它們**。若 B 認為需要某項變更，B 開一個
issue / EVOLVE-GOAL 描述所欲變更，由 A 處理。這
防止「兩個 session 各自用不同方式修同一個 Win 路徑 bug」的競爭。

---

## Rule 3 — 單一二進位真相：GitHub release tag

```
[trigger]                       [process]                    [distribution]
git tag v0.1.x ────────▶  GitHub Actions matrix    ────▶  release artifacts:
git push --tags             ├─ macos-arm64   build      phantom-macos-arm64.tar.gz
                            ├─ macos-x86_64  build      phantom-macos-x86_64.tar.gz
                            ├─ windows-x64   build      phantom-windows-x64.zip
                            ├─ linux-x64     build      phantom-linux-x64.tar.gz
                            ├─ linux-arm64   build      phantom-linux-arm64.tar.gz
                            └─ codesign each artefact   each signed by maintainer key
                                                ▼
                                  every machine: `phantom upgrade`
                                  → curl latest tag's matching artefact
                                  → verify signature
                                  → atomic swap (bootout → swap → bootstrap)
                                  → restart healthcheck
```

**正式部署不用本地 `cargo build && cp`。** 那正是
引入 codesign SIGKILL bug 的整日模式（見 commit
`85c8377`）。本地 cargo build 保留 — 但僅供開發 /
測試你即將 PR 的變更。**Mesh 跑的是來自
release tag 的二進位檔。**

**Tag 節奏：** 至少每個協調好的多 session 衝刺一次。
`v0.1.x-multidevice-N` 系列在快速的發佈前階段沒問題；
5/15 之後採用正式 semver（語意化版本）。

---

## Rule 4 — 設定即程式碼、密鑰走頻外

```
[committed]                           [per-machine, .gitignore]
agents.base.toml                      ~/.phantom-mesh/local.toml
├── [providers.*]                     ├── [cluster]
│   default_model, type, url          │   node_name = "mac-coordinator"
├── [agent.*]                         │   cluster_secret = "..."         ← from 1Password
│   provider, tools                   ├── [overrides.providers.opencode]
└── [cluster]                         │   api_key = "..."                ← from shell env
    peers = [list of all 5]               (or rely on api_key_env which we propagate
                                            via service install — see commit dfadc9d)
```

`AgentsConfig::load()` 讀取 `agents.base.toml`，然後將
`~/.phantom-mesh/local.toml` 深度合併在其上。local 中存在的任何欄位都
覆蓋 base。

**Cluster secret 散佈。** 在 node-c (Mac) 上一次性產生：
```
$ phantom keys generate-cluster-secret
   Wrote ~/.phantom-mesh/local.toml with cluster_secret=<16 random bytes>
   Now: copy that line to the local.toml on every other machine.
```

頻外（out-of-band）：1Password 共享項目、加密 gist、經由
ssh 的簽章訊息。密鑰永不出現在 commit 中。

**Per-machine api_key**：依靠 `phantom service install` 將
shell env → plist EnvironmentVariables（commit `dfadc9d`）。金鑰
永不存在於任何已提交的檔案中。

---

## Rule 5 — Wire-protocol 版本管理

`EvolveCheckpoint`、mesh handoff payload（交接酬載）、RPC body 等的 schema（結構描述）
會改變。我們需要舊 peer 以清楚的
錯誤拒絕新酬載，而非在 `serde::de::Error` 上崩潰。

### 我們加什麼

每個 RPC 回應都包含 `wire_version: u32`：
```json
{ "ok": true, "wire_version": 3, "data": {...} }
```

`/rpc/ping` 成為正式的相容性檢查：
```json
GET /rpc/ping
→ { "wire_version": 3, "phantom_version": "0.1.4", "core_sha": "7a3f2b1" }
```

`wire_version` 比本二進位低的 peer：**降級警告**，
但 RPC 仍照走（盡力向後相容）。較高：**拒絕**，
明確錯誤：「peer is wire v4, this binary is v3, run `phantom upgrade`」。

### 何時提升版本

- 在既有 schema 加一個欄位：**不提升**（向前相容）
- 移除一個欄位：**提升**
- 改變一個欄位的型別或語意：**提升**
- 加一個新的 RPC endpoint（端點）：**不提升**（呼叫者對未知者預期 404）

單一整數保存在 `core/src/lib.rs::WIRE_VERSION`。

---

## Rule 6 — 每日驗證：`phantom doctor --mesh`

這是綠燈/紅燈。每個 session、每個工作日、在
任何其他工作之前開啟：

```
$ phantom doctor --mesh

◆ phantom 0.1.4 / wire 3 / core-sha 7a3f2b1
        ↑          ↑           ↑
        │          │           └── content hash; if differs from upstream tag,
        │          │                you've got a local fork
        │          └── this binary's wire version
        └── upstream tag

Peers (configured):
  ✓ mac-coordinator    <mac-tailscale-ip>:7878    wire 3   ↔  same
  ✓ node-a-windows        100.64.0.10:7879    wire 3   ↔  same
  ✗ linux-arm          100.64.0.11:7878  unreachable (no response 5s)
  ⚠ ios-iphone         100.108.x.x:7878     wire 2   ⚠ stale, run phantom upgrade
  ○ android-pixel      100.103.x.x:7878     wire 3   ↔  same  (not currently configured as peer)

Cross-checks:
  ✓ all peers' agents.base.toml SHA matches mine
  ✗ node-a-windows local.toml.cluster_secret SHA differs — HMAC will fail!
  ✓ EvolveCheckpoint schema_version matches across all peers

Summary: 3/5 peers fully aligned. Issues: linux-arm unreachable, node-a secret drift.
```

三個 exit code（離開碼）：
- `0` — 所有 peer 綠燈
- `1` — 降級（一個以上 peer 警告）
- `2` — 故障（一個以上 peer 無法 HMAC 或 schema 不符）

`phantom doctor --mesh --fix` 用於可自動修復的情況（輪替
密鑰、提示 `phantom upgrade` 等）。

---

## Rule 7 — 衝突解決

### 同檔、不同 session

1. 誰先 pull / rebase 誰贏。第二個 session 撞上衝突。
2. 試 `git mergetool`。若是瑣碎的，解決並 force-push 到你的
   `platform/*` 分支。
3. 若衝突涉及業務邏輯（非空白 / 格式），**停。**
   開一個 draft PR，兩份 diff 並排。在 commit 訊息正文中 ping 另一個 session：
   `Resolve in coordination with @session-zwin`。
4. 預設啟用 `git rerere` — 重複的機械式衝突
   在第一次之後自動解決。

### 對 `core/` 的並行編輯

嚴格規則：**沒有明確交接時，不得有兩個 session 同時編輯 `core/`**。
使用「lock file（鎖檔）」模式：

```
~/.phantom-mesh/core-lock.json — committed, in repo root as `.phantom-core-lock.json`
{
  "owner_session": "mac-m3",
  "acquired_at_ms": 1777580000000,
  "intent": "fixing TUI scroll calc",
  "expires_at_ms": 1777583600000
}
```

一個 session 透過在其第一個 commit 編輯此檔來取得鎖，透過
在最後一個 commit 刪除它來釋放。鎖在 1 小時後自動過期。Session
on-fetch 時輪詢：若 lock 檔存在且不是你的，改去做 `app/`
或平台特定路徑的工作。

**是的這很非正式。** 對能彼此交談的 5 個人類 + agent 來說夠用了；
我們不需要真正的 lock service（鎖服務）。若規模
超過那個，就改用 GitHub branch protection（分支保護）+ 必要審查者。

---

## Rule 8 — Session 在 push 前必跑什麼

Pre-push 檢查清單（每個 session 在本地執行）：

| 檢查 | 何時 | 命令 |
|---|---|---|
| `cargo fmt --check` | 總是 | 已在 pre-commit |
| `cargo clippy -D warnings` | 總是（每平台） | 讓平台特定的 bug 可見 |
| `cargo test --lib` | 總是 | 抓共享程式碼的破壞 |
| `cargo build --release --target <platform>` | 平台特定 | 證明該平台仍能編譯 |
| `phantom doctor --mesh` | 合併進 main 前 | 證明執行期 mesh 仍可運作 |

檢查失敗 → 不要 push。開一個 draft PR 記錄失敗
並 ping 求助。

---

## Rule 9 — 稽核軌跡

每個 commit 日後僅靠 `git log` 即可審查。本協議
新增的東西：

- **Scope 前綴**（`[mac]`、`[win]`、`[core]`、`[shared]`）— 可搜尋
- **Co-Authored-By: Claude Opus 4.7 (1M context)** trailer（結尾標記）— 已
  建立的慣例（memory `feedback_commit_attribution.md`）
- **`[Session: <name>]`** trailer，當 push 來自非預設
  session 時，例如：
  ```
  [win] feat(scheduled-task): bootout race fix

  Add 300ms sleep between bootout and binary copy so launchd
  releases the binary mapping before we overwrite. Reproduces on
  Win10/Win11 alike.

  Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
  Session: node-a-windows
  ```

`git log --grep '\[Session: node-a-windows\]'` 顯示該
session 曾做過的一切。

---

## 上線一個新 session

當你（Claude / 人類 / 等）在新機器上開啟一個 session 時：

1. **從頭到尾讀完本檔。** 不可跳過。
2. **讀 `EVOLVE-GOALS.md`** — 看什麼在進行中、什麼被卡住。
3. **`git pull origin main && git checkout -b platform/<your-os>`**，若
   尚不存在；否則 `git checkout platform/<your-os>` 並
   rebase。
4. **`phantom doctor --mesh`** — 看還有誰在線上。
5. **在你的第一個 commit 中陳述你的 scope**，讓別人看到你出現了：
   ```
   [scope] add session: linux-arm joining the mesh

   Scope:    Linux platform binary, systemd template, /etc/phantom-mesh
   Reads:    everything
   Coordinates: nothing currently

   Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
   Session: linux-arm
   ```

---

## 尚未實作的部分（缺口 + 緩解）

> **狀態更新 2026-05-28**：v1 的 6 個缺口中有 5 個現已由 v2 phantom-coord skill 涵蓋。
> 下表保留作 audit；目前有意義的缺口位於 SPEC-81 §10
> 遷移計畫（P5/Phase B-D）內。

| 缺口 | 狀態 (2026-05-28) | 替代方案 |
|---|---|---|
| ~~RPC 上的 `wire_version`~~ | ✅ 2026-05-01 出貨 | — |
| ~~`phantom doctor --mesh` 一個命令健檢~~ | ⚠️ v2 用 `.ai-shared/heartbeat/<host>-last.txt` + `scripts/ai/coord/reclaim-dead-host.sh --dry-run` 取代 | SPEC-81 §6.2.4 heartbeat protocol |
| `phantom upgrade` 各機自動拉 binary | ❌ 仍要 `curl install.sh \| sh` 重跑 | install.sh 5/27 驗 Mac arm64 OK；其他 OS 404 |
| ~~GitHub Actions 5-OS release matrix~~ | ⏳ 設計階段 — T3 brief `.ai-shared/queue/node-a/task-2026052703.brief.md` 已 ship | 等 node-a bootstrap 後 claim |
| ~~`agents.toml` split drift~~ | ✅ `configs/` 9 個 per-role 範本 + `configs/README.md` 對照表 + `cluster_secret` 走 env var | `.env` 各機獨立、`agents.toml` gitignored |
| ~~`.phantom-core-lock.json` multi-session 保護~~ | ✅ 替換為 SPEC-81 P2 `.ai-shared/conflicts/<file>.lock` (檔案級 scope lock) | `acquire-scope-locks.sh` + `release-scope-locks.sh` |

**v2 gaps 待補（per SPEC-81 §10）**：

| Phase | 內容 | ETA |
|---|---|---|
| P4 node-a/node-b/node-d 真實 bootstrap | follower-windows.ps1 寫完 + 4 機都 cron active | 6/1 |
| P5 multi-CLI 真用（codex/agy 各 task 分派）| 5/27 T1 codex 已驗 ship；剩 T2-T6 待 exercise | 6/3 |
| Phase B `/rpc/swarm` 取代 git push race | 待 G1 Tailscale (✅ stub shipped 087cf12) + G2 swarm-bridge | v0.6.0+ |
| Phase C phantom 自寫 brief（health check loop）| — | v0.7.0 |
| Phase D 閉環 self-improvement | — | v0.8.0+ |

---

## 快速參考卡

把這個釘在每個 session 上：

```
1. git rebase origin/main          ← session start
2. work in platform/<your-os>       ← never main directly
3. commit with [scope] prefix       ← every time
4. phantom doctor --mesh            ← before merge
5. tag releases on main             ← never on platform/*
6. local.toml stays out of git      ← always
7. cluster_secret never in commits  ← always
8. wire_version mismatch = stop     ← upgrade first
```
