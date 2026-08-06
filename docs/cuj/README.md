# Critical User Journeys (CUJ) — spectyn-mesh v0.6.0

> **What is this folder**: 5 condensed user journeys that cover the **entire user lifecycle** for v0.6.0 (Life Track 領頭). Per Google SRE definition, a CUJ is an "ordered sequence of interactions that the user cares about, which we measure (SLO) and rehearse end-to-end (test + synthetic monitor)."
>
> 每條 CUJ = 1 page、含 outcome / happy path / degraded paths / SPEC refs / SLO target / test pointers。**所有 spec 都要 traceable 回某一條 CUJ**；不能 trace 的 spec 是 spec-drift 候選。

## 5 條 CUJ 對應 lifecycle 階段

| # | CUJ | Lifecycle 階段 | 風險 / 槓桿 |
|---|---|---|---|
| 01 | install → first habit captured | activation (Day 1) | **最高槓桿** ── 業界 day-1 churn 20-25%、唯一預測指標是「user 有沒有完成 meaningful first action」 |
| 02 | daily habit / food / focus capture loop | daily use | 主要 retention driver、產品本體 |
| 03 | cross-device resume (mobile log → desktop coach) | daily use (multi-device) | P1 mesh 命脈、3 句 BIG-GOAL「跑在你所有裝置上」就靠它兌現 |
| 04 | degraded states (offline / quota / token expired) | degraded | 多數團隊漏測、bug 出在這 ── 直接決定「production-ready 與否」 |
| 05 | data export + uninstall (GDPR + re-install) | churn / safety | 法務 + 信任 ── 不測會踩 GDPR Article 17 (right to erasure) |

## 每條 CUJ 結構（template）

```markdown
# CUJ-XX: <name>
- Outcome: <user 拿到什麼 1 行話>
- Lifecycle phase: <activation / daily / degraded / churn>
- Primary pillar served: <P1/P2/P3/P4>
- SLO: <p50/p95 latency 或 success rate>
- SPECs implementing: <列表>

## Happy path (5-7 steps)
1. ...

## Degraded paths
- offline: ...
- quota exhausted: ...
- token expired: ...

## Test scaffolding
- Maestro YAML: `flow/cuj-XX.maestro.yaml`
- Playwright TS: `flow/cuj-XX.playwright.ts`
- Promptfoo eval (agent layer): `eval/cuj-XX/cases.yaml`
- Manual playbook: `playbook/cuj-XX.md`
- Synthetic monitor: `infra/checkly/cuj-XX.check.yaml` (prod canary)

## Coverage matrix (auto-populated by status.md generator)
| Scenario | iOS | Android | macOS | Windows | Linux |
|---|---|---|---|---|---|
| happy | ? | ? | ? | ? | ? |
| offline | ? | ? | ? | ? | ? |
| ... |
```

## 防 CUJ drift 三規則

1. **新功能 spec 必須點名屬於哪條 CUJ**（frontmatter `parent_cuj: 01..05`）── 點不到就是不該做
2. **每條 CUJ 的 happy path 必須能在 mac 開發機跑通**（CLI mode、不要靠 mobile emulator）── 否則沒人會跑
3. **CUJ 任何 degraded path 紅 → 整條 CUJ 紅**（不是個別 scenario 紅 / 整體 yellow）── 強迫修
