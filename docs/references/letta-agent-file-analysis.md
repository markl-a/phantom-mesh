# Letta Agent File (.af) 深度技術分析

> 分析對象: `LLM-Cluster-Project/references/letta-agent-file/`
> 分析日期: 2026-03-12
> 目的: 從開發者視角深入解析 .af 格式規格、架構模式，並評估 clawtex 專案可採用之設計

---

## 1. 專案結構

```
letta-agent-file/
├── agents/                          # 已發布的 Agent 目錄
│   ├── @cpfiffer/                   # 社群貢獻者
│   │   ├── co-3/                    # Agent: co-3
│   │   │   ├── co-3.af             # Agent File (JSON)
│   │   │   ├── co-3.webp           # 頭像
│   │   │   └── README.md
│   │   ├── grunk/                   # Agent: grunk (Bluesky 機器人)
│   │   └── void/                    # Agent: void
│   ├── @letta-ai/                   # 官方 Agent
│   │   ├── evie/                    # Discord 社群助手
│   │   ├── ezra/                    # 另一官方 Agent
│   │   ├── lettabot/                # 多頻道通訊 Agent
│   │   ├── lettabot-builder/        # Agent 建構器
│   │   └── loop/                    # 記憶型對話 Agent (旗艦)
│   ├── CONTRIBUTING.md              # 貢獻指南
│   └── README.md                    # 目錄說明
├── customer_service_agent/          # 範例: 客服 Agent
│   ├── customer_service.af
│   ├── customer_service_agent.py    # 建立腳本
│   └── README.md
├── deep_research_agent/             # 範例: 深度研究 Agent
│   ├── deep_research_agent.af
│   ├── deep_research_agent.py       # 建立腳本 (含 MCP 整合)
│   ├── analyze_and_search.py
│   ├── research_tools.py
│   └── example_report.md
├── memgpt_agent/                    # 範例: MemGPT 經典 Agent
│   ├── memgpt_agent.af
│   ├── memgpt_agent.py
│   ├── memgpt_agent_with_convo.af   # 含對話歷史版本
│   └── memgpt_agent_with_convo.py
├── workflow_agent/                   # 範例: 工作流 Agent
│   ├── outreach_workflow_agent.af
│   ├── workflow_agent.py            # 含 tool_rules 工作流定義
│   └── README.md
├── src/                             # Next.js 展示前端
│   ├── app/                         # App Router 頁面
│   │   ├── agents/[ownerId]/[agentKey]/page.tsx  # Agent 詳情頁
│   │   └── (main)/page.tsx          # 首頁
│   ├── lib/
│   │   ├── agents-loader.ts         # .af 檔案解析器 (核心)
│   │   └── client/components/       # UI 元件
│   └── components/                  # 共用元件
├── .skills/                         # Letta Skills 系統
│   └── releasing-agentfiles/        # 發布 Agent 的技能定義
│       ├── SKILL.md
│       └── scripts/process-avatar.sh
├── specs/index.spec.tsx             # 測試
├── index.d.ts                       # TypeScript 宣告 (含 *.af 模組宣告)
├── package.json                     # Next.js 16 + React 19
├── CITATION.cff                     # 學術引用格式
└── LICENSE                          # Apache-2.0
```

**關鍵觀察**: 此專案有雙重角色 -- 既是 `.af` 格式的規格定義與範例倉庫，也是一個 Next.js 靜態網站用於展示/瀏覽已發布的 Agent。前端部分並非核心 -- 真正的序列化/反序列化邏輯位於 Letta Server 端 (`letta-ai/letta` 倉庫的 `letta/serialize_schemas/pydantic_agent_schema.py`)。

---

## 2. 檔案格式規格

### 2.1 格式本質

`.af` 檔案是 **純 JSON** 格式。沒有使用 YAML、Binary、Protocol Buffers 或任何壓縮格式。

確認來源 -- TypeScript 模組宣告:

```typescript
// index.d.ts (第 15-18 行)
declare module '*.af' {
  const value: any;
  export default value;
}
```

解析器直接使用 `JSON.parse()`:

```typescript
// src/lib/agents-loader.ts (第 154-156 行)
const fileContent = fs.readFileSync(agentFilePath, 'utf-8');
const agentData = JSON.parse(fileContent);
```

### 2.2 頂層 Schema 結構

根據所有 .af 範例的交叉分析，頂層 JSON 結構如下:

