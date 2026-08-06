# spectyn-test — 黑箱情境測試框架（black-box scenario harness）

一個以 bash 為基礎的測試框架（harness），透過**已在執行中的 spectyn** 的
公開介面（CLI、HTTP/RPC、磁碟上狀態）來驅動它並斷言（assert）其行為。不需要
cargo build — 它與原始碼樹內的 `cargo test` 測試套件並存，而非取代它。

這兩層是互補的：

| 層級 | 位於 | 驗證 | 各次執行間是否重置 |
|---|---|---|---|
| `cargo test`（例如 `tui_render_tests`） | `core/src/**/*.rs` | 程式碼層級的不變量（invariant）、渲染路徑、剖析器（parser）狀態 | 每個測試都是全新的 |
| `spectyn-test`（此目錄） | `scripts/spectyn-test/` | 傳輸協定（wire protocol）、磁碟上狀態、CLI 介面、真實 LLM/RPC 往返 | 副作用會累積（events.jsonl 會增長等）— 此為刻意設計 |

## Quick start

```bash
# Run all scenarios against the local spectyn serve.
scripts/spectyn-test/harness.sh

# List discovered scenarios without executing.
scripts/spectyn-test/harness.sh --list

# Run a subset by filename prefix.
scripts/spectyn-test/harness.sh 02 06 08

# Retry any FAIL scenario once after a 30s cooldown (helpful for free-tier
# LLM rate-limit flakes — scenarios 13/21 in particular hit OpenCode hard
# enough that under suite-burst load they flake ~10% of the time, but pass
# clean on a calmer second attempt).
scripts/spectyn-test/harness.sh --retry-failed
SPECTYN_TEST_RETRY_COOLDOWN_S=60 scripts/spectyn-test/harness.sh --retry-failed
```

若每個情境都通過，離開碼（exit code）為 0；任一失敗為 1；若沒有任何情境
符合篩選條件則為 2。被略過的情境（exit 77 — 通常是缺少相依套件，例如在
Linux 主機上缺 PowerShell）不算失敗。第一次嘗試失敗但重試後通過的情境，會在
摘要中標記為 `PASS-RETRY`，讓審查者能看出哪些情境需要跑第二次。

## Required environment

| 變數 | 預設值 | 用途 |
|---|---|---|
| `SPECTYN_BIN` | `spectyn` | PATH 上的執行檔，或絕對路徑 |
| `SPECTYN_HOST` | `127.0.0.1` | serve 主機 |
| `SPECTYN_PORT` | `7879` | serve 連接埠（對應 agents.toml 中的 `[core].port`） |
| `SPECTYN_CLUSTER_SECRET` | `changeme-cluster-secret` | 供 HMAC RPC 使用 |
| `SPECTYN_CONFIG_DIR` | `~/.spectyn-mesh` | DB / events.jsonl 所在位置 |
| `OPENCODE_API_KEY`（或任一 LLM 供應商金鑰） | — | 會實際呼叫 LLM 的情境（06）所需 |

對於情境 07（no-key graceful fail，無金鑰時優雅失敗），框架會明確以 `env -u`
清除所有常見的 LLM 金鑰變數，因此不論你的 shell 環境為何，這項檢查都能運作。

## Scenarios

`scenarios/` 中的每個 `.sh` 檔都是一個獨立完整的情境。目前的測試套件
（16 個情境，在一台 node-a 上總計約 290 秒 wall time）：

```
01-doctor-baseline.sh                spectyn doctor structural checks
02-rpc-hmac-roundtrip.sh             POST /rpc/task/assign happy path (ASCII)
03-rpc-hmac-bad-secret.sh            wrong secret → unauthorized
04-rpc-chinese-prompt-encoding.sh    LOCK-IN: MSYS UTF-8 wire-byte gotcha
05-cluster-peer-list.sh              `spectyn peer list` shape
06-llm-roundtrip-cli.sh              `spectyn repl --agent master -c …` (real LLM)
07-llm-no-key-graceful-fail.sh       missing key → exit 1, no Rust panic
08-tui-snapshot-desktop.sh           PowerShell screen capture works (Windows)
09-events-jsonl-grows.sh             events.jsonl appended after a command
10-rpc-bogus-job-id.sh               GET status for missing job_id
11-cluster-rpc-remote-peer.sh        cross-machine HMAC dispatch (auto-discovers
                                       online non-self peer, or uses
                                       SPECTYN_REMOTE_PEER env var)
12-mock-llm-deterministic.sh         in-process mock LLM server: substring,
                                       regex, default fallback, cost reporting
13-agent-shell-tool-call.sh          agent dispatches shell({whoami}), gets stdout,
                                       echoes the local user back — full tool path
14-swarm-fan-out-synthesis.sh        `spectyn swarm` discovery + dispatch + result
                                       collection + synthesis (asserts structure,
                                       not synthesis text — that's LLM-variable)
15-session-persistence-resume.sh     turn 1 writes ./conversations/<sid>.jsonl;
                                       `--session <sid>` recalls the passphrase
16-autoevolve-check-noop.sh          `autoevolve --once --target check` on green
                                       tree exits 0 + appends 1 line to autoevolve.log
                                       (60s timeout catches McAfee/Defender hangs)
```

