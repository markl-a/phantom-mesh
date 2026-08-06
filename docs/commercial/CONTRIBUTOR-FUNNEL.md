> ⚠️ pre-pivot — 方向已被現行 4-pillar Life/Work(superpowers/BIG-GOAL.md)取代;戰術細節或許仍可用。

# Contributor Funnel — 使用者端 autoevolve 作為上游貢獻迴圈

**Status（狀態）**: 🟡 DESIGN DRAFT（設計草稿）— 紀錄使用者 5/2 的願景；目標為 v0.2-v0.3 衝刺
**Effective（生效時間）**: 尚未生效 — v0.1.0 出貨時不含此功能；這是上線後的演進
**Companion（搭配文件）**: `docs/CO-EVOLUTION.md`（Tier 1/2/3 沙箱模型 — 此基礎）
**Authority（權威性）**: 本文件擴充 CO-EVOLUTION.md；並非取代它。

---

## 0. 願景（使用者的描述，2026-05-02）

> 「每位使用者運行 spectyn 並使用 LLM（大型語言模型，或修改自己的版本）；
> 應該要有一套機制，讓他們的修改成為一個候選版本，使所有系統都能運行。
> 使用者的名字會被加入 contributor（貢獻者）清單。
> 他們的版本可以維持為自己客製化的特化版，
> 但他們的修改可以向上流動到開發者審查。
> 他們解決的 issue（問題）、建構的功能，都能進入下一個版本。」

這就是**貢獻漏斗（contribution funnel）**：使用者修改 → 自動
成為候選 → maintainer（維護者）審查 → 下一個釋出版本含有他們的
名字 → 所有使用者在執行 `spectyn upgrade` 時得到此修正。

這就是 OSS（開放原始碼軟體）的夢想：**每位使用者都可能是貢獻者；每一個
成功的本地修正都可能是一次全域升級**；除了使用者選擇是否分享之外，沒有任何阻力。

---

## 1. 目前 autoevolve 已經做到的事（此基礎）

| Capability（能力） | Status（狀態，5/2） |
|---|---|
| 單次 agent 迴圈（`spectyn evolve "<goal>"`） | ✅ 4/27 首次成功，在免費方案上花費 $0 |
| EvolveCheckpoint 作為 content-addressed（內容定址）JSON（原子儲存、稽核軌跡） | ✅ Phase 1 出貨 |
| Mesh handoff（網格交棒）RPC（HMAC 保護的跨機接力棒） | ✅ Phase 2 出貨（commit `027afe8`） |
| 目標佇列（`EVOLVE-GOALS.md` 往返解析） | ✅ 已出貨 |
| 免費方案 provider（供應商）鏈（Groq + opencode `*-free`） | ✅ 已出貨 — autoevolve 運行成本為 $0 |
| 分散式 evolve（`--distributed` 拆解並扇出） | 🟡 接線已存在，但尚未經真實網格驗證 |

→ **EvolveCheckpoint JSON** 已經是正確的價值單位。
它攜帶 goal（目標）+ plan（計畫）+ dead-ends（死路）+ patch（修補）+ journey（歷程）。漏斗只
需要再加上：identity（身分）、broker ingestion（中介伺服器擷取）、上游 PR 管線、
attribution（署名歸屬）。

---

## 2. 使用者願景中尚缺的部分（具體缺口清單）