```json
{
  "agents": [ ... ],        // Agent 定義陣列 (通常 1 個，可多個)
  "groups": [ ... ],        // Agent 群組定義 (多 Agent 協作)
  "blocks": [ ... ],        // 記憶區塊 (Memory Blocks) 定義
  "files": [ ... ],         // 關聯檔案
  "sources": [ ... ],       // 資料來源 (Archival Memory 來源)
  "tools": [ ... ],         // 工具定義 (含程式碼 + JSON Schema)
  "mcp_servers": [ ... ],   // MCP 伺服器配置 (路線圖中)
  "metadata": {             // 檔案層級中繼資料
    "revision_id": "..."    // 修訂版本 ID
  },
  "created_at": "..."       // ISO 8601 建立時間戳
}
```

### 2.3 Agent 物件完整欄位

每個 `agents[]` 元素包含以下欄位 (從 `loop.af`, `lettabot.af`, `grunk.af`, `evie.af` 交叉比對):

```json
{
  // === 身份識別 ===
  "id": "agent-0",                    // 檔案內部參照 ID
  "name": "Loop",                     // Agent 顯示名稱
  "description": "I'm Loop...",       // Agent 描述
  "agent_type": "letta_v1_agent",     // Agent 類型標識
  "tags": ["origin:letta-chat"],      // 標籤 (來源、視圖類型)

  // === 系統提示 ===
  "system": "You are Loop...",        // 完整 System Prompt (可極長)

  // === 記憶系統 ===
  "memory_blocks": [],                // 內聯記憶區塊 (已棄用，改用 block_ids)
  "block_ids": ["block-0", ...],      // 參照 blocks[] 陣列的 ID
  "memory_variables": null,           // 記憶變數

  // === 工具系統 ===
  "tools": [],                        // 內聯工具定義 (已棄用，改用 tool_ids)
  "tool_ids": ["tool-0", ...],        // 參照 tools[] 陣列的 ID
  "tool_rules": [ ... ],              // 工具執行規則 (工作流定義核心)
  "tool_exec_environment_variables": {}, // 工具執行環境變數
  "secrets": null,                    // 密鑰 (匯出時設為 null)

  // === LLM 配置 ===
  "llm_config": {
    "model": "claude-sonnet-4-5-20250929",
    "model_endpoint_type": "anthropic",
    "model_endpoint": "https://api.anthropic.com/v1",
    "provider_name": "anthropic",
    "provider_category": "base",
    "context_window": 90000,
    "temperature": 1.0,
    "max_tokens": 16384,
    "enable_reasoner": true,
    "max_reasoning_tokens": 1024,
    "parallel_tool_calls": true,
    "handle": "anthropic/claude-sonnet-4-5-20250929",
    "put_inner_thoughts_in_kwargs": false,
    "model_wrapper": null,
    "frequency_penalty": null,
    "response_format": null,
    "strict": false
  },

  // === Embedding 配置 ===
  "embedding_config": {
    "embedding_endpoint_type": "openai",
    "embedding_endpoint": "https://api.openai.com/v1",
    "embedding_model": "text-embedding-3-small",
    "embedding_dim": 1536,
    "embedding_chunk_size": 300,
    "handle": "openai/text-embedding-3-small",
    "batch_size": 32
  },

  // === 訊息歷史 ===
  "in_context_message_ids": ["message-0", ...],  // 當前上下文窗口中的訊息
  "messages": [ ... ],                            // 完整訊息歷史

  // === 行為控制 ===
  "include_base_tools": false,
  "include_multi_agent_tools": false,
  "include_base_tool_rules": false,
  "include_default_source": false,
  "initial_message_sequence": null,
  "message_buffer_autoclear": false,  // 工作流模式: true = 每次請求獨立
  "enable_sleeptime": false,          // 睡眠時間記憶整理
  "parallel_tool_calls": null,

  // === 檔案系統 ===
  "max_files_open": 10,
  "per_file_view_window_char_limit": 25000,
  "files_agents": [],

  // === 模板系統 ===
  "from_template": null,
  "template": false,
  "template_id": null,
  "base_template_id": null,

  // === 其他 ===
  "source_ids": [],
  "folder_ids": null,
  "group_ids": [],
  "identity_ids": null,
  "project_id": null,
  "timezone": "UTC",
  "hidden": null,
  "metadata": null
}
```

### 2.4 關鍵差異: Agent 間的配置多樣性

