# SPEC-25 Skill Extraction (skill bank) — Windows Prototype（原型）

> **Stage 3/3** · [線框稿（wireframe）](./SPEC-25-skill-extraction-windows-wireframe.md) → [視覺稿（mockup）](./SPEC-25-skill-extraction-windows-mockup.md) → 原型（prototype，互動腳本 + 元件草圖）
> **Status**: draft v0.1 · **Last updated**: 2026-05-28
> **Scope**: Windows only — 互動腳本（每個可點處按下去發生什麼）+ apply（套用）/ decline（拒絕）流程 + 「刪除」確認 + Narrator（朗讀器）focus 順序 + Nielsen 5（尼爾森 5 大易用性 heuristic）+ usability walkthrough（易用性走查）+ HTML 草圖。
> **Spec**: [`SPEC-25-SYSTEM-skill-extraction`](../specs/v060-deep-spec/SPEC-25-SYSTEM-skill-extraction.md) · [`SPEC-42`](../specs/v060-deep-spec/SPEC-42-PLATFORM-Windows-foundations.md) · [`SPEC-43`](../specs/v060-deep-spec/SPEC-43-PLATFORM-Windows-screens-flows.md)

## 設計溯源（trace）

| 維度 | 對應 |
|---|---|
| **BIG-GOAL pillar** | **P3 進化網**（主，「越用越懂你」）；cross-cut P4 加密為先、P1 跨裝置 Mesh、X.coach。操作原則 **consent-gated（需同意）** |
| **Source spec** | SPEC-25-SYSTEM-skill-extraction |
| **Platform** | windows（桌面） |
| **Pipeline stage** | 3/3 prototype |

## 為什麼 skill 庫要獨立 Windows prototype

skill 庫互動有兩條前作沒有的核心路徑：
1. **被動命中 → consent 套用** — skill 不是 user 主動開的，是任務命中時 banner 跳出，user 一秒內決定採用/拒絕（不阻塞）
2. **「刪除」是破壞性 + 可逆的雙重性** — 刪 skill（破壞）但保留歷史 event（可逆來源仍在）；需確認後果

## 互動狀態機

```
每日 judge 完成 + 有新 skill --> WinRT toast(§10.2) --(點/「看看」)--> deep-link phantom-mesh://skills --> SKILLS_TAB(新 skill 在 Recall 段頂)
SKILLS_TAB(list) --(點 row)--> DETAIL --(Edit/Decline/Delete/Promote/share)--> {更新 + 留在 DETAIL}
任務流程中 --(recall 命中)--> APPLY_BANNER --(好/不點)--> 採用注入 prompt
                                          --(這次不要)--> skip + measure 負回饋
                                          --(詳情)--> 跳 SKILLS_TAB DETAIL
「Delete」--(點)--> CONFIRM(後果說明) --(確認)--> 刪 skill row(歷史 event 保留) --> 列表移除
                                    --(取消)--> 回 DETAIL
```

## Nielsen 5 易用性檢核（Windows 對應）

| Heuristic | 本設計如何滿足 |
|---|---|
| #1 系統狀態可見 | apply banner 明示「套用了哪個 skill」；DETAIL 顯 quality + 套用次數 + provenance |
| #3 掌控 + 自由（consent + reversible） | apply 可「這次不要」；skill 可 Decline（降分）/ Delete（刪但歷史在）/ Promote to Core / 全域關演化 |
| #5 錯誤預防 | 「Delete」走 CONFIRM + 說明後果（不是一鍵誤刪） |
| #6 認得勝過回想 | skill 自動 recall + banner 推到眼前；user 不用記得「上次怎麼解的」 |
| #10 透明 | provenance「N 次重複」可審；user 知道 skill 怎麼來的、不是黑箱 |

## 螢幕 A — Skills tab + B — apply banner：tap targets

| 元件 | 點擊行為 | 鍵盤 |
|---|---|---|
| 左列表 row（tier 分段 Core/Recall/Archival） | → 右欄展開 DETAIL | `Up/Down` 選、`Enter` 展開 |
| 「Edit」(pencil) | inline 改 name / steps → 存 | `Enter` |
| 「Decline」(thumbs-down) | 降 quality_score（≈ CLI `skill decline`）→ 可能滑到 Archival 段 | `Enter` |
| 「Delete」(trash) | → CONFIRM「永久刪除此技能（歷史紀錄保留）？」→ 確認才刪（≈ CLI `skill delete`） | `Enter` → `Enter` 確認 |
| 「Promote to Core」(arrow-up-circle) | 升 tier 到 Core（永遠注入 prompt） | `Enter` |
| 「跨機分享」(share-2) | push 到 cluster skill bank（`/rpc/skill/sync`）→ 顯「已分享到 N 台」 | `Enter` |
| WinRT toast「看看」(§10.2) | deep-link `phantom-mesh://skills` → Skills tab（app 沒開則 cold-launch：splash → 等 serve ready → 跳） | — |
| 「從對話抽取」(sparkle) | 手動觸發 judge（不等每日排程）→ spinner →新 skill 進列表 | `Enter` |
| **apply banner「好」** | 採用、注入 prompt（預設、不點也採用 — 不阻塞流程） | 不點即採用 |
| **apply banner「這次不要」** | 本次 skip 注入 + measure 記負回饋（influence next judge weights） | `Esc` = 這次不要 |
| apply banner「詳情」 | 跳 Skills tab 該 skill DETAIL | — |