| # | Gap（缺口） | Status（狀態，5/2 更新） | Lands in（落地於） |
|---|---|---|---|
| 1 | **每位使用者的 identity（身分）** — ed25519 金鑰對 | ✅ 已出貨（commit `4a61a0c`） | v0.1.0 — `spectyn keys init` |
| 2a | Recipe（配方）匯出 — `spectyn evolve publish`（本地 + ed25519 簽署） | ✅ 已出貨（commit `cbbbe50`） | v0.1.0 — `--private` 為預設 |
| 2b | `spectyn evolve adopt <recipe>` — 驗證 + 套用 | ⏸ 延後 | v0.2 |
| 3 | **Broker（中介伺服器）作為 recipe 收件匣** — spectynmesh.com 接受已簽署的 recipe、分類 tier（分層）、排入佇列 | ⏸ 延後（Cloudflare DNS 遷移為前置條件） | v0.2 |
| 4 | **選擇加入式自動發布** — `spectyn autoevolve --share-recipes` | ⏸ 延後 | v0.2（依賴 §3） |
| 5 | **Auto-PR 管線** — broker fork 上游、推送 patch、開啟 PR | ⏸ 延後 | v0.2 |
| 6 | **合併時自動附加至 CONTRIBUTORS.md** | ⏸ 延後 | v0.2（依賴 §5） |
| 7 | **聲望 / 公開貢獻者儀表板** | ⏸ 延後 | v0.4 |
| 8 | **特化保存** — `~/.spectyn-mesh/extensions/{prompts,skills,hooks}/` 資料夾慣例 | ✅ 已出貨（commit `ed6e2dd`） | v0.1.0 — 資料夾已存在；loader（載入器）也在 v0.1.0 出貨 |
| 9 | **Issue → 解法的署名歸屬** — `spectyn evolve --solve <issue-num>` 關閉該 issue | ⏸ 延後 | v0.3 — 需要 gh api 整合 |
| 10a | **隱私預設** — `--private` 是 `spectyn evolve publish` 的預設 | ✅ 已出貨（commit `cbbbe50`） | v0.1.0 |
| 10b | `--share` 旗標用於明確的選擇加入上傳 | ⏸ 延後 | v0.2（依賴 §3） |
| 11 | **CO-EVO Phase 1 沙箱守衛** — autoevolve 預設拒絕寫入 `core/` `app/` `templates/` `scripts/` | ✅ 已出貨（commit `fcd9bd1`） | v0.1.0 — `--allow-core-evolve` 可選擇退出 |

**Tally（統計）**: 11 項中有 5 項在 v0.1.0 出貨（45%）；6 項延後至 v0.2/v0.3/v0.4。

這五項已出貨的（#1、#2a、#8、#10a、#11）構成了**第一天的 OSS 使用者
基礎設施**：identity + workspace（工作空間）+ sandbox（沙箱）+ 已簽署但本地的
recipe 匯出。其餘六項需要 broker + GitHub OAuth（開放授權）
流程 + automerge bot（自動合併機器人）上線，而這依賴 Cloudflare DNS
遷移（上線後衝刺的 Phase 0）。

---

## 3. 完整架構（3 層流程）

