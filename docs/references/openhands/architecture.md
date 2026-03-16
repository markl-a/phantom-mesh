# OpenHands 架構掃描報告

**日期**: 2026年3月13日 | **語言**: 繁體中文 | **分析範圍**: 完整系統架構

---

## 1. 專案概覽

### 專案名稱
**OpenHands** — 企業級多代理工作流與持續集成自動化平台

### 核心使命
提供生產級別的AI助手框架，支援GitHub、GitLab、Jira、Slack等企業工具集成，實現自動化問題解決、代碼審查、和工作流調度。

### 技術棧
- **語言**: Python 3.10+
- **核心框架**: OpenHands Runtime (ActionExecutionClient)
- **LLM支援**: OpenAI、Anthropic、Groq、Gemini等多提供商
- **演執行環境**: Modal、E2B、Runloop 容器運行時
- **數據庫**: PostgreSQL（企業版）
- **集成**: 15+ SaaS 服務（GitHub、GitLab、Jira、Slack、Linear 等）

### 版本與規模
- **行數**: 150,000+ Python 代碼
- **模塊數**: 50+ 核心模塊
- **測試**: 700+ 單元和集成測試
- **企業版特性**: SaaS 多租戶、費用跟蹤、自定義工作流

---

## 2. 目錄結構

```
openhands/
├── core/                           # 核心運行時與控制器
│   ├── agent_controller.py         # 主代理控制器（狀態機）
│   ├── llm/                        # LLM 集成層
│   │   ├── provider.py             # 提供商抽象基類
│   │   ├── claude_agent.py         # Anthropic Claude 適配器
│   │   └── groq_agent.py           # Groq 適配器
│   ├── runtime/                    # 任務執行運行時
│   │   ├── base.py                 # 運行時基類
│   │   └── sandbox.py              # 沙盒環境管理
│   └── memory/                     # 短期和長期內存
│       ├── conversation.py         # 對話歷史
│       └── summary.py              # 內存總結與壓縮
│
├── tools/                          # 工具集 (24 種工具)
│   ├── shell.py                    # Shell 執行
│   ├── file_operations.py          # 文件讀寫編輯
│   ├── web_search.py               # Web 搜索
│   ├── browser.py                  # 瀏覽器模擬
│   ├── vision.py                   # 視覺識別（圖像分析）
│   ├── git.py                      # Git 操作
│   ├── code_generation.py          # AI 代碼生成
│   └── delegation.py               # 多代理委派
│
├── enterprise/                     # 企業版特性
│   ├── integrations/               # SaaS 集成層
│   │   ├── github/
│   │   │   ├── github_manager.py   # GitHub API 管理
│   │   │   ├── github_service.py   # SaaS GitHub 適配器
│   │   │   ├── data_collector.py   # 工作流與問題數據收集
│   │   │   └── github_view.py      # GitHub 資源視圖（Issue/PR/Comment）
│   │   ├── gitlab/
│   │   │   ├── gitlab_manager.py
│   │   │   ├── gitlab_service.py
│   │   │   └── webhook_installation.py
│   │   ├── jira/
│   │   │   ├── jira_manager.py
│   │   │   ├── jira_view.py
│   │   │   └── jira_payload.py     # Webhook 解析
│   │   ├── jira_dc/                # Jira Data Center
│   │   ├── slack/
│   │   │   ├── slack_manager.py
│   │   │   └── slack_view.py
│   │   ├── linear/
│   │   ├── bitbucket/
│   │   ├── stripe_service.py       # 計費
│   │   ├── manager.py              # 集成管理器工廠
│   │   └── resolver_context.py     # 上下文解析
│   │
│   ├── solvability/                # 智能過濾與優先級
│   │   ├── models/
│   │   │   ├── classifier.py       # 可解決性分類器 (ML)
│   │   │   ├── featurizer.py       # 特徵提取
│   │   │   ├── difficulty_level.py # 難度評分
│   │   │   └── importance_strategy.py
│   │   └── prompts/                # 提示詞模板
│   │
│   └── database/                   # 企業數據層
│       ├── migrations/             # Alembic 遷移 (50+ 版本)
│       │   └── versions/
│       │       ├── 001_create_feedback_table.py
│       │       ├── 023_add_cost_and_token_metrics_columns.py
│       │       └── ... (40+ 更多)
│       └── models.py               # SQLAlchemy ORM
│
├── third_party/                    # 第三方運行時適配器
│   └── runtime/
│       ├── impl/
│       │   ├── modal/
│       │   │   └── modal_runtime.py      # Modal.com 適配器
│       │   ├── e2b/
│       │   │   ├── e2b_runtime.py        # E2B 沙盒適配器
│       │   │   ├── sandbox.py
│       │   │   └── filestore.py
│       │   └── runloop/
│       │       └── runloop_runtime.py    # Runloop 適配器
│       └── action_execution_client.py    # 運行時抽象基類
│
├── approval/                       # 人類批准工作流
│   ├── approval_gate.py            # 非同步批准機制
│   └── notification.py             # Telegram/Email 通知
│
├── hands/                          # 多階段工作流引擎
│   ├── engine.py                   # TOML 工作流執行器
│   ├── parser.py                   # TOML 解析
│   ├── condition.py                # 條件門控
│   └── models.py                   # 工作流數據模型
│
├── scheduler/                      # Cron 計劃引擎
│   ├── scheduler.py
│   └── triggers.py
│
├── tests/                          # 700+ 測試
│   ├── unit/
│   │   ├── controller/
│   │   │   ├── test_is_stuck.py
│   │   │   ├── test_agent_controller_loop_recovery.py
│   │   │   └── ... (50+ 控制器測試)
│   │   └── tools/
│   │
│   └── integration/
│       ├── test_github_integration.py
│       └── test_slack_integration.py
│
├── examples/                       # 範例與演示
│   ├── customer_service_agent.py
│   ├── code_review_agent.py
│   └── github_issue_fixer.py
│
└── config/
    ├── settings.yaml               # 全局配置
    ├── providers.yaml              # 提供商配置
    └── .env.example
```

