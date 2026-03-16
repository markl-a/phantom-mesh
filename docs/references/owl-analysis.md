# OWL (CAMEL-AI) 深度技術分析

> 分析對象: `references/owl/` -- CAMEL-AI 的 OWL (Optimized Workforce Learning) 多智能體協作框架
> 分析日期: 2026-03-12
> 目的: 為 clawtex-core 的多智能體委派 (delegate) 機制提供架構參考

---

## 1. 專案結構

```
owl/
├── owl/                           # 核心套件
│   ├── utils/
│   │   ├── __init__.py            # 匯出所有公開 API
│   │   ├── common.py              # extract_pattern() XML 標籤解析
│   │   ├── enhanced_role_playing.py  # *** 核心: OwlRolePlaying + run_society ***
│   │   ├── document_toolkit.py    # DocumentProcessingToolkit (多格式文件處理)
│   │   └── gaia.py                # GAIABenchmark (GAIA 基準測試)
│   ├── webapp.py                  # Gradio Web UI (英文版)
│   ├── webapp_zh.py               # Gradio Web UI (中文版)
│   ├── webapp_jp.py               # Gradio Web UI (日文版)
│   └── webapp_backup.py           # 備份版本
├── examples/                      # 入口點 (各平台範例)
│   ├── run.py                     # *** Workforce 模式: OpenAI GPT-5.2 ***
│   ├── run_claude.py              # Workforce 模式: Anthropic Claude Opus 4.6
│   ├── run_deepseek.py            # Workforce 模式: DeepSeek
│   ├── run_groq.py                # Workforce 模式: Groq (Llama 3.3 70B)
│   ├── run_qwen.py                # Workforce 模式: Qwen 3.5-plus
│   ├── run_vllm.py                # Workforce 模式: VLLM/OpenAI-compatible
│   └── __init__.py
├── community_usecase/             # 社群用例
│   ├── Airbnb-MCP/                # MCP + Airbnb 搜尋
│   ├── Notion-MCP/                # MCP + Notion 管理
│   ├── Mcp_use_case/              # MCP + Firecrawl 內容策展
│   ├── Puppeteer MCP/             # MCP + Playwright 瀏覽器自動化 (Streamlit UI)
│   ├── Whatsapp-MCP/              # MCP + WhatsApp
│   ├── qwen3_mcp/                 # MCP + Qwen3 SSE 模式
│   ├── a_share_investment_agent/  # A股投資多智能體辯論系統
│   ├── cooking-assistant/         # 烹飪助手
│   ├── learning-assistant/        # 學習助手
│   ├── excel_analyzer/            # Excel 分析
│   ├── stock-analysis/            # 股票分析 + SEC API
│   ├── virtual_fitting_room/      # 虛擬試衣間
│   ├── resume-analysis-assistant/ # 履歷分析
│   └── PHI_Sanitization_.../      # PHI 清洗 + 文章撰寫
├── .container/                    # Docker 部署
│   ├── Dockerfile
│   ├── docker-compose.yml
│   └── build_docker.sh/bat
├── assets/                        # 架構圖、技術報告
└── README.md                      # 文件 (英/中/日)
```

**關鍵觀察**: OWL 本身的核心程式碼極為精簡 -- 整個 `owl/utils/` 只有 4 個 Python 檔案。真正的多智能體基礎設施完全建立在 CAMEL-AI 框架之上 (`camel` 套件)。OWL 的價值在於 **組合模式** (composition pattern) 而非底層實作。

---

## 2. 入口點與運行模式

OWL 有兩種完全不同的多智能體協作模式:

### 模式 A: RolePlaying (角色扮演對話迴圈)

這是 OWL 最早的模式，使用 User Agent + Assistant Agent 的雙智能體對話。

**入口**: 各 `construct_society()` 函式 (社群用例中大量使用)

```python
# 來源: owl/utils/enhanced_role_playing.py (第 31-66 行)
class OwlRolePlaying(RolePlaying):
    def __init__(self, **kwargs):
        self.user_role_name = kwargs.get("user_role_name", "user")
        self.assistant_role_name = kwargs.get("assistant_role_name", "assistant")
        super().__init__(**kwargs)
        init_user_sys_msg, init_assistant_sys_msg = self._construct_gaia_sys_msgs()
        self._init_agents(init_assistant_sys_msg, init_user_sys_msg, ...)
```