```
═══════════════════════════════════════════════════════════════════════
USER LAYER 使用者層（每位使用者的機器）
═══════════════════════════════════════════════════════════════════════

  Onboarding 入門（一次性）:
    $ spectyn keys init
      → ~/.spectyn-mesh/keys/{ed25519.priv, ed25519.pub}
    $ spectyn keys link --github
      → 與 broker 進行 OAuth 流程
      → broker 儲存: { pub_key → github_user → email }
      → 使用者現在可在整個 mesh 中被識別

  Daily flow 日常流程:
    $ spectyn autoevolve --watch --share-recipes
      ↓
    LLM agent 在 core/*.rs（或 extensions/）中做了一項變更
      ↓
    cargo test 通過
      ↓
    EvolveCheckpoint 序列化至 ~/.spectyn-mesh/evolve-checkpoints/
      ↓
    spectyn evolve publish（自動，若設了 --share-recipes）
      ↓
    Recipe = ed25519 簽署的 JSON，內含:
      - goal, plan, dead_ends, journey
      - patch（git format-patch 的 blob，若有觸及程式碼）
      - descriptor: {platform, spectyn_version, target_files_class}
      - signature: ed25519(body, 使用者的私鑰)
      - author: { pub_key, github_user（入門時連結） }
      ↓
    POST https://spectynmesh.com/recipe

═══════════════════════════════════════════════════════════════════════
BROKER LAYER 中介伺服器層（spectynmesh.com / Cloudflare Workers）
═══════════════════════════════════════════════════════════════════════

  POST /recipe 處理器:
    1. 以使用者已知的公鑰驗證 ed25519 簽章
    2. 依 patch 中的檔案路徑分類:
         - 僅觸及 ~/.spectyn-mesh/extensions/        → Tier 1 catalog（目錄）
         - 觸及 scripts/, docs/, tests/              → Tier 2 fast-track（快速通道）
         - 觸及 core/*.rs, app/*.rs                  → Tier 3 PR 佇列
         - 觸及敏感檔（auth/, mesh.rs, keys.rs）     → Tier 3 + 人工
    3. 記錄於 D1: {recipe_sha, author, tier, classification, status}
    4. 回傳: { recipe_url, tier, status: "queued" }

  Tier 1（僅目錄）:
    → recipe 留在 registry（登錄表），其他人可 `spectyn evolve adopt <url>`
    → 無上游變更

  Tier 2（快速通道）:
    → broker 透過 gh api 自動建立 PR
    → CI 執行（跨平台測試矩陣）
    → 若綠燈 → automerge 至上游
    → 若黃燈 → 通知 maintainer

  Tier 3（core/* PR）:
    → broker 將 markl-a/spectyn-mesh fork 至沙箱 repo
    → 將 patch 推送至名為 auto/<sha> 的新分支
    → 對上游開 PR，body = EvolveCheckpoint markdown
    → 為 PR 標記: auto-evolve, <platform>, <classification>
    → PR body Co-Authored-By: <github_user> <noreply email>

═══════════════════════════════════════════════════════════════════════
UPSTREAM LAYER 上游層（github.com/markl-a/spectyn-mesh）
═══════════════════════════════════════════════════════════════════════

  GitHub Actions / CI:
    .github/workflows/co-evolution.yml
    - 在 macos-latest + windows-latest + ubuntu-latest 上執行 cargo test
    - CodeQL
    - cargo-audit
    - clippy -D warnings
    - 4-agent QA 審查（subagent + codex + gemini + opencode）

  Automerge bot 規則（3 個條件，全部 AND）:
    (a) 所有平台 CI 綠燈
    (b) 未觸及敏感路徑（core/auth/, mesh.rs, keys.rs,
        serve.rs, templates/）
    (c) 作者的公鑰位於 MAINTAINERS.md 信任清單中

  全部 ✓ → automerge → 觸發 release Action
  任一 ✗ → 標記 "human-review-required" → maintainer 人工審查

  Post-merge 合併後自動化:
    1. .github/workflows/credit-contributor.yml
       - 從已合併的 commit 擷取 Co-Authored-By
       - 附加至 CONTRIBUTORS.md（若尚未存在）
       - 開啟後續 PR 以更新 CONTRIBUTORS.md
    2. 自動產生 CHANGELOG 條目:
       "fix(<area>): <goal-summary> by @github_user"
    3. 若符合 auto-tag 規則則標記 release

  每位使用者的 `spectyn upgrade`:
    - Curl 最新 tag 對應的 artefact（產物）
    - 驗證 maintainer 簽章
    - 原子替換（bootout → swap → ad-hoc codesign → bootstrap）
    - 曾貢獻的使用者 A 會看到:
        $ spectyn --version
        → "spectyn 0.2.1 (... built 2026-05-22)"
        → release notes 為他們署名

  使用者 A 繼續運行:
    canonical（標準正規的）spectyn 0.2.1 core
    + ~/.spectyn-mesh/extensions/（他們個人的客製化）

  → 「每個人都用同一份標準正規核心，再加上各使用者專屬的 extensions」
```

---

## 4. 特化保存（「個人 fork」機制）

每位使用者有兩個 scope（範圍）:

```
~/.spectyn-mesh/
├─ keys/                  ← identity（簽署 recipe）
├─ extensions/            ← Tier 1: 個人客製化，永不上傳上游
│  ├─ prompts/
│  │  ├─ coder-vim.md             # 使用者的 vim 風格 coder prompt 覆寫
│  │  └─ master-zh-tw.md          # 使用者的 zh-TW 系統 prompt
│  ├─ skills/
│  │  ├─ git-rebase-helper.json   # 3 步驟的組合工具
│  │  └─ deploy-staging.json
│  └─ hooks/
│     ├─ pre-tool/
│     │  └─ audit-shell.sh        # 記錄每一條 shell 命令
│     └─ post-agent/
│        └─ notify-slack.sh
├─ recipes/               ← Tier 2 候選（本地產生、選擇加入發布）
│  ├─ <sha1>.json
│  └─ <sha2>.json
├─ evolve-checkpoints/    ← autoevolve 狀態（短暫的）
└─ events.jsonl           ← 診斷日誌（永不自動上傳）
```