| Agent | 模型 | Context Window | 記憶區塊數 | 工具數 | 特殊功能 |
|-------|------|---------------|-----------|--------|----------|
| Loop | claude-sonnet-4-5 | 90,000 | 9 | 9 | 深度人格系統 |
| LettaBot | glm-5 (ZAI) | 200,000 | 11 | 2 | 多頻道通訊 |
| Grunk | gpt-5-mini | 40,000 | 9 | 22 | Bluesky 整合 |
| Evie | claude-opus-4-5 | 150,000 | 12 | 17 | Discord + 多 Agent |
| Customer Service | gpt-4o-mini | 32,000 | 2 | 7 | 客服工作流 |
| Workflow Agent | gpt-4o-mini | 32,000 | 0 | 4 | 工具鏈規則 |

---

## 3. 核心架構

### 3.1 序列化: Agent 到 .af 檔案

序列化透過 Letta SDK/Server 完成，不在本倉庫中實作。建立流程:

```python
# customer_service_agent/customer_service_agent.py (第 111-114 行)
import json
with open("customer_service.af", "w") as f:
    json.dump(client.agents.export_file(agent_id=agent.id), f, indent=2)
```

SDK 端序列化流程:

```python
# Python SDK
schema = client.agents.export_file(agent_id="<AGENT_ID>")

# TypeScript SDK
const schema = await client.agents.exportFile("<AGENT_ID>");
```

REST API:
```
GET /v1/agents/{AGENT_ID}/export
```

**序列化核心邏輯** 位於 Letta Server 的 `letta/serialize_schemas/pydantic_agent_schema.py`，採用 Pydantic 模型定義。匯出過程中:
- 所有內部 UUID 被重新映射為 `agent-0`, `block-0`, `tool-0` 等序列 ID
- 密鑰 (secrets) 被設為 `null`
- 訊息歷史保留 `in_context` 標記以指示哪些訊息在上下文窗口中

### 3.2 反序列化: .af 檔案載入為執行中的 Agent

```python
# Python SDK
agent_state = client.agents.import_file(file=open("/path/to/file.af", "rb"))

# TypeScript SDK
const agentState = await client.agents.importFile(file, {});
```

REST API:
```
POST /v1/agents/import -F "file=/path/to/agent/file.af"
```

匯入時 Server 會:
1. 解析 JSON，驗證 Schema
2. 為所有物件生成新的 UUID (取代 `agent-0` 等佔位符)
3. 重建 ID 交叉參照 (`block_ids`, `tool_ids`, `in_context_message_ids`)
4. 建立工具 (含 source_code 和 json_schema)
5. 建立記憶區塊
6. 恢復訊息歷史
7. 重建 tool_rules 工作流

### 3.3 記憶區塊 (Memory Blocks) 設計

記憶區塊是 .af 格式的核心創新。每個區塊定義如下:

```json
// blocks[] 陣列中的元素
{
  "id": "block-5",
  "label": "persona",                          // 區塊標籤 (唯一識別符)
  "value": "I'm Loop. I persist...",           // 區塊內容 (純文字)
  "limit": 20000,                              // 字元上限
  "read_only": false,                          // 是否唯讀
  "description": "The persona block...",        // 區塊用途描述
  "metadata": {},                              // 自訂中繼資料
  "hidden": null,                              // 是否隱藏
  "tags": null,                                // 分類標籤
  "is_template": false,                        // 是否為模板
  "template_id": null,
  "preserve_on_migration": false               // 遷移時是否保留
}
```

#### 記憶區塊在 System Prompt 中的注入方式

記憶區塊被注入到系統提示的 `<memory_blocks>` XML 標籤中，形成 Agent 的「可編輯核心記憶」:

```xml
<memory_blocks>
The following memory blocks are currently engaged in your core memory unit:

<about_user>
<description>...</description>
<metadata>
- chars_current=241
- chars_limit=20000
</metadata>
<value>
Facts and context about the user.
Name: [unknown]
Role/Work: [unknown]
...
</value>
</about_user>

<persona>
<description>
The persona block: Stores details about your current persona...
</description>
<metadata>
- chars_current=406
- chars_limit=20000
</metadata>
<value>
I'm Loop. I persist.
...
</value>
</persona>
</memory_blocks>
```

#### 常見記憶區塊模式

**Loop Agent (9 個區塊)** -- 最精緻的範例:

| 標籤 | 用途 | 字元上限 |
|------|------|---------|
| `about_user` | 使用者資訊 (姓名、工作、偏好) | 20,000 |
| `active_hypotheses` | 進行中的觀察假設 | 20,000 |
| `conversation_patterns` | 對話模式追蹤 | 20,000 |
| `custom_instructions` | 使用者設定的明確規則 | 20,000 |
| `learned_corrections` | 錯誤修正記錄 | 20,000 |
| `persona` | Agent 人格定義 | 20,000 |
| `preferences` | 使用者互動偏好 | 20,000 |
| `scratchpad` | 工作記憶/暫存區 | 20,000 |
| `soul` | Agent 存在意義 | 20,000 |

**LettaBot Agent (11 個區塊)** -- 多分類人類資訊:

| 標籤 | 用途 |
|------|------|
| `system/human/family` | 家庭成員、寵物 |
| `system/human/interests` | 興趣愛好 |
| `system/human/overview` | 基本資訊 |
| `system/human/personality` | 個性特質 |
| `system/human/preferences` | 喜好偏好 |
| `system/human/routines` | 日常作息 |
| `system/human/work` | 工作資訊 |
| `system/persona/expression` | 表達風格 |
| `system/persona/interests` | Agent 興趣 |
| `system/persona/learned_behaviors` | 學習到的行為模式 |
| `system/persona/soul` | Agent 靈魂/核心信念 |

**Evie Agent (12 個區塊)** -- 社群管理導向:

| 標籤 | 用途 |
|------|------|
| `persona` | 人格定義 (含 Discord 整合規則) |
| `guardrails` | 安全護欄 (唯讀) |
| `likeability_system` | 使用者好感度系統 (0-100) |
| `social_scores` | 社交評分系統 |
| `title_registry` | 頭銜註冊表 |
| `community_authority_figures` | 管理員清單 |
| `discord_message_formats` | Discord 訊息格式 |
| `memory_editing_rules` | 記憶編輯規則 (唯讀) |
| `persona_rules` | 人格規則 (唯讀) |
| `server_rules` | 伺服器規則 |
| `source_management` | 資料來源管理 |
| `about_me` | Agent 自我介紹 (唯讀) |

### 3.4 工具配置: 程式碼 + Schema

工具在 `tools[]` 陣列中定義，有兩種類型:

#### 核心工具 (letta_core / letta_sleeptime_core)

不包含 source_code，由 Letta Server 內建提供:

```json
{
  "id": "tool-1",
  "tool_type": "letta_core",
  "name": "archival_memory_insert",
  "description": "Add information to long-term archival memory...",
  "source_type": "python",
  "source_code": null,         // 核心工具無需原始碼
  "json_schema": {
    "name": "archival_memory_insert",
    "description": "...",
    "parameters": {
      "type": "object",
      "properties": {
        "content": {
          "type": "string",
          "description": "The information to store."
        },
        "tags": {
          "type": "array",
          "items": { "type": "string" },
          "description": "Optional list of category tags"
        }
      },
      "required": ["content"]
    }
  },
  "return_char_limit": 50000,
  "enable_parallel_execution": false
}
```

常見核心工具:
- `archival_memory_insert` / `archival_memory_search` -- 長期記憶
- `conversation_search` -- 對話歷史搜尋
- `memory_replace` / `memory_insert` / `memory_rethink` -- 記憶區塊編輯
- `send_message` -- 發送訊息

#### 自訂工具 (custom)

包含完整的 Python 原始碼:

```json
{
  "id": "tool-5",
  "tool_type": "custom",
  "name": "cancel_order",
  "description": "Cancels an order.",
  "source_type": "python",
  "source_code": "def cancel_order(order_number: int, reason: str):\n    \"\"\"\n    Cancels an order.\n    ...\n    \"\"\"\n    dummy_message = f\"The order {order_number} could not be cancelled.\"\n    return dummy_message\n",
  "json_schema": {
    "name": "cancel_order",
    "parameters": {
      "type": "object",
      "properties": {
        "order_number": { "type": "integer", "description": "..." },
        "reason": { "type": "string", "description": "..." }
      },
      "required": ["order_number", "reason"]
    }
  },
  "tags": [],
  "return_char_limit": 6000,
  "pip_requirements": null,
  "npm_requirements": null,
  "default_requires_approval": null,
  "enable_parallel_execution": false
}
```