**運行迴圈**: `run_society()` / `arun_society()`

```python
# 來源: owl/utils/enhanced_role_playing.py (第 481-543 行)
def run_society(society: OwlRolePlaying, round_limit: int = 15):
    input_msg = society.init_chat(init_prompt)
    for _round in range(round_limit):
        assistant_response, user_response = society.step(input_msg)
        # ... token 追蹤 ...
        if "TASK_DONE" in user_response.msg.content:
            break
        input_msg = assistant_response.msg
    answer = chat_history[-1]["assistant"]
    return answer, chat_history, token_info
```

### 模式 B: Workforce (階層式任務分解)

這是 OWL 的進階模式，使用 Coordinator + Task Agent + 多個 Worker Agent 的階層結構。

**入口**: 各 `examples/run_*.py` 的 `construct_workforce()` 函式

```python
# 來源: examples/run.py (第 173-214 行)
def construct_workforce() -> Workforce:
    task_agent = ChatAgent(
        "You are a helpful assistant that can decompose tasks and assign tasks to workers.",
        **task_agent_kwargs,
    )
    coordinator_agent = ChatAgent(
        "You are a helpful assistant that can assign tasks to workers.",
        **coordinator_agent_kwargs,
    )
    workforce = Workforce("Workforce", task_agent=task_agent, coordinator_agent=coordinator_agent)

    agent_list = construct_agent_list()
    for agent_dict in agent_list:
        workforce.add_single_agent_worker(agent_dict["description"], worker=agent_dict["agent"])
    return workforce
```

**使用方式**:

```python
# 來源: examples/run.py (第 217-238 行)
task = Task(content=task_prompt)
workforce = construct_workforce()
processed_task = workforce.process_task(task)  # 階層式任務處理
print(f"Answer: {processed_task.result}")
```

### 模式選擇矩陣

| 特性 | RolePlaying 模式 | Workforce 模式 |
|------|-----------------|---------------|
| 智能體數量 | 2 (User + Assistant) | N+2 (Coordinator + TaskAgent + N Workers) |
| 協調方式 | 對話迴圈 (step-by-step) | 階層式任務分解 + 分派 |
| 工具使用者 | 僅 Assistant | 各 Worker 獨立持有工具 |
| 任務分解 | User Agent 逐步指導 | Task Agent 自動分解 |
| 終止條件 | `TASK_DONE` 訊號 | 所有子任務完成 |
| 適用場景 | MCP 整合、簡單任務 | 複雜多步驟任務 |

---

## 3. 核心架構

### 3.1 多智能體協調: RolePlaying 的對話協議

OWL 的 RolePlaying 模式實作了一個精巧的 **互惠式對話協議** (Reciprocal Dialogue Protocol):

```
User Agent                              Assistant Agent
    |                                        |
    |  (1) 接收 assistant 的前次回應            |
    |  (2) 基於回應產生下一個指令               |
    |  (3) 附加任務上下文資訊                   |
    |  ──────── Instruction ──────────>       |
    |                                  (4) 執行指令 (可能呼叫工具)
    |                                  (5) 產生 Solution 回應
    |                                  (6) 附加任務進度資訊
    |  <──────── Solution ─────────────       |
    |                                        |
    | (迴圈直到 User Agent 發出 TASK_DONE)     |
```

**關鍵實作 -- `step()` 方法**:

```python
# 來源: owl/utils/enhanced_role_playing.py (第 255-323 行)
def step(self, assistant_msg: BaseMessage) -> Tuple[ChatAgentResponse, ChatAgentResponse]:
    # Phase 1: User Agent 產生指令
    user_response = self.user_agent.step(assistant_msg)
    user_msg = self._reduce_message_options(user_response.msgs)
    modified_user_msg = deepcopy(user_msg)

    # Phase 2: 注入上下文輔助資訊
    if "TASK_DONE" not in user_msg.content:
        modified_user_msg.content += f"""
        Here are auxiliary information about the overall task:
        <auxiliary_information>{self.task_prompt}</auxiliary_information>
        If there are available tools and you want to call them, never say 'I will ...',
        but first call the tool and reply based on tool call's result.
        """
    else:
        modified_user_msg.content += f"""
        Now please make a final answer of the original task: <task>{self.task_prompt}</task>
        """

    # Phase 3: Assistant Agent 處理指令
    assistant_response = self.assistant_agent.step(modified_user_msg)

    # Phase 4: 附加進度追蹤提示
    modified_assistant_msg.content += f"""
        Provide me with the next instruction based on my response and our current task:
        <task>{self.task_prompt}</task>
        Before producing the final answer, please check whether I have rechecked
        the final answer using different toolkit as much as possible.
        If you think our task is done, reply with `TASK_DONE` to end our conversation.
    """

    return (modified_assistant_msg_response, modified_user_msg_response)
```