當 `spectyn upgrade` 替換標準正規二進位檔（例如 v0.1.0 → v0.2.0）時:
1. 二進位檔被原子地替換
2. **`extensions/` 被原封不動地保存**
3. spectyn 在下次啟動時重新載入 extensions，將它們套用於新核心之上
4. 若 extension API 出現破壞性變更（罕見；Tier 1 有穩定的契約）：會提示使用者透過以下方式解決：
   - `spectyn extensions migrate <ext-name>`（盡可能自動修正）
   - 或在附帶變更摘要的情況下接受 extension 的失效

**關鍵不變量（invariant）**: 升級永不靜默地丟棄客製化。
它要嘛乾淨地遷移，要嘛明確地告知使用者。

這正是 **Emacs/Neovim 外掛系統**模型:
- 各處都是同一份標準正規的 Emacs
- 各使用者的 `init.el` / `init.lua` 加上個人客製化
- 新版 Emacs 保存使用者的 `init.el`

---

## 5. 身分 + 署名歸屬（「你的名字進入致謝名單」這一塊）

### 5.1 ed25519 金鑰對（每位使用者、每台機器）

```
$ spectyn keys init
✓ 已在 ~/.spectyn-mesh/keys/ 產生金鑰對
  - ed25519.priv（0600 權限；永不離開此機器）
  - ed25519.pub （首次同步時廣播至 broker）
```

### 5.2 GitHub OAuth 連結（一次性）

```
$ spectyn keys link --github
→ 開啟瀏覽器 → 在 spectynmesh.com 上進行 OAuth 流程
→ broker 儲存: pub_key_b64 → github_username → noreply_email
→ 這是 spectyn 攜帶至上游的唯一識別資訊
```

### 5.3 自動 recipe 發布

```
$ spectyn autoevolve --watch --share-recipes
... [agent 達成目標，cargo test 綠燈]
✓ Recipe ~/.spectyn-mesh/recipes/cdf3a8b9.json（已簽署）
✓ 已發布至 spectynmesh.com/recipe → tier=2, status=queued
✓ PR https://github.com/markl-a/spectyn-mesh/pull/847 由 spectyn-bot 開啟
  Co-Authored-By: yourname <yourname@users.noreply.github.com>
```

### 5.4 合併時自動致謝

`.github/workflows/credit-contributor.yml`:

```yaml
on:
  pull_request:
    types: [closed]

jobs:
  credit:
    if: github.event.pull_request.merged == true
    steps:
      - extract Co-Authored-By trailer from PR commits
      - if not in CONTRIBUTORS.md: open PR appending the user
      - on the next release CHANGELOG, group commits by author
```

合併後，`CONTRIBUTORS.md` 看起來像:

```markdown
## Contributors

- @markl-a (maintainer)
- @user-a (4 PRs: CJK render, ...)
- @user-b (2 PRs: ...)
- @user-c (1 PR: fix the spinner overflow)
```

而下一個釋出版本的 CHANGELOG:

```markdown
## v0.2.1 (2026-05-22)

### Fixes
- fix(tui): CJK render width on combining diacritics — by @user-a (#847)
- fix(mesh): retry deadline jitter — by @user-c (#891)

### Features
- feat(tools): web_search via Brave API — by @user-b (#852)
```

使用者開啟該 PR，在 CONTRIBUTORS.md 中看到自己的名字，在
CHANGELOG 中看到自己的修正。

---

## 6. Issue → 解法的署名歸屬

對於想要主動貢獻（而非僅是偶然貢獻）的使用者:

```
$ spectyn evolve --solve 234
→ spectyn 透過 gh api 取得 issue #234 的內容
→ agent 閱讀 issue、規劃做法
→ 標準 autoevolve 迴圈，附帶額外脈絡: "this is to close #234"
→ 成功時，recipe 攜帶 "solved_issue: 234"
→ 開啟的 PR 含 body 行 "Closes #234"
→ 合併時，GitHub 自動關閉 #234 + 為使用者致謝
```

