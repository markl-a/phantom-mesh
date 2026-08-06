# spectyn-test - 黑箱場景測試 harness

[English version](README.md)

這是一套 bash-based 測試 harness。它透過 public surfaces 操作已經啟動的
`spectyn`：CLI、HTTP/RPC、磁碟狀態與真實 LLM/RPC round-trip。它不需要
`cargo build`，與 repo 內的 `cargo test` 互補，不是替代品。

| 層級 | 位置 | 驗證內容 | 執行間是否重置 |
|---|---|---|---|
| `cargo test` | `core/src/**/*.rs` | 內部 invariant、render path、parser state | 每個測試全新 |
| `spectyn-test` | `scripts/spectyn-test/` | Wire protocol、磁碟狀態、CLI、真實 round-trip | Side effects 會累積，這是刻意設計 |

## 快速開始

```bash
# 執行全部 scenarios
scripts/spectyn-test/harness.sh

# 只列出 scenarios
scripts/spectyn-test/harness.sh --list

# 依檔名前綴執行子集合
scripts/spectyn-test/harness.sh 02 06 08

# FAIL 後冷卻 30 秒並 retry 一次
scripts/spectyn-test/harness.sh --retry-failed
SPECTYN_TEST_RETRY_COOLDOWN_S=60 scripts/spectyn-test/harness.sh --retry-failed
```

全部通過時 exit `0`；有失敗時 exit `1`；沒有符合 filter 的 scenario 時 exit `2`。
Skip scenario 使用 exit `77`，通常代表缺少依賴，不計為 failure。第一次失敗、
retry 後通過時，summary 會標記為 `PASS-RETRY`。

## 必要環境變數

| 變數 | 預設值 | 用途 |
|---|---|---|
| `SPECTYN_BIN` | `spectyn` | PATH 上的 binary 或絕對路徑 |
| `SPECTYN_HOST` | `127.0.0.1` | Serve host |
| `SPECTYN_PORT` | `7879` | Serve port |
| `SPECTYN_CLUSTER_SECRET` | `spectyn-cluster-2026` | HMAC RPC secret |
| `SPECTYN_CONFIG_DIR` | `~/.spectyn-mesh` | DB 與 event storage |
| `OPENCODE_API_KEY` 或其他 provider key | 無 | 真實 LLM scenario 使用 |

Scenario 07 會主動移除常見 LLM key env vars，驗證沒有 key 時能清楚失敗。

## Scenarios

`scenarios/` 中每個 `.sh` 都是獨立 scenario。README 原文列出最初 16 個；
實際目錄已擴充，請用 `harness.sh --list` 取得目前完整清單。

初始 scenarios 涵蓋：

| Prefix | 驗證內容 |
|---|---|
| `01` | `spectyn doctor` baseline |
| `02`-`04` | RPC HMAC round-trip、錯誤 secret、中文 UTF-8 encoding |
| `05` | Cluster peer list |
| `06`-`07` | 真實 LLM round-trip 與 no-key graceful fail |
| `08` | Windows TUI snapshot |
| `09` | `events.jsonl` append |
| `10`-`11` | Job status 與跨主機 RPC |
| `12` | Deterministic mock LLM |
| `13` | Agent shell tool-call |
| `14` | Swarm fan-out synthesis |
| `15` | Session persistence resume |
| `16` | Autoevolve noop check |

## 新增 scenario

```bash
cat > scripts/spectyn-test/scenarios/11-my-new-thing.sh <<'EOF'
#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"
source "$SPECTYN_TEST_LIB/inspect.sh"

scenario "what this scenario verifies"
require_cmd "$SPECTYN_BIN"

step "doing the first observable thing"
out=$("$SPECTYN_BIN" some-subcommand 2>&1)
ASSERT_CONTAINS "$out" "expected substring" "label for this assertion"

[ "$SPECTYN_TEST_FAILED" -eq 0 ]
EOF
chmod +x scripts/spectyn-test/scenarios/11-my-new-thing.sh
```

Runner 使用 glob 自動搜尋，不需要修改 registry。請使用 `NN-` 數字前綴維持排序。

## Assertions

```text
ASSERT_EQ <a> <b> [label]
ASSERT_CONTAINS <hay> <needle> [label]
ASSERT_NOT_CONTAINS <hay> <needle> [label]
ASSERT_HTTP <url> <expected_code> [label]
ASSERT_FILE_GREW <path> <prev_size> [label]
```

## RPC helpers

```text
rpc_url
rpc_dispatch <agent> <prompt>
rpc_dispatch_with_secret <secret> <agent> <prompt>
rpc_status <job_id>
rpc_state <job_id>
rpc_output <job_id>
rpc_error <job_id>
rpc_wait_done <job_id> <max_seconds>
```

## State inspectors

```text
events_count
events_tail <n>
events_since <ts_ms>
conversations_count
conversation_latest
costs_recent <n>
cluster_peer_count
doctor_summary
now_ms
```

## Mock LLM server

`lib/mock-llm-server.py` 與 `lib/mock.sh` 提供最小 OpenAI-compatible API：

- `GET /v1/models`
- `POST /v1/chat/completions`
- `GET /healthz`

Scripted responses 放在 `fixtures/mock-responses.toml`。Scenario 可這樣啟動：

```bash
source "$SPECTYN_TEST_LIB/mock.sh"
trap 'mock_stop' EXIT

mock_start
agents_dir=$(mock_temp_agents_dir)
out=$(cd "$agents_dir" && spectyn repl --agent master -c "ping")
ASSERT_CONTAINS "$out" "pong" "ping -> pong"
```

Mock 使用 Python stdlib，不需要 `pip install`，也不會修改使用者真實 config。

## TUI snapshot（僅 Windows）

```powershell
powershell -ExecutionPolicy Bypass -File lib/snapshot.ps1
powershell -ExecutionPolicy Bypass -File lib/snapshot.ps1 -Window "spectyn"
```

PNG 會存到 `~/.spectyn-mesh/snapshots/`。

## 已知限制

| 缺口 | 後續方案 |
|---|---|
| 對執行中的 TUI 注入按鍵 | 新增 `spectyn tui --headless --script keys.txt` |
| Headless TUI buffer dump | 新增 `spectyn tui render --state fixture.json --output snap.txt` |
| Linux/macOS TUI snapshot | 將 `snapshot.ps1` 移植到 `xdotool` / `screencapture` |
| Concurrent / load testing | 使用 xargs 或 GNU parallel 包裝 helpers |
| Tool-call mocking | 擴充 mock server 產生 OpenAI `tool_calls` delta |

## 為什麼使用 bash

- 驗證真實 wire protocol、HMAC、serve binary 與 persistence。
- 同一 scenario 可以操作 Linux、Mac、Android 上的 daemon。
- 不需要 build，適合 build 壞掉時做 regression check。
- 內部 refactor 後，只要 CLI、RPC payload、磁碟格式穩定，scenario 仍可使用。

## 什麼時候應該改寫 `cargo test`

適合使用 code-level `cargo test` 的情況：

- 驗證 internal data structure 或 pure function。
- 需要在 CI 中離線執行。
- Failure 應在 compile/test 階段阻擋。

適合使用 `spectyn-test` scenario 的情況：

- 驗證跨 process E2E 行為。
- 涉及 persistence、networking 或 CLI ergonomics。
- 希望捕捉出貨行為 regression。