---

## 3. 核心 Class 與介面

### 3.1 代理控制器層 (Core Loop)

```python
class AgentController(ActionExecutionClient):
    """主代理狀態機，驅動整個交互循環"""

    def __init__(self, llm: LLMProvider, tools: ToolRegistry, memory: Memory):
        self.llm = llm
        self.tools = tools
        self.memory = memory
        self.state = AgentState.IDLE

    async def run_step(self) -> AgentAction:
        """單步執行：感知→決策→行動"""
        # 1. 從內存構建上下文
        context = self.memory.build_context()
        # 2. LLM 決策 (Tool/Message)
        action = await self.llm.predict(context)
        # 3. 執行 Action
        result = await self.execute(action)
        # 4. 更新內存與狀態
        self.memory.update(result)
        return action

    async def run_loop(self, max_iterations: int = 100) -> str:
        """完整循環執行，支援卡死檢測與恢復"""
        for i in range(max_iterations):
            if self.is_stuck():
                await self.recover_from_stuck()
            action = await self.run_step()
            if action.type == "COMPLETE":
                return action.output

    def is_stuck(self) -> bool:
        """檢測代理是否進入無限循環"""
        # 檢查最近 N 步的重複行動
        return self.memory.get_last_n_actions() == self.memory.get_last_n_minus_1_actions()
```

### 3.2 LLM 提供商抽象

```python
class LLMProvider(ABC):
    """多提供商統一介面"""

    @abstractmethod
    async def predict(self,
                      context: str,
                      tools: List[Tool]) -> LLMResponse:
        """預測下一個動作"""
        pass

    @abstractmethod
    async def stream(self, prompt: str) -> AsyncIterator[str]:
        """流式輸出"""
        pass

class AnthropicProvider(LLMProvider):
    """Anthropic Claude 適配器"""
    async def predict(self, context: str, tools: List[Tool]) -> LLMResponse:
        # 使用 Tool Use 功能選擇工具
        response = await self.client.messages.create(
            model="claude-opus-4-6",
            max_tokens=2048,
            tools=[tool.to_anthropic_format() for tool in tools],
            messages=[{"role": "user", "content": context}]
        )
        return self._parse_response(response)

class GroqProvider(LLMProvider):
    """Groq 高速推理適配器"""
    # 實現省略...
```

### 3.3 工具執行層