這讓 spectyn 成為一個**個人單兵貢獻 agent**：提交一個你想修的
issue，執行 `spectyn evolve --solve <num>`，PR 就會自動
開啟。

---

## 7. 聲望 + 能見度（選用但有價值）

spectynmesh.com 公開儀表板:
- 依被接受的 recipe 數排名的頂尖貢獻者
- 依被合併的 Tier 3 PR 數排名的頂尖貢獻者
- 被採用 N 次的 recipe（人氣）
- spectyn 本週解決的 issue

→ 使用者擁有可分享的公開個人檔案。成為一份作品集
作品。部分貢獻者可能獲得:
- 對非敏感路徑的審查者權限
- 對受信任貢獻者的直接合併權限
- 受邀加入私人頻道（測試預先釋出版本）

---

## 8. 隱私 + 選擇退出（負責任的路徑）

### 8.1 預設：選擇加入式發布

```
$ spectyn autoevolve --watch
→ 僅在本地執行；無任何 broker 呼叫
$ spectyn autoevolve --watch --share-recipes
→ 明確選擇加入
```

### 8.2 每個 recipe 的覆寫

```
$ spectyn evolve "<sensitive personal automation>"
... [成功]
$ spectyn evolve publish --private
→ 儲存於本地，不推送至 broker
```

### 8.3 永不上傳的內容

| Data type（資料類型） | 是否送往 broker？ |
|---|---|
| Recipe 內容（goal, plan, patch） | ✅ 當設了 --share-recipes 時 |
| ed25519 公鑰 | ✅ |
| GitHub 使用者名稱 | ✅（你已透過 OAuth 選擇加入） |
| ed25519 私鑰 | ❌ 永不 |
| 當機日誌（Crash logs） | ❌ 永不（留在本地 ~/.spectyn-mesh/crashes/） |
| 對話逐字稿 | ❌ 永不 |
| 工具呼叫輸出（你私有檔案的 file_read） | ❌ 永不 |

唯一離開你機器的東西是 **recipe**（你
選擇分享的內容）。而且只有在你選擇加入時。

---

## 9. v0.2-v0.3 衝刺的具體 EVOLVE-GOALS

依優先順序排列:

### v0.2 衝刺（5/15 → 5/22；1 週）

```
- [ ] CO-EVO Phase 1 — sandbox guard（autoevolve 在沒有 --allow-core-evolve
      旗標時拒絕寫入 ~/.spectyn-mesh/extensions/ 之外）
- [ ] MULTI-DEV Gap 1 — GitHub Actions release 矩陣（跨 5 個平台的
      單一二進位真相）
- [ ] MULTI-DEV Gap 3 — spectyn doctor --mesh（漂移偵測）
- [ ] MULTI-DEV Gap 4 — spectyn upgrade（原子替換並保存
      extension）
- [ ] CO-EVO Phase 2 — spectyn evolve publish/adopt（recipe 匯出 +
      ed25519 簽署，僅本地，尚無 broker）
- [ ] CO-EVO Phase 3 — spectyn keys init + 透過 broker 的 GitHub OAuth 連結
```

### v0.3 衝刺（5/22 → 5/29；1 週）

```
- [ ] CO-EVO Phase 4 — auto-PR 管線（broker fork + 推送 + 開啟
      PR；CI 執行 Phase 5 的 .yml）
- [ ] CO-EVO Phase 5 — CI 關卡 + automerge bot
- [ ] CONTRIBUTOR-FUNNEL §5 — 合併時自動附加至 CONTRIBUTORS.md
- [ ] CONTRIBUTOR-FUNNEL §6 — spectyn evolve --solve <issue> 整合
- [ ] CONTRIBUTOR-FUNNEL §8 — 隱私 + 選擇退出旗標 + 稽核
- [ ] spectyn upgrade 附帶 extension 遷移提示
```

### v0.4 衝刺（5/29 → 6/5；1 週）

```
- [ ] CONTRIBUTOR-FUNNEL §7 — spectynmesh.com 貢獻者儀表板
- [ ] CO-EVO Phase 6 — spectyn evolve sync（每日上游拉取
      選用；替換前驗證簽章）
- [ ] Recipe registry / 人氣計數器 / 搜尋
- [ ] 基於聲望、給予受信任貢獻者的審查者權限
```

