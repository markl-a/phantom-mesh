> ⚠️ pre-pivot — 方向已被現行 4-pillar Life/Work(superpowers/BIG-GOAL.md)取代;戰術細節或許仍可用。

# spectyn ↔ 其他 AI 工具 — 上下文分享與跨工具呼叫

**Status（狀態）**：design / analysis（設計／分析，尚未實作）。
**Target（目標版本）**：v0.2（在 5/15 OSS（開源軟體）發佈之後）。
**Estimated cost（預估成本）**：完整功能面約需 4-5 天。

使用者想要兩件不同的事：

1. **Context sharing（上下文分享）** — spectyn 可以把它目前的狀態（近期
   對話、改動過的檔案、已做的決策、TODO（待辦）清單、repo（程式碼倉庫）
   上下文）交接給其他 AI 工具（Claude Code、Codex、Gemini CLI、Antigravity、Aider…），
   讓下一個工具從 spectyn 中斷的地方繼續。

2. **Cross-tool invocation（跨工具呼叫）** — 在 spectyn 內部，把另一個 AI
   CLI（命令列介面）當成可呼叫的工具。像是 `delegate(tool="gemini", prompt="…")`
   — 與今天的 `subagent` 工具形狀相同，但這裡的「agent（代理）」是一個
   *不同的產品*，而非一台不同的 *機器*。

## TL;DR — 可行性

| 功能 | 可行嗎？ | 原因 |
|---|---|---|
| 匯出上下文給 Claude Code（CLAUDE.md / @-include） | ✅ 簡單 | 已經會寫類似 AGENTS.md 的檔案 |
| 匯出上下文給 Codex（transcript，逐字記錄） | ✅ 中等 | Codex 有自己的格式；從 JSONL 轉換 |
| 匯出上下文給 Gemini CLI | ✅ 簡單 | 接受 stdin（標準輸入）/ `@file` |
| 匯出上下文給 Antigravity | ⚠️ 部分 | Antigravity 的 CLI 是 VS Code-shell 形狀（`-d`、`-g`、`--diff`）；沒有 LLM-prompt（大型語言模型提示）旗標。能做到的極限是：把一個上下文檔案丟進 project（專案），讓 Antigravity 的 IDE 內建聊天去讀 |
| 把 `gemini` CLI 當成工具呼叫 | ✅ 約半天 | spawn（衍生）子行程、pipe（管線）stdin、讀取 stdout（標準輸出） |
| 把 `codex` CLI 當成工具呼叫 | ✅ 約半天 | 相同形狀 |
| 把 `claude` CLI（Claude Code）當成工具呼叫 | ✅ 約半天 | 相同形狀；透過 `ANTHROPIC_API_KEY` 做 auth（認證） |
| 把 Antigravity 當成「幫我做這件事」的 agent 呼叫 | ❌ 目前不行 | 現行 Antigravity 版本（1.107.0）沒有 headless（無介面）/ prompt-API |
| 雙向上下文（把其他工具的狀態讀進 spectyn） | ⚠️ 逐工具處理 | Claude Code 的 session JSONL 算好讀；Codex 類似；Antigravity / Cline / Continue 在 IDE 側，較難 |

**沒有硬性技術障礙**，只是水管接線（plumbing）加上每個工具的 adapter（轉接器）工作。

## 設計

### 功能 A — `/share` slash 指令 + `spectyn share` 子指令

```
/share claude                  # 寫到 ~/.spectyn-mesh/share/claude-<sid>.md
/share codex                   # 寫到 ~/.spectyn-mesh/share/codex-<sid>.md
/share gemini                  # 寫到 ~/.spectyn-mesh/share/gemini-<sid>.md
/share antigravity             # 寫到 project 內的 .antigravity/context.md
/share AGENTS.md --append      # 把 "## Recent context" 區塊附加到 project 的 AGENTS.md
/share --copy                  # 放到剪貼簿（使用 `pbcopy` / `xclip` / `clip`）
/share --to <path>             # 寫到任意位置
```

格式轉換器放在新的 `core/src/share.rs` 模組裡：

```rust
pub enum ShareFormat {
    ClaudeCode,    // CLAUDE.md 風味的 markdown，含 `@file` 引入
    Codex,         // transcript JSON（每行一則訊息，OpenAI 風格）
    Gemini,        // 純 markdown，含 code fence（程式碼圍欄）
    Antigravity,   // CLAUDE.md，但寫到 .antigravity/context.md
    Generic,       // 純 markdown — 今天 /export 產出的格式
    Agents,        // AGENTS.md 插入（`## Recent context` 區塊）
}

pub fn render(history: &[ChatMessage], format: ShareFormat) -> String { … }
```

`/export`（已上線）變成 `share::render` 搭配 `Generic` 的薄包裝。
新格式重用相同的資料。

### 功能 B — `delegate` 工具

註冊在 `subagent` 旁邊的一個新工具：

```toml
# ~/.spectyn-mesh/agents.toml — 頂層 [tools.delegate] 區塊
[tools.delegate]
default_context = true               # 自動納入當前 session 最後 6 個回合
default_timeout_secs = 120