## Adding a new scenario（新增情境）

```bash
cat > scripts/spectyn-test/scenarios/11-my-new-thing.sh <<'EOF'
#!/usr/bin/env bash
source "$SPECTYN_TEST_LIB/common.sh"
source "$SPECTYN_TEST_LIB/cluster-rpc.sh"   # if you need RPC helpers
source "$SPECTYN_TEST_LIB/inspect.sh"        # if you need event/db helpers

scenario "what this scenario verifies"
require_cmd "$SPECTYN_BIN"   # skip if binary missing

step "doing the first observable thing"
out=$("$SPECTYN_BIN" some-subcommand 2>&1)
ASSERT_CONTAINS "$out" "expected substring" "label for this assertion"

# Many more ASSERT_* helpers — see lib/common.sh.

[ "$SPECTYN_TEST_FAILED" -eq 0 ]   # final exit code reflects assertion outcome
EOF
chmod +x scripts/spectyn-test/scenarios/11-my-new-thing.sh
```

執行器（runner）以 glob（萬用字元比對）來探索情境 — 沒有需要更新的中央
登錄表。請使用 `NN-` 數字前綴，讓 `--list` 中的順序保持穩定。

### Assertions available（可用的斷言）（`lib/common.sh`）

```
ASSERT_EQ <a> <b> [label]                  exact string equality
ASSERT_CONTAINS <hay> <needle> [label]     substring present
ASSERT_NOT_CONTAINS <hay> <needle> [label] substring absent
ASSERT_HTTP <url> <expected_code> [label]  GET, expect HTTP code
ASSERT_FILE_GREW <path> <prev_size> [label] file is bigger than before
```

### RPC helpers（RPC 輔助函式）（`lib/cluster-rpc.sh`）

```
rpc_url                                     -> http://host:port
rpc_dispatch <agent> <prompt>               -> job_id (or empty on auth fail)
rpc_dispatch_with_secret <secret> <agent> <prompt>
rpc_status <job_id>                         -> raw JSON
rpc_state <job_id>                          -> running|done|failed
rpc_output <job_id>                         -> agent text output
rpc_error <job_id>                          -> error string or empty
rpc_wait_done <job_id> <max_seconds>        -> 0 done / 1 failed / 2 timeout
```

### State inspectors（狀態檢查器）（`lib/inspect.sh`）

```
events_count                                # lines in events.jsonl
events_tail <n>                             # last n lines
events_since <ts_ms>                        # lines after ts_ms
conversations_count                         # *.jsonl in conversations/
conversation_latest                         # path to newest jsonl
costs_recent <n>                            # last n LLM cost rows
cluster_peer_count                          # rows in cluster_nodes
doctor_summary                              # `spectyn doctor` ANSI-stripped
now_ms                                      # current epoch ms
```

### Mock LLM server（模擬 LLM 伺服器）（`lib/mock-llm-server.py` + `lib/mock.sh`）

一個獨立的 Python HTTP 伺服器，僅實作 OpenAI 相容 API 中剛好足夠的部分，
讓 spectyn 以為自己正在與真實的 LLM 對話：

- `GET  /v1/models`              — 回傳腳本化（scripted）的模型清單
- `POST /v1/chat/completions`    — 回傳腳本化的回覆（串流或非串流）
- `GET  /healthz`                — 供存活性（liveness）檢查使用

腳本化的回應放在 `fixtures/mock-responses.toml`：

```toml
[[response]]
prompt_match = "ping"          # substring (case-insensitive)
text = "pong"

[[response]]
prompt_regex = "(?i)reply.*ok" # regex (re.search)
text = "ok"
delay_ms = 50                  # optional simulated latency

[default]
text = "MOCK: no scripted response matched"
```

透過 `lib/mock.sh` 在情境中將它接上：

```bash
source "$SPECTYN_TEST_LIB/mock.sh"
trap 'mock_stop' EXIT          # always clean up

mock_start                     # uses fixtures/mock-responses.toml by default
agents_dir=$(mock_temp_agents_dir)
out=$(cd "$agents_dir" && spectyn repl --agent master -c "ping")
ASSERT_CONTAINS "$out" "pong" "ping → pong"
```

