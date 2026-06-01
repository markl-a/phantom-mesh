# Mac 平台 — 全測試用例庫 v2 (2026-05-31)

> **覆蓋範圍**: 5 CUJ × Mac 平台行為 (install / codesign / launchctl / TCC / Apple Silicon) + 跨 SPEC P4 invariant。
>
> **更新**: v2 schema 加 `Auto` / `Setup` / `cmd` / `expected` / `last_run` 欄、讓 CLI runner（或 Appium）可直接讀檔跑、不用人翻譯。
>
> **編號規約**: `MAC-CUJ{NN}-{feat}-{nnn}` 或 `MAC-PLAT-{feat}-{nnn}` (mac 平台專屬) 或 `MAC-INV-{pillar}-{nnn}` (跨 SPEC invariant)

---

## §0. Schema legend

每條 case 欄位：

| 欄 | 意義 |
|---|---|
| **ID** | 唯一識別 (永不重用) |
| **Type** | `unit` (cargo test 內部) / `integ` (cargo test --test) / `e2e` (Maestro/Playwright/shell drive) / `manual` (人眼 / 人手) / `monitor` (synthetic prod canary cron) |
| **Auto** | `✅` 完全自動跑 / `⚠` 需 env 或 fixture / `❌` manual only / `⏰` cron 排程 |
| **Setup** | 跑前準備 (TempDir / env var / mock 服務) |
| **cmd** | 實際命令 (CLI runner 直接抄、shell 可執行) |
| **expected** | 通過條件 (exit code / stdout match / file exist) |
| **Verifies** | 對應 CUJ step + SPEC G[X] |
| **last_run** | 最後驗證日時 + runner |
| **狀態** | `✅` 過 / `🟡` partial / `🔴` 重要缺 / `⬜` 未做 |

---

## §1. CUJ-01: install → 第一筆 habit (activation, 35 條)

### 1.A 線上 install.sh 抓 + 解 (12 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ01-INST-001 | e2e | ⚠ HOME=tmp | `HOME=$(mktemp -d)` 清乾淨 | `curl -fsSL https://phantommesh.io/install.sh \| sh` | exit 0 + `~/.local/bin/phantom` 存在 | CUJ-01 step 1 / SPEC-28 | ⬜ | ⬜ |
| MAC-CUJ01-INST-002 | e2e | ⚠ aarch64 | uname=arm64 | `curl phantommesh.io/install.sh \| sh; cat /tmp/install.log` | log 含 `phantom-darwin-arm64` 字串 | SPEC-28 §arch detect | ⬜ | ⬜ |
| MAC-CUJ01-INST-003 | e2e | ❌ Intel | x86_64 機 | `curl phantommesh.io/install.sh \| sh` | exit 非 0 + stderr 有 "Intel Macs not supported" | SPEC-28 anti-goal | ⬜ | ⬜ |
| MAC-CUJ01-INST-004 | e2e | ✅ | - | `PHANTOM_INSTALL_BASE=http://x.com sh scripts/install.sh` | exit 非 0 + stderr "HTTPS only" | F-CRIT-3 invariant | ⬜ | ⬜ |
| MAC-CUJ01-INST-005 | e2e | ⚠ R2 mock | 篡改 R2 binary、不改 sha256 | (跑線上 install) | exit 非 0 + "SHA mismatch" | F-CRIT-3 sidecar verify | ⬜ | ⬜ |
| MAC-CUJ01-INST-006 | e2e | ✅ | - | `PHANTOM_SKIP_VERIFY=1 sh scripts/install.sh` | install 過 + stderr loud warning | install.sh §override flag | ⬜ | 🟡 |
| MAC-CUJ01-INST-007 | integ | ✅ | isolated HOME | `PHANTOM_INSTALL_DRY_RUN=1 sh scripts/install.sh` | exit 0 + dry-run banner/"no files written" + 無 phantom binary | dry-run invariant | 2026-05-31 cargo | ✅ install_sh_dryrun |
| MAC-CUJ01-INST-008 | manual | ❌ | 先 install 過一次 | (重 install) | 舊 binary 覆蓋、size 變新 | re-install idempotent | ⬜ | ⬜ |
| MAC-CUJ01-INST-009 | e2e | ⚠ kill mid | curl 半途 Ctrl+C | (重複跑 install + kill) | 半成 binary 刪掉、`ls ~/.local/bin/` 乾淨 | atomic install | ⬜ | ⬜ |
| MAC-CUJ01-INST-010 | e2e | ⚠ no-network | wifi off | `curl phantommesh.io/install.sh \| sh` | exit 非 0 + 印 connect error | graceful no-network | ⬜ | ⬜ |
| MAC-CUJ01-INST-011 | manual | ❌ | 3 個 shell | bash/zsh/dash 各跑 install | 3 個都 exit 0 | POSIX compat | ⬜ | ⬜ |
| MAC-CUJ01-INST-012 | monitor | ⏰ nightly | CI 從 clean | (cron job) | 24h ≤ 1 fail | install regression guard | ⬜ | ⬜ |