---

## 10. 為何這行得通（信任模型）

CO-EVOLUTION.md 的 3 層模型提供了圍堵。
貢獻者漏斗加上了署名歸屬 + 雙向流動。兩者結合:

| Concern（顧慮） | Resolution（解法） |
|---|---|
| 「使用者改了它，但我被迫跟著走」 | Tier 1 留在本地；只有 Tier 2/3 在 CI +（通常）人工審查後才上游 |
| 「使用者的特化版本」 | Tier 1 extensions 是隔離的；標準正規核心維持不變；兩者各自獨立更新 |
| 「變更品質參差不齊」 | 每個 PR 都跑 CI 矩陣；敏感路徑需要人工；受信任貢獻者獲得快速通道 |
| 「貢獻者有獲得致謝嗎」 | CONTRIBUTORS.md 自動附加、CHANGELOG 依作者分組、公開儀表板 |
| 「使用者隱私」 | 預設為選擇加入；沒有明確的 `--share-recipes` 就不會有任何東西離開機器 |
| 「惡意行為者問題」 | 每個 recipe 都有 ed25519 簽章；broker 可撤銷某把金鑰；敏感路徑以人工審查把關 |
| 「像 jcode 那樣的 fork 漂移」 | release 矩陣 = 單一二進位真相；`spectyn doctor --mesh` 漂移偵測；`spectyn upgrade` 原子替換 |

這是帶有 auto-PR 人因設計的 **Linux 核心模型**:
- Linus / lieutenants（副手）= automerge bot + 敏感路徑的 maintainer
- 任何人都能修補上游 = `spectyn evolve --share-recipes`
- Linux 基金會負責 release 矩陣 = GitHub Actions
- 長尾核心使用者保留自訂設定 / 模組 = Tier 1 extensions
- 新核心對照你的設定進行 rebase = `spectyn upgrade --migrate-extensions`

---

## 11. 它不會做的事（刻意設下的界限）

- **不靜默上傳。** 每一次分享都是選擇加入。
- **不強制更新。** 使用者可以永遠停留在舊版 spectyn。
- **不對 fork 進行中央控制。** 任何人都能 fork spectyn-mesh 並
  運行自己的 broker；auto-PR 流程只對
  `markl-a/spectyn-mesh` 的上游有效，因為那是
  broker 指向的地方。
- **recipe 產生時不強制程式碼品質。** 品質檢查是在
  上游的 CI，而非 broker（broker 是一層薄薄的分類層）。
- **不接受匿名貢獻。** 若你想讓自己的名字出現在
  CONTRIBUTORS.md，你必須連結一個 GitHub 帳號。（你可以使用
  化名的 GitHub 帳號。）

---

## 12. 待解問題（針對 v0.5+）

- **多 broker 聯邦（federation）** — 是否應該有替代的 broker
  （不只 spectynmesh.com）？對於想在公司內部有私有
  貢獻者漏斗的組織很有用。
- **Recipe 版本管理** — recipe 標示「for spectyn 0.2.0」；它能否
  被自動 rebase 到 0.3.0？
- **Recipe registry 搜尋** — 對 goal 文字做模糊比對，「此修正
  與 issue #X 相似」
- **跨 recipe 組合** — recipe A 與 recipe B 都觸及
  tui.rs；broker 能否將它們序列化排入佇列，或進行自動合併解析？
- **Recipe 聲望** — recipe 被 1000 位使用者採用；這是否應該
  自動晉升至 Tier 3 快速通道？

---

## References

- `docs/CO-EVOLUTION.md` — 3 層沙箱 / recipe / core PR 模型（本文件擴充的此基礎）
- `docs/SELF-EVOLVE.md` — 首次成功自我修正的逐字稿（4/27）
- `core/src/evolve_checkpoint.rs` — EvolveCheckpoint 序列化的 JSON 形狀
- `EVOLVE-GOALS.md` — v0.2 衝刺目標佇列（CO-EVO Phase 1-6 + MULTI-DEV Gap 1-6）
- Sakana AI 的 Evolutionary Model Merging — recipe 模式的靈感來源
- jcode `ReloadContext` — EvolveCheckpoint 的靈感來源（單機；本文件擴充至 mesh）
