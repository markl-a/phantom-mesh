# 協同演化架構（Co-Evolution Architecture）

phantom-mesh 如何處理 **agent 自我修改（agent self-modification）** 與
**跨所有已安裝實例維持一致共享程式碼庫（coherent shared codebase）** 之間的張力。

簡短版：每位使用者的 phantom 都能自主修正它在自己機器上遇到的問題，
而那些修正會以 PR（pull request，拉取請求）形式向上游（upstream）匯流到單一權威
（canonical）發行版。三個層級（sandbox（沙盒）→ recipes（配方）→ core PR（核心 PR））
讓「phantom v0.1.x」在每台機器上都是同一個東西，同時保留每位使用者自由演化的空間。

## 問題所在

`autoevolve` 讓 phantom 二進位檔（binary）修改自己的原始碼。如果我們原封不動地交付這個功能，
每位使用者安裝的 phantom 在一週後就會分歧成私有分支（private fork）。「phantom v0.1.0」在專案層級的意義
會蒸發殆盡：bug 修正無法傳播、新功能被孤立封存、同一個 mesh（網狀網路）裡的兩個 phantom
無法就 wire format（傳輸線格式）達成一致，而我們發佈的任何「release（發行版）」都會立刻被
本地的 autoevolve commit（提交）覆寫。

這是一個真實的架構決策，不是假設情境。截至 2026 年，在所調查的
14 個主流 AI agent CLI（Aider、Goose、OpenHands、Continue、Cline、
Roo、Claude Code、Codex CLI、Gemini CLI、sst/opencode、fabric、llm、mods、
jcode）中，**只有 jcode 讓 agent 修改 agent 自己的原始碼**，而且即使是
jcode 也沒有解決分歧問題——他們賭使用者是會手動 git-push 的進階使用者
（power-user）。我們在 OSS（開源軟體）發佈時沒辦法下這個賭注。

## 我們**不**採用的兩種模式

**(1) 純粹原始碼修改並重建（jcode）：** 威力最大，但沒有版本敘事。
每位使用者在第一週後都會得到一個 fork。對 OSS 而言是錯的。

**(2) 純粹在二進位檔之外做擴充（Goose、Cline、Continue）：** 二進位檔維持
不可變（immutable）；客製化是放在 `~/.tool/` 裡的 markdown／YAML。一致，但失去了
phantom 的差異化特色——二進位檔本身無法變得更聰明。

我們採取結合兩者的第三條路。

## 模型：Sandbox + Recipes + Gated Core PR（沙盒 + 配方 + 受門控的核心 PR）

```
┌────────────────────────────────────────────────────────────────────┐
│  Tier 1 — SANDBOX  (autoevolve default)                            │
│  Writable: ~/.phantom-mesh/extensions/{prompts,skills,hooks}/      │
│  Read-only from agent: core/*.rs, anything under repo root         │
│  Distribution: optional. Local until user chooses to share.        │
└────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼ user opts to share
┌────────────────────────────────────────────────────────────────────┐
│  Tier 2 — RECIPE  (shareable artifact)                             │
│  Unit: one EvolveCheckpoint exported as content-addressed JSON     │
│  Carries: goal, plan, dead-ends, journey, patch (if any), descriptor│
│  Signed: ed25519 (per-user key in ~/.phantom-mesh/keys/)           │
│  Distribution: gist, git remote, or registry repo (Tier 2.5)       │
└────────────────────────────────────────────────────────────────────┘
                                  │
                                  ▼ recipe touches core/*.rs
┌────────────────────────────────────────────────────────────────────┐
│  Tier 3 — CORE PR  (gated upstream merge)                          │
│  Gate: --allow-core-evolve flag + interactive consent + signature   │
│  Output: NOT a local commit. A `git format-patch` on a fork branch │
│          + automated PR via `gh api` to the upstream repo          │
│  CI: cross-platform test matrix (mac/win/linux), CodeQL,           │
│       sensitive-path human-review gate (auth/, mesh.rs, keys.rs)   │
│  Merge: automerge bot if green AND no sensitive paths touched      │
│         else: human review label                                   │
│  Release: tagged version → all phantoms `phantom upgrade`          │
└────────────────────────────────────────────────────────────────────┘
```