`mock_temp_agents_dir` 會寫出一個暫時的 `./agents.toml`，把 `[providers.mock]`
接到 `http://127.0.0.1:11999/v1`。spectyn 會優先採用 cwd（目前工作目錄）內的
`agents.toml`，而非 `$HOME/.spectyn-mesh/agents.toml`（依專案
`agents.toml.example` 中的優先序說明），因此 `cd $agents_dir && spectyn …`
能以決定性（deterministic）方式執行，且不會碰到使用者的真實設定檔。

僅用標準函式庫的 Python（在 3.11+ 上使用 `tomllib`，在較舊的 Python 上則退回
一個 25 行的內嵌 TOML 讀取器）。不需要 `pip install`。

為何採用這種方式，而不是把 `[providers.mock]` 直接編進二進位執行檔？
- **零 Rust 變更** — 不論建置狀態如何，今天就能運作
- **不在 spectyn 中新增公開介面** — 讓二進位執行檔維持窄小的範圍
- **容易演進腳本化回應的語意** — fixture（測試固定資料）格式可以擴充
  （工具呼叫模擬、多輪狀態機）而不必動到 core/

### TUI snapshot（TUI 快照）（`lib/snapshot.ps1`，僅限 Windows）

```powershell
# Full virtual desktop (all monitors)
powershell -ExecutionPolicy Bypass -File lib/snapshot.ps1

# Just one window matching a title substring
powershell -ExecutionPolicy Bypass -File lib/snapshot.ps1 -Window "spectyn"
```

輸出：在 stdout 印出 PNG 路徑，並存檔到 `~/.spectyn-mesh/snapshots/`。

## Limitations & roadmap（限制與藍圖）

這個框架今天還做不到的事（以及該如何修補）：

| 缺口 | 修補方式 |
|---|---|
| 將鍵盤輸入注入執行中的 TUI | 需要在 core 中加入 `spectyn tui --headless --script keys.txt` 模式（已延後 — 需要 Rust 變更） |
| 無頭式（headless）TUI 緩衝區傾印（不靠螢幕擷取） | 需要 `spectyn tui render --state fixture.json --output snap.txt` 子命令（已規劃） |
| ~~決定性 LLM（無真實 API 呼叫）~~ | ✅ 已交付：見上文「Mock LLM server」 |
| Linux/macOS 的 TUI 快照 | 將 `snapshot.ps1` 移植到 `xdotool` / `screencapture`（機械式作業） |
| 並行 / 負載測試 | 用 xargs / GNU parallel 包裝 dispatch 輔助函式（食譜式作法，非內建功能） |
| 工具呼叫模擬 | 擴充 `lib/mock-llm-server.py`，依 fixture 條目發出 OpenAI tool_calls 增量（deltas） |

每個缺口都已記錄為後續工作（follow-up）；框架的結構設計使得新增任一項都是
增量式的（新增 lib/ 輔助函式 + 新增情境），而非侵入式的。

## Why bash, not Rust integration tests?（為何用 bash，而非 Rust 整合測試？）

- **驗證的是傳輸協定，而非抽象的函式呼叫** — 它演練真實的 HMAC、真實的
  serve 二進位執行檔、真實的對話持久化。在 core/ 中直接建構 `AgentRuntime`
  的 `cargo test` 會漏掉序列化器（serializer）錯誤、標頭名稱（header-name）
  拼字錯誤，或服務啟動順序的問題。
- **驅動跨語言介面** — 同一個情境同樣可以打到在 Linux/Mac/Android 上執行的
  spectyn serve。這個框架只需要 bash + curl + python3 + openssl，而這些在本
  repo 矩陣中的每一台開發機器上都有。
- **不需建置** — 可在 `cargo build` 壞掉的機器上運作（例如 node-a 上 McAfee
  即時掃描踩踏 `.rmeta` 寫入），而那正是你最需要做回歸檢查的時候。
- **能撐過大型重構** — 內部型別 / 函式簽章可以自由變更；只要 CLI 參數、
  RPC 酬載（payload）與磁碟上格式維持穩定，情境就會持續通過。

## When to add a `cargo test` instead（何時改用 `cargo test`）

在以下情況使用程式碼層級的測試（位於 `core/src/**/*.rs` 中、標記
`#[cfg(test)]`）：

- 斷言的對象是內部資料結構或純函式（pure function）
- 該測試應能在 CI 中於無網路、無執行中 serve 的情況下執行
- 失敗應該擋下編譯，而非在執行期才發生

在以下情況使用 `spectyn-test` 情境：

- 斷言的對象是跨行程邊界（process boundaries）的端對端行為
- 該測試會演練持久化、網路，或 CLI 的使用體驗
- 你想抓出已上線行為的回歸，即使內部程式碼並未變更（例如相依套件升級悄悄
  破壞了傳輸格式）