**動畫 / timing**：apply banner slide-in 150ms（不搶焦點，inline 在回應上方）；採用後 banner 收起 100ms；「刪除」CONFIRM 是 modal（須明確選擇）。

## 螢幕 — 「刪除」CONFIRM（破壞性需確認）

```
+--------------------------------------------------+
| 刪除「Rust 錯誤先看 cargo check」？               |
|   會刪除此技能、不再自動套用。                    |
|   （你過去的對話 / event 歷史不受影響，保留）     |   ← 釐清「刪 skill ≠ 刪歷史」
|                          [ 取消 ]  [ 刪除 ]      |   刪除 = phantom-danger（唯一破壞性確認）
+--------------------------------------------------+
```
- 唯一用 phantom-danger 的地方（破壞性動作）；說明「歷史保留」降低焦慮（reversible 來源仍在）

## 失敗路徑（per SPEC-04 + SPEC-25 §11 P3.skillbank.*）

- judge LLM fail（手動抽取時）→ 「這次沒抽到新技能，過幾天再試」（既有 skill 照常 recall）
- skill bank empty → recall 回空（SPEC-23 coach graceful 不受影響）
- 跨機分享時 peer 離線 → 「已排隊，peer 上線後同步」（不報硬 error）
- recall 命中多個 → banner 只顯最相關 1 個（tier-aware top-k，不洗版）

## Narrator focus order（per SPEC-43 §12.2 + WCAG 2.2 AA 無障礙）

- **SKILLS_TAB**：分頁標題 → 「從對話抽取」→ 列表（tier 分段，每 row「{name}，{tier} 層，品質{高/中/低}，套用{n}次」）→ DETAIL（trigger → steps → quality → last applied → Edit → Decline → Delete → Promote → 分享）
- **APPLY_BANNER**：`aria-live="polite"`「套用了你上次的做法：{skill}。可按『這次不要』略過。」（不搶焦點、不阻塞）
- **CONFIRM**：modal 取得焦點，朗讀完整後果「會刪除此技能，歷史紀錄保留」

## 元件草圖（HTML，可貼進 Tauri webview 試互動）

```html
<style>
  .banner{background:#221a2e;border-radius:8px;padding:12px;color:#e8e8f0;max-width:520px}
  .banner .h{color:#8ab4f8;font-size:14px}
  .b-act{margin-top:8px;display:flex;gap:8px}
  .ok{background:#8ab4f8;border:none;border-radius:6px;padding:6px 14px;cursor:pointer}
  .no{background:none;border:none;color:#6b6b80;cursor:pointer}
  .q{width:8px;height:8px;border-radius:50%;display:inline-block;margin:0 1px}
  .q.hi{background:#81c995}.q.off{background:#6b6b80}
</style>
<div class="banner" role="status" aria-live="polite">
  <div class="h">套用了你上次的做法：Rust 錯誤先看 cargo check</div>
  <div>會先跑 cargo check 讀第一個 error</div>
  <div class="b-act">
    <button class="ok" onclick="applySkill()">好</button>
    <button class="no" onclick="declineSkill()">這次不要</button>
    <button class="no" onclick="openDetail()">詳情</button>
  </div>
</div>
<!-- quality: 此草圖用點示意；正式詳情用 10 格進度條 ▮▮▮▮▮▮▮▮▯▯（per mockup §quality + spec §10.1 B） -->
<span class="q hi"></span><span class="q hi"></span><span class="q hi"></span><span class="q off"></span>
<script>
  // 不點也視同採用（預設注入）；declineSkill -> invoke('skill_decline',{id}) + measure 負回饋
  function applySkill(){ /* 已預設注入，這裡只記正回饋 */ }
  function declineSkill(){ /* -> invoke('skill_decline', {skill_id}) */ }
  function openDetail(){ /* navigate Skills tab */ }
</script>
```
> 註：`skill_decline` 為 Tauri command 佔位；實際 wire 見 SPEC-17 + SPEC-25 §6（recall/apply/measure）。banner 用 `role="status"` + `aria-live="polite"` 讓 Narrator 朗讀但不搶焦點。

## Walkthrough 腳本（usability test：「phantom 自動套用上次做法」）

1. 連續幾天解 Rust 編譯錯都先跑 cargo check → 預期：某天 Skills tab 出現「Rust 錯誤先看 cargo check」technique（provenance 顯重複次數）
2. 再遇 Rust 錯 → 預期：apply banner 跳出「套用了你上次的做法」，覺得被理解
3. 不想這次套用 → 點「這次不要」→ 預期：本次 skip、不阻塞、不被質問
4. 某 skill 沒用 → DETAIL 點「刪除」→ 預期：CONFIRM 說明「歷史保留」、確認後消失
5. 想分享給另一台機 → 「跨機分享」→ 預期：「已分享到 N 台」

**通過判準**：受測者感到「它真的學會了我的做法」（P3 越用越懂你 主驗證點）；apply 可拒絕不被綁架（consent）；「刪除」不怕誤刪歷史。

## 開放問題（留實作 / 後續）

1. apply banner 出現頻率上限（避洗版）— 待 measure 階段對齊
2. 跨機分享衝突解 UI（兩 peer 同 skill 不同版）— 待 SPEC-25 §6 + SPEC-10
3. 「從對話抽取」手動 judge 的成本提示 — 待 SPEC-07 observability

## Pipeline 完成

SPEC-25 skill-extraction Windows 三階段（wireframe → mockup → prototype）齊備。完成記錄見 `.ai-shared/done/design-skill-extraction-windows.md`（含多 AI review 結果）。
