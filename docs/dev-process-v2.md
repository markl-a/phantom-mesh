# Phantom-mesh 完整 Dev / Test / Debug 流程 v2 (2026-05-31)

> **目的**: 從「spec drift / 沒一版實際測過」翻成 spec-driven + AI-augmented 自動化迴圈。AI 跑大部分（寫測試、實作、debug、QA），人只 sign off + manual sanity。

---

## §1. 一頁全鏈條（top-down）

```
       BIG-GOAL.md 🔒
            │
            ▼
       4 Pillars (P1-P4)
            │
            ▼
       5 CUJ (docs/cuj/)
            │
            ▼
   ┌─────── SPEC (docs/spec/) ───────┐
   │                                 │
   ▼                                 ▼
Job Stories          Acceptance criteria (G[X])
   │                                 │
   ▼                                 ▼
docs/flow/cuj-XX/     docs/test-cases/mac.md
(行為 7±3 步)         (130 條 case × 5 欄)
   │                                 │
   └────────────┬────────────────────┘
                ▼
   ┌────────────────────────────┐
   │  TEST 4 層（從快到慢）       │
   ├────────────────────────────┤
   │ 1. Unit (cargo test --lib)  │  ✅ 全自動、~5s
   │ 2. Integ (cargo test --test)│  ✅ 全自動、~30s
   │ 3. E2E (Maestro/Playwright/ │  ⚠ 需 fixture、~5min
   │         shell driven)        │
   │ 4. AI QA (Promptfoo /        │  ⚠ 需 LLM key、~10min
   │   LangSmith / agent walks)   │
   └────────────────────────────┘
                ▼
       Implementation
                │
                ▼
       AI Verify (auto-triage)
                │
                ▼
       Human manual sanity (playbook)
                │
                ▼
       Production canary (Checkly)
                │
                ▼ (alert)
       AI auto-debug + propose fix
                │
                ▼
       Human review + ship
```

---

## §2. 四層測試詳細

### Layer 1: Unit (cargo test --lib)
- 跑時：`cargo test --target aarch64-apple-darwin --lib`
- 對象：純邏輯 fn (slug validate / cron parse / streak algo / wire shape)
- 速度：~5-10s
- 不需：HOME / network / LLM
- 範例：`MAC-CUJ01-FH-008` slug 驗證、`MAC-CUJ02-HAB-007` HabitFrequency 序列化

### Layer 2: Integration (cargo test --test)
- 跑時：`cargo test --target aarch64-apple-darwin --test cuj02_daily_habit_subset`
- 對象：跨 module 整合 (sqlite + encrypt + wire + cli)
- 速度：~30s
- 需：TempDir HOME isolation
- 範例：`cuj02_daily_habit_subset.rs`、`cuj05_backup_export.rs`、`spec15_broker_vault_e2ee_regression.rs`

### Layer 3: E2E (shell / Maestro / Playwright)
- 跑時：`scripts/test-runner-mac.sh` (shell 直接 invoke phantom CLI)
- 對象：完整 user flow (install → first habit → coach review)
- 速度：~3-5min
- 需：build 過的 binary + 可能 fixture (mock LLM / mock broker)
- 範例：`MAC-CUJ01-INST-001` install + `MAC-CUJ01-FH-001` first habit chain

### Layer 4: AI QA (Promptfoo + agent walking)

#### 4A. Promptfoo eval (LLM 行為 regression)
- 跑時：`promptfoo eval -c eval/cuj-02-coach.yaml`
- 對象：coach review 在 30 個 frozen example 的輸出品質、不退化
- 速度：~10min
- 需：所有 active LLM provider key
- 例 yaml：
```yaml
prompts:
  - file://prompts/coach-review.txt
providers:
  - openai:gpt-4o
  - anthropic:claude-3-5-sonnet
  - gemini:gemini-1.5-pro
tests:
  - vars:
      events: |
        habit: water 250ml @ 7am
        habit: water 250ml @ 9am
    assert:
      - type: contains
        value: "水"
      - type: llm-rubric
        value: 不帶羞辱、提供 actionable insight
      - type: javascript
        value: output.length > 100 && output.length < 1000
```

#### 4B. Agent-driven QA walk (新概念)
- 跑時：`scripts/ai-qa-walk.sh CUJ-01`
- AI agent 拿 CUJ-01 doc + test-cases/mac.md 的 §1 表 → 自己 invoke phantom 命令 → 自己 assert → 報告
- 例：
```
[Agent] 讀 docs/cuj/01-install-to-first-habit.md
[Agent] 跑 happy path step 1: 模擬 install → ok
[Agent] 跑 step 5: phantom habit water --qty 250
        → 預期 streak=1
        → 實際 streak=1 ✓
[Agent] 跑 degraded "no identity.key":
        → 預期 fail-loud
        → 實際 panic at line X ✗
[Agent] auto-triage: panic at core/src/key_derivation.rs:80
        → 提議 patch: add Err arm
[Agent] open PR with fix proposal
```