### 1.B Mac codesign / ad-hoc 處理 (5 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ01-SIGN-001 | e2e | ✅ | install 完 | `~/.local/bin/phantom --version` | exit 0、輸出含 `phantom 0.6.0-rc.1` | Gatekeeper passes | 2026-05-31 02:25 | 🟡 |
| MAC-CUJ01-SIGN-002 | manual | ❌ | 不 ad-hoc | 手動 cp binary、跑 | macOS 印「無法驗證開發者」 | SPEC-63 codesign | ⬜ | ⬜ |
| MAC-CUJ01-SIGN-003 | e2e | ✅ | R2 + local | size 比較 R2 vs `~/.local/bin/phantom` | size 差 < 200KB | resign 期望差 | 2026-05-31 02:25 (差 -139KB) | 🟡 |
| MAC-CUJ01-SIGN-004 | manual | ❌ | - | `ls -la@ ~/.phantom-mesh/bin/phantom` | 行尾無 `@` (xattr 已清) | provenance xattr | 2026-05-31 02:05 | ✅ |
| MAC-CUJ01-SIGN-005 | manual | ⚠ sudo | TCC 鎖檔 | `sudo xattr -c <path>` | exit 0、之後 cp 可進行 | TCC unlock | 2026-05-31 02:05 | ✅ |

### 1.C First-run 環境初始化 (10 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ01-INIT-001 | integ | ✅ | TempDir HOME | `cargo test --test identity_init_outcome_integration` | test result: ok 1 passed | SPEC-12 G1 | ⬜ | ✅ existing |
| MAC-CUJ01-INIT-002 | integ | ✅ | TempDir | 跑兩次 `phantom habit water --qty 1` | `stat ~/.phantom-mesh/identity.key` inode 不變 | identity 持久 | ⬜ | ⬜ |
| MAC-CUJ01-INIT-003 | unit | ✅ | - | `cargo test --lib key_derivation` | test result: ok | HKDF derive | ⬜ | ✅ existing |
| MAC-CUJ01-INIT-004 | integ | ✅ | TempDir HOME | first-run + `sqlite3 ~/.phantom-mesh/events.sqlite ".tables"` | 含 `events` + `events_fts` | SPEC-16 schema | ⬜ | ✅ existing |
| MAC-CUJ01-INIT-005 | integ | ✅ | TempDir | `phantom habit palette list` 首跑 | 印 12 列、含 "水" "咖啡" | SPEC-22 G1 starter | ⬜ | 🟡 (cuj02 verified palette OK) |
| MAC-CUJ01-INIT-006 | unit | ✅ | - | `cargo test --lib event_storage_wire identity_corrupt` | test 過 (返 IdentityKeyMissing) | P4 fail-loud | ⬜ | 🟡 |
| MAC-CUJ01-INIT-007 | unit | ⚠ mock | mock fs err | `cargo test --lib write_failure` | 不 panic、回 Err | P4 disk-full graceful | ⬜ | ⬜ |
| MAC-CUJ01-INIT-008 | integ | ⚠ unset HOME | `HOME=` (empty) | `phantom habit water --qty 1` | exit 非 0 + 印 "HOME unset" | env graceful | ⬜ | 🟡 |
| MAC-CUJ01-INIT-009 | integ | ✅ | 預存 sqlite + first-run | first-run 後 `sqlite3 .. count(*)` | row count 不變 | migration safe | ⬜ | ⬜ |
| MAC-CUJ01-INIT-010 | manual | ❌ | clock | install → first habit ≤ 90s | 計時 ≤ 90s | SLO target | ⬜ | ⬜ |

### 1.D 第一條 habit 流程 (8 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ01-FH-001 | integ | ✅ | TempDir | `phantom habit water --qty 250` | exit 0、stdout 含 "streak" | CUJ-01 step 5 / SPEC-22 G2 | ⬜ | ✅ (cuj02 covered) |
| MAC-CUJ01-FH-002 | unit | ✅ | isolated HOME + EventKey | `record_checkin` → EventStore read (cargo test --ignored) | events/ dir 建 + streak 反映 (event 寫入+可讀) | event 寫入確認 | 2026-05-31 cargo | ✅ capture_habit_wire::tests::checkin_routes_through_encrypted_event_store_no_plaintext_pii |
| MAC-CUJ01-FH-003 | unit | ✅ | isolated HOME + EventKey | checkin note=PII → walk data dir for plaintext | PII note 在磁碟任何檔案 0 次 (加密邊界) | SPEC-22 G7 plaintext boundary | 2026-05-31 cargo | ✅ capture_habit_wire::tests::checkin_routes_through_encrypted_event_store_no_plaintext_pii |
| MAC-CUJ01-FH-004 | integ | ❌ | post-FH-001 | `sqlite3 ~/.phantom-mesh/events.sqlite "SELECT summary FROM events_fts WHERE summary MATCH '水' LIMIT 1"` | 至少 1 列 | FTS5 search 入庫 | 2026-05-31 audit | 🔴 NOT-IMPL: `index_fts5` 唯一 caller 是測試 (event_storage_wire.rs:1159)；無 production capture path 填 events_fts。record_checkin 只寫加密 EventStore。需先把 indexer 接進 capture 流程 (見 AUTONOMOUS-WORKLOG G9-FH004) |
| MAC-CUJ01-FH-005 | unit | ✅ | isolated HOME + EventKey | 同日 2 筆 `record_checkin` (cargo test --ignored) | current_streak=1 + longest=1 (同日 dedup, distinct-day count) | SPEC-22 G3 streak algo | 2026-05-31 cargo | ✅ capture_habit_wire::tests::two_checkins_same_day_keep_streak_at_one |
| MAC-CUJ01-FH-006 | integ | ✅ | TempDir | `phantom habit "讀完 SICP ch3"` | exit 0、event 寫入、chip_id="freetext" | SPEC-22 G5 freetext | 2026-05-31 02:50 | ✅ (cuj02) |
| MAC-CUJ01-FH-007 | manual | ⚠ | TUI | (TUI 介面 tap chip 「水」) | 顯示 "streak=1"、版面不亂 | TUI 渲染 (Bug A) | 2026-05-31 | 🟡 Bug A 渲染面真 PTY 已驗未重現；chip→streak 互動仍待手動/Maestro |
| MAC-CUJ01-FH-008 | unit | ✅ | - | `cargo test --lib capture_habit_wire::tests::validate_slug_enforces_spec22_shape` | test 過 | SPEC-22 slug validate (HABIT-004) | 2026-05-31 | ✅ G1 (fixed live bug: validate_slug + InvalidSlug now enforced; old cited fn `invalid_slug` never existed = false-green) |