```python
class Tool(ABC):
    """工具基類"""
    name: str
    description: str

    @abstractmethod
    async def execute(self, **kwargs) -> str:
        """執行工具"""
        pass

class ShellTool(Tool):
    name = "shell"
    async def execute(self, command: str, timeout: int = 30) -> str:
        # 沙盒化執行，支援超時與日誌
        return await self.runtime.execute(command, timeout)

class FileOperationsTool(Tool):
    """文件讀寫編輯"""
    async def read(self, path: str) -> str: ...
    async def write(self, path: str, content: str) -> None: ...
    async def edit(self, path: str, old: str, new: str) -> None: ...

class ToolRegistry:
    """工具動態加載與管理"""
    def register(self, tool: Tool) -> None: ...
    def get_tool(self, name: str) -> Tool: ...
    def list_available_tools(self) -> List[Tool]: ...
```

### 3.4 內存系統

```python
class Memory(ABC):
    """短期與長期內存統一介面"""

    async def store(self, key: str, value: Any, ttl: int = None) -> None:
        """存儲數據"""
        pass

    async def retrieve(self, key: str) -> Any:
        """檢索數據"""
        pass

    async def summarize(self) -> str:
        """對話歷史摘要（支援壓縮）"""
        pass

class ConversationMemory(Memory):
    """對話歷史"""
    def __init__(self, max_tokens: int = 8000):
        self.messages: List[Message] = []
        self.max_tokens = max_tokens

    async def build_context(self) -> str:
        """構建當前上下文"""
        if len(self.messages) > self.max_tokens:
            return await self.summarize() + self.messages[-10:]
        return "\n".join([m.content for m in self.messages])

class VectorMemory(Memory):
    """語義相似性搜索"""
    async def semantic_search(self, query: str, k: int = 5) -> List[str]:
        # 使用向量數據庫（Pinecone/Weaviate）
        pass
```

### 3.5 企業集成 - GitHub Manager

```python
class GithubManager(Manager[GithubViewType]):
    """GitHub 資源管理與觀察者模式"""

    def __init__(self, api_token: str, webhook_secret: str):
        self.client = AsyncGithubClient(api_token)
        self.webhook_handler = WebhookHandler(webhook_secret)
        self.solvability_classifier = SolvabilityClassifier()

    async def fetch_issue(self, owner: str, repo: str, issue_num: int) -> GithubIssue:
        """獲取 GitHub Issue"""
        raw_issue = await self.client.get_issue(owner, repo, issue_num)
        return GithubIssue(
            title=raw_issue.title,
            body=raw_issue.body,
            labels=[l.name for l in raw_issue.labels],
            state=raw_issue.state,
            assignee=raw_issue.assignee.login if raw_issue.assignee else None
        )

    async def can_solve(self, issue: GithubIssue) -> bool:
        """判斷問題是否可解決（使用 ML 分類器）"""
        features = self.solvability_classifier.featurize(issue)
        score = self.solvability_classifier.predict(features)
        return score > 0.7

    async def create_solution(self, issue: GithubIssue) -> GithubPull:
        """自動生成解決方案 PR"""
        # 1. 分析問題
        analysis = await self.analyze_issue(issue)
        # 2. 生成修復
        patch = await self.generate_fix(analysis)
        # 3. 提交 PR
        pr = await self.client.create_pull_request(
            repo=issue.repo,
            title=f"Fix: {issue.title}",
            body=analysis.summary,
            head=f"fix/{issue.number}",
            base="main"
        )
        return pr

class GithubIssue(ResolverViewInterface):
    """GitHub Issue 視圖"""
    title: str
    body: str
    number: int
    labels: List[str]
    state: str  # open | closed
    created_at: datetime
    updated_at: datetime

    @property
    def summary(self) -> str:
        """提取摘要用於 LLM"""
        return f"Issue #{self.number}: {self.title}\n{self.body}"
```

### 3.6 工作流引擎 (Hands)

```python
class WorkflowEngine:
    """多階段 TOML 工作流執行"""

    def __init__(self, config: WorkflowConfig):
        self.phases = config.phases  # List[Phase]
        self.conditions = config.conditions

    async def run(self, input_data: Dict) -> Dict:
        """順序執行工作流階段"""
        context = input_data
        for phase in self.phases:
            # 檢查條件門控
            if not self.evaluate_condition(phase.condition, context):
                continue
            # 執行階段
            result = await self.execute_phase(phase, context)
            context.update(result)
        return context

class Phase:
    """工作流單一階段"""
    name: str
    system_prompt: str  # LLM 系統提示
    tools: List[str]
    condition: Optional[str]  # 條件表達式
    settings: Dict[str, str]  # TOML [settings] 區塊
```

