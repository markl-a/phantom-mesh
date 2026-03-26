//! Test harnesses — CoreHarness, ApiHarness, SystemHarness.

use std::path::PathBuf;
use std::sync::Arc;

use phantom_mesh::providers::mock::MockProvider;
use phantom_mesh::providers::ProviderRouter;
use phantom_mesh::{AgentRuntime, AgentResult, LlmRouter, ToolRegistry};
use phantom_mesh::tools::SecurityConfig;

use super::fixtures;

/// In-process test harness — no HTTP server.
/// Tests agent runtime + tool execution + MockProvider.
pub struct CoreHarness {
    pub agent_runtime: Arc<AgentRuntime>,
    pub tool_registry: Arc<ToolRegistry>,
    pub llm_router: Arc<LlmRouter>,
    provider: MockProvider,
    pub _temp_dir: tempfile::TempDir,
}

impl CoreHarness {
    pub fn builder() -> CoreHarnessBuilder {
        CoreHarnessBuilder {
            provider: None,
        }
    }

    /// Run the master agent with a prompt.
    pub async fn run_agent(&self, prompt: &str) -> anyhow::Result<AgentResult> {
        self.agent_runtime.run(
            "master",
            prompt,
            &[],
            &self.llm_router,
            &self.tool_registry,
            None,
        ).await
    }

    /// Run the master agent with conversation history.
    pub async fn run_agent_with_history(
        &self,
        prompt: &str,
        history: &[phantom_mesh::ChatMessage],
    ) -> anyhow::Result<AgentResult> {
        self.agent_runtime.run(
            "master",
            prompt,
            history,
            &self.llm_router,
            &self.tool_registry,
            None,
        ).await
    }

    /// Get the number of LLM calls made.
    pub fn provider_call_count(&self) -> usize {
        self.provider.call_count()
    }

    /// Get a specific LLM call record.
    pub fn provider_call(&self, index: usize) -> Option<phantom_mesh::providers::mock::MockCallRecord> {
        self.provider.get_call(index)
    }

    /// Path to the temporary workspace.
    pub fn workspace_path(&self) -> PathBuf {
        self._temp_dir.path().join("workspace")
    }

    /// Execute a tool directly by name with the given JSON args.
    pub async fn run_tool(&self, name: &str, args: serde_json::Value) -> anyhow::Result<phantom_mesh::tools::ToolResult> {
        self.tool_registry.execute_tool(name, args).await
    }
}

pub struct CoreHarnessBuilder {
    provider: Option<MockProvider>,
}

impl CoreHarnessBuilder {
    pub fn provider(mut self, provider: MockProvider) -> Self {
        self.provider = Some(provider);
        self
    }

    pub async fn build(self) -> CoreHarness {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let workspace = temp_dir.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();

        // Write test agents.toml
        let config_path = fixtures::write_test_agents_toml(temp_dir.path());

        // Create MockProvider — Clone shares call tracking via Arc<Mutex>
        let mock = self.provider.unwrap_or_else(|| MockProvider::fixed("default test response"));
        let tracking_ref = mock.clone(); // shares call_log via Arc

        // Build LlmRouter with the mock
        let mut pr = ProviderRouter::empty();
        pr.register_provider("mock", Box::new(mock));
        let llm_router = Arc::new(LlmRouter::from_router(pr));

        // Create ToolRegistry with workspace pointing to temp dir
        let security = SecurityConfig {
            workspace_dir: workspace.to_string_lossy().to_string(),
            workspace_only: false,
            allowed_commands: vec![
                "echo".to_string(),
                "pwd".to_string(),
                "cd".to_string(),
                "ls".to_string(),
                "dir".to_string(),
                "cat".to_string(),
                "set".to_string(),
                "export".to_string(),
                "env".to_string(),
                "printenv".to_string(),
            ],
            ..Default::default()
        };
        let tool_registry = Arc::new(ToolRegistry::new(security));

        // Create AgentRuntime from test config
        let agent_runtime = Arc::new(
            AgentRuntime::new(config_path.to_str().unwrap())
                .expect("Failed to create AgentRuntime from test config")
        );

        CoreHarness {
            agent_runtime,
            tool_registry,
            llm_router,
            provider: tracking_ref,
            _temp_dir: temp_dir,
        }
    }
}