---

## §2. CUJ-02: daily capture loop (44 條)

### 2.A `phantom food` photo capture (8 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ02-FOOD-001 | integ | ⚠ mock LLM | wiremock LLM | `phantom food --image tests/fixtures/breakfast.jpg --provider mock` | exit 0、event 寫入 | SPEC-20 G1 | ⬜ | 🟡 |
| MAC-CUJ02-FOOD-002 | integ | ✅ | TempDir + image | (post-001) `ls ~/.phantom-mesh/events/*/modality_image.json` | 開頭是 age v1 magic bytes (`age-encryption.org/v1`) | SPEC-13 P4 image 加密 | ⬜ | ✅ (E004 ship) |
| MAC-CUJ02-FOOD-003 | integ | ⚠ all-fail mock | wiremock 所有 provider 失敗 | `phantom food --image x.jpg` | exit 非 0、不寫 event | provider fail graceful | ⬜ | 🟡 |
| MAC-CUJ02-FOOD-004 | e2e | ⚠ real key | `GEMINI_API_KEY=...` | `cargo test --test life_node_gemini_round_trip` | test result: ok | SPEC-14 real LLM | ⬜ | 🟡 existing |
| MAC-CUJ02-FOOD-005 | manual | ❌ | 7MB jpg | `phantom food --image big.jpg` | exit 非 0 + "size limit" | size guard | ⬜ | ⬜ |
| MAC-CUJ02-FOOD-006 | manual | ❌ | 0 bytes file | `phantom food --image empty.jpg` | exit 非 0 + "non-image" | corrupt guard | ⬜ | ⬜ |
| MAC-CUJ02-FOOD-007 | unit | ✅ | - | `cargo test --lib floor_char_boundary` | test 過 (200 字中文不切 utf-8) | utf-8 safe truncate | ⬜ | ⬜ (was ✅ — cited fn `floor_char_boundary` does not exist; Pool A: write it) |
| MAC-CUJ02-FOOD-008 | monitor | ⏰ cron | real LLM + sample | nightly food capture run | 24h ≤ 1 fail | regression canary | ⬜ | ⬜ |

### 2.B `phantom focus` audio capture (7 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ02-FOC-001 | integ | ⚠ mock ASR | mock | `phantom focus start --duration 1` | exit 0、event 寫入、audio blob 加密 | SPEC-21 G1 | ⬜ | 🟡 |
| MAC-CUJ02-FOC-002 | integ | ⚠ ASR | mock ASR | `phantom focus stop` | takeaway 字串存 metadata | SPEC-21 G2 takeaway | ⬜ | 🟡 |
| MAC-CUJ02-FOC-003 | manual | ❌ | 麥克風關 | `phantom focus start` | 印權限請求、不寫垃圾 | macOS perm | ⬜ | ⬜ |
| MAC-CUJ02-FOC-004 | integ | ⚠ wav | 真 wav 1 min | `phantom focus process tests/fixtures/1min.wav` | OOM 不發生、處理完 | duration cap | ⬜ | ⬜ |
| MAC-CUJ02-FOC-005 | unit | ✅ | - | `cargo test --lib focus_duration_ms` | 1000ms wav → metadata.duration_ms=1000 | duration calc | ⬜ | ⬜ |
| MAC-CUJ02-FOC-006 | manual | ❌ | mid-session | Ctrl+C 中途 | 不寫垃圾 event | cancel cleanup | ⬜ | ⬜ |
| MAC-CUJ02-FOC-007 | unit | ✅ | - | `cargo test --lib focus_asr_all_providers_failed` | 印 ASR fail error | SPEC-21 error catalog | ⬜ | ⬜ (was ✅ — cited fn does not exist; Pool A: write it) |

