# Evolve Goals

> Pool A 佇列 — 本機可 hermetic 自動化、硬 exit-code gate 的 leaf goal。
> 從 `docs/GOAL-LIST.md` 軸1 Pool A + sweep top-10 gaps 生成(2026-05-31)。
> 每條 = 一個 TDD 循環:寫測試(紅)→ 修/實作 → `cd core && cargo test --test <name>`(RC=0)→ commit。
> 完成移到 Done 並更新 `mac.md` 狀態欄。

## Pending

> **2026-05-31 status (autonomous session, branch `auto/2026-05-31-coverage`)**: Pool A G1–G10 全部 ✅ done & verified green (見 `docs/AUTONOMOUS-WORKLOG.md` + git log)。唯一例外 **MAC-CUJ01-FH-004 🔴 NOT-IMPL**：`events_fts` 無 production indexer（`index_fts5` 唯一 caller 是測試），不造假綠，需先把 FTS5 index 接進 capture flow。額外加了一條 EventStore 多模態 byte-roundtrip invariant 測試（`life_node::storage::tests::multimodal_event_round_trips_byte_identical`，非編號 goal；Image/Audio bytes 經 age 加密後 read_file 解回 byte-identical）。下面 checklist 待搬到 Done。

- [ ] G-BUG-CHECKIN [E2E-discovered, P4] **全新安裝後第一次 `habit checkin` 必失敗** `habit.store: EventKey not loaded (vault locked)`(exit 2)。根因:沒有任何 CLI 命令會建 `~/.phantom-mesh/identity.key` —— `keys init` 只建 `keys/ed25519.*`(mesh 簽章)、`init` 只建 `PHANTOM.md`。`habit create` 走明文 sqlite 過,`checkin` 走加密 EventStore 死。屬 P4 identity 線(中)。修法候選:checkin 前 auto-derive identity.key,或 `keys init` 一併建。由 `scripts/e2e/full-lifecycle-mac.sh` 抓到(12/13 步,step 4 FAIL)。
- [ ] G2 [MAC-CUJ04-DB-001 + Bug#2] sqlite 損毀復原:寫垃圾進 events.sqlite → 開啟時 rename 成 .corrupt-* sibling + Ok(P4,S,#143 假綠 badge 補真測)
- [ ] G3 [MAC-INV-P4-003 + Bug#3] identity.key 永不外洩:寫 events 後 walkdir events/ 斷言 identity.key bytes + EventKey bytes 出現 0 次(P4 核心隱私 invariant,S,claimed 無測)
- [ ] G4 [MAC-CUJ04-DB-002] first-run 自動建 sqlite:無 events.sqlite 時跑 capture → 檔案被建 + 不 panic(P4,S)
- [ ] G5 [MAC-CUJ05-EXP-001/002] data export 到 stdout:phantom data export --format json|md 對 seeded TempDir HOME → serde_json 解析 / "# Life Node export";驗真 demo 命令(P4/Life,S)
- [ ] G6 [MAC-CUJ04-LLM-003] capture 不靠 LLM:所有 *_API_KEY unset 時 record_checkin 回 Ok/streak 無 HTTP(P1 local-first invariant,S)
- [ ] G7 [MAC-CUJ01-INST-007] install.sh dry-run:PHANTOM_INSTALL_DRY_RUN=1 → exit 0 + 印 URL + "would install" + binary 不變(P1,S,純離線)
- [ ] G8 [MAC-CUJ01-INST-004] install.sh HTTPS-only:PHANTOM_INSTALL_BASE=http://x.com → exit≠0(P4 F-CRIT-3 security invariant,S,純離線)
- [ ] G9 [MAC-CUJ02-FH-002/003/004/005] identity-init test helper(單一 unblocker 點亮 4 case):建 helper 後斷言 event 寫入 / metadata 加密非明文 / fts5 MATCH '水' / 同日 streak(P4/Life,M)
- [ ] G10 [MAC-CUJ02-PROV-002/003/004/005] provider 逐一錯誤碼跳棒:wiremock 回 429/404/401/503 經 PHANTOM_MESH_<SLUG>_BASE_URL,斷言 skip→next + model_used 是第二個;擴充 cuj04_coach_review_llm.rs harness(P1,M)

## Done

- [x] (2026-05-31) G1 [MAC-CUJ01-FH-008 + Bug#1] create_habit slug 驗證 — 加 validate_slug() 守 SPEC-22 §8.2 `[a-z0-9_]{1,32}`,create_habit Step 0 呼叫,InvalidSlug 不再死碼。純函式測試 validate_slug_enforces_spec22_shape(6 壞→InvalidSlug + 5 好邊界→Ok)PASS(TEST_RC=0);cargo build --all-targets 0 errors(BUILD_RC=0)。