**架構亮點**:
1. **動態上下文注入**: 每一步都將原始任務 (`self.task_prompt`) 重新注入，防止智能體偏離目標
2. **交叉驗證提示**: 自動要求 Assistant 使用不同工具驗證答案
3. **工具先行原則**: "never say 'I will ...', but first call the tool" -- 強制工具執行而非規劃
4. **明確終止信號**: `TASK_DONE` 作為對話結束的統一協議

### 3.2 多智能體協調: Workforce 的階層分派

Workforce 模式使用三層架構:

```
                    ┌──────────────┐
                    │  Task Agent  │  ← 負責任務分解
                    └──────┬───────┘
                           │
                    ┌──────┴───────┐
                    │ Coordinator  │  ← 負責任務分派
                    └──────┬───────┘
                           │
              ┌────────────┼────────────┐
              │            │            │
        ┌─────┴─────┐ ┌───┴─────┐ ┌───┴──────────┐
        │ Web Agent │ │ Doc     │ │ Reasoning    │
        │           │ │ Agent   │ │ Coding Agent │
        └───────────┘ └─────────┘ └──────────────┘
```

Worker 定義方式:

```python
# 來源: examples/run.py (第 147-170 行)
agent_list = [
    {
        "name": "Web Agent",
        "description": "A helpful assistant that can search the web, extract webpage content,
                        simulate browser actions, and retrieve relevant information.",
        "agent": web_agent,
    },
    {
        "name": "Document Processing Agent",
        "description": "A helpful assistant that can process a variety of local and remote
                        documents, including pdf, docx, images, audio, and video, etc.",
        "agent": document_processing_agent,
    },
    {
        "name": "Reasoning Coding Agent",
        "description": "A helpful assistant that specializes in reasoning, coding, and
                        processing excel files. However, it cannot access the internet.",
        "agent": reasoning_coding_agent,
    },
]
```

**Coordinator 透過 description 進行語義匹配分派** -- 這與 clawtex-core 的 classifier.rs 路由概念相似。

### 3.3 專業化智能體角色

OWL 定義了 3 個核心 Worker 角色:

| 角色 | 工具集 | 模型要求 | 職責 |
|------|--------|---------|------|
| **Web Agent** | SearchToolkit (DuckDuckGo, Wiki, Baidu), DocumentProcessingToolkit, BrowserToolkit | 需要 tool calling | 網頁搜尋、瀏覽器模擬、URL 內容擷取 |
| **Document Processing Agent** | DocumentProcessingToolkit, ImageAnalysisToolkit, CodeExecutionToolkit, FileToolkit | 需要多模態 | 文件解析、圖片分析、檔案操作 |
| **Reasoning Coding Agent** | CodeExecutionToolkit, ExcelToolkit, DocumentProcessingToolkit | 需要推理能力 | 數學推理、程式撰寫、Excel 處理 |

**關鍵設計原則**: 每個角色有 **明確的工具邊界** -- Web Agent 不持有 CodeExecutionToolkit，Reasoning Agent 不持有 SearchToolkit。這避免了工具濫用並強化了角色專業化。

### 3.4 Groq 範例中的模型分級

```python
# 來源: examples/run_groq.py (第 62-91 行)
# 工具密集型角色使用大模型
web_model = ModelFactory.create(
    model_platform=ModelPlatformType.GROQ,
    model_type=ModelType.GROQ_LLAMA_3_3_70B,  # 70B 大模型
)

# 來源: examples/run_groq.py (第 184-198 行)
# Coordinator 和 Task Agent 使用小模型 (不需要工具能力)
coordinator_agent_kwargs = {
    "model": ModelFactory.create(
        model_platform=ModelPlatformType.GROQ,
        model_type=ModelType.GROQ_LLAMA_3_1_8B,  # 8B 小模型
    )
}
```