**重要設計**: 自訂工具將 Python source_code 直接嵌入 JSON，使 Agent 完全自包含。Letta Server 在匯入時可直接執行這些工具函式。

### 3.5 工具規則 (Tool Rules) -- 工作流引擎

`tool_rules` 是 .af 格式中最具架構價值的部分，定義工具間的執行流程:

```json
// workflow_agent/outreach_workflow_agent.af 中的 tool_rules
"tool_rules": [
  {
    "type": "run_first",              // 必須首先執行
    "tool_name": "retrieve_candidate"
  },
  {
    "type": "constrain_child_tools",  // 執行後限制可用工具
    "tool_name": "retrieve_candidate",
    "children": ["evaluate_candidate"]
  },
  {
    "type": "conditional",            // 根據輸出條件分支
    "tool_name": "evaluate_candidate",
    "child_output_mapping": {
      "True": "send_email",
      "False": "reject"
    },
    "default_child": "reject"
  },
  {
    "type": "exit_loop",              // 終止執行迴圈
    "tool_name": "reject"
  },
  {
    "type": "exit_loop",
    "tool_name": "send_email"
  }
]
```

支援的規則類型:

| 類型 | 說明 | 用途 |
|------|------|------|
| `run_first` | 強制首先執行 | 初始化步驟 |
| `constrain_child_tools` | 限制後續可用工具 | 線性工作流 |
| `conditional` | 根據工具輸出分支 | 決策節點 |
| `continue_loop` | 繼續執行迴圈 | 記憶更新後繼續 |
| `exit_loop` | 終止執行 | 完成/中止 |

另一個範例 -- Grunk 的規則:
```json
{
  "tool_name": "add_post_to_bluesky_reply_thread",
  "type": "constrain_child_tools",
  "children": ["archival_memory_insert"]  // 發文後必須存檔
}
```

---

## 4. Checkpointing (狀態檢查點)

### 4.1 訊息歷史作為檢查點

.af 檔案中的 `messages[]` 陣列保存完整的訊息歷史，而 `in_context_message_ids` 標記哪些訊息在當前上下文窗口中:

```json
{
  "in_context_message_ids": [
    "message-0",    // system prompt
    "message-50",   // 最近的互動
    "message-51",
    // ... 只有在上下文中的訊息
  ],
  "messages": [
    // 完整歷史 -- 可能數千條
    {
      "type": "message",
      "role": "system",
      "content": [{ "type": "text", "text": "..." }],
      "id": "message-0",
      "model": "claude-sonnet-4-5-20250929",
      "agent_id": "agent-0",
      "tool_calls": null,
      "tool_returns": [],
      "created_at": "2026-01-22T02:06:34.048456+00:00"
    },
    {
      "role": "assistant",
      "tool_calls": [{
        "id": "35823630-...",
        "function": {
          "name": "send_message",
          "arguments": "{\"message\": \"I'm Loop. I remember.\"}"
        }
      }]
    },
    {
      "role": "tool",
      "tool_call_id": "35823630-...",
      "tool_returns": [{
        "tool_call_id": "35823630-...",
        "status": "success",
        "func_response": "{\"status\": \"OK\"}"
      }]
    }
  ]
}
```

### 4.2 記憶區塊作為持久化狀態

記憶區塊的 `value` 欄位保存了 Agent 在多次對話中累積的知識。例如 Grunk 的 `messages` 包含 68 條訊息歷史 (message-0 到 message-67)，其記憶區塊也已被 Agent 自身修改過 (如 `favorites` 區塊記錄了 "Dima" 和 "Turtle")。

### 4.3 匯出即檢查點

每次 `export_file()` 都是一個完整的狀態快照。要恢復到特定時間點，只需保留該時間的 .af 檔案。

---

## 5. 版本控制

### 5.1 revision_id 欄位

```json
{
  "metadata": {
    "revision_id": "b1c2d3e4f5a6"   // 修訂版本 ID
  }
}
```

`metadata.revision_id` 欄位提供了一個輕量級的版本標識。然而，目前 .af 格式本身**不包含差異追蹤 (diff)**機制。

### 5.2 Git 作為版本控制

.af 是純 JSON 文字檔案，天然適合 Git 版本控制:

```
agents/@letta-ai/loop/loop.af   # Git 追蹤每次修改
```

由於是 JSON，`git diff` 可以顯示精確的欄位變更。

### 5.3 路線圖

