# Phantom Mesh Configuration

Phantom Mesh loads configuration in three layers, each overriding the previous:

1. **Built-in defaults** -- hardcoded in `src/phantom-mesh/config.py`
2. **Config file** -- `~/.phantom-mesh/config.json` (optional, silently ignored if missing or malformed)
3. **Environment variables** -- `PPI_*` prefixed vars; always win

Access anywhere via the singleton:

```python
from config import config
print(config.ollama_url)       # str
print(config.cluster_nodes)    # list[dict]
print(config.to_dict())        # full dump
```

---

## Config File

**Location:** `~/.phantom-mesh/config.json`

Only keys that match a known default are loaded; unknown keys are ignored.
Values are stored as strings internally and cast by typed properties.

### Example with all options

```json
{
  "ollama_url":           "http://localhost:11434",
  "default_model":        "gemma3:12b",
  "coding_model":         "qwen2.5-coder:32b",
  "reasoning_model":      "qwen2.5:32b",
  "embed_model":          "nomic-embed-text",
  "db_path":              "C:/Users/you/.phantom-mesh/knowledge.db",
  "litellm_proxy_url":    "http://localhost:4000",
  "cluster_nodes":        "node-c=http://100.64.0.2:11434,node-b=http://100.64.0.3:11434",
  "similarity_threshold": "0.8",
  "max_parallel_tasks":   "4"
}
```

---

## Environment Variables

| Variable | Config key | Default | Description |
|---|---|---|---|
| `CLAWTEX_OLLAMA_URL` | `ollama_url` | `http://localhost:11434` | Base URL for the local Ollama instance |
| `CLAWTEX_DEFAULT_MODEL` | `default_model` | `gemma3:12b` | General-purpose chat model |
| `CLAWTEX_CODING_MODEL` | `coding_model` | `qwen2.5-coder:32b` | Model routed for code tasks |
| `CLAWTEX_REASONING_MODEL` | `reasoning_model` | `qwen2.5:32b` | Model routed for reasoning tasks |
| `CLAWTEX_EMBED_MODEL` | `embed_model` | `nomic-embed-text` | Embedding model for knowledge DB |
| `CLAWTEX_DB_PATH` | `db_path` | `~/.phantom-mesh/knowledge.db` | SQLite knowledge-base path |
| `PPI_LITELLM_URL` | `litellm_proxy_url` | *(empty)* | LiteLLM proxy URL; `None` when empty |
| `CLAWTEX_CLUSTER_NODES` | `cluster_nodes` | *(empty)* | Cluster node list (see below) |
| `CLAWTEX_SIMILARITY_THRESHOLD` | `similarity_threshold` | `0.8` | Cosine similarity cutoff for retrieval |
| `CLAWTEX_MAX_PARALLEL_TASKS` | `max_parallel_tasks` | `4` | Concurrent task limit, clamped to 1-32 |

---

## Cluster Node Configuration

`cluster_nodes` accepts two formats:

**Simple (comma-separated, good for env vars):**

```
name=url,name=url
```

```bash
export CLAWTEX_CLUSTER_NODES="node-c=http://100.64.0.2:11434,node-b=http://100.64.0.3:11434"
```

If you omit the name, the hostname is extracted automatically:

```
http://100.64.0.2:11434,http://100.64.0.3:11434
```

**JSON (more control, better for config file):**

```json
{
  "cluster_nodes": "[{\"name\":\"node-c\",\"url\":\"http://100.64.0.2:11434\"},{\"name\":\"node-b\",\"url\":\"http://100.64.0.3:11434\"}]"
}
```

When no nodes are configured, Phantom Mesh runs in single-machine mode using only the local Ollama instance.

---

## Model Selection

Phantom Mesh exposes three model slots, each targeting a different workload:

| Slot | Property | Default | Use case |
|---|---|---|---|
| General | `config.default_model` | `gemma3:12b` | Chat, summarization, general Q&A |
| Coding | `config.coding_model` | `qwen2.5-coder:32b` | Code generation, review, debugging |
| Reasoning | `config.reasoning_model` | `qwen2.5:32b` | Multi-step logic, planning, analysis |

The embedding model (`config.embed_model`) is used exclusively by the knowledge DB for vector search.

### Derived URLs

The config object also provides computed URLs you do not set directly:

- `config.ollama_chat_url` -- `{ollama_url}/api/chat`
- `config.ollama_embed_url` -- `{ollama_url}/api/embed`

---

## Quick Override Examples

Override just the model for one session:

```bash
CLAWTEX_DEFAULT_MODEL=llama3:8b python -m phantom-mesh
```

Point at a remote Ollama over Tailscale:

```bash
CLAWTEX_OLLAMA_URL=http://100.64.0.2:11434 python -m phantom-mesh
```

Dump the resolved config:

```python
from config import config
import json
print(json.dumps(config.to_dict(), indent=2))
```

## 中文對照

這份文件在說明 Phantom Mesh 的設定載入順序。系統會依序讀取三層設定，而且後面的會覆蓋前面的：第一層是程式內建預設值，第二層是 `~/.phantom-mesh/config.json`，第三層是環境變數；其中環境變數優先權最高。

設定檔只會讀取系統已知的 key，未知欄位會被忽略。檔案裡的值先以字串保存，之後再由對應的 typed property 轉成正確型別。文件中的 JSON 範例把所有常用選項都列出來了，例如 Ollama URL、預設模型、coding/reasoning 模型、embedding 模型、資料庫位置、LiteLLM proxy，以及 cluster node 清單。

環境變數表格是在對照「變數名稱」「設定 key」「預設值」和「用途」。重點是：
- `CLAWTEX_OLLAMA_URL` 控制 Ollama 主機位址。
- `CLAWTEX_DEFAULT_MODEL`、`CLAWTEX_CODING_MODEL`、`CLAWTEX_REASONING_MODEL` 分別控制通用、寫程式、推理任務的模型。
- `CLAWTEX_EMBED_MODEL` 控制向量化模型。
- `CLAWTEX_CLUSTER_NODES` 用來宣告多台節點。
- `CLAWTEX_SIMILARITY_THRESHOLD` 與 `CLAWTEX_MAX_PARALLEL_TASKS` 分別控制檢索門檻與並行任務數。

`cluster_nodes` 支援兩種格式。環境變數比較適合用逗號分隔的 `name=url,name=url`，設定檔則比較適合用 JSON 陣列字串來描述每個節點的 `name` 和 `url`。如果完全不設定節點，Phantom Mesh 就會退回單機模式，只使用本地 Ollama。

模型選擇部分把模型分成三個槽位：General、Coding、Reasoning。一般聊天與摘要會走 `default_model`，程式生成與除錯走 `coding_model`，多步驟推理與規劃走 `reasoning_model`。而 `embed_model` 只提供知識庫做向量搜尋，不會拿來聊天。

最後一段示範幾個快速覆寫方式：你可以只在單次命令前加上環境變數，臨時改模型或改 Ollama 位址；也可以用 `config.to_dict()` 把最終生效的設定完整印出來，方便確認實際載入結果。