**這種模型分級策略與 clawtex-core 的 smart_routing 直接對應**: simple -> 小模型 (coordinator/task agent), complex -> 大模型 (worker agents)。

---

## 4. 工具整合

### 4.1 統一工具介面: FunctionTool

所有工具透過 CAMEL 的 `FunctionTool` 統一包裝:

```python
# 來源: examples/run.py (第 117-124 行)
web_agent = ChatAgent(
    "...",  # 系統提示
    model=web_model,
    tools=[
        FunctionTool(search_toolkit.search_duckduckgo),
        FunctionTool(search_toolkit.search_wiki),
        FunctionTool(document_processing_toolkit.extract_document_content),
        *browser_toolkit.get_tools(),  # 展開瀏覽器工具集
    ],
)
```

`FunctionTool` 自動從 Python 函式簽名生成 JSON Schema，供 LLM 進行 function calling。

### 4.2 BrowserToolkit 的雙層架構

```python
# 來源: examples/run.py (第 90-94 行)
browser_toolkit = BrowserToolkit(
    headless=False,
    web_agent_model=browsing_model,      # 專用瀏覽模型
    planning_agent_model=planning_model,  # 專用規劃模型
)
```

BrowserToolkit 內部又包含兩個 sub-agent:
- **Web Agent Model**: 負責理解頁面內容和決定下一步操作
- **Planning Agent Model**: 負責規劃多步驟瀏覽策略

這是一個 **嵌套式智能體架構** (nested agent architecture) -- 工具本身內部又有智能體。

### 4.3 DocumentProcessingToolkit

```python
# 來源: owl/utils/document_toolkit.py (第 41-322 行)
class DocumentProcessingToolkit(BaseToolkit):
    def __init__(self, cache_dir=None, model=None):
        self.image_tool = ImageAnalysisToolkit(model=model)
        self.excel_tool = ExcelToolkit()
        self.uio = UnstructuredIO()

    def extract_document_content(self, document_path: str) -> Tuple[bool, str]:
        # 圖片 -> ImageAnalysisToolkit
        if any(document_path.endswith(ext) for ext in [".jpg", ".jpeg", ".png"]):
            return True, self.image_tool.ask_question_about_image(...)
        # Excel -> ExcelToolkit
        if any(document_path.endswith(ext) for ext in ["xls", "xlsx"]):
            return True, self.excel_tool.extract_excel_content(document_path)
        # ZIP -> 解壓
        if any(document_path.endswith(ext) for ext in ["zip"]):
            return True, self._unzip_file(document_path)
        # 網頁 -> Firecrawl (付費) / crawl4ai (免費備援)
        if self._is_webpage(document_path):
            return True, self._extract_webpage_content(document_path)
        # 其他 -> UnstructuredIO
        return True, self.uio.parse_file_or_url(document_path)
```

**多層備援策略**: Firecrawl API (優先) -> crawl4ai (免費備援) -> UnstructuredIO (最後備援)

---

## 5. CAMEL 框架依賴

OWL 完全建立在 CAMEL-AI 框架之上，直接使用以下核心元件:

| CAMEL 元件 | OWL 中的使用 |
|-----------|-------------|
| `camel.agents.ChatAgent` | 所有智能體 (User, Assistant, Worker, Coordinator, Task) |
| `camel.societies.RolePlaying` | `OwlRolePlaying` 的父類別 |
| `camel.societies.Workforce` | Workforce 模式的核心 |
| `camel.models.ModelFactory` | 統一建立各平台模型 |
| `camel.toolkits.*` | SearchToolkit, BrowserToolkit, CodeExecutionToolkit, etc. |
| `camel.toolkits.MCPToolkit` | MCP 工具整合 |
| `camel.toolkits.FunctionTool` | 將 Python 函式轉為 LLM 可呼叫的工具 |
| `camel.tasks.Task` | Workforce 模式的任務表示 |
| `camel.types.ModelPlatformType` | 模型平台列舉 (OPENAI, ANTHROPIC, GROQ, ...) |
| `camel.responses.ChatAgentResponse` | 智能體回應封裝 |
| `camel.messages.BaseMessage` | 訊息基礎類別 |
| `camel.loaders.UnstructuredIO` | 文件解析 |
| `camel.benchmarks.BaseBenchmark` | GAIA 基準測試基礎類別 |