### 為什麼這行得通

- **Tier 1 是 autoevolve 的預設值。** 使用者安裝 phantom 並開啟
  autoevolve，並不會悄悄開始變動（mutate）Rust 原始碼。他們磁碟上的 `core/*.rs`
  會持續與上游的 `phantom v0.1.x` 相符。Agent 改善的是
  prompt（提示詞）、hook（鉤子）與特定使用者的調適——恰恰是 agent CLI 生態系其餘成員
  都同意應該可客製化的那個介面層。

- **Tier 2 是跨域交流（cross-pollination）的基本單位。** EvolveCheckpoint 本身已經是一份
  內容定址（content-addressed）的 JSON 文件，帶有完整的稽核軌跡（audit trail）（goal、plan、dead-ends、
  journey、artifacts、binary swaps）。加上 `phantom evolve publish` 與
  `phantom evolve adopt <recipe>` 就把它變成一個配方生態系，恰恰是
  Sakana AI 的演化式模型合併（Evolutionary Model Merging）模式：小而宣告式的產物（artifact），
  笨重的東西在本地重建。

- **Tier 3 是上游得以存續的方式。** 多數 OSS 專案之所以能存活，
  是因為貢獻會回流。我們只是把這個回流自動化：使用者 A 的 Mac 上的 phantom
  發現一個 CJK（中日韓文字）渲染 bug，autoevolve 修好它，該配方被
  標記為觸及核心原始碼，`--allow-core-evolve` 經互動式批准，
  該 patch（修補檔）變成一個 PR，CI 跑的是每個人類 PR 都會跑的同一套跨平台
  測試矩陣，automerge bot（自動合併機器人）把它合進去，下一個 release 就把它交付給使用者
  B 和 C。

### 為什麼是三層，而不是一層

如果我們只做 Tier 1，phantom 永遠不會成為一個會自我改進的 Rust agent——
只會是一個自我改進的 prompt 集合。我們會失去差異化要素。

如果我們只做 Tier 3（所有東西都走受門控的 PR），像「我偏好把輸入框放在最上面」
這種例行的個人微調，都得跑一趟 CI 來回。摩擦力會扼殺
採用率。

如果我們只做 Tier 2（自由分享配方而沒有核心 PR），phantom 就會變成
沒有權威版本的無邊界外掛濃湯（plugin soup），跟 jcode 同樣的處境。

三層讓使用者在自己那塊地盤上擁有 jcode 等級的威力（Tier 1+2），同時
保留「phantom v0.1.x 在任何地方都代表同一個東西」（Tier 3 對權威 release 設下門控）。

## 實作階段

| 階段 | 目標 | 狀態 |
|---|---|---|
| **0. Foundations（基礎建設）** | EvolveCheckpoint 模組 + mesh handoff（網狀交接） | ✓ 已交付（Phase 1+2） |
| **1. Sandbox（沙盒）** | autoevolve 受限於 `~/.phantom-mesh/extensions/`；`core/` 唯讀 | 待辦 |
| **2. Recipe export/import（配方匯出／匯入）** | `phantom evolve publish/adopt` + 內容定址的簽署 JSON | 待辦 |
| **3. Trust chain（信任鏈）** | 對每份已發佈配方做 ed25519 簽署 + 維護者金鑰圈（maintainer keychain） | 待辦 |
| **4. Core-PR pipeline（核心 PR 管線）** | `--allow-core-evolve` flag → fork 分支 + 透過 `gh api` 自動發 PR | 待辦 |
| **5. CI gate + automerge（CI 門控 + 自動合併）** | GitHub Actions 跨平台測試矩陣 + automerge bot | 待辦 |
| **6. Sync（同步）** | `phantom upgrade` 拉取已簽署的 release；每日計時器抓取新配方 | 待辦 |

每個階段是一個 commit。各階段不依賴下一個；我們可以在 Phase 1 之後停下來
就擁有一個有用且已沙盒化的產品。Phase 4+5+6 合起來才解鎖
協同演化迴圈（co-evolution loop）。