---

## §3. AI debug 迴圈（新關鍵）

當 layer 1-4 出錯、AI 自動：

```
[FAIL detected]
    │
    ▼
1. 解析 panic / assertion msg
2. git blame 找最近相關 commit
3. 讀 spec / cuj doc 看「應該如何」
4. 對照「實際如何」找 gap
5. 提議 patch (with rationale)
6. 跑 patch + 重 layer 1-4
7. 過 → open PR + commit
   不過 → escalate to human + 留 detailed log
```

實作：寫 `scripts/ai/auto-triage.sh`、給 Claude / Codex 跑、寫進 mesh-orch.

### 觸發條件
- cargo test fail → 自動 invoke
- Checkly canary 紅 → 自動 invoke
- TUI 渲染異常 (e.g. Bug A) → 人類 paste screenshot 觸發

### 邊界
- 不自動 push 主軸 commit ── PR 要人 review
- 不改 SPEC / BIG-GOAL（凍結錨）
- 不改 install.sh F-CRIT-3 invariants
- 失敗 3 次 escalate

---

## §4. CI/CD 整合 (.github/workflows/)

```yaml
# .github/workflows/dev-pipeline.yml
name: spec-driven dev pipeline
on: [push, pull_request]
jobs:
  layer-1-unit:
    runs-on: macos-latest
    steps:
      - cargo test --lib  # ~5s
  layer-2-integ:
    needs: layer-1-unit
    steps:
      - cargo build --release --bin phantom
      - cargo test --test 'cuj*'  # ~1min
  layer-3-e2e:
    needs: layer-2-integ
    steps:
      - scripts/test-runner-mac.sh
  layer-4-promptfoo:
    needs: layer-3-e2e
    if: env.GEMINI_API_KEY
    steps:
      - promptfoo eval
  layer-5-canary:
    on: schedule (every 15 min)
    steps:
      - checkly run cuj-01.check.yaml
```

---

## §5. 跟現有 docs/ 對應

| Doc | Layer 對應 |
|---|---|
| `docs/cuj/*.md` | spec → flow 上游 |
| `docs/spec/SPEC-*.md` | spec 本體（5/19 凍結） |
| `docs/flow/cuj-XX/*.md` | user flow drill-down |
| `docs/playbook/cuj-XX/*.md` | **§6 手動測試** 用 |
| `docs/test-cases/mac.md` v2 | **§2 四層** 對映 |
| `core/tests/cuj*.rs` | Layer 2 integ |
| `scripts/test-runner-mac.sh` | Layer 3 e2e |
| `eval/cuj-*/*.yaml` | Layer 4A Promptfoo |
| `scripts/ai/auto-triage.sh` | §3 AI debug |
| `docs/status.md` | 自動產覆蓋表 (gen-status.sh) |

---

## §6. 手動測試流程（給 user 親手跑）

見 `docs/manual-playbook/mac.md`（separate file、保持精簡可印）。

---

## §7. 對應到本次 session 已 ship 的東西

✅ Layer 1 unit: `capture_habit_wire` inline tests (4 條)
✅ Layer 2 integ: `cuj02_daily_habit_subset.rs` (1+3) + `cuj05_backup_export.rs` (3) + `spec15_broker_vault_e2ee_regression.rs` (5)
✅ phantom backup CLI (commit 391bd38b)
✅ sqlite corruption recovery (commit ed9c996f + 57b5a282)
🟡 Layer 3 e2e: scripts/test-runner-mac.sh 範例（test-cases/mac.md §11 草稿）
⬜ Layer 4 Promptfoo: 沒實作
⬜ §3 AI auto-triage: 沒實作

---

## §8. 下一步優先序

1. **修剩 4 條 task** (#140 broker DELETE / #141 reinstall / #142 stats-only / #144 補測)
2. **寫 scripts/test-runner-mac.sh** 把 §2 Layer 3 跑起來
3. **寫 1 條 Promptfoo eval set**（CUJ-02 coach review）
4. **寫 §6 manual playbook** (next file)
5. **寫 §3 AI auto-triage 腳本** (post-noon)

---

## §9. 對 5 平台共用

雖然本 doc 主寫 mac，原則對 5 平台都通。後續：
- `docs/test-cases/ios.md` v2 (Maestro for mobile)
- `docs/test-cases/android.md` v2
- `docs/test-cases/linux.md` v2
- `docs/test-cases/windows.md` v2

共用骨架、各填平台特有測項。
