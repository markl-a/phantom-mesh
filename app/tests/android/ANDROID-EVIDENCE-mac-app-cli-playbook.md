# Android emulator evidence — App-mobile rows of the 99-case playbook

Supplements `docs/integration/2026-05-30-mac-app-cli-test-playbook.md`. That playbook
runs most App-mobile rows via `playwright_browser` (the web build at localhost:5173 in a
narrow viewport) or marks them `real_app_only`. This file gives **real Android emulator**
evidence (Pixel_API_36, x86_64, current main builds 5–7) for those rows — stronger than
browser-dev, and the only way to cover the `real_app_only` ones.

Status: `pass` = ran on device, works · `partial` = renders/handles but full path needs a
live coordinator/provider/login · `untested` = not yet run on device.

| Playbook | Feature | Android-emulator result |
|---|---|---|
| ONB-006 | First-launch mode picker (MobileFirstLaunch) | **pass** — fresh `pm clear` → launch shows the 3 entry cards (立即試用 / 加入既有 cluster / phantommesh.io 登入), no crash. |
| ONB-007 | 立即試用 (Demo mode) | **pass** — tap → onboards → lands on 對話 chat shell with the bottom tab bar. |
| ONB-008 | 加入既有 cluster (MobileJoinCluster) | **partial** — "加入 cluster · 步驟 1/3" renders; Coordinator URL field is empty (post-OSS-scrub default) with placeholder `https://my-mac.local:8443`, accepts input, no crash. Full 1→2→3 + connection test not exercised (no live coordinator). |
| ONB-009 | phantommesh.io 登入 (MobileOnboardingV2) | **untested** — needs a real OAuth deep-link round-trip; not run. |
| CHA-009 | 對話 Chat tab (MobileConversation) | **partial** — renders empty-state welcome + prompt-suggestion chips (創作/學習/工作/生活). Send not exercised (no provider key → would surface humanized error). |
| LTC-005 | 專注 Focus timer tab (FocusPage) | **pass** — 番茄鐘 25分 → start → mic permission dialog (SPEC-21 gate) → after grant, state **RECORDING**, timer counts down (24:57), no BigInt error. |
| CLM-005 | 集群 Mesh tab (MobileMesh) | **pass** — renders 集群; local peer shows **Online** badge, **1/1 online**. (This is the screen whose white-screen crash I fixed — peerBadge snake_case case-fix.) |
| CLM-006 | 派送 Dispatch tab (MobileDispatch) | **partial** — prompt textarea + char counter work; tap Dispatch → `E_DISPATCH_AUTH_REQUIRED` (graceful, no key configured), button stays enabled, no crash. Real dispatch needs a coordinator/provider. |
| LTR-006 | 歷史 Dispatch history tab (MobileHistory) | **pass** — renders 歷史 "No dispatches yet" empty state. (Tab revived by the route-fix; was a dead tab.) |
| LTR-007 | 教練 Daily coach review tab (CoachReviewReader, /review) | **pass** — new tab (post-iOS-merge) renders 教練回顧 header, no crash. Full review state (events/empty/locked) needs the daily_review backend; not exercised on device. |
| SEC-006 | 設定 Settings hub (MobileSettings) | **pass** — all 11 sections render (登入 / 診斷 / 手動填 LLM key / 從 Mac 匯入 / Cluster 派送 / 節點管理 / 權限與背景存活 / Providers / Agents / Security / 更新). |
| SEC-007 | 設定 → Cluster 派送 (MobileClusterSettings) | **partial** — section renders (post-scrub example IP `192.0.2.1`). Dispatch round-trip not exercised. |
| SEC-008 | 設定 → 權限與背景存活 (MobilePermissions) | **pass (real_app_only)** — renders mic row (麥克風 · 尚未決定 · `android.permission.RECORD_AUDIO`) + SPEC-33 §15.2 graceful-degradation copy + MIUI guide. Playbook says browser-dev can't verify this — confirmed on real Android. |

## Cross-cutting Android findings (this session)
- All 7 bottom-nav tabs route + render with **zero crashes / no ErrorBoundary trips** on current main.
- Systemic `coerceBigInts()` in `tauri-compat.ts` safeInvoke kills the BigInt-invoke class at the IPC boundary (verified: focus reaches RECORDING; `habit_checkin` reaches backend, not a serialize crash).
- `ErrorBoundary` now wraps the route `<Outlet>` — a route crash like the old `/cluster` one is now contained, not a white-screen.
- **Open (core/identity, not a mobile bug):** Life-Track persistence (habit/food/focus-complete) hits `EventKey not loaded (vault locked)` on a fresh launch with the try-now ephemeral identity. Whether a real phantommesh.io/cluster login sustains the EventKey across restart is untested.

Method: x86_64 debug APK via the `.so`→jniLibs→gradle workaround; driven by `webview-cdp-smoke.mjs` (CDP) + `adb`/logcat. See `README.md` + `verify-runtime.sh`.