根據 README.md 中的路線圖，未來計劃:
- `Migration support between schema changes` -- Schema 變更間的遷移支援
- `Multi-agent .af files` -- 多 Agent 檔案 (目前 agents[] 雖是陣列但通常只有 1 個 agent；Evie 是例外，包含 2 個 agent)

---

## 6. 跨框架相容性

### 6.1 現狀

根據 README.md:

> Theoretically, other frameworks could also load in .af files if they convert the state into their own representations. Some concepts, such as context window "blocks" which can be edited or shared between agents, are not implemented in other frameworks, so may need to be adapted per-framework.

目前 .af 主要為 Letta 框架設計。其他框架需要映射以下概念:

| .af 概念 | 映射難度 | 說明 |
|----------|---------|------|
| system prompt | 低 | 所有框架都支援 |
| llm_config | 低 | 模型名稱+端點，通用 |
| memory_blocks | 高 | Letta 特有概念，需轉換為 prompt injection |
| tool (custom) | 中 | Python source_code 可能需適配 |
| tool (core) | 高 | Letta 內建工具語義需重新實作 |
| tool_rules | 高 | 工作流引擎為 Letta 特有 |
| messages | 中 | 需轉換訊息格式 |
| archival memory | 高 | 需向量資料庫支援 |

### 6.2 路線圖

```
- [ ] Converters between frameworks
```

這意味著官方計劃提供框架轉換器，但目前尚未實作。

---

## 7. Letta 平台整合

### 7.1 ADE (Agent Development Environment)

ADE 是 Letta 的 Web IDE，位於 `app.letta.com`:

**匯入**:
1. 下載 `.af` 檔案
2. 開啟 ADE
3. 點擊 "Import Agent" 並選擇檔案
4. Server 端反序列化建立 Agent

**匯出**:
1. 在 ADE 中點擊 "Export Agent"
2. Server 端序列化產生 .af
3. 下載 JSON 檔案

### 7.2 SDK 整合

Python SDK:
```python
from letta_client import Letta

client = Letta(base_url="http://localhost:8283")

# 匯入
agent_state = client.agents.import_file(file=open("agent.af", "rb"))

# 匯出
schema = client.agents.export_file(agent_id="<AGENT_ID>")
```

TypeScript SDK:
```typescript
import { LettaClient } from '@letta-ai/letta-client';

const client = new LettaClient({ baseUrl: "http://localhost:8283" });

// 匯入
const agentState = await client.agents.importFile(file, {});

// 匯出
const schema = await client.agents.exportFile("<AGENT_ID>");
```

### 7.3 展示前端

本倉庫的 Next.js 應用程式作為 Agent 目錄網站:

```typescript
// src/lib/agents-loader.ts -- 核心解析邏輯

export interface AgentFull extends AgentMetadata {
  agents: any[];
  blocks: any[];
  tools: any[];
  groups: any[];
  files: any[];
  sources: any[];
}

// 載入所有 Agent
export async function getAllAgents(): Promise<AgentMetadata[]> {
  // 遍歷 agents/ 目錄
  // 讀取每個 .af 檔案
  // JSON.parse() 解析
  // 提取 name, description, system, blocks, tools
}

// 載入特定 Agent (完整資料)
export async function getAgentByPath(
  ownerId: string,
  agentKey: string
): Promise<AgentFull | null> {
  const fileContent = fs.readFileSync(agentFilePath, 'utf-8');
  const agentData = JSON.parse(fileContent);
  // 返回完整 Agent 資料
}
```

前端提供四個 Tab 檢視:
```typescript
// src/app/agents/_components/AgentDetails.tsx
type Tabs = 'overview' | 'memoryBlocks' | 'tools' | 'system';
```

---

## 8. 值得採用的關鍵模式

### 8.1 Agent 序列化格式概念

**模式**: 將整個 Agent 的完整狀態序列化為一個自包含的 JSON 檔案。

**核心設計決策**:
- 使用參照 ID 系統 (`agent-0`, `block-0`, `tool-0`) 而非嵌套
- 頂層分離 `agents[]`, `blocks[]`, `tools[]` 實現正規化
- 匯出時自動清除密鑰 (`secrets: null`)
- `in_context_message_ids` 區分上下文內/外訊息

**優勢**: 一個檔案即可完整重建 Agent，包含人格、記憶、工具、對話歷史。

### 8.2 記憶區塊設計

**模式**: 將 Agent 的「可編輯記憶」結構化為命名區塊，注入到 System Prompt 中。