## 信任模型

聯邦式自動 PR（federated auto-PR）的難題是惡意 patch：壞行為者
發佈一個夾帶後門（backdoor）的「修正」，CI 通過、automerge 合進去，
1000 個 phantom 升級，全部被植入 root 權限。

依重要性排序的防禦措施：

1. **敏感路徑人工審查門控（sensitive-path human-review gate）。** 任何觸及 `core/src/auth/`、
   `core/src/mesh.rs`、`core/src/keys.rs`、`core/src/serve.rs::rpc_*` 或
   `templates/*.plist.tmpl` 的 patch，無論 CI 狀態如何都需要人工審查。
   在 `.github/co-evolution.toml` 裡定義為一個 label（標籤）。

2. **維護者金鑰圈（maintainer keychain）。** 每份已發佈的配方都經 ed25519 簽署。
   上游 repo 維護一份 `MAINTAINERS.md` 列出受信任的公鑰。
   來自未列名金鑰的配方會進入一個獨立的「community（社群）」佇列，門控更嚴格
   （更多審查者、自動合併前有更長的沉澱期（soak time））。

3. **沙盒化 CI（Sandboxed CI）。** 測試在全新容器（container）中執行，沒有任何 secret（機密）、
   除了 crates.io 之外沒有網路存取、在測試樹之外沒有寫入權限。
   即使某個 patch 試圖外洩資料，也沒有東西可拿。

4. **CodeQL + cargo-audit + clippy `-D warnings`。** 標準的供應鏈衛生（supply-chain hygiene）。

5. **已簽署的 release（Signed releases）。** 每個 release tag 都經簽署。`phantom upgrade` 會
   針對嵌在前一個二進位檔裡的硬編碼維護者金鑰驗證簽章。
   要破壞它需要同時掌控上游**以及**一個先前的 release。

6. **automerge 要求至少 2 個平台 CI 通過（green）。** 單一平台
   CI 失敗就足以要求人工審查。可防禦
   針對特定平台的惡意分支。

## 版本控制

`phantom --version` 會印出三個數字：

```
phantom 0.1.4 / core-sha 7a3f2b1 / extensions-rev 23
        ↑           ↑                  ↑
        │           │                  └── monotonic counter, user-local
        │           └── content hash of core/ — should match
        │                upstream tag's sha; if not, you forked
        └── upstream release semver
```

現有工具把這三者塌縮成一個數字。我們保持它們分開，這樣
「我分歧了嗎？」就能機械式地被回答：`core-sha` 與上游
release sha 不同，當且僅當使用者（或他們的 autoevolve）修改了 `core/`。那就是
「你跑的是不是權威 phantom？」這個問題的單一位元（single bit）答案。

## 路線圖（Roadmap）

- **5/2（週六）** — 本文件 + 5 個目標排入 `EVOLVE-GOALS.md` 佇列
- **5/2-3 週末** — Phase 1（Sandbox）
- **5/3-4** — Phase 2（Recipe export/import）
- **5/5 週一** — Phase 3（Signing，簽署）
- **5/6-7** — Phase 4+5（Core-PR pipeline + CI gate）
- **5/8** — Phase 6（Sync）+ 第一次端對端（end-to-end）測試
- **5/9** — 面試展示（interview demo）：Mac 上的 phantom 演化出一個 bug 修正、自動發 PR、
  Linux 上的 phantom 拉取已合併的 release
- **5/15** — OSS 發佈

## 參考資料（References）

綜整自 2026-05-01 進行的研究調查，涵蓋 jcode、Aider、
Goose、Continue、Cline、Roo Code、Claude Code、Codex CLI、Gemini CLI、
sst/opencode、fabric、llm、mods、OpenHands、NixOS overlays/flakes、
Sakana AI 的 CycleQD + 演化式模型合併、POET/Enhanced-POET、
MAP-Elites、Homebrew taps、OSS-Fuzz、GitHub Copilot Autofix、Project Naptime。
