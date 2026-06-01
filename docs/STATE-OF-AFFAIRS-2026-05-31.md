# Phantom-mesh — State of affairs (2026-05-31)

> 從 BIG-GOAL 凍結版 ([`docs/superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md)) 起，逐層往下對映到 45 條 active SPEC、5 條 CUJ、實作狀態、test 覆蓋。
>
> **這份是現況快照、不是規劃**。規劃在 [`docs/cuj/`](cuj/)；自動覆蓋表在 [`docs/status.md`](status.md)。

---

## 1. 凍結錨點

```
BIG-GOAL.md            🔒 2026-05-19 鎖定（5/19 後只 2 條 scope 細化、無大改）
4 pillars (P1-P4)      🔒 不可變
2 tracks (Life/Work)   🔒 v0.6.0 Life Track 領頭
3 build principles     🔒 shame-free / consent-gated / reversible
RULES OUT              🔒 不做 IDE 綁定 / 純雲端 / 單機最佳化 / 廠商綁定
```

**這 16 條鐵律是所有下游決定的判準**。任何 CUJ / SPEC / commit 違反 → 退回不收。

---

## 2. 4 Pillar 一句話

| Pillar | 一句話 | Anti-goal |
|---|---|---|
| **P1 跨裝置 Mesh** | 你所有裝置都是對等節點、不是 client/server | ❌ 純雲端 federation 裝飾 |
| **P2 多模態理解** | 圖 + 音 + 文 + 行為「全部納入」、不是純文字外掛圖片 | ❌ 多模態鎖在 Pro 付費 |
| **P3 進化網** | Hermes 6 步閉環、每節點貢獻技能 / 每節點受惠 | ❌ pre-baked 能力假裝在演化 |
| **P4 加密為先** | 每裝置金鑰、12+ provider BYOM | ❌ 對外宣稱加密但 v0.7.0 範圍還在明文 |

**v0.6.0 P4 範圍嚴格限縮**（5/21 加註）：只 `events/` + `identity.key` 加密。`agents.toml` / `conversations/` / `memory.db` / auth tokens 全在 v0.7.0+ 才加密。**不可以對外宣稱「加密」**直到 v0.7.0+ 落地。

---

## 3. 2 Track + v0.6.0 範圍

| Track | 情境 | 錨定 agent | v0.6.0 狀態 |
|---|---|---|---|
| **Life Track 陪你進步** | 減脂 / 專注 / 習慣 / 每日回顧 | `agent.coach` | **領頭** ── 5 CUJ 中 4 條服務本軌道 |
| **Work Track 替你做事** | 程式碼 / 自動化 / 跨機派工 / 演化 | `agent.master / coder / researcher` | 自 v0.5.0 延續、v0.7.0+ 深化、v0.6.0 不主推 |

CUJ-01 / 02 / 03 / 04 / 05 都偏 Life Track。Work Track 在 v0.6.0 維持「不破」原則、但不擴張。

---

## 4. 5 CUJ × 4 Pillar 對映

```
              P1 Mesh   P2 Modal   P3 Evolve   P4 Encrypt
CUJ-01 install   ●         ○          ○           ●
CUJ-02 daily     ○         ●          ●           ●
CUJ-03 sync      ●         ○          ○           ●
CUJ-04 degraded  ●         ○          ○           ●
CUJ-05 export    ○         ○          ○           ●

● = primary  ○ = secondary
```

**觀察**：
- **P4 加密** 是 5 條 CUJ 都掛勾的橫切要求 ── 任何 CUJ failure 都不能因此走明文
- **P3 Evolve** 只在 CUJ-02 daily loop 明確點到 (coach review 用 Hermes) ── 其他 CUJ 主要靠 P1/P2/P4 兌現
- **沒有 Work Track CUJ** ── v0.6.0 範圍內合理；v0.7.0+ 應補 CUJ-06 (Work Track 啟動 onboarding) / CUJ-07 (skill evolve loop)

---

## 5. 5 CUJ × 45 SPEC 對映

### CUJ-01 install → first habit (activation, P1+P4)
- 主要: SPEC-28 (30s-hello) / SPEC-12 (identity) / SPEC-13 (age 加密) / SPEC-16 (events) / SPEC-22 (habit palette starter)
- 平台支援: SPEC-30 (iOS) / SPEC-33 (Android) / SPEC-40 (macOS) / SPEC-44 (Linux)
- 預期測試: install + first chip tap 自動化 (Maestro + Playwright)

### CUJ-02 daily capture loop (retention, P2+P3+P4)
- 主要: SPEC-20 (food) / SPEC-21 (focus) / SPEC-22 (habit) / SPEC-23 (coach engine) / SPEC-24 (coach delivery)
- 支援: SPEC-14 (LLM providers fallback) / SPEC-16 (event storage)
- Hermes 進化: SPEC-25 (skill extraction — 跑在 coach review 內)
- 預期測試: 24h soak + Promptfoo eval set + nightly cron review

### CUJ-03 cross-device resume (P1+P4)
- 主要: SPEC-10 (mesh-rpc) / SPEC-11 (mDNS) / SPEC-15 (broker vault E2EE) / SPEC-26 (cluster-dispatch)
- 支援: SPEC-12 (identity) / SPEC-13 (encryption) / SPEC-50 (broker-api)
- 平台: 所有 platform spec 都要驗 background sync
- 預期測試: 2-device staging soak + SPEC-15 E2EE regression test (✓ 已有)

### CUJ-04 degraded states (生產分水嶺, P1+P4)
- 主要: SPEC-04 (error catalog) / SPEC-14 (LLM fallback chain) / SPEC-15 (broker token expired)
- 支援: SPEC-13 (P4 fail-loud) / SPEC-16 (sqlite corrupted recovery)
- 預期測試: 7 degraded scenarios 每週輪 cron

### CUJ-05 export + uninstall (法律安全網, P4)
- 主要: SPEC-12 / SPEC-13 / SPEC-15 / SPEC-16 / SPEC-50
- ⚠️ **目前 0% 實作** ── release-blocker for EU
- 預期測試: GDPR walkthrough manual playbook + 兩個 CLI command

---

## 6. Orphan / cross-cutting SPECs (不在任一 CUJ 主路上)

這 14 條不是 orphan、是 **服務所有 CUJ 的橫切 SPEC**：

| SPEC | 角色 | 服務的 CUJ |
|---|---|---|
| SPEC-01 BIG-GOAL mapping | meta-doc | all |
| SPEC-02 design tokens | UI 跨平台統一 | 所有有 UI 的 CUJ |
| SPEC-03 information arch | 跨平台 sitemap | 同上 |
| SPEC-05 i18n | 字串 catalog | all (跨語言 UI) |
| SPEC-06 a11y | 無障礙 WCAG 2.2 | all (UI 部分) |
| SPEC-07 observability | OTEL 追蹤 | all (production 監測) |
| SPEC-08 threat-model | STRIDE 安全分析 | 主要 CUJ-04 + 05 |
| SPEC-09 decision log | ADR catalog | meta |
| SPEC-17 tauri bridge | command + event protocol | 所有 desktop CUJ |
| SPEC-29 release-pipeline | build / sign / ship | meta (release infra) |
| SPEC-51 broker deployment | phantommesh.io infra | CUJ-03 + 05 後端 |
| SPEC-52 demo-relay | TURN/STUN demo | onboarding edge case |
| SPEC-60/61 testing | 測試框架 + 場景 | meta (B 階段依據) |
| SPEC-62/63 release channels + signing | beta / stable + codesign | meta |
| SPEC-81 multi-cli orchestration | dev infra (5/27 validated) | meta (內部開發) |

**Work Track 專屬** (v0.6.0 維持、不在 5 CUJ 內):
- SPEC-25 skill-extraction (P3 評估 + 萃取 ── 跨 Life/Work、Life CUJ-02 也用)
- SPEC-27 smart-task-decompose (主 Work Track)

---

## 7. 平台 SPEC × Track 對映 (45 條中佔 11 條)

| Platform | Foundations | Screens/Flows | v0.6.0 狀態 |
|---|---|---|---|
| iOS | SPEC-30 | SPEC-31 (screens) + SPEC-32 (flows) | 5/4 broker login + vault sync chain 通 |
| Android | SPEC-33 | SPEC-34 | 5/30 z13-android ship 大部分 SPEC-34 §10 條目 |
| macOS | SPEC-40 | SPEC-41 | install.sh + tui-combined 對齊 |
| Linux | SPEC-44 | SPEC-45 | z13-wsl 在做 .deb GUI (5/30 slice-2 done) |
| Windows | (archived 5/30) | (archived) | deferred — 不在 v0.6.0 sprint |

**重要**：Windows SPEC 已 archive 到 `docs/spec/history/2026-05-30/deferred/`、v0.6.0 不主打 Windows。

---

## 8. 覆蓋現況 (from `docs/status.md`)

```
45 active SPECs (post-archive)
36 in code (62%)         ← 大部分 SPEC 有實作起點
19 in test (33%)         ← test 覆蓋率還是低
17 has BOTH ✓           ← 真正算「做完」的不到 1/3
17 code-but-no-test     ← 定時炸彈
10 zombie (no code)     ← foundation/testing/release 故意留 active 等補 impl
```

**進步軌跡**：今天剛 SPEC-22 從定時炸彈下車 (18 → 17)、剩下 17 條 backlog。

**最重要的 5 條定時炸彈** (對 v0.6.0 release 影響大):
1. **SPEC-22** ✓ 今天剛解 (habit)
2. SPEC-33/34 Android (code 都在、整條 e2e 沒測)
3. SPEC-17 Tauri bridge (跨平台 IPC、UI 動就會踩)
4. SPEC-50 broker-api (CUJ-03/05 後端 ── server-side 沒 e2e)
5. SPEC-29 release-pipeline (沒測過 = 不知道能不能 ship)

---

## 9. v0.6.0 ship readiness (對照 BIG-GOAL 鐵律)

| BIG-GOAL 要求 | 現況 | 風險 |
|---|---|---|
| P1 跨裝置 mesh | ✓ SPEC-10/11/15/26 都有 code | ⚠️ CUJ-03 跨裝置 5s SLO 沒量過 |
| P2 多模態 | ✓ SPEC-20/21/22 三條 capture 都活 | 🟡 SPEC-21 focus session ASR 完整 e2e? |
| P3 進化網 | ✓ SPEC-23 coach + SPEC-25 skill 都有 code | 🟡 Hermes 6 步閉環 trajectory eval 不存在 |
| P4 加密 (v0.6.0 嚴格範圍) | ✓ events + identity.key 確認 | 🔴 v0.7.0+ 範圍 (agents.toml/memory.db/auth) 是否誤宣稱 |
| 3 build principles | ✓ 有 SPEC-02 + i18n + reversible | 🟡 coach prompt 羞辱滲漏沒系統審 |
| `phantom data delete --all` | ⬜ CLI 存在? | 🔴 CUJ-05 export+uninstall 0% impl ── EU release block |

---

## 10. 立即可做的 4 件事 (作為下一步 starter pack)

按優先序、各對應一個我可以接手的工作單位：

### 🥇 W1：把 17 條 code-but-no-test 補完 (B 階段延伸)
複製 SPEC-22 → cuj02_daily_habit_subset.rs 的 pattern、逐 spec 補 integration test。最高槓桿。

### 🥈 W2：CUJ-05 export + delete CLI MVP
**release-blocker for EU**。`phantom export --to <path>` + `phantom delete-all --confirm`、broker DELETE endpoint + 24h GC。

### 🥉 W3：升級 gen-status.sh 認 CUJ 結構
從 spec-by-spec 改成 CUJ × platform × scenario 矩陣表。讓 status.md 直接反映 CUJ 覆蓋。

### 🏅 W4：跑 CUJ-02 vertical slice 把 4 件套全收
flow / playbook 已有 habit subset。補 food / focus / coach subset + Promptfoo eval。CUJ-02 變成第一條 4/4 complete 的 CUJ。

---

## 11. 我看到的 3 個系統性風險

1. **P4 加密 v0.6.0 vs v0.7.0+ scope 容易誤宣**：明文 agents.toml / memory.db / auth 還在、market copy 寫「加密」會被打臉。**release marketing 要逐字 review**。

2. **CUJ-05 (export + uninstall) 完全沒實作**：歐盟用戶碰 = 罰款。**不能上 EU 直到 MVP ship**。

3. **AI agent 層 (coach review LLM 行為) 完全沒 eval**：Promptfoo / LangSmith trajectory eval 不存在 ── coach prompt 改了沒人知道 regression。**這是研究 agent 找到的最高槓桿補洞**。

---

## 12. 更新節奏

本檔每週一更新一次、或當下列事件發生時即時更新：
- BIG-GOAL 解凍（罕見、應該不會）
- 加 / 刪 CUJ
- SPEC archive / revive
- v0.6.0 ship 日期定下

下次更新預期: 2026-06-07 或 W2 完成時。
