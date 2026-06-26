# SPEC-25 Skill Extraction (skill bank) — Windows Wireframe（線框稿）

> **Stage 1/3** · 線框稿（wireframe，低保真版型骨架）→ [視覺稿（mockup，待補）] → [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only。SPEC-25 是 **技能庫 6 步技能自我演進迴圈**（judge → extract → store → recall → apply → measure，演化引擎）。引擎多在背景；本檔描述桌面 user 看得到 / 控得到的三面：(1) **skill bank（技能庫）list** — 看抽出的技能；(2) **recalled-skill（喚回技能）apply banner** — 任務命中技能時顯示 + decline（拒絕）；(3) `phantom skill` CLI。
> **Spec**: [`SPEC-25-SYSTEM-skill-extraction`](../specs/v060-deep-spec/SPEC-25-SYSTEM-skill-extraction.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P3 進化網**（主 — 「越用越懂你」line 17 + 技能庫 6 步 line 73）；cross-cut **P4 加密為先**（skill bank age 加密）、**P1 跨裝置 Mesh**（`/rpc/skill/sync` 跨 peer）、**X.coach**（coach 消費 skill）。操作原則 **consent-gated（需同意）**（apply 有 decline） |
| **Source spec** | SPEC-25-SYSTEM-skill-extraction |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 1/3 wireframe |

## 為什麼 skill extraction 要 Windows wireframe

「越用越懂你」是對抗 Cursor / Claude Code / Devin 的核心差異 — user 必須**感受到 + 控制得到**演化。Windows 三個面：
1. **skill bank list 在 main window**（大畫面適合列表 + 詳情雙欄）
2. **recalled-skill apply banner 在 chat / task 流程內** — 命中技能時 inline 顯示「套用了你上次的做法」+ decline
3. **背景 judge 走 Task Scheduler**（同 SPEC-23/24 scheduler 抽象）

## 縮寫對照表

> - **skill（技能）**：從重複行為抽出的「下次自動套用的做法」，存 sqlite `skills` 表
> - **skill bank（技能庫）**：所有 skill 的集合（加密、可跨 peer 同步）
> - **judge / extract / store / recall / apply / measure**：技能庫 6 步（見 SPEC-25 標題）
> - **recall（喚回）**：新任務進來時撈相關 skill 注入 prompt
> - **trigger pattern（觸發樣式）**：skill 何時該被 recall 的條件
> - **quality_score（品質分）**：skill 歷史命中表現分（measure 階段更新）
> - **consent-gated（需同意）**：套用 skill 前 user 看得到、可 decline
> - **FTS5 / embedding / RLHF / LoRA**：見 SPEC-25 §1

## 入口點

| 進入點 | v0.6.0 | 說明 |
|---|---|---|
| Main window `[Skills tab]`（由技能庫引擎驅動，§10.1 Screen A） | ✅ | skill bank 列表（Core/Recall/Archival 三段）+ 詳情 |
| **每日「學到新 skill」通知**（§10.2，judge 跑完有新 skill 才發） | ✅ | WinRT toast「技能庫學到 N 個新 skill 等你 review」→ 點開到 Skills tab |
| **Recalled-skill apply banner**（chat/task 內 inline，§10.1 Screen C） | ✅ | 命中時顯示 + decline；consent-gated |
| CLI `phantom skill status / review / decline <id> / delete <id> / sync`（per §9.7） | ✅ | 終端機查統計 / 看上輪新 skill / 拒絕 / 永久刪 / 跨 peer 同步 |
| Deep-link `phantom-mesh://skills`（+ `?id=<skill_id>`） | ✅ | toast 啟動 / 外部跳 Skills tab（待加入 SPEC-43 §12.1 白名單，同 capture prefix） |
| Task Scheduler `PhantomMeshSkillJudge` 每日 | ✅ | 背景 judge → extract（user 不需手動） |
| Settings → 技能演化 → 演化開關 | ✅ | 全域 on/off skill 演化（Reversible） |

## 螢幕 A — Skill bank list（Skills tab，列表 + 詳情雙欄）

列表按 SPEC-25 §10.1 Screen A 的 **tier（分層）三段**呈現（Core 核心 / Recall 召回 / Archival 封存；archival 預設 collapsed）：

```
+----------------------------------------------------------+
| Skills (技能庫)                          [+ 從對話抽取]   |
+---------------------------+------------------------------+
| Core (2)                  |  Rust 錯誤先看 cargo check    |  ← 左:tier 分段 / 右:詳情
|  > Rust 錯誤先看 cargo    |  [Core] 品質 ▮▮▮▮▮▮▮▮▯▯ 套用12 |  quality 10 格進度條(spec §10.1 B)
|    check          [Core]  |  觸發：當任務含「Rust 編譯錯」 |
| Recall (23)               |  last applied：今天 09:14     |
|  > git rebase 衝突 [Rec]  |  步驟：1. cargo check ...      |
|  > 壓圖到 720p     [Rec]  |       2. 讀第一個 error[Exxxx]|
| Archival (41) [展開 v]    |  來源：5/20-5/27 對話 8 次重複 |
|                           |  [Edit] [Decline] [Delete]    |  ← spec §10.1 Screen B 三鈕
|                           |  [Promote to Core]  [跨機分享] |  + 升級到 Core + sync
+---------------------------+------------------------------+
```

**設計重點**：
- 左列表分 **Core / Recall / Archival 三 tier 段**（每段 `(n)` 計數；archival 預設折疊）— 對齊 SPEC-25 §10.1 Screen A，每 row 顯 `name + tier badge`
- 右詳情（§10.1 Screen B）：trigger pattern + steps（有序）+ **quality 10 格進度條** + last applied + tier + provenance（來源透明可審）
- 行動鈕對齊 spec Screen B：**[Edit]**（改 name/steps）/ **[Decline]**（拒絕、降 quality_score、≈ CLI `skill decline`）/ **[Delete]**（永久刪、不可逆、≈ CLI `skill delete`、需 confirm）/ **[Promote to Core]**（升 tier 永遠注入）+ **[跨機分享]**（`/rpc/skill/sync`）
- 「從對話抽取」= 手動觸發 judge（不等每日排程，≈ CLI `skill review` 看上輪結果）

## 螢幕 B — Recalled-skill apply banner（consent-gated，inline）

新任務命中 skill 時，在 chat / task 回應**前**插一條 banner：

```
+----------------------------------------------------------+
| (sparkle) 套用了你上次的做法：Rust 錯誤先看 cargo check  |  ← 取 skillbank.apply.banner
|   會先跑 cargo check 讀第一個 error                       |
|                              [ 好 ]  [ 這次不要 ]  [詳情] |
+----------------------------------------------------------+
```
- **consent-gated 核心**：套用前 user 看得到「我要套哪個 skill」+ 「這次不要」(decline) → 該次不注入、measure 記一筆負回饋
- 「好」= 採用（預設、不點也視同採用 — 不阻塞流程）；「這次不要」= 本次 skip；「詳情」= 跳 Skills tab 該 skill
- banner 溫和（sparkle，非 alert）— 是「我學會了」非「警告」

## 螢幕 D — 每日「學到新 skill」通知（§10.2，job story J1）

每日 judge 跑完（預設 23:00，per SPEC-25 §6.1）且**有新 skill**時，發一則 WinRT toast（ActionCenter）：

```
+------------------------------------------+
| Phantom Mesh                             |
|  技能庫學到 3 個新 skill 等你 review      |  ← 取 skillbank.notify.learned (§10.2 文案)
|  Rust 錯誤先看 cargo check 等 . 點開看看  |
|                          [ 看看 ]        |  ← action → deep-link phantom-mesh://skills
+------------------------------------------+
```
- 只在「judge 完成 + 有新 skill」才發（沒新 skill 不打擾，per §10.2）；`scenario="reminder"` persist、**無 audio**（夜間、shame-free）
- 點 toast / 「看看」→ deep-link `phantom-mesh://skills` → Skills tab（新 skill 在 Recall 段頂、可逐一 keep/edit/delete，per job story J1）
- Focus Assist 期間折疊到 ActionCenter（非急事）

## 螢幕 C — 空狀態（skill bank 還沒長出東西）

```
|  還沒有學到技能                                          |
|  多用幾天，phantom 會從你的重複做法裡學會自動套用         |  ← 不催促、解釋機制
|  (要 ≥ 5 次重複才會抽成技能，避免學到雜訊)               |
```
- 解釋「為何還沒有」（≥5 次門檻）— user 才不會以為壞了

## Task Scheduler + Settings

- task `PhantomMeshSkillJudge` 每日 23:00（per SPEC-25 §6.1，與 coach 21:00 自然錯開）→ `phantom skill judge --since 7d`
- Settings → 技能演化 → 「技能演化」總開關（off = 不 judge / 不 recall，但保留已存 skill）；「跨機分享」開關（off = skill 只留本機，不 `/rpc/skill/sync`）
- 全域 off 是 Reversible 原則的體現

## 失敗 / 邊界（per SPEC-04 + SPEC-25 §11 P3.skillbank.*）

- judge LLM fail → 該日不抽新 skill（既有 skill 照常 recall）→ log，不崩
- skill bank empty → recall 回空陣列（SPEC-23 coach graceful degradation 不受影響）
- recall 命中多個 skill → 依 tier-aware policy 取 top-k（SPEC-01 §8.3 **AC14**：frontier core+recall top-10+archival top-3 / commodity core+recall top-3 / on-device 只 core）；banner 只顯最相關 1-2 個（不洗版）
- 跨機 sync 衝突（兩 peer 同 skill 不同版）→ 取 quality_score 高者 + 保留兩版歷史

## 待補（下一 pipeline stage）

- **Stage 2 mockup**：skill 列表 / 詳情配色、quality 點視覺、apply banner 配色（sparkle 暖調非 alert）、來源 provenance 樣式、終版文案、Narrator a11y
- **Stage 3 prototype**：list/detail 雙欄互動 + apply banner decline + 「刪除」確認 + HTML 草圖
- skill_sync wire 細節在 SPEC-25 §6 + SPEC-10 mesh-rpc；本檔只到 UI「跨機分享」鈕