### 3.7 Solvability 分類器 (機器學習)

```python
class SolvabilityClassifier:
    """可解決性 ML 模型"""

    def __init__(self, model_path: str):
        self.model = joblib.load(model_path)  # scikit-learn RandomForest
        self.featurizer = IssueFeaturizer()

    def featurize(self, issue: Issue) -> np.ndarray:
        """提取特徵"""
        features = [
            len(issue.title),                    # 標題長度
            len(issue.body),                     # 描述長度
            len(issue.labels),                   # 標籤數
            issue.has_reproduction_steps(),      # 有重現步驟？
            issue.has_error_message(),           # 有錯誤消息？
            issue.has_code_sample(),             # 有代碼示例？
        ]
        return np.array(features).reshape(1, -1)

    def predict(self, features: np.ndarray) -> float:
        """預測可解決性分數 (0-1)"""
        return self.model.predict_proba(features)[0][1]

class Featurizer:
    """特徵工程"""
    def extract_keywords(self, text: str) -> List[str]:
        # NLP 預處理
        pass
```

---

## 4. 啟動流程

### 4.1 應用啟動序列

```
1. 配置加載
   └─ config/settings.yaml
   └─ config/providers.yaml
   └─ .env 密鑰

2. 初始化層
   ├─ LLM 提供商連接
   │  └─ Claude API / Groq API / Anthropic
   ├─ 運行時環境
   │  └─ Modal / E2B / Runloop 連接
   ├─ 數據庫連接
   │  └─ PostgreSQL 或 SQLite
   └─ 企業集成
      ├─ GitHub Webhook 監聽
      ├─ Slack 應用程式
      └─ Jira 實例

3. 代理控制器初始化
   ├─ 加載工具 (24 工具註冊)
   ├─ 初始化內存
   └─ 啟動事件監聽

4. 服務啟動
   ├─ REST API 伺服器 (FastAPI)
   │  └─ POST /agent/run
   │  └─ POST /webhook/github
   │  └─ POST /webhook/slack
   ├─ Webhook 監聽器
   └─ Cron 計劃任務

5. 代理準備完成
   └─ 等待輸入信號
```

### 4.2 代碼示例

```python
# main.py
import asyncio
from openhands import AgentController, LLMProvider, ToolRegistry, Memory

async def main():
    # 1. 載入配置
    config = Config.from_file("config/settings.yaml")

    # 2. 初始化 LLM
    llm = AnthropicProvider(api_key=config.anthropic_key)

    # 3. 初始化運行時
    runtime = E2BRuntime(api_key=config.e2b_key)

    # 4. 註冊工具
    tools = ToolRegistry()
    tools.register(ShellTool(runtime))
    tools.register(FileOperationsTool(runtime))
    tools.register(WebSearchTool(config.search_api_key))
    tools.register(GitHubTool(config.github_token))

    # 5. 初始化內存
    memory = ConversationMemory(max_tokens=8000)

    # 6. 啟動控制器
    controller = AgentController(llm=llm, tools=tools, memory=memory)

    # 7. 運行循環
    result = await controller.run_loop(max_iterations=100)
    print(f"任務完成: {result}")

if __name__ == "__main__":
    asyncio.run(main())
```

---

## 5. 資料流 ASCII 圖

### 5.1 主控制循環