### 2.C `phantom habit` chip + freetext (10 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ02-HAB-001 | integ | ✅ | TempDir | `for i in $(seq 1 13); do phantom habit palette add --id "t$i" --zh t$i --en t$i; done` | 第 13 加應 fail + "PALETTE-FULL" | SPEC-22 G1 limit | 2026-05-31 02:50 | ✅ (cuj02) |
| MAC-CUJ02-HAB-002 | integ | ✅ | post-001 + remove | `phantom habit palette remove --id t1; phantom habit palette add --id t13 --zh t13 --en t13; phantom habit palette list` | 12 列 | CRUD cycle | ⬜ | ⬜ |
| MAC-CUJ02-HAB-003 | integ | ✅ | TempDir | `phantom habit palette add --id new --zh 新 --en N; phantom habit palette reorder --id new --to 0; phantom habit palette list` | 第 1 列是 new | reorder | ⬜ | ⬜ |
| MAC-CUJ02-HAB-004 | integ | ⚠ time mock | TempDir + mockable clock | 寫 5 連天 events、`phantom habit streak --chip water` | current_streak=5 | SPEC-22 G3 | ⬜ | 🔴 (mockable clock 缺) |
| MAC-CUJ02-HAB-005 | integ | ⚠ time mock | 同上 | 寫 1 day 1+2+4 (跳 3) → streak | current_streak=1 | streak 中斷邏輯 | ⬜ | 🔴 |
| MAC-CUJ02-HAB-006 | integ | ✅ | TempDir | `phantom habit palette add --id bad --freq custom --cron 'not cron'` | exit 非 0 + InvalidCron | cron parse | 2026-05-31 02:50 | ✅ (cuj02) |
| MAC-CUJ02-HAB-007 | unit | ✅ | - | `cargo test --lib habit_frequency_serializes_with_kind_tag` | test 過 | wire shape stable | ⬜ | ✅ existing |
| MAC-CUJ02-HAB-008 | integ | ✅ | TempDir | `phantom habit water --qty 0; --qty -1; --qty 10000` | 邊界處理 (預期 spec 補) | qty validate | ⬜ | ⬜ |
| MAC-CUJ02-HAB-009 | manual | ❌ | desktop UI | Cmd+Shift+H 按 | popover 出 + chip list | global shortcut | ⬜ | ⬜ |
| MAC-CUJ02-HAB-010 | integ | ⚠ data | 30 天 events | `phantom habit grid --chip water --weeks 4` | ASCII 7×4 cells | SPEC-22 grid render | ⬜ | ⬜ |

### 2.D coach engine + delivery (10 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ02-COA-001 | integ | ⚠ LLM | mock LLM + 1 event | `phantom coach review --date 2026-05-30` | exit 0、stdout markdown | SPEC-23 G1 | 2026-05-31 | ✅ cuj04_coach_review_llm (happy path → Completed) |
| MAC-CUJ02-COA-001a | integ | ✅ | TempDir + 25h-ago event | `phantom coach review --date yesterday` | review 不含 25h 前 event | aggregator window | ⬜ | ⬜ |
| MAC-CUJ02-COA-001b | integ | ⚠ mock | mock LLM | (same as 001) | LLM 被 call (mock 收 request) | SPEC-23 LLM hook | ⬜ | ⬜ |
| MAC-CUJ02-COA-001c | integ | ⚠ mock | mock LLM | (same as 001) | output markdown 格式正確 (h1 + bullets) | markdown 結構 | ⬜ | ⬜ |
| MAC-CUJ02-COA-002 | integ | ⚠ LLM | mock | empty events.sqlite | review 含 "昨日無資料" | 0 data graceful | ⬜ | ⬜ |
| MAC-CUJ02-COA-003 | integ | ✅ | mock skill DB | `phantom coach review --include-skills` | review 提到 pattern | SPEC-25 skill 拉 | ⬜ | ⬜ |
| MAC-CUJ02-COA-004 | e2e | ✅ | TempDir | `phantom coach schedule install` | exit 0、`~/Library/LaunchAgents/ai.phantommesh.coach.plist` 存在 | SPEC-23 G1 launchd | ⬜ | 🟡 ship |
| MAC-CUJ02-COA-005 | manual | ❌ | 1 天 | install + 等 7:00 | `~/.phantom-mesh/reviews/<date>.md` 存在 | launchd 觸發 | ⬜ | ⬜ |
| MAC-CUJ02-COA-006 | manual | ❌ | 5 scenario prompts | LLM 跑 + 看 output | 不含羞辱字 | shame-free lint | ⬜ | ⬜ |
| MAC-CUJ02-COA-007 | integ | ⚠ TG token | `TELEGRAM_BOT_API_KEY=...` | cron 觸 + 看 bot | bot 收 markdown | SPEC-24 telegram | ⬜ | ⬜ |
| MAC-CUJ02-COA-008 | monitor | ⏰ nightly | sample + judge | 跑 100 review + auto-eval | 95%+ useful insight | quality canary | ⬜ | ⬜ |

