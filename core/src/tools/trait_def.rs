//! `Tool` trait — the future plugin surface.
//!
//! Phantom's existing dispatcher (`tools::execute`) is a 1500-line
//! match statement that hands every named tool off to a free function
//! in this module's siblings. That works fine for the in-process
//! built-ins, but it's a closed enum: third-party plugins, future
//! channel adapters (OpenClaw-style), and the cluster RPC bridge all
//! want to register *new* tool names without recompiling phantom-mesh.
//!
//! [`Tool`] is the smallest abstraction that supports those use
//! cases. The two reference impls in this file —
//! [`BuiltinTool`] and [`McpToolWrapper`] — show how to wrap the
//! existing dispatch paths so future plugin sources can be added
//! alongside without breaking anything.
//!
//! Modeled on Codex's `ToolHandler` trait
//! (`references/codex/codex-rs/core/src/tools/registry.rs:36-90`)
//! and claurst-rust's `Tool` trait
//! (`references/claurst-rust/src-rust/crates/tools/src/lib.rs:333`).
//!
//! ## Why it's a scaffold
//!
//! The existing `tools::execute` already routes MCP-first then falls
//! through to built-ins, so end-to-end behaviour doesn't change with
//! this trait. The trait exists so future work — plugin loader, hook
//! lifecycle, cross-peer tool routing — has a single shape to talk
//! to. Migrating the giant match to `dyn Tool` dispatch is a separate
//! follow-up; doing both at once would be a big invasive churn for
//! marginal user-visible value.

use async_trait::async_trait;
use serde_json::Value;

use crate::config::ToolsConfig;

/// Read-only context handed to every tool invocation. New fields can
/// be added without breaking impls because tools borrow what they
/// need by name.
#[derive(Clone)]
pub struct ToolContext<'a> {
    pub config: &'a ToolsConfig,
}

/// Universal tool surface. Every named tool — built-in, MCP, future
/// 3P plugin — implements this. The agent loop interacts only with
/// `&dyn Tool`, so adding a new source amounts to writing a struct +
/// impl + register.
#[async_trait]
pub trait Tool: Send + Sync {
    /// The name the LLM calls. Built-ins use bare names (`"shell"`,
    /// `"file_read"`); MCP tools are namespaced (`"<server>_<tool>"`)
    /// to avoid collisions.
    fn name(&self) -> &str;

    /// OpenAI-style `{"type":"function","function":{...}}` envelope.
    /// Splice straight into the `tools=[...]` field of the LLM
    /// request. Returning `None` is allowed for tools that exist for
    /// internal use only and shouldn't be advertised to the model.
    fn schema(&self) -> Option<Value>;

    /// Invoke the tool. Tools must be cancel-aware; for now they
    /// finish their work even if the caller's interrupt fires
    /// (see `agent::AgentRuntime::is_interrupted`). A future
    /// extension will pass a `CancellationToken` into the context.
    async fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> String;
}

/// Wraps a built-in dispatcher entry so the in-process tools can be
/// presented through the trait alongside MCP / plugin tools.
///
/// Internally calls `tools::execute(name, args, config)` — the same
/// path the agent loop already uses — so behaviour is identical to
/// the legacy match dispatch.
pub struct BuiltinTool {
    name: String,
}

impl BuiltinTool {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl Tool for BuiltinTool {
    fn name(&self) -> &str { &self.name }

    fn schema(&self) -> Option<Value> {
        super::schema(&self.name)
    }

    async fn call(&self, args: &Value, ctx: &ToolContext<'_>) -> String {
        super::execute(&self.name, args, ctx.config).await
    }
}

/// Wraps the MCP registry so each prefixed external tool
/// (`<server>_<tool>`) appears as its own `dyn Tool`.
///
/// Schema is captured at construction time from the registry's
/// cached tool list; future schema changes (rare) require a
/// re-registration.
pub struct McpToolWrapper {
    prefixed_name: String,
    schema: Value,
}

impl McpToolWrapper {
    pub fn new(prefixed_name: impl Into<String>, schema: Value) -> Self {
        Self {
            prefixed_name: prefixed_name.into(),
            schema,
        }
    }
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str { &self.prefixed_name }

    fn schema(&self) -> Option<Value> { Some(self.schema.clone()) }

    async fn call(&self, args: &Value, _ctx: &ToolContext<'_>) -> String {
        match crate::mcp_client::global() {
            Some(reg) => reg
                .dispatch(&self.prefixed_name, args)
                .await
                .unwrap_or_else(|| format!(
                    "[mcp] no client matched prefix for tool '{}'",
                    self.prefixed_name
                )),
            None => format!(
                "[mcp] registry not initialised; cannot call '{}'",
                self.prefixed_name
            ),
        }
    }
}

/// Build the full live tool list: every built-in name from
/// `all_tool_names()` wrapped in [`BuiltinTool`], plus every MCP
/// tool from `mcp_client::global().tool_defs()` wrapped in
/// [`McpToolWrapper`]. Future plugin sources will append here.
///
/// Returned as `Box<dyn Tool>` so the caller doesn't have to
/// distinguish sources.
pub async fn live_tools() -> Vec<Box<dyn Tool>> {
    let mut out: Vec<Box<dyn Tool>> = super::all_tool_names()
        .into_iter()
        .map(|n| Box::new(BuiltinTool::new(n)) as Box<dyn Tool>)
        .collect();

    if let Some(reg) = crate::mcp_client::global() {
        for def in reg.tool_defs().await {
            if let Some(name) = def["function"]["name"].as_str() {
                out.push(Box::new(McpToolWrapper::new(name.to_string(), def)));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn builtin_tool_round_trips_through_execute() {
        // `todo_list` is a no-side-effect built-in available on every
        // platform, so we can drive it through the trait without
        // standing up the full agent runtime.
        let cfg = ToolsConfig::default();
        let ctx = ToolContext { config: &cfg };
        let tool = BuiltinTool::new("todo_list");
        assert_eq!(tool.name(), "todo_list");
        assert!(tool.schema().is_some());
        // Output shape varies by underlying todo state; we only
        // assert that the call returned *something*.
        let out = tool.call(&json!({}), &ctx).await;
        assert!(!out.is_empty());
    }

    #[tokio::test]
    async fn mcp_wrapper_returns_clear_error_when_registry_missing() {
        let cfg = ToolsConfig::default();
        let ctx = ToolContext { config: &cfg };
        let tool = McpToolWrapper::new(
            "fake_server_some_tool",
            json!({"type":"function","function":{"name":"fake_server_some_tool"}}),
        );
        let out = tool.call(&json!({}), &ctx).await;
        // We don't try to spawn an MCP server here — only verify the
        // wrapper's error path produces a useful message rather than
        // panicking. (init_global may have been called by another
        // test; either branch is acceptable.)
        assert!(out.contains("fake_server_some_tool"));
    }

    #[test]
    fn live_tools_is_object_safe() {
        // Compile-time check: `Box<dyn Tool>` round-trips through a
        // Vec without trait-object errors. Refactors that break
        // object safety (e.g., adding a generic method to Tool) will
        // fail to compile this test.
        fn _accepts(_: Vec<Box<dyn Tool>>) {}
    }
}