[tools.delegate.targets.gemini]
cmd       = "gemini"                 # 或絕對路徑
args      = ["-"]                    # `-` = 從 stdin 讀取
context_via = "stdin_prepend"        # 我們如何輸送上下文

[tools.delegate.targets.codex]
cmd       = "codex"
args      = ["chat"]
context_via = "context_flag"
context_flag = "--context-file"

[tools.delegate.targets.claude]
cmd       = "claude"
args      = []
context_via = "stdin_prepend"

[tools.delegate.targets.antigravity]
cmd       = "/Applications/Antigravity.app/Contents/Resources/app/bin/antigravity"
mode      = "open_project"          # 不是 prompt；只是打開 project
share_to  = ".antigravity/context.md"  # 上下文被寫到哪裡
```

工具 schema（Claude Code / spectyn REPL（互動式命令列）所看到的）：

```json
{
  "name": "delegate",
  "description": "將當前任務委派給另一個 AI CLI（gemini、codex、claude、…）並回傳其回應。",
  "parameters": {
    "tool":    {"type": "string", "enum": ["gemini","codex","claude","antigravity","aider"]},
    "prompt":  {"type": "string"},
    "with_context": {"type": "boolean", "default": true},
    "context_session": {"type": "string", "description": "覆寫要輸送哪個 session；預設 = 當前"}
  }
}
```

實作（`core/src/tools/delegate.rs`）：

```rust
pub async fn delegate(args: &Value, cfg: &ToolsConfig) -> String {
    let tool   = args.get("tool")?.as_str()?;
    let prompt = args.get("prompt")?.as_str()?;
    let target = cfg.delegate.targets.get(tool)?;

    // 1. 取得上下文（若有要求）
    let context_md = if args.get("with_context").as_bool().unwrap_or(true) {
        let sid = args.get("context_session").as_str().unwrap_or(&CURRENT_SESSION);
        let history = ConversationStore::default().get_history(sid).await;
        share::render(&history, share::format_for(tool))
    } else {
        String::new()
    };

    // 2. 衍生子行程
    let mut cmd = std::process::Command::new(&target.cmd);
    cmd.args(&target.args);
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());

    // 3. 依模式輸送上下文
    match target.context_via.as_str() {
        "stdin_prepend" => {
            let mut child = cmd.spawn()?;
            let stdin = child.stdin.take()?;
            stdin.write_all(format!("{}\n\n---\n\n{}\n", context_md, prompt).as_bytes())?;
            // 擷取 stdout，回傳
        }
        "context_flag" => {
            let tmp = write_to_tempfile(&context_md)?;
            cmd.args([&target.context_flag, &tmp.path().to_str().unwrap()]);
            cmd.stdin(prompt.as_bytes());
            // 擷取
        }
        "open_project" => {
            // Antigravity：寫上下文檔案、以 detached（分離）方式衍生 IDE
            let project_root = workspace::root()?;
            std::fs::write(project_root.join(&target.share_to), context_md)?;
            cmd.args(&[project_root.to_str().unwrap()]);
            cmd.spawn()?.wait();   // 使用者接手，沒有 stdout 可回傳
            return format!("Opened {} in {}; context dropped at {}", project_root.display(), tool, target.share_to);
        }
        _ => unreachable!()
    }
}
```

### 功能 C — 把其他工具的狀態讀進 spectyn

這是反向操作：spectyn 從 Claude Code 中斷的地方接續。

```
spectyn --import claude-code        # 讀取 ~/.claude/sessions/<latest>.jsonl
spectyn --import codex              # 讀取 codex transcript
spectyn --import-file <path>        # 明確指定
```

把每種格式轉換成 spectyn 的 `ChatMessage` 形狀，附加到一個新的 session，
當作接續執行。

優先度低於 A+B，但只要 share 轉換器存在，加上去很簡單（只要把它們反向跑）。

## 已經能用的功能（使用者可能還不知道）

- `/copy all` 與 `/export` 已經能產出中性的 markdown（不綁定任何工具的格式）。
  使用者可以貼進任何聊天，該工具就能讀。所以「分享給另一個工具」的
  **手動流程**已經只差一個 slash 指令 + ⌘V。
- `AGENTS.md` 已經會自動載入。如果 spectyn 在下次執行時寫入 AGENTS.md
  （或任何其他尊重 AGENTS.md 的工具），狀態就會往後延續。
- spectyn 的 MCP server 讓 Claude Code / Codex 等工具能呼叫 spectyn。
  所以功能 B 的反向操作已經做好了。

## 棘手之處

### Auth（認證）的拉扯

每個被委派的工具都有自己的 auth：
- `gemini` — `~/.config/google-cloud-sdk` 或 `GOOGLE_APPLICATION_CREDENTIALS`
- `codex` — `OPENAI_API_KEY`
- `claude` — `ANTHROPIC_API_KEY`
- `aider` — env vars（環境變數）或 `~/.aider/config`
- `antigravity` — 它自己的登入

spectyn 必須「不」覆蓋這些設定，也不能在工具之間外洩它們。每個 delegate
target（委派目標）會繼承父行程的 env（這沒問題 — 使用者的 shell 已經設好了）。

### 跨工具的成本記帳

今天 spectyn 的 CostTracker 只追蹤 spectyn 自己發出的 API 呼叫。
當我們委派給 `gemini` 時，gemini 的成本對我們是不可見的。v0.2 可以接受；
v0.3 或許會加上 `delegate_cost_usd` 欄位，從目標的最終回應裡解析出來
（大多數 CLI 會印出 "tokens used: N"）。

### Antigravity 特有問題

Antigravity 1.107.0 的 CLI 是 VS-Code-shell 形狀：
```
antigravity [paths...]                # 開啟檔案/資料夾
-d --diff <file> <file>               # diff（差異比對）
-m --merge ...                        # 3-way merge（三方合併）
-g --goto <file:line>                 # 在指定位置開啟
-w --wait                             # 阻塞直到檔案關閉
```

沒有 `--prompt`，也沒有 headless agent 呼叫。所以 `delegate(tool="antigravity")`
只能做 `mode = "open_project"`：把上下文寫到一個 Antigravity 會讀的檔案
（例如 `.antigravity/context.md` 或 AGENTS.md），打開 project，讓使用者
互動式地繼續。

如果 Antigravity 之後推出 `--ask` 或 `--prompt` 旗標，只要更新 toml target
就完成了。

### 標準生態現況

- **MCP**（Anthropic）：工具協定，「不是」上下文同步。我們已經是
  MCP server。
- **AGENTS.md**：一種慣例，多個工具都會讀它。算是合理的最小公分母。
- **OpenAI Agents API**：不是 CLI 標準。
- **所有常駐 IDE 的工具**（Cline、Continue、Antigravity）都沒有
  文件化的 import/export schema。我們會隨它們演進而調整。

## 交付計畫（在 5/15 OSS 發佈之後）

**Day 1** — `core/src/share.rs` 含 `ShareFormat` + 5 個轉換器。
            `/export --format` 旗標（目前 `/export` 預設為 Generic）。
            `/share <format>` slash 指令，寫到 `~/.spectyn-mesh/share/`。
**Day 2** — `core/src/tools/delegate.rs` + agents.toml schema。
            接好 3 個 backend（後端）：gemini、codex、claude。
**Day 3** — Auth-passthrough（認證透傳）驗證 + 每個 target 的 stderr 處理
            （擷取但不污染 delegate 回應的 stdout）。
            Antigravity 的 `open_project` 模式。
**Day 4** — `spectyn --import <tool>` 反向操作。
**Day 5** — 文件 + 4 個 demo（示範）腳本：
            - 「3 個工具做 Code review」（spectyn → 委派給 gemini → 委派給 codex → 綜合）
            - 「在 Antigravity 繼續這個」（spectyn /share antigravity → 切換到 GUI（圖形介面））
            - 「在 spectyn 接續 Claude Code session」（`spectyn --import claude-code`）
            - 成本：每個工具各增加了多少。

**Total（總計）**：發佈後 4-5 天的專注工作。沒有任何一項是高風險。

## 風險

- **CLI 旗標穩定性**：如果 `gemini` 2.0 改了旗標名稱，我們就調整
  toml target。不需改程式碼。
- **Auth 意外**：被委派的工具認證失敗 → spectyn 擷取 stderr，
  呈現給使用者並提示設定 `ANTHROPIC_API_KEY` 等等。
- **路徑敏感性**：工具透過 PATH 解析 `cmd`；如果使用者有多個版本，
  以預設的 `which` 順序為準。和在 shell 裡跑 `which gemini` 相同。

## 這項功能帶來什麼（使用者層面）

```
# Spectyn REPL
> 我寫好的 PR diff 在 ~/work/repo, 請 gemini 看看，再給 claude review,
> 兩家結果合在一起寫 review 留言
```

Spectyn 內部會呼叫：
1. `delegate(tool=gemini, prompt="review the diff at ...", with_context=true)`
2. `delegate(tool=claude, prompt="review the same diff", with_context=true)`
3. （master agent，主代理）綜合這兩個回應

或者：

```
> 我這個 session 切過去 antigravity 繼續寫
> /share antigravity
> # spectyn 寫 .antigravity/context.md 並 spawn antigravity 開 project
```

使用者切換工具，不會遺失狀態。

## 待決定事項

**我們要在 v0.1.0（5/1）還是 v0.2（5/15+）出貨這個功能？**

建議：v0.2。理由：
1. v0.1.0 的功能面已經很大（mesh、MCP、evolve、snapshots）。別再
   往上堆。
2. 分享的「手動」變通做法（`/export` + 貼上）已經夠用。
3. 跨工具 delegate 是一個 **growth（成長型）** 功能，不是核心。發佈後
   以「現在 spectyn 能編排你其餘的 AI 技術棧」來推銷，比把它當成
   「spectyn 是什麼？」的一部分來出貨，要容易得多。

但 v0.2 的建置與 broker（中介伺服器）/ billing（計費）/ Apple Sign In 無關，
所以它可以與 SaaS（軟體即服務）路線平行進行。