**關鍵設計**:
- 每個區塊有 `label` (名稱)、`description` (用途說明)、`value` (內容)、`limit` (大小限制)
- 區塊透過 XML 標籤注入到系統提示中 (`<memory_blocks>...</{label}>`)
- Agent 可以透過工具 (`memory_replace`, `memory_insert`) 自行修改區塊
- `read_only` 欄位控制是否允許 Agent 修改
- 區塊可以在 Agent 之間共享 (同一 `block_id` 被多個 Agent 參照)

**Loop 的記憶分層模型是最佳實踐**:

```
核心記憶 (始終在上下文中)
├── persona          # 我是誰
├── soul             # 存在意義
├── about_user       # 使用者資訊
├── preferences      # 互動偏好
├── custom_instructions  # 明確規則
├── active_hypotheses    # 進行中假設
├── conversation_patterns # 對話模式
├── learned_corrections  # 錯誤修正
└── scratchpad       # 工作暫存

外部記憶 (需工具存取)
├── archival_memory  # 長期事實儲存
└── conversation_search  # 對話歷史搜尋
```

### 8.3 可攜式 Agent 狀態

**模式**: Agent 不只是配置，而是「有記憶的實體」。匯出包含:
- 靜態配置 (人格、工具、模型)
- 動態狀態 (記憶內容、對話歷史)
- 行為規則 (工具規則、工作流)

### 8.4 工具規則工作流

**模式**: 用宣告式規則定義工具執行流程，而非硬編碼。

```
run_first -> constrain_child_tools -> conditional -> exit_loop
```

這讓同一個 Agent 框架既能做自由對話 (無規則)，也能做結構化工作流 (嚴格規則)。

---

## 9. Clawtex 採用建議

### 9.1 設計 `.claw` (Clawtex Agent File) 格式

建議 clawtex 建立自己的可攜式 Agent 格式，以下是根據 .af 模式的設計提案:

```json
{
  "format": "clawtex-agent-file",
  "version": "1.0.0",
  "created_at": "2026-03-12T...",
  "metadata": {
    "revision_id": "...",
    "clawtex_version": "0.1.0"
  },

  "agent": {
    "name": "master",
    "description": "...",
    "system_prompt": "...",
    "provider_config": {
      "provider": "ollama",
      "model": "llama3:latest",
      "context_window": 8192,
      "temperature": 0.7
    },
    "memory": {
      "blocks": [
        {
          "label": "persona",
          "value": "...",
          "limit": 10000,
          "read_only": false
        },
        {
          "label": "user_context",
          "value": "...",
          "limit": 5000,
          "read_only": false
        }
      ],
      "semantic_entries": [
        // 來自 SQLite/pgvector 的語意記憶匯出
      ]
    }
  },

  "tools": [
    {
      "name": "shell",
      "type": "builtin",
      "enabled": true
    },
    {
      "name": "custom_tool",
      "type": "custom",
      "source_code": "...",
      "json_schema": { ... }
    }
  ],

  "hands": [
    {
      "name": "outreach",
      "phases": [ ... ],
      "tools": ["email_send", "web_search"],
      "chain_to": "freelancer",
      "settings": { ... }
    }
  ],

  "cron_jobs": [
    {
      "name": "daily_scan",
      "schedule": "0 9 * * *",
      "action": { "type": "hand", "hand_name": "freelancer" }
    }
  ],

  "secrets_manifest": [
    // 列出需要的密鑰名稱 (不含值)
    "TELEGRAM_BOT_TOKEN",
    "STRIPE_SECRET_KEY"
  ]
}
```

### 9.2 Hands 匯出/匯入

Letta 的 .af 對應 clawtex 的場景:

| Letta 概念 | Clawtex 對應 |
|-----------|-------------|
| Agent | Agent (agents.toml 中的 agent 區塊) |
| Memory Blocks | MemoryStore (SQLite 語意記憶) |
| Tool Rules | Hand phases (hand.toml 工作流) |
| Custom Tools | Tools (src/tools/) |
| tool_exec_environment_variables | agents.toml 中的 provider/tool 配置 |

建議實作 `clawtex-core export-agent` 和 `clawtex-core import-agent` CLI 指令:

```bash
# 匯出
clawtex-core export-agent master --output master.claw
clawtex-core export-hand outreach --output outreach.claw

# 匯入
clawtex-core import-agent master.claw
clawtex-core import-hand outreach.claw
```

