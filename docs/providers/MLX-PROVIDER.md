# 在 Apple Silicon 上執行 MLX 本地 LLM

透過 Apple 的 MLX 框架在裝置端執行大型語言模型（large language model，LLM）。零
每個 token（權杖）的 API 成本，模型下載完成後即可完全離線，速度
足以應付 M1/M2/M3 Mac 上的 autoevolve（自動演化）/ subagent（子代理）流程。

spectyn 本身不會綑綁 MLX，也不會下載模型——它只負責協調
`mlx_lm.server`（與 OpenAI 相容）以及 `huggingface-cli`。

---

## 安裝（一次即可）

```bash
pip3 install mlx-lm
```

（或使用 `uv tool install mlx-lm` / `pipx install mlx-lm`。任何
能把 `mlx_lm` 放上你預設 `python3` import 路徑的方法都可以。）

`spectyn doctor` 接著會顯示：

```
MLX local LLM
  ✓ mlx_lm: importable (`pip install mlx-lm` available)
```

---

## 拉取模型

```bash
spectyn mlx pull                         # default: Llama 3.1 8B 4-bit (~5 GB)
spectyn mlx pull mlx-community/Llama-3.3-70B-Instruct-4bit   # ~38 GB, 32+ GB RAM
spectyn mlx pull mlx-community/Qwen2.5-Coder-7B-Instruct-4bit  # ~4 GB, code-focused
```

預設的 `Llama-3.1-8B-Instruct-4bit` 在 16 GB 的 M1 上可順暢執行。
70B 變體需要 32+ GB 的 Mac 才能避免發生記憶體置換（swapping）。

---

## 提供服務（Serve）

```bash
spectyn mlx serve                # foreground, default model + port 8080
spectyn mlx serve --port 9090    # custom port
spectyn mlx serve --model mlx-community/Llama-3.3-70B-Instruct-4bit
```

`mlx_lm.server` 會在
`http://127.0.0.1:<port>/v1` 暴露一個與 OpenAI 相容的端點（endpoint）。它只監聽 127.0.0.1——將其
暴露給叢集（cluster）屬於後續工作（請使用具 Tailscale 感知的反向代理（reverse proxy），或在
你信任自己的 tailnet（Tailscale 網路）時使用 `mlx_lm.server --host 0.0.0.0`）。

服務執行期間，doctor 的 MLX 區段會轉為綠色：

```
MLX local LLM
  ✓ mlx_lm: importable
  ✓ server: mlx-community/Llama-3.1-8B-Instruct-4bit on :8080 reachable
```

---

## 接入 agents.toml

附加到 `~/.spectyn-mesh/agents.toml`：

```toml
[providers.mlx-local]
type          = "openai"
base_url      = "http://127.0.0.1:8080/v1"
api_key       = "mlx"
default_model = "mlx-community/Llama-3.1-8B-Instruct-4bit"

[agent.local]
provider     = "mlx-local"
model        = "mlx-community/Llama-3.1-8B-Instruct-4bit"
instructions = "You are spectyn-mesh's on-device agent running via MLX. Use tools when needed; respond concisely."
tools        = ["shell", "file_read", "ls", "content_search"]
```

從任何地方使用它：

```bash
# CLI
spectyn evolve --agent local "fix the failing tests"

# MCP (Claude Code / Codex)
mcp__spectyn__subagent({ agent: "local", prompt: "..." })

# Web mobile UI agent dropdown will list 'local' alongside master/coder.
```

`spectyn autoevolve --agent local` 會將每小時的自我改善
迴圈完全在裝置端執行——在下一次成本定價更新後，來自 `local` 代理的 autoevolve.log 條目
會顯示 `cost: $0.000`。

---

## 效能說明（M1 16 GB 基準）

| 模型 | 大小 | 冷啟動載入 | 暖啟動生成 | 備註 |
|---|---|---|---|---|
| Llama-3.1-8B-Instruct-4bit | 4.2 GB | ~150 s | 50 個 token 約 ~5-15 s | 預設——快速、品質尚可 |
| Llama-3.3-70B-Instruct-4bit | ~38 GB | 在 16 GB 上嚴重置換 | n/a | 請等 32+ GB RAM |
| Qwen2.5-Coder-7B-Instruct-4bit | 3.8 GB | ~120 s | ~5 s | 在工具 schema（結構描述）上表現較佳 |

8B 4-bit 的工具呼叫（tool-calling）品質時好時壞——這些小模型
有時會幻覺出（hallucinate）spectyn 的工具 schema。對於工具忠實度
攸關重要的 evolve / autoevolve，建議優先使用 **Qwen2.5-Coder-7B**，或退回
使用付費的 Groq/Anthropic 供應商。對於純聊天流程，8B Llama
已經夠用。

---

## 停止 / 重啟

```bash
spectyn mlx stop                         # pkill -f mlx_lm.server
spectyn mlx status                       # is it up?
spectyn mlx serve --model Qwen2.5-Coder…  # swap model
```

serve 指令是前景（foreground）執行的。若要將它作為背景常駐服務（daemon）執行，
請以 launchd 包裝（類似 `spectyn service install` 的 LaunchAgent
範本）——目前尚未以一行指令（one-liner）形式提供。

---

## 為何這很重要

- **零成本**——適用於 autoevolve 每小時的修復嘗試
- **完全離線**——模型下載後即可離線，spectyn Mac 在飛機上、火車上、
  或在 SCIF（敏感隔離資訊設施）中都能持續修復並提交
- **延遲（Latency）**——對於不需要 Sonnet 級推理的提示（prompt），Apple Silicon 的
  GPU（或 M3+ 上的 NPU）比往返 api.anthropic.com 的網路來回更快
- **隱私**——沒有任何 token、提示或回應會離開這台機器
- **沒有任何競品 CLI 代理具備此能力**——OpenCode / Codex CLI / Gemini
  CLI 全都假設要與供應商雲端進行一次 HTTPS 來回
