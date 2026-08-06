# P1-2 Mobile Supervisor — Real-Device Checklist

> 從屬於 apex (`docs/superpowers/BIG-GOAL.md`)。本檔只是 P1-2 手機監督面的真機驗證清單,
> 不主張任何 apex/北極星/唯一真相地位。

Backend prep: on the supervised node run `spectyn serve` with a non-empty
`[cluster].cluster_secret` in agents.toml. Note its Tailscale URL + secret.
In the app Settings (設定) tab: enter Base URL + Cluster Secret, tap 測試連線 → 連線成功.

Cross-refs: SPEC-31 (iOS screens), SPEC-32 (iOS flows), SPEC-34 (Android screens/flows).

Backend contract is locked by the hermetic Rust tests
(`serve::squad_dispatch_tests::rpc_tasks_list_*` / `rpc_captures_recent_*` /
`rpc_review_*`, run via `cargo test -p spectyn-mesh --lib serve::squad_dispatch_tests::rpc_`)
and the app-side parser + wire-contract tests under `app/tests/p12/`. This file
covers what only a real phone + real backend can exercise.

## 任務 (Task State) — SPEC-32 flow

- [ ] Dispatch a task from the 對話 tab to the backend, then open 任務.
- [ ] The task appears with agent name + RUNNING, then flips to DONE/ERROR.
- [ ] Tap ↻ — list refreshes; leave the tab open 15s — it auto-refreshes (no tap).
- [ ] With a governed run mid-approval, a 待審核 card shows tool + risk.
- [ ] Offline backend → inline 讀取失敗 error + TopBar pill goes offline.

## 擷取 (Capture History) — SPEC-31 screen

- [ ] Capture a food/focus/habit event on the backend (`spectyn event capture …`).
- [ ] Open 擷取 → the event shows with kind emoji, local time, goal tags, newest first.
- [ ] Empty backend → 尚無擷取事件 (not a crash).

## 回顧 (Coach Review) — SPEC-31 screen #3

- [ ] With ≥1 capture today, open 回顧 → 「Daily review — <today>」 + grouped bullets.
- [ ] Each bullet shows kind · time · summary; goal-tag group headers present.
- [ ] No captures today → 今天還沒有擷取事件 (graceful empty).

## Auth / security

- [ ] Wrong secret in Settings → every supervisor tab shows 讀取失敗 (401/403), no data leak.
- [ ] Confirm no hardcoded backend address or secret shipped (re-run the ANDAPP-LEAK scrub grep).

## Android system-back (SPEC-34 §10D)

- [ ] On the first tab, Android back exits the app (passthrough).
- [ ] After navigating tabs, Android back pops within the SPA first.