**OWL 的核心附加值**: 只有 `enhanced_role_playing.py` 和 `document_toolkit.py` 是 OWL 自身的程式碼。框架價值在於 **最佳化組合配方** (optimized composition recipe)。

---

## 6. 瀏覽器自動化

### 6.1 BrowserToolkit 整合模式

Web Agent 同時持有搜尋工具和瀏覽器工具，但有明確的使用策略:

```python
# 來源: examples/run.py (第 97-124 行) -- Web Agent 系統提示 (節選)
"""
- Do not solely rely on document tools or browser simulation to find the answer,
  you should combine document tools and browser simulation to comprehensively
  process web page information.
- Search results typically do not provide precise answers. The search query should
  be concise and focuses on finding sources rather than direct answers.
- Browser simulation is also helpful for finding target URLs.
"""
```

### 6.2 跨智能體瀏覽器協調

在 Workforce 模式下，只有 Web Agent 持有 BrowserToolkit。若 Reasoning Agent 需要網頁資料，必須透過 Coordinator 將子任務分派給 Web Agent，再將結果回傳。

在 RolePlaying 模式下 (MCP 用例)，Assistant Agent 直接持有所有工具 (包括 MCP 瀏覽器工具)。

### 6.3 Puppeteer MCP 範例

```python
# 來源: community_usecase/Puppeteer MCP/demo.py (第 59-85 行)
async def run_task(task: str) -> str:
    config_path = Path(__file__).parent / "mcp_servers_config.json"
    mcp_toolkit = MCPToolkit(config_path=str(config_path))
    await mcp_toolkit.connect()

    # MCP 工具 + 搜尋工具合併
    tools = [*mcp_toolkit.get_tools(), SearchToolkit().search_duckduckgo]
    society = await construct_society(task, tools)
    answer, chat_history, token_count = await arun_society(society)

    await mcp_toolkit.disconnect()
    return answer
```

MCP 伺服器設定:

```json
// 來源: community_usecase/Puppeteer MCP/mcp_servers_config.json
{
  "mcpServers": {
    "playwright": {
      "command": "npx",
      "args": ["-y", "@executeautomation/playwright-mcp-server", "--browser", "chromium"]
    }
  }
}
```

---

## 7. MCP 整合

### 7.1 MCP 在多智能體中的使用模式

OWL 透過 CAMEL 的 `MCPToolkit` 整合 MCP:

```python
# 來源: community_usecase/Airbnb-MCP/Airbnb_MCP.py (第 22-63 行)
async def construct_society(question: str, tools: List[FunctionTool]) -> OwlRolePlaying:
    user_agent_kwargs = {"model": models["user"]}
    assistant_agent_kwargs = {
        "model": models["assistant"],
        "tools": tools,              # MCP 工具注入到 Assistant Agent
    }
    return OwlRolePlaying(
        task_prompt=question,
        user_role_name="content_curator",     # 自訂角色名稱
        assistant_role_name="research_assistant",
        user_agent_kwargs=user_agent_kwargs,
        assistant_agent_kwargs=assistant_agent_kwargs,
    )

async def main():
    config_path = Path(__file__).parent / "mcp_servers_config.json"
    mcp_toolkit = MCPToolkit(config_path=str(config_path))
    await mcp_toolkit.connect()

    tools = [*mcp_toolkit.get_tools()]  # 取得所有 MCP 工具
    society = await construct_society(task, tools)
    answer, chat_history, token_count = await arun_society(society)

    await mcp_toolkit.disconnect()
```

### 7.2 MCP 伺服器設定格式

```json
// 來源: community_usecase/Mcp_use_case/mcp_servers_config.json
{
  "mcpServers": {
    "desktop-commander": {
      "command": "npx",
      "args": ["-y", "@wonderwhy-er/desktop-commander"]
    },
    "playwright": {
      "command": "npx",
      "args": ["-y", "@executeautomation/playwright-mcp-server", "--browser", "chromium"]
    },
    "mcp-server-firecrawl": {
      "command": "npx",
      "args": ["-y", "firecrawl-mcp"]
    }
  }
}
```