### 2.E provider fallback chain (SPEC-14, 9 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ02-PROV-001 | unit | ✅ | - | `cargo test --lib providers_wire::score_5_factor` | 對 mock 5 provider 排序對 | SPEC-14 G2 | ⬜ | 🟡 |
| MAC-CUJ02-PROV-002 | integ | ✅ | wiremock 429 on primary (groq) + 200 on anthropic | `complete_with_fallback` via run_daily_review | 跳第 2 provider → Completed + next_action | quota fallback | 2026-05-31 cargo | ✅ cuj04_coach_review_llm::cuj04_coach_review_llm_paths |
| MAC-CUJ02-PROV-003 | integ | ✅ | wiremock 404 on primary + 200 on second | (same) | 跳第 2 → Completed | model-not-found skip | 2026-05-31 cargo | ✅ cuj04_coach_review_llm::cuj04_coach_review_llm_paths |
| MAC-CUJ02-PROV-004 | integ | ✅ | wiremock 401 on primary + 200 on second | (same) | 跳第 2 → Completed | auth-fail skip | 2026-05-31 cargo | ✅ cuj04_coach_review_llm::cuj04_coach_review_llm_paths |
| MAC-CUJ02-PROV-005 | integ | ✅ | wiremock 503 on primary + 200 on second | (same) | 5xx → 跳第 2 → Completed | 5xx retry/skip | 2026-05-31 cargo | ✅ cuj04_coach_review_llm::cuj04_coach_review_llm_paths |
| MAC-CUJ02-PROV-006 | integ | ⚠ mock | wiremock 全 fail | (same) | exit 0、review 退化 stats-only | 全 fail fallback (#142) | 2026-05-31 | ✅ cuj04_stats_only_fallback |
| MAC-CUJ02-PROV-007 | unit | ✅ | - | `cargo test --lib providers_wire::tests::same_class_models_rank_by_cost_and_ttft` | TTFT/cost 排序對 | SPEC-14 latency factor | 2026-05-31 | ✅ re-pointed (was absent `ttft_sort_interactive`) |
| MAC-CUJ02-PROV-008 | unit | ✅ | - | `cargo test --lib floor_char_boundary_200_zh` | 不切 utf-8 char | utf-8 safe | ⬜ | ⬜ (was ✅; cited fn absent — Pool A: write it) |
| MAC-CUJ02-PROV-009 | e2e | ✅ | tmux 真 PTY + 無 provider key | `COLS=100 ROWS=30 scripts/e2e/tui-provider-error.sh` | 無溢位 + 無 escape leak + 邊框完整 + error 有出現 | TUI 渲染 Bug A | 2026-05-31 PTY | 🟢 60/100/200 全 PASS (未重現) |

---

## §3. CUJ-03: cross-device sync (15 條)

### 3.A broker login + token mgmt (5 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ03-LOG-001 | e2e | ⚠ broker mock | `BROKER_URL=mock` | `phantom login --provider google` | exit 0、`broker.json` 存在 | OAuth flow | ⬜ | ✅ existing |
| MAC-CUJ03-LOG-002 | integ | ✅ | post-001 + clear cache | `phantom cluster status` | token 有效、200 OK | token persistence | ⬜ | ⬜ |
| MAC-CUJ03-LOG-003 | integ | ⚠ mock | mock broker 401 | `phantom cluster status` | UI 印 "重新登入"、不 panic | token expired UX | ⬜ | 🟡 |
| MAC-CUJ03-LOG-004 | manual | ❌ | 等 token expire | (等 + 再用) | 自動 refresh、user 無感 | refresh-token flow | ⬜ | ⬜ |
| MAC-CUJ03-LOG-005 | unit | ✅ | - | `cargo test --lib spec12_zeroize_token_after_use` | reachable mem 不存 token | SPEC-12 P4 zeroize | ⬜ | 🟡 |

### 3.B cluster join + mDNS (5 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ03-CLU-001 | e2e | ⚠ peer | 同網段 phantom serve | `phantom cluster join` | mdns list ≥ 1 peer | mDNS discovery | 2026-05-31 02:25 (1/4) | ✅ partial |
| MAC-CUJ03-CLU-002 | integ | ⚠ mock | 3 dead peers + 1 live | (cluster status) | 持續 ping、不 hang、印 ✗ × 3 + ✓ × 1 | retry graceful | 2026-05-31 02:25 | ✅ |
| MAC-CUJ03-CLU-003 | integ | ✅ | - | `cargo test --test mesh_tailscale_test` | tailscale IP 偏好 + serde round-trip | SPEC-10 §6.4 | 2026-05-31 | ✅ re-pointed (was absent `mesh::tailscale_stub`) |
| MAC-CUJ03-CLU-004 | e2e | ✅ | post-CLU-001 | `phantom cluster status` | 印 ✓/✗ 每節點 ping ms | status command | 2026-05-31 02:25 | ✅ |
| MAC-CUJ03-CLU-005 | unit | ✅ | - | `cargo test --test cluster_heartbeat_selection` | 假 token 連 → 拒 + log | auth invariant | ⬜ | ✅ existing |

### 3.C sealed event sync (5 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ03-SYN-001 | integ | ✅ | - | `cargo test --test spec15_broker_vault_e2ee_regression` | test result: ok | round trip seal/unseal | 跑著 | ✅ existing |
| MAC-CUJ03-SYN-002 | integ | ✅ | - | (same test) | 監 VaultSetRequest 無 plaintext | broker no-plaintext | 跑著 | ✅ existing |
| MAC-CUJ03-SYN-003 | e2e | ⚠ 2 device | 2 mac + 同帳號 broker | `mac1: phantom habit water; mac2: phantom habit streak --chip water` | mac2 看得到 ≤ 5s | CUJ-03 5s SLO | ⬜ | 🔴 沒量過 |
| MAC-CUJ03-SYN-004 | integ | ⚠ mock | 2 EventStore mock | (race log 同秒) | 兩筆都進、UUID 不重 | concurrent insert | ⬜ | ⬜ |
| MAC-CUJ03-SYN-005 | manual | ❌ | broker down/up | log 5 筆 → broker open | queue 自動 drain | offline queue | ⬜ | ⬜ |

---

## §4. CUJ-04: degraded states (16 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ04-OFF-001 | manual | ❌ | wifi off | `phantom habit water --qty 250` | exit 0、event 寫本地 | local-first | ⬜ | ⬜ |
| MAC-CUJ04-OFF-002 | integ | ⚠ mock | mock all LLM fail | `phantom coach review` | exit 0、stats-only output | stats-only mode (#142) | 2026-05-31 | ✅ cuj04_stats_only_fallback |
| MAC-CUJ04-OFF-003 | manual | ❌ | wifi back on | (再 sync) | ≤ 30s drain backlog | online recover | ⬜ | ⬜ |
| MAC-CUJ04-LLM-001 | integ | ⚠ mock | wiremock all fail | (same as OFF-002) | (same) | 同 OFF-002 | 2026-05-31 | ✅ cuj04_stats_only_fallback |
| MAC-CUJ04-LLM-002 | e2e | ⚠ broken cfg | agents.toml 含 404 model | `phantom exec test` | fallback 跳第 2 | model 過期 skip | 2026-05-31 02:25 | ✅ (今天觸發過) |
| MAC-CUJ04-LLM-003 | unit | ✅ | isolated HOME + 全 `*_API_KEY` unset | `record_checkin` (cargo test --ignored) | Ok + streak≥1 (capture 不靠 LLM/HTTP) | local-first capture | 2026-05-31 cargo | ✅ capture_habit_wire::tests::record_checkin_is_local_first_with_no_llm_keys |
| MAC-CUJ04-LLM-004 | integ | ✅ | TempDir 無 LLM key | `phantom coach review --date yesterday` | exit 非 0 或友善 "設定 → providers" | no-key UX | ⬜ | ⬜ |
| MAC-CUJ04-BRK-001 | integ | ⚠ mock | mock broker 401 mid-sync | (sync request) | queue 留本地、UI 印 hint | token expired graceful | ⬜ | 🟡 |
| MAC-CUJ04-BRK-002 | integ | ⚠ mock + relogin | mock | (re-login + drain) | ≤ 30s queue drain | drain after relogin | ⬜ | ⬜ |
| MAC-CUJ04-IK-001 | integ | ✅ | TempDir + corrupt identity | `phantom habit water --qty 250` | exit 非 0、fail-loud (不退化明文) | P4 fail-loud | ⬜ | 🟡 |
| MAC-CUJ04-IK-002 | integ | ✅ | TempDir + 0-byte identity | (same) | exit 非 0 + "IdentityKeyMissing" | empty file guard | ⬜ | ⬜ |
| MAC-CUJ04-IK-003 | manual | ❌ | regen 後 | (regen + read 舊 event) | 之前 event 仍可讀 | recovery flow | ⬜ | ⬜ |
| MAC-CUJ04-DB-001 | integ | ✅ | TempDir + 壞 sqlite header | `phantom habit water --qty 250` | exit 0、rename to `.corrupt-<ts>`、新建 fresh | sqlite recovery (✅ #143) | 2026-05-31 02:55 | ✅ ship |
| MAC-CUJ04-DB-002 | integ | ✅ | isolated HOME 無 events.sqlite | `index_fts5` 第一次 capture | 自動建 sqlite + 不 panic + 無 .corrupt sibling + search 找得到 | first-run auto-create | 2026-05-31 cargo | ✅ cuj04_sqlite_recovery::cuj04_first_run_creates_events_sqlite_when_absent |
| MAC-CUJ04-DSK-001 | manual | ❌ | mock disk full | (capture) | 印 err、不寫部分檔 | disk-full atomic | ⬜ | ⬜ |
| MAC-CUJ04-PERM-001 | manual | ❌ | TCC 撤銷 | (read Documents) | 印 friendly hint | TCC graceful | 2026-05-31 02:05 | ✅ (今天碰到) |

---

## §5. CUJ-05: export + uninstall (12 條, 50% 已 ship)

> **2026-05-31 audit**: E004 Task 6 已 ship `phantom data export` (JSON/MD) + `phantom data delete --all --yes` (本機 events_dir). 新加 `phantom backup --to PATH` (tar.gz 全 backup, commit 391bd38b).

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-CUJ05-EXP-001 | integ | ✅ | isolated HOME (empty vault) | `phantom data export --format json` | exit 0 + stdout parses as JSON array | E004 export | 2026-05-31 cargo | ✅ cuj05_data_export::cuj05_data_export_json_emits_parseable_array |
| MAC-CUJ05-EXP-002 | integ | ✅ | isolated HOME (empty vault) | `phantom data export --format md` | exit 0 + stdout 含 `# Life Node export` header | E004 export | 2026-05-31 cargo | ✅ cuj05_data_export::cuj05_data_export_md_has_life_node_header |
| MAC-CUJ05-EXP-003 | manual | ❌ | mid-export | Ctrl+C | 半成 file 刪 | atomic | ⬜ | ⬜ |
| MAC-CUJ05-EXP-004 | integ | ✅ | TempDir + 4 sample files | `phantom backup --to /tmp/x.tar.gz; tar -xzf /tmp/x.tar.gz -C /tmp/restore; diff -r ~/.phantom-mesh /tmp/restore/.phantom-mesh` | 0 diff | CUJ-05 tar.gz roundtrip (✅ #139) | ⬜ | ✅ ship |
| MAC-CUJ05-EXP-005 | integ | ✅ | TempDir | `phantom backup` (missing --to) | exit 非 0 + 印 "--to" hint | required-flag | ⬜ | ✅ ship |
| MAC-CUJ05-EXP-006 | integ | ✅ | TempDir 無 ~/.phantom-mesh/ | `phantom backup --to /tmp/x.tar.gz` | exit 非 0、不留垃圾 | first-run guard | ⬜ | ✅ ship |
| MAC-CUJ05-DEL-001 | integ | ✅ | TempDir + events | `phantom data delete --all --yes` | exit 0、`ls events/` 空 | E004 delete | ⬜ | ✅ ship |
| MAC-CUJ05-DEL-001a | integ | ✅ | post-DEL-001 | `ls ~/.phantom-mesh/identity.key && ls ~/.phantom-mesh/agents.toml` | 兩檔仍存 | scope invariant | ⬜ | ✅ ship |
| MAC-CUJ05-DEL-002 | e2e | ⚠ broker mock | mock broker DELETE | `phantom data delete --all --yes --include-broker` | broker DELETE 收 req | broker wipe (🔴 #140) | ⬜ | 🔴 待 #140 |
| MAC-CUJ05-DEL-003 | integ | ✅ | TempDir | `phantom data delete` (no --all) | exit 非 0 + "require --all" | safety flag | ⬜ | ✅ ship |
| MAC-CUJ05-DEL-004 | integ | ✅ | TempDir | `phantom data delete --all` (no --yes) | exit 非 0 + confirm prompt | confirm flag | ⬜ | ✅ ship |
| MAC-CUJ05-REI-001 | e2e | ⚠ 2-step | backup → delete-all → reinstall + restore | `tar -xzf backup.tar.gz -C ~/; phantom habit streak --chip water` | 之前 event 全可解讀 | reinstall roundtrip (🔴 #141) | ⬜ | 🔴 待 #141 |
| MAC-CUJ05-REI-002 | manual | ❌ | reinstall + 沒 identity | (open phantom) | 印「沒 key 無法解 events」hint | no-key-import UX (🔴 #141) | ⬜ | 🔴 待 #141 |

---

## §6. Mac 平台專屬 (PLAT, 9 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-PLAT-LCD-001 | e2e | ✅ | TempDir | `phantom service install` | LaunchAgent .plist 寫入 | SPEC-40 LaunchAgent | ⬜ | ✅ existing |
| MAC-PLAT-LCD-002 | e2e | ✅ | post-LCD-001 | `launchctl kickstart -k gui/$(id -u)/ai.phantommesh.serve` | service 重啟 | LaunchAgent restart | 2026-05-31 02:25 (install.sh 用) | ✅ |
| MAC-PLAT-LCD-003 | e2e | ✅ | post-LCD-001 | `phantom service uninstall` | .plist 刪、service 停 | LaunchAgent teardown | ⬜ | ⬜ |
| MAC-PLAT-LCD-004 | unit | ✅ | - | `cargo test --lib coach_scheduler::tests::launchd_plist_has_label_command_and_calendar_interval` | 含 Label + ProgramArguments | plist schema | 2026-05-31 | ✅ re-pointed (was absent `render_cli_unit_mac_plist`) |
| MAC-PLAT-PRO-001 | manual | ❌ | 新 build | `./phantom --version` | 不被 Gatekeeper 擋 | provenance OK exec | 2026-05-31 02:25 | ✅ |
| MAC-PLAT-PRO-002 | manual | ❌ | new build + TCC default | `cp target/.../phantom /tmp/p` | "Operation not permitted" | provenance blocks cp | 2026-05-31 02:30 | ✅ (今天碰過) |
| MAC-PLAT-PRO-003 | manual | ⚠ sudo | new build | `sudo xattr -c <path>; cp <path> /tmp/p` | cp 可進行 | provenance clear via sudo | 2026-05-31 02:05 | ✅ |
| MAC-PLAT-SPT-001 | manual | ❌ | post first-run | `mdfind ~/.phantom-mesh/events.sqlite` | 不出現 (Spotlight 不 index 或不洩明文) | SPEC-13 P4 Spotlight | ⬜ | ⬜ |
| MAC-PLAT-NOT-001 | manual | ❌ | first coach delivery | (coach 推 notification) | 印權限請求對話框 | macOS NSUNotificationCenter | ⬜ | ⬜ |

---

## §7. 跨 SPEC P4 invariant (5 條)

| ID | Type | Auto | Setup | cmd | expected | Verifies | last_run | 狀態 |
|---|---|---|---|---|---|---|---|---|
| MAC-INV-P4-001 | integ | ✅ | TempDir + 1 event | `head -c 30 ~/.phantom-mesh/events/*/meta.json \| xxd` | 開頭是 age-v1 magic (`age-encryption.org/v1\n`) | events 加密 | ⬜ | ✅ existing |
| MAC-INV-P4-002 | integ | ✅ | post-FH-001 | `sqlite3 ~/.phantom-mesh/events.sqlite "SELECT length(metadata_json) FROM events LIMIT 1"` | length > 0 + base64/binary、非 plain JSON | metadata 加密 | ⬜ | 🟡 |
| MAC-INV-P4-003 | integ | ✅ | TempDir + identity | `grep -r "$(head -c 16 ~/.phantom-mesh/identity.key)" ~/.phantom-mesh/events/` | 0 matches | identity 不洩 events | ⬜ | ✅ |
| MAC-INV-P4-004 | unit | ✅ | - | `cargo test --test spec15_broker_vault_e2ee_regression seal_unseal_roundtrip` | byte-identical | seal/unseal 對稱 | 跑著 | ✅ existing |
| MAC-INV-P4-005 | unit | ✅ | - | `cargo test --test spec15_broker_vault_e2ee_regression no_plaintext_payload` | VaultSetRequest 序列化全 sealed | broker no-plaintext | 跑著 | ✅ existing |

---

## §8. ship gate 對映 (noon target)

| Gate | 對應測試 IDs | 狀態 |
|---|---|---|
| Gate 1: 本機 build | MAC-CUJ01-INST-002 | ✅ build 02:22 |
| Gate 2: R2 + install.sh | MAC-CUJ01-INST-001, 002, 005, MAC-CUJ01-SIGN-001 | 🟡 |
| Gate 3: CUJ-01 first habit | MAC-CUJ01-INIT-001..010, MAC-CUJ01-FH-001..008 | 🟡 大部分過；Bug A 渲染面真 PTY 已驗未重現，FH-007 剩 chip→streak 互動待 Maestro |
| Gate 4: CUJ-02 capture | MAC-CUJ02-FOOD-001 OR HAB-001 | 🟡 |
| Gate 5: mac+mac sync | MAC-CUJ03-SYN-003 | ⬜ |

---

## §9. 統計

```
Total: 130 條 (v1 127 + v2 補 3: COA-001a/b/c + DEL-001a)

By Type:
  unit:    19 (+1)
  integ:   56 (+2)
  e2e:     24
  manual:  26
  monitor: 5

By Auto:
  ✅ 全自動:           62
  ⚠ 需 env / fixture: 32
  ❌ manual-only:     31
  ⏰ cron:             5

By 狀態 (2026-05-31 02:55):
  ✅ 過 / 已驗:        22 (+4 from v1: cuj02 cuj05 backup 等驗過)
  🟡 partial:         28
  ⬜ 未做:              66
  🔴 重要缺:           14 (拆細後 14 條)
```

---

## §10. 14 條 🔴 重要缺洞優先序

```
🔴 高 (release-blocker):
  MAC-CUJ04-OFF-002 / LLM-001 / PROV-006  ← stats-only 同 task #142
  MAC-CUJ05-REI-001 / REI-002             ← reinstall import task #141
  MAC-CUJ05-DEL-002                        ← broker DELETE task #140

🔴 中 (P0 sprint):
  MAC-CUJ03-SYN-003 (5s SLO 沒量過)
  MAC-CUJ02-HAB-004 / HAB-005 (streak 跨天需 mockable clock)
```

---

## §11. CLI runner 範例 (能 ✅ Auto 的全跑)

```bash
#!/bin/bash
# scripts/test-runner-mac.sh — 跑所有 Auto=✅ 的 test
set -e
RESULTS=/tmp/mac-test-results-$(date +%Y%m%d-%H%M%S).json
TEMP=$(mktemp -d)
export PHANTOM_TEST_BIN=$(realpath core/target/aarch64-apple-darwin/release/phantom)

# CUJ-01 + CUJ-02 integration via cargo test
cd core
cargo test --target aarch64-apple-darwin --test cuj02_daily_habit_subset
cargo test --target aarch64-apple-darwin --test cuj05_backup_export
cargo test --target aarch64-apple-darwin --test spec15_broker_vault_e2ee_regression
cargo test --target aarch64-apple-darwin --test identity_init_outcome_integration
cargo test --target aarch64-apple-darwin --test cluster_heartbeat_selection

# CLI-driven cases (per row 的 `cmd` column)
HOME=$TEMP $PHANTOM_TEST_BIN habit water --qty 250 | grep -q streak  # FH-001
sqlite3 $TEMP/.phantom-mesh/events.sqlite "SELECT count(*) FROM events" | grep -q '^[1-9]'  # FH-002
$PHANTOM_TEST_BIN habit "讀完 SICP ch3" | grep -q '✓\|streak'  # FH-006
HOME=$TEMP $PHANTOM_TEST_BIN data export --format json | jq -e '.events | length >= 1'  # EXP-001
$PHANTOM_TEST_BIN backup --to /tmp/backup-test.tar.gz && tar tzf /tmp/backup-test.tar.gz | grep -q identity.key  # EXP-004

# CUJ-03 cluster (assumes 1+ peer running)
$PHANTOM_TEST_BIN cluster status | grep -qE "✓|✗"  # CLU-004

echo "All ✅ Auto tests passed" > $RESULTS
```

---

## §12. Appium / Maestro 範例 (manual=❌ 但 GUI 自動化可能的部分)

對 TUI 渲染 + onboarding 流：

```yaml
# flow/cuj-01-tui-onboarding.maestro.yaml
appId: ai.phantommesh.app
---
- launchApp
- assertVisible: "Phantom Mesh"  # MAC-CUJ01-FH-007
- tapOn: "水"
- inputText: "250"
- tapOn: "✓"
- assertVisible: "streak=1"
- takeScreenshot: cuj01-first-habit
```

或對 terminal 端走 AppleScript 驅動：

```bash
# scripts/appium-terminal-cuj01.sh
osascript -e 'tell application "Terminal" to do script "phantom habit water --qty 250"'
sleep 2
osascript -e 'tell application "Terminal" to display dialog "Check streak=1 visible?"'
```

---

## §13. 改 v2 的 5 大強化（vs v1）

1. ✅ 加 `Auto` 欄 ── CLI runner 可直接 filter `Auto=✅` 跑
2. ✅ 加 `Setup` 欄 ── 每 test 之間 isolated、可平行
3. ✅ 加 `cmd` 欄 ── runner 不用翻譯中文描述
4. ✅ 加 `last_run` 欄 ── 看 ✅ 是否最近驗過
5. ✅ Fix v1 漏判：CUJ-05 不是 0%、50% 已 ship 為 `data` namespace；CUJ-04 DB-001 已 ✅ ship 為 #143

---

## §14. 對 Appium 自動化 roadmap

Phase 1 (現在做): CLI runner (§11 已寫) 跑所有 ✅ Auto
Phase 2 (B 後): Maestro YAML 跑 mobile + TUI flows
Phase 3 (CUJ-02 全收): Promptfoo eval 跑 LLM 行為 regression
Phase 4 (production): Checkly 跑 nightly synthetic monitor

Phase 1 已可直接被 `scripts/test-runner-mac.sh` 啟用。
x