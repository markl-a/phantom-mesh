# SPEC archive — 2026-05-30

凍結日：2026-05-30。為避免 cluster session boot 時把 design exploration / deferred 工作誤認為 live spec、把這批檔搬離 active spec 區。

**規則**：此目錄是 **archive-only**，不再被新 spec / code / test 引用。修改或 revive 任何一條 spec 前，**git mv 回 active 區並更新 status frontmatter**。

---

## `design-variants/` ── 33 條

每個 SYSTEM SPEC（capture-food / capture-focus / capture-habit / coach-engine / coach-delivery）的多平台 mockup / prototype / wireframe 變體。設計探索完成後沒人再讀、但 session 看到「flows/」字串以為是 user flow ── 誤導。

真正的 user flow（行為層 7±3 步、跨 OS 同一份）會在 C 階段 build 起、放 `docs/flow/<spec-slug>.md`。

## `deferred/` ── 10 條

| Spec | 為何 archive |
|---|---|
| SPEC-42 PLATFORM Windows foundations | Windows 版未啟動推進、deferred |
| SPEC-43 PLATFORM Windows screens-flows | 同上 |
| SPEC-70 EXP web-dashboard | future idea、no implementation owner |
| SPEC-71 EXP multi-user-household | future |
| SPEC-72 EXP paid-broker | future |
| SPEC-73 EXP watch-companion | future |
| SPEC-74 EXP extensions-share-widget | future |
| SPEC-75 EXP spectyn-recall | future |
| SPEC-76 EXP spectyn-personas | future |
| SPEC-80 INFRA mode-b-collab-dev | 多 CLI 協作模式、未啟動 |

---

## 沒搬走的 zombie specs（code 0、留 active 故意）

- `SPEC-06-FOUNDATION-a11y.md` ── critical foundation 待補 impl、不是廢
- `SPEC-07-FOUNDATION-observability.md` ── 同上
- `SPEC-08-FOUNDATION-threat-model.md` ── 同上
- `SPEC-60-TESTING-strategy.md` ── 重設計時依據
- `SPEC-61-TESTING-scenarios.md` ── 同上
- `SPEC-51-SERVER-deployment.md` ── release 路徑必要
- `SPEC-63-RELEASE-signing.md` ── Apple+Win codesign 必要
- `SPEC-81-INFRA-multi-cli-orchestration.md` ── 5/27 已 validated（雖然 spec ID 沒被 grep 到、實作走 scripts/ai/）
- `SPEC-44/45 PLATFORM Linux` ── z13-wsl 當下正在做 Linux .deb GUI