### 7.3 MCP + 自訂工具混合使用

```python
# 來源: community_usecase/Puppeteer MCP/demo.py (第 73 行)
tools = [*mcp_toolkit.get_tools(), SearchToolkit().search_duckduckgo]
```

MCP 工具和原生 CAMEL 工具可以自由混合，因為都被統一為 `FunctionTool` 介面。

### 7.4 已知的 MCP 伺服器整合

| MCP 伺服器 | 功能 | NPM 套件 |
|-----------|------|---------|
| Airbnb | 住宿搜尋 | `@openbnb/mcp-server-airbnb` |
| Notion | 筆記管理 | `@notionhq/notion-mcp-server` |
| Playwright | 瀏覽器自動化 | `@executeautomation/playwright-mcp-server` |
| Firecrawl | 網頁爬蟲 | `firecrawl-mcp` |
| Desktop Commander | 桌面操作 | `@wonderwhy-er/desktop-commander` |

---

## 8. 值得借鑑的關鍵模式

### 8.1 互惠式對話協議 (Reciprocal Dialogue Protocol)

**模式描述**: User Agent 扮演「指導者」、Assistant Agent 扮演「執行者」。User Agent 不斷將任務分解為子步驟並指導 Assistant Agent 執行。

**核心優勢**:
- **自然的任務分解**: User Agent 像人類主管一樣逐步指導
- **交叉驗證**: 自動提示使用不同工具驗證結果
- **動態上下文**: 每步都重新注入原始任務，防止偏離

```python
# 來源: owl/utils/enhanced_role_playing.py (第 183-211 行)
# User Agent 系統提示中的核心指導原則
"""
You must instruct me based on my expertise and your needs to solve the task step by step.
The format of your instruction is: `Instruction: [YOUR INSTRUCTION]`
You must give me one instruction at a time.
...
Keep giving me instructions until you think the task is completed.
When the task is completed, you must only reply with a single word <TASK_DONE>.
"""
```

**clawtex-core 對應**: 可在 `delegate` 工具中實作類似的 User-Assistant 雙角色對話迴圈。目前 clawtex 的 delegate 是單次呼叫，缺少這種迭代式的任務分解能力。

### 8.2 階層式 Workforce 分派

**模式描述**: Task Agent 分解任務 -> Coordinator 匹配最佳 Worker -> Worker 執行並回報。

**核心優勢**:
- **關注點分離**: 分解、分派、執行各有專責
- **語義匹配**: Coordinator 根據 Worker 的文字描述進行匹配
- **可擴展性**: 新增 Worker 只需定義 description + agent + tools

**clawtex-core 對應**: 可在 Hands 引擎中引入 Coordinator 概念。目前 Hands 是線性的 phase-to-phase 執行，缺少基於能力描述的動態分派。

### 8.3 模型分級策略

**模式描述**: 管理層智能體 (Coordinator, Task Agent) 使用小模型，工作層智能體 (Workers) 使用大模型。

```
Coordinator: 8B 模型 (快速、便宜)
Task Agent: 8B 模型
Web Agent: 70B 模型 (需要 tool calling)
Reasoning Agent: 70B 模型 (需要推理能力)
```

**clawtex-core 對應**: 直接映射到 `[smart_routing]` 配置。可以將 classifier (路由決策) 固定用小模型，而實際執行用大模型。

### 8.4 工具邊界隔離

**模式描述**: 每個 Worker 只持有其職責範圍內的工具，不能越界。

**核心優勢**:
- 防止 LLM 在不適當的場景使用工具 (如用搜尋工具回答數學題)
- 降低工具選擇的認知負擔
- 強制任務流經正確的智能體

**clawtex-core 對應**: 目前 clawtex 的 master agent 持有所有 24 個工具。可以考慮在 `delegate_to_provider` 工具中限制被委派智能體可用的工具子集。

### 8.5 MCP 工具的統一注入

**模式描述**: MCP 工具透過 `MCPToolkit.get_tools()` 自動轉為與原生工具相同的 `FunctionTool` 介面，可以自由混合。

**clawtex-core 對應**: clawtex 的 `src/mcp/mod.rs` 已有 McpBridge + McpToolProxy。可以改進為：自動將 MCP 工具注冊到 ToolRegistry，與原生工具統一管理。