```
┌─────────────────────────────────────────────────────────────────┐
│                     AgentController Main Loop                    │
└─────────────────────────────────────────────────────────────────┘

           ┌───────────────┐
           │  START INPUT  │  (user message, webhook event, etc.)
           └────────┬──────┘
                    │
                    ▼
         ┌──────────────────────┐
         │  Build Context       │
         │  ├─ Conv History     │
         │  ├─ Task Memory      │
         │  └─ Tool Descriptions
         └─────────┬────────────┘
                   │
                   ▼
      ┌────────────────────────────┐
      │    LLM Provider Predict    │  (Claude/Groq/etc)
      │    ├─ Input: Context      │
      │    ├─ Tools: [Tool...]    │
      │    └─ Output: Action      │
      └────────┬───────────────────┘
               │
         ┌─────┴─────┐
         │           │
    ┌────▼───┐  ┌────▼────┐
    │ Tool   │  │ Message  │
    │ Action │  │ Action   │
    └────┬───┘  └────┬─────┘
         │           │
    ┌────▼───────────▼────┐
    │  Execute Action     │
    │  ├─ Run Shell       │
    │  ├─ Read File       │
    │  ├─ Search Web      │
    │  └─ ... (24 Tools)  │
    └────┬────────────────┘
         │
         ▼
    ┌──────────────────┐
    │ Get Result       │
    │ (output/error)   │
    └────┬─────────────┘
         │
         ▼
    ┌──────────────────────┐
    │ Update Memory        │
    │ ├─ Add to History    │
    │ ├─ Update Summary    │
    │ └─ Check Stuck?      │
    └────┬─────────────────┘
         │
    ┌────┴──────────┐
    │               │
┌───▼──┐    ┌──────▼─────┐
│STUCK?│    │   STOP?     │
└───┬──┘    └──────┬──────┘
    │YES           │YES
    │              │
    ▼              ▼
┌────────┐   ┌────────────┐
│Recover │   │  RETURN    │
└────┬───┘   │  RESULT    │
     │       └────────────┘
     │
     └──► (retry)
```

### 5.2 GitHub Issue 解決流程

```
┌────────────────────────────────────┐
│  GitHub Webhook Event: Issue Open   │
└─────────┬──────────────────────────┘
          │
          ▼
    ┌──────────────────┐
    │  Webhook Handler │
    │  Parse Payload   │
    └────────┬─────────┘
             │
             ▼
    ┌──────────────────────┐
    │ GithubManager        │
    │ fetch_issue(#123)    │
    └────────┬─────────────┘
             │
             ▼
    ┌─────────────────────────────┐
    │ SolvabilityClassifier       │
    │ can_solve(issue) ?          │
    │ ├─ Featurize                │
    │ ├─ ML Predict (score)       │
    │ └─ Return bool (>0.7)       │
    └────────┬────────────────────┘
             │
        ┌────┴──────────┐
        │               │
     YES│               │NO
       ▼               ▼
   ┌────────┐     ┌──────────┐
   │Analyze │     │ Skip     │
   │Issue   │     │ (Label)  │
   └────┬───┘     └──────────┘
        │
        ▼
   ┌──────────────────────┐
   │ Agent Loop (100 iter)│
   │ ├─ Understand Issue  │
   │ ├─ Reproduce Error   │
   │ ├─ Write Fix         │
   │ ├─ Test Fix          │
   │ └─ Create PR         │
   └────┬─────────────────┘
        │
        ▼
   ┌──────────────────┐
   │ GithubManager    │
   │ create_pr()      │
   │ Link to Issue    │
   └────┬─────────────┘
        │
        ▼
   ┌──────────────┐
   │ Notify User  │
   │ (Slack/PR)   │
   └──────────────┘
```

### 5.3 多提供商 LLM 路由

```
┌────────────────────────────┐
│   Agent Predict Request    │
│   context + tools          │
└─────────┬──────────────────┘
          │
          ▼
    ┌──────────────────────┐
    │ Router Policy        │
    │ ├─ Cost (speed)      │
    │ ├─ Model Capability  │
    │ └─ Load Balance      │
    └────────┬─────────────┘
             │
        ┌────┼────┬────────┐
        │    │    │        │
    ┌───▼──┐ │ ┌──▼───┐ ┌──▼────┐
    │Groq  │ │ │Claude│ │Gemini │
    │(fast)│ │ │(power)
    │      │ │ │      │ │ (cache
    └───┬──┘ │ └──┬───┘ └──┬─────┘
        │    │    │        │
        │    │    │        │
    ┌───▼────▼────▼────────▼───┐
    │  Parallel Execution      │
    │  (or sequential fallback) │
    └───┬──────────────────────┘
        │
        ▼
    ┌──────────────────────┐
    │ Select Best Response │
    │ (fastest / best)     │
    └────┬─────────────────┘
         │
         ▼
    ┌──────────────┐
    │ Return Action│
    └──────────────┘
```

---