### 9.3 記憶區塊系統

clawtex 目前使用 `MemoryStore` (SQLite key-value) 做語意記憶。建議增加結構化記憶區塊:

```toml
# agents.toml 中的新增區塊
[agents.master.memory_blocks]

[agents.master.memory_blocks.persona]
value = "I am the master agent of clawtex..."
limit = 10000
read_only = true

[agents.master.memory_blocks.user_context]
value = ""
limit = 5000
read_only = false
description = "Information about the current user"

[agents.master.memory_blocks.scratchpad]
value = ""
limit = 5000
read_only = false
description = "Working memory for in-progress tasks"
```

在 System Prompt 注入時，模仿 Letta 的 XML 標籤格式:

```rust
// src/context.rs 中新增
fn inject_memory_blocks(system_prompt: &str, blocks: &[MemoryBlock]) -> String {
    let mut injected = String::new();
    injected.push_str("<memory_blocks>\n");
    for block in blocks {
        injected.push_str(&format!(
            "<{label}>\n<value>\n{value}\n</value>\n</{label}>\n",
            label = block.label,
            value = block.value
        ));
    }
    injected.push_str("</memory_blocks>\n");
    format!("{}\n\n{}", system_prompt, injected)
}
```

### 9.4 工作流規則系統

Letta 的 `tool_rules` 對應 clawtex 的 Hand phases，但 tool_rules 更細粒度。建議在 hand.toml 中增加工具約束:

```toml
# ~/.clawtex/hands/outreach/hand.toml

[[phases]]
name = "research"
tools = ["web_search", "content_search"]
required_first_tool = "web_search"     # 類似 run_first
next_phase_condition = "has_results"    # 類似 conditional

[[phases]]
name = "contact"
tools = ["email_send"]
requires_approval = true               # 類似 Letta 的 approval gate
exit_on_complete = true                 # 類似 exit_loop
```

### 9.5 實作優先順序

1. **Phase 1**: 定義 `.claw` JSON Schema (參考 .af 結構)
2. **Phase 2**: 實作 `export-agent` CLI (序列化 agent + tools + memory)
3. **Phase 3**: 實作 `import-agent` CLI (反序列化 + 建立 agent)
4. **Phase 4**: 增加 Hands 匯出/匯入
5. **Phase 5**: 結構化記憶區塊 (inject into system prompt)
6. **Phase 6**: Web UI 展示 (參考 letta-agent-file 的 Next.js 前端)

---

## 附錄: 重要檔案路徑索引

| 用途 | 絕對路徑 |
|------|---------|
| 專案根目錄 | `C:\Users\m4932\Desktop\adreanalai\LLM-Cluster-Project\references\letta-agent-file` |
| TypeScript 宣告 | `...\letta-agent-file\index.d.ts` |
| Agent 載入器 (核心) | `...\letta-agent-file\src\lib\agents-loader.ts` |
| Agent 詳情頁 | `...\letta-agent-file\src\app\agents\[ownerId]\[agentKey]\page.tsx` |
| Agent 詳情元件 | `...\letta-agent-file\src\app\agents\_components\AgentDetails.tsx` |
| Loop .af (旗艦範例) | `...\letta-agent-file\agents\@letta-ai\loop\loop.af` |
| LettaBot .af (多頻道) | `...\letta-agent-file\agents\@letta-ai\lettabot\lettabot.af` |
| Evie .af (多 Agent) | `...\letta-agent-file\agents\@letta-ai\evie\evie.af` |
| Grunk .af (社群範例) | `...\letta-agent-file\agents\@cpfiffer\grunk\grunk.af` |
| 客服 Agent 建立腳本 | `...\letta-agent-file\customer_service_agent\customer_service_agent.py` |
| 工作流 Agent 建立腳本 | `...\letta-agent-file\workflow_agent\workflow_agent.py` |
| 深度研究 Agent 腳本 | `...\letta-agent-file\deep_research_agent\deep_research_agent.py` |
| MemGPT Agent 腳本 | `...\letta-agent-file\memgpt_agent\memgpt_agent.py` |
| 發布技能定義 | `...\letta-agent-file\.skills\releasing-agentfiles\SKILL.md` |
| 貢獻指南 | `...\letta-agent-file\agents\CONTRIBUTING.md` |
| 學術引用 | `...\letta-agent-file\CITATION.cff` |
