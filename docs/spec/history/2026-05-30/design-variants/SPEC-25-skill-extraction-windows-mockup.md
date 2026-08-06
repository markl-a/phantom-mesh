# SPEC-25 Skill Extraction (skill bank) — Windows Mockup（視覺稿）

> **Stage 2/3** · [線框稿（wireframe）](./SPEC-25-skill-extraction-windows-wireframe.md) → 視覺稿（mockup，配色 + icon + 終版文案 + a11y）→ [原型（prototype，待補）]
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — Skills tab（技能分頁）list/detail 配色 + quality（品質）點視覺 + recalled-skill apply banner（喚回技能套用橫幅）配色 + provenance（來源溯源）樣式 + 終版文案 + Narrator（朗讀器）AutomationName。沿用 SPEC-20/21/22/23/24 mockup 的 design token（設計變數）速查。
> **Spec**: [`SPEC-25-SYSTEM-skill-extraction`](../specs/v060-deep-spec/SPEC-25-SYSTEM-skill-extraction.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md) · [`SPEC-02-FOUNDATION-design-tokens`](../specs/v060-deep-spec/SPEC-02-FOUNDATION-design-tokens.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P3 進化網**（主，「越用越懂你」）；cross-cut P4 加密為先、P1 跨裝置 Mesh、X.coach。操作原則 **consent-gated（需同意）** |
| **Source spec** | SPEC-25-SYSTEM-skill-extraction |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 2/3 mockup |

## 為什麼 skill 庫有獨立 Windows mockup

wireframe 鎖了版型 + consent 契約；本檔鎖實作視覺：
1. **quality 4 格點配色**（沿用 SPEC-20 confidence 點語意 — 高綠 / 中藍 / 低橘，但語意是「skill 命中表現」非「食物信心」）
2. **apply banner 配色** — sparkle 暖調「我學會了」，**不是 alert**（學習非警告）
3. **provenance（來源）樣式** — 「5/20-5/27 對話 8 次重複」透明可審的低調呈現
4. **低分 / Archival tier skill 的灰階**（不刪、可 Promote 回 Core/Recall）
5. **終版文案** + **Narrator AutomationName**

## Design token 對映（per SPEC-02，沿用前作）

| Token | Hex | skill 用途 |
|---|---|---|
| `spectyn-bg` | `#0f0f1a` | Skills tab 背景 |
| `spectyn-card` | `#1a1a2e` | 左列表 row / 右詳情卡 |
| `spectyn-card-warm` | `#221a2e` | **apply banner 背景**（暖塊 — 同 SPEC-23 coach action「邀請」色溫，學習是正向） |
| `spectyn-primary` | `#8ab4f8` | 「跨機分享」鈕、apply「好」鈕 |
| `spectyn-success` | `#81c995` | quality 高分點、套用次數正向 |
| `spectyn-warning` | `#ff9800` | quality 低分點（建議檢視，非錯誤） |
| `spectyn-muted` | `#6b6b80` | Archival/低分 skill 灰階、provenance 文字、quality 空格點 |

### quality 10 格進度條（per SPEC-25 §10.1 Screen B；配色沿用 SPEC-20 confidence 語意）

詳情顯 **10 格進度條**（`▮` 實 × round(score×10) + `▯` 空），填色依 score band（非 4 點；對齊 spec §10.1 Screen B「quality bar 10 格」）：

| quality_score | 10 格範例 | 填色 | 語意 |
|---|---|---|---|
| `>= 0.8` | ▮▮▮▮▮▮▮▮▯▯ | `spectyn-success` 綠 | 命中表現好，優先 recall |
| `0.5 - 0.8` | ▮▮▮▮▮▮▯▯▯▯ | `spectyn-primary` 藍 | 中等，照常 recall |
| `< 0.5` | ▮▮▮▯▯▯▯▯▯▯ | `spectyn-warning` 橘 | 表現差，列表沉底 / 建議 Decline 或降到 Archival（**不染紅** — 低分不是 error） |

（左欄列表的 compact 視覺可用 tier badge + 1 個 score 數字；10 格 bar 只在右詳情 Screen B 出現。）

## Lucide icon 對映

| 角色 | Lucide icon | 用途 |
|---|---|---|
| 學會 / apply | `sparkle` | apply banner + 「從對話抽取」，16px spectyn-primary（靈感感，非 alert） |
| 編輯 (Edit) | `pencil` | 詳情改 name/steps，14px spectyn-primary |
| 拒絕 (Decline) | `thumbs-down` | 降 quality_score（≈ CLI skill decline），14px spectyn-muted |
| 刪除 (Delete) | `trash-2` | 永久刪 skill（不可逆、需 confirm），14px spectyn-danger |
| 升級到 Core (Promote) | `arrow-up-circle` | 升 tier 永遠注入，14px spectyn-success |
| 跨機分享 (Share) | `share-2` | `/rpc/skill/sync`，14px |
| 來源 | `git-commit` | provenance 行首，12px spectyn-muted（Work Track 技能；Life Track 食物/習慣 skill 改用 `sparkle`） |
| tier badge | 文字 `[Core]/[Recall]/[Archival]` | 每 row + 詳情 tier 標，11px |
| 詳情展開 | `chevron-right` | 左列表 row，14px |

## 文案 keys（per SPEC-05 i18n）

| key | 繁中 | English |
|---|---|---|
| `skillbank.tab.title` | 技能庫 | Skills |
| `skillbank.btn.extract_now` | 從對話抽取 | Extract from chat |
| `skillbank.detail.quality` | 品質 {dots}　套用 {n} 次 | Quality {dots} · used {n}x |
| `skillbank.detail.trigger` | 觸發：{pattern} | Triggers when: {pattern} |
| `skillbank.detail.provenance` | 來源：{range} 對話 {n} 次重複 | From: {n} repeats over {range} |
| `skillbank.btn.edit` | 編輯 | Edit |
| `skillbank.btn.decline` | 拒絕 | Decline |
| `skillbank.btn.delete` | 刪除 | Delete |
| `skillbank.btn.promote` | 升級到 Core | Promote to Core |
| `skillbank.btn.share` | 跨機分享 | Share to cluster |
| `skillbank.notify.learned` | 技能庫學到 {n} 個新 skill 等你 review | Skill bank learned {n} new skills to review |
| `skillbank.apply.banner` | 套用了你上次的做法：{skill} | Applied what worked last time: {skill} |
| `skillbank.apply.decline` | 這次不要 | Not this time |
| `skillbank.empty.title` | 還沒有學到技能 | No skills learned yet |
| `skillbank.empty.body` | 多用幾天，spectyn 會從你的重複做法裡學會自動套用（要 ≥ 5 次重複才會抽成技能，避免雜訊） | Keep using it a few days — spectyn learns from your repeated patterns (>=5 repeats before a skill forms, to avoid noise) |

## 螢幕 A — Skills tab（list + detail 雙欄）

```
+----------------------------------------------------------+
| 技能庫                                  [sparkle 從對話抽取]|
+---------------------------+------------------------------+
| Core (2)                  | Rust 錯誤先看 cargo check    |   左:tier 分段 (§10.1 Screen A)
|  Rust 錯誤... cargo [Core]| [Core] 品質 ▮▮▮▮▮▮▮▮▯▯ 套用12 |   quality 10 格進度條 spectyn-success
| Recall (23)               | 觸發：任務含「Rust 編譯錯」    |   trigger spectyn-muted
|  git rebase 衝突   [Rec]  | last applied：今天 09:14     |
|  壓圖到 720p       [Rec]  | (git-commit) 來源：5/20-5/27  |   provenance spectyn-muted 12px
| Archival (41) [展開 v]    |             對話 8 次重複     |
|                           | [pencil Edit][thumbs Decline][trash Delete]|  spec §10.1 B
|                           | [arrow-up Promote to Core] [share 跨機分享]|
+---------------------------+------------------------------+
```
- 左列表分 **Core / Recall / Archival** 三 tier 段（archival 預設折疊），每 row `name + [tier] badge`（對齊 SPEC-25 §10.1 Screen A）
- 詳情行動鈕（§10.1 Screen B）：**Edit**（pencil）/ **Decline**（thumbs-down，降 score）/ **Delete**（trash，spectyn-danger，需 confirm）/ **Promote to Core**（arrow-up-circle）+ **跨機分享**（share-2 → `/rpc/skill/sync`）
- **只 Delete 用紅**（spectyn-danger，破壞性）；Decline / 低分 / tier 都不染紅

**Narrator AutomationName**：
- 列表 row：「{name}，{tier} 層，品質 {高/中/低}，已套用 {n} 次。按鈕，展開詳情。」
- 「Delete」鈕：「刪除這個技能，永久刪除、歷史紀錄保留。按鈕。」（朗讀後果 — consent）

## 螢幕 B — Recalled-skill apply banner（consent-gated，inline）

```
+----------------------------------------------------------+
| (sparkle) 套用了你上次的做法：Rust 錯誤先看 cargo check  |   spectyn-card-warm 暖塊
|   會先跑 cargo check 讀第一個 error                       |   說明 spectyn-muted 13px
|                          [ 好 ]  [ 這次不要 ]  [ 詳情 ]  |   好=spectyn-primary 主鈕
+----------------------------------------------------------+
```
- **暖塊（spectyn-card-warm）+ sparkle** — 語氣「我學會了、幫你套上」，**不是 alert / warning**（無橘無紅無驚嘆號）
- 「好」spectyn-primary（預設、不點也採用、不阻塞）；「這次不要」spectyn-muted 文字鈕（decline → 本次 skip + measure 負回饋）；「詳情」→ 跳 Skills tab
- Narrator：「套用了你上次的做法：{skill}。可按『這次不要』略過。」（用「可」非「必須」）

## 螢幕 D — 每日「學到新 skill」toast（§10.2，WinRT XML 終值）

```xml
<toast scenario="reminder" activationType="protocol"
       launch="spectyn-mesh://skills">
  <visual><binding template="ToastGeneric">
    <text>技能庫學到 3 個新 skill 等你 review</text>
    <text>Rust 錯誤先看 cargo check 等 . 點開看看</text>
    <image placement="appLogoOverride" hint-crop="circle" src="spectyn-tray-idle.png"/>
  </binding></visual>
  <actions><action content="看看" activationType="protocol"
            arguments="spectyn-mesh://skills"/></actions>
</toast>
```
- 取 `skillbank.notify.learned`；只在 judge 完成 + 有新 skill 才發（§10.2）；`reminder` persist、**無 audio**（夜間溫和）
- 點開 → `spectyn-mesh://skills` → Skills tab，新 skill 在 Recall 段頂

## 螢幕 C — 空狀態

```
| (sparkle outline 淡) 還沒有學到技能                       |   empty.title 16px
| 多用幾天，spectyn 會從你的重複做法裡學會自動套用          |   empty.body 14px spectyn-muted
| （要 ≥ 5 次重複才會抽成技能，避免雜訊）                   |
```
- 淡色 `sparkle` outline 插畫（可選）；**不畫「空腦袋 / 問號」**（避「你還沒教我」的隱含催促）

## Cross-platform invariants 對齊

- quality 點配色（高綠/中藍/低橘）跨 5 平台 + 與 SPEC-20 confidence 點同語意
- apply banner consent（好 / 這次不要）+ 暖塊「邀請」色溫跨平台一致（同 SPEC-23 coach action）
- 「刪除」朗讀後果（consent）跨平台一致

## 已決（per wireframe + 本檔拍板）

- quality 點沿用 confidence 三段色（本檔）；apply banner = 暖塊 + sparkle 非 alert（本檔）
- Decline / 低分 / Archival / 空狀態全不染紅（只 Delete 用 danger，本檔）；provenance 透明低調呈現（本檔）

## 開放問題（留 prototype / 後續）

1. quality 點 hover 是否顯原始 score 數值 — 待 prototype
2. 跨機分享的衝突解（兩 peer 同 skill 不同版）UI — 待 SPEC-25 §6 + SPEC-10
3. apply banner 出現頻率上限（避洗版）— 待 measure 階段對齊

## 下一步

**Stage 3 prototype**：list/detail 雙欄互動 + apply banner decline + 「刪除」確認 + HTML 草圖。