## 6. 子系統清單

### P0 - 核心基礎 (必要系統)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **AgentController** | `core/agent_controller.py` | 主狀態機 | ✅ Production |
| **LLM Provider** | `core/llm/provider.py` | 多提供商抽象 | ✅ Production |
| **Tool Registry** | `tools/registry.py` | 工具管理 | ✅ Production |
| **Memory System** | `core/memory/` | 對話+語義記憶 | ✅ Production |
| **Runtime Env** | `third_party/runtime/impl/` | 容器化執行 (E2B/Modal) | ✅ Production |
| **Shell Tool** | `tools/shell.py` | 命令行執行 | ✅ Production |
| **File I/O** | `tools/file_operations.py` | 文件讀寫編輯 | ✅ Production |

### P1 - 企業集成 (核心價值)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **GitHub Manager** | `enterprise/integrations/github/` | Issue/PR 自動化 | ✅ Production |
| **Solvability ML** | `enterprise/solvability/models/` | 可解決性分類 | ✅ Production |
| **Slack Manager** | `enterprise/integrations/slack/` | Slack 通知與命令 | ✅ Production |
| **Jira Manager** | `enterprise/integrations/jira/` | Jira 工單自動化 | ✅ Production |
| **Webhook Handler** | `enterprise/integrations/*/` | Webhook 事件 | ✅ Production |
| **Approval Gate** | `approval/approval_gate.py` | 人類審批工作流 | ✅ Production |
| **Cost Tracking** | `enterprise/database/` | 費用與令牌計數 | ✅ Production |

### P2 - 高級特性 (可選增強)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **Hands Workflow** | `hands/engine.py` | 多階段 TOML 工作流 | ✅ Production |
| **Cron Scheduler** | `scheduler/scheduler.py` | 計劃任務 | ✅ Production |
| **GitLab Integration** | `enterprise/integrations/gitlab/` | GitLab 適配 | ✅ Production |
| **Linear Integration** | `enterprise/integrations/linear/` | Linear 工作流 | ✅ Production |
| **Vision Tool** | `tools/vision.py` | 圖像分析 | ✅ Production |
| **Code Generation** | `tools/code_generation.py` | AI 代碼生成 | ✅ Production |
| **Browser Tool** | `tools/browser.py` | 瀏覽器自動化 | ✅ Production |
| **Stripe Integration** | `enterprise/integrations/stripe_service.py` | 計費系統 | ✅ Production |
| **Database Migrations** | `enterprise/database/migrations/` | 50+ Alembic 遷移 | ✅ Production |

### P3 - 監測與運維 (非關鍵)

| 子系統 | 檔案 | 說明 | 狀態 |
|-------|------|------|------|
| **Stuck Detection** | `core/agent_controller.py` | 卡死檢測與恢復 | ✅ Production |
| **Logging & Monitoring** | `core/logger.py` | 結構化日誌 | ✅ Production |
| **Test Suite** | `tests/` | 700+ 單元測試 | ✅ Production |

---

## 7. 核心交互流程

### 7.1 典型 Issue 解決流程

```
1. GitHub Webhook 到達
   └─ POST /webhook/github → GithubManager.handle_webhook()

2. Issue 提取與分類
   ├─ GithubManager.fetch_issue(#123)
   ├─ SolvabilityClassifier.can_solve(issue) → bool
   └─ 如果 NO: 標籤為 "needs-investigation" 返回

3. 代理分析與修復迴圈 (max 100 步)
   ├─ AgentController.run_loop()
   │  ├─ 第 1-10 步: 理解問題
   │  │  └─ 讀取 Issue 內容 + 相關代碼
   │  ├─ 第 11-30 步: 重現 Bug
   │  │  └─ 編寫測試用例 / 運行容器
   │  ├─ 第 31-60 步: 開發修復
   │  │  └─ 編輯代碼 + 測試
   │  └─ 第 61-100 步: 驗證 & PR
   │     └─ 創建 PR + 鏈接到 Issue

4. 人類批准 (可選)
   ├─ ApprovalGate 發送通知到 Telegram/Email
   ├─ 等待人類批准 (timeout 24h)
   └─ 如果批准: 自動合併 PR

5. 完成並通知
   └─ Slack 發送完成通知
```

### 7.2 LLM 決策邏輯