---

## 9. 對 clawtex-core 的具體建議

### 9.1 增強 delegate 工具: 引入對話迴圈

目前 clawtex 的 `delegate` 工具是單次呼叫:

```
master -> delegate("sub_agent", "do something") -> 單次 LLM 呼叫 -> 回傳結果
```

OWL 模式建議改為:

```
master -> delegate("sub_agent", "do something", max_rounds=10) ->
  內部: user_agent.step() <-> assistant_agent.step() 迴圈 ->
  TASK_DONE 或 max_rounds 後回傳結果
```

**實作建議**: 在 `delegate` 工具中加入可選的 `iteration_mode: bool` 參數。啟用後，被委派的智能體內部進行多輪對話，模擬 OWL 的 `run_society()` 迴圈。

### 9.2 引入 Worker 描述匹配

OWL 的 Coordinator 使用 Worker 的文字描述進行語義匹配。clawtex 可以:

1. 在 `agents.toml` 中為每個 agent 加入 `description` 欄位
2. 在 `delegate` 工具中，當目標為 `"auto"` 時，使用 classifier 將任務語義匹配到最佳 agent
3. 這比硬編碼路由更靈活

### 9.3 工具子集化

為不同 Hands phase 或被委派的智能體定義工具白名單:

```toml
# agents.toml 範例
[agents.web_researcher]
tools = ["web_search", "http_request", "content_search"]

[agents.coder]
tools = ["shell", "file_read", "file_write", "file_edit"]
```

### 9.4 MCP 工具自動註冊

將 MCP 工具自動注入 `ToolRegistry`，使其對所有智能體可用:

```rust
// 概念偽碼
let mcp_tools = mcp_bridge.list_tools().await?;
for tool in mcp_tools {
    tool_registry.register(McpToolProxy::new(tool));
}
```

### 9.5 交叉驗證機制

借鑑 OWL 的 "recheck the final answer using different toolkit" 策略，可在關鍵 Hand phase 結束前自動加入驗證步驟:

```toml
# hand.toml 範例
[[phases]]
name = "execute"
tools = ["web_search", "shell"]

[[phases]]
name = "verify"
prompt = "Verify the previous result using a different method"
tools = ["content_search", "http_request"]
auto_verify = true
```

---

## 10. 總結

### OWL 的核心設計哲學

1. **組合優於實作**: OWL 不重新發明輪子，而是找到 CAMEL 元件的最佳組合
2. **角色專業化**: 每個智能體有明確的職責範圍和工具邊界
3. **迭代式任務解決**: 不追求一次性解決，而是多輪對話逐步逼近答案
4. **動態上下文保持**: 每步都重新注入原始任務，防止智能體迷失
5. **多層備援**: 工具失敗時自動切換到替代方案

### 與 clawtex-core 的對比

| 面向 | OWL | clawtex-core |
|------|-----|-------------|
| 語言 | Python (CAMEL 框架) | Rust (自建) |
| 多智能體 | RolePlaying 對話 + Workforce 階層 | delegate 工具 + Hands 工作流 |
| 工具數量 | ~15 (CAMEL 提供) | 24 (自建) |
| MCP | MCPToolkit (CAMEL) | McpBridge (自建 JSON-RPC) |
| 任務分解 | LLM 驅動 (User Agent / Task Agent) | 固定 phase 序列 (Hands) |
| 模型支援 | OpenAI, Anthropic, Groq, Qwen, DeepSeek, VLLM | Ollama, OpenAI, Anthropic, Gemini, Groq, ChatGPT |
| 部署 | Python + Docker | Rust 二進位 + Docker |

### clawtex-core 可從 OWL 借鑑的前 3 項改進

1. **迭代式 delegate**: 將 delegate 從單次呼叫改為多輪對話迴圈，大幅提升複雜任務處理能力
2. **智能體工具隔離**: 為不同角色定義工具白名單，避免工具濫用
3. **語義匹配分派**: 用 agent description 進行自動路由，替代硬編碼的 delegate 目標

---

*本分析基於 OWL 專案 (camel-ai/owl) main 分支的源碼，GAIA benchmark 得分 69.09%，NeurIPS 2025 收錄。*