```
Input:
  context = "Issue #123: Database migration fails"
  tools = [ShellTool, FileOps, CodeGen, WebSearch, ...]

↓ (Claude API Call)

Output: Decision
  {
    "type": "tool_use",
    "name": "shell",
    "input": {
      "command": "cd repo && npm test"
    }
  }

↓ (Execution)

Result:
  {
    "status": "error",
    "output": "Test failed: migration timeout"
  }

↓ (Update Memory & Retry)

Next iteration uses:
  - Previous output
  - Updated context
  - Same tool set
```

---

## 8. 技術亮點與設計模式

### 8.1 設計模式應用

1. **觀察者模式** (Webhook)
   - GitHub/Slack/Jira Webhook 觸發事件
   - Manager 類觀察並響應

2. **策略模式** (LLM Router)
   - 多個 LLM 提供商互相切換
   - 運行時選擇最佳提供商

3. **責任鏈** (工具執行)
   - Tool 請求通過 Registry
   - 動態檢查權限與日誌

4. **狀態機** (Agent Loop)
   - IDLE → THINKING → EXECUTING → DONE
   - 支援卡死偵測與恢復

### 8.2 容錯機制

- **Stuck Detection**: 檢測最後 5 步是否重複
- **Webhook Retry**: 失敗自動重試 3 次 (exponential backoff)
- **LLM Fallback**: Claude 失敗 → Groq 備用
- **Database Transaction**: Alembic 自動回滾失敗遷移

### 8.3 可擴展性

- **多租戶** (企業版): 每用戶獨立配置
- **自定義工具**: ToolRegistry 支援動態註冊
- **自定義工作流**: Hands TOML 引擎
- **提供商無關**: LLMProvider 抽象支援任何 API

---

## 9. 與 Clawtex-Core 的對標

| 功能 | OpenHands | Clawtex-Core |
|-----|-----------|--------------|
| LLM 多提供商 | ✅ (Claude/Groq/Gemini) | ✅ (6 提供商 + 路由) |
| 工具系統 | ✅ 24 工具 | ✅ 24 工具 |
| 內存系統 | ✅ Conv + Summary | ✅ Memory Store + Recall |
| 企業集成 | ✅ 15+ SaaS | ✅ (Telegram/Email/HTTP) |
| 工作流引擎 | ✅ Hands (TOML) | ✅ Hands (TOML) |
| 卡死檢測 | ✅ 內置 | ✅ (可添加) |
| 計費追蹤 | ✅ PostgreSQL | ✅ SQLite (costs.db) |
| 多語言支援 | ✅ (EN) | ✅ (繁體中文優先) |
| Cron 調度 | ✅ 內置 | ✅ (6 個工作 cron) |

---

## 10. 部署與配置建議

### 10.1 最小化部署 (開發環境)

```yaml
# config/settings.yaml
llm_provider: anthropic
llm_model: claude-opus-4-6
runtime: e2b
memory_backend: sqlite

# 禁用企業特性
enable_github_integration: false
enable_slack_integration: false
enable_webhooks: false
```

### 10.2 生產部署 (企業版)

```yaml
# config/settings.yaml
llm_provider: classifier  # 自動路由
llm_models:
  - anthropic: claude-opus-4-6
  - groq: mixtral-8x7b
  - gemini: gemini-2.0-pro

runtime: modal  # 分布式容器

database:
  backend: postgresql
  host: prod-db.example.com
  pool_size: 20

integrations:
  github:
    enabled: true
    webhook_secret: ${GITHUB_WEBHOOK_SECRET}
  slack:
    enabled: true
    bot_token: ${SLACK_BOT_TOKEN}
  jira:
    enabled: true
    instance_url: https://company.atlassian.net

approval:
  enabled: true
  channel: slack  # 或 telegram
  timeout_hours: 24
```

---

## 結論

OpenHands 是一個**企業級代理框架**，展示了如何構建：
- 強大的 LLM 控制循環 (AgentController)
- 可靠的工具執行環境 (E2B/Modal)
- 緊密的 SaaS 整合 (GitHub/Slack/Jira)
- 智能優先級排序 (Solvability ML)
- 生產級別的可靠性 (Webhook、批准、重試機制)

該架構可直接參考為 Clawtex-Core 的**企業版升級方向**。
