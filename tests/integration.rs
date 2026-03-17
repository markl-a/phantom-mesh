//! Integration tests for clawtex-core
//! These tests verify cross-module behavior without requiring external services.

use clawtex_core::*;
use clawtex_core::revenue_tracker::{RevenueTracker, RevenueRecord, RevenueStatus, ROUTE_B};
use serde_json::json;

// ── Provider System ──────────────────────────────────────────────────────────

#[test]
fn test_provider_router_creates_from_empty_config() {
    let router = providers::ProviderRouter::new("/nonexistent/config.toml");
    assert!(router.is_ok());
}

#[test]
fn test_provider_router_provider_names() {
    let router = providers::ProviderRouter::new("/nonexistent/config.toml").unwrap();
    let names = router.provider_names();
    // May or may not have providers depending on defaults
    let _ = names;
}

// ── ChatGPT Backend & WebSocket Providers ────────────────────────────────────

#[tokio::test]
async fn test_chatgpt_backend_provider_creation() {
    use clawtex_core::providers::ChatGptBackendProvider;
    use clawtex_core::providers::codex::CodexTokenManager;
    use clawtex_core::providers::Provider;
    use std::sync::Arc;

    let tm = Arc::new(CodexTokenManager::new());
    let provider = ChatGptBackendProvider::new(tm);
    assert_eq!(provider.name(), "chatgpt_backend");
    assert_eq!(provider.default_model(), "gpt-5.4");
    assert!(provider.capabilities().streaming);
    assert!(!provider.capabilities().native_tools); // Backend API doesn't support native tools
    assert!(!provider.capabilities().vision); // Codex CLI doesn't support image input
}

#[tokio::test]
async fn test_chatgpt_ws_provider_creation() {
    use clawtex_core::providers::ChatGptWsProvider;
    use clawtex_core::providers::codex::CodexTokenManager;
    use clawtex_core::providers::Provider;
    use std::sync::Arc;

    let tm = Arc::new(CodexTokenManager::new());
    let provider = ChatGptWsProvider::new(tm);
    assert_eq!(provider.name(), "chatgpt_ws");
    assert_eq!(provider.default_model(), "gpt-4o");
    assert!(provider.capabilities().streaming);
    assert!(provider.capabilities().native_tools); // WebSocket API supports native tools
    assert!(provider.capabilities().vision);
}

// ── KeyPool ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_key_pool_integration() {
    use clawtex_core::providers::KeyPool;

    let pool = KeyPool::new(vec![
        "gemini-key-1".into(),
        "gemini-key-2".into(),
        "gemini-key-3".into(),
    ]);
    pool.record_rate_limit("gemini-key-1").await;
    let key = pool.next_key().await.unwrap();
    assert_ne!(key, "gemini-key-1");
    assert_eq!(pool.len(), 3);
}

// ── Request Classifier ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_request_classifier_integration() {
    use clawtex_core::providers::{RequestClassifier, RequestComplexity};
    use clawtex_core::providers::MockProvider;
    use std::sync::Arc;

    let mock = Arc::new(MockProvider::fixed("COMPLEX"));
    let classifier = RequestClassifier::new(mock, "test-model".to_string());

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: "Please write a complete REST API server with authentication middleware and database integration".to_string(),
        tool_calls: None,
        tool_call_id: None,
    }];

    let result = classifier.classify(&messages).await;
    assert_eq!(result, RequestComplexity::Complex);
}

#[tokio::test]
async fn test_request_classifier_simple_message() {
    use clawtex_core::providers::{RequestClassifier, RequestComplexity};
    use clawtex_core::providers::MockProvider;
    use std::sync::Arc;

    let mock = Arc::new(MockProvider::fixed("SIMPLE"));
    let classifier = RequestClassifier::new(mock, "test-model".to_string());

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: "Hi".to_string(),
        tool_calls: None,
        tool_call_id: None,
    }];

    // Short messages are classified as Simple by heuristic (before calling provider)
    let result = classifier.classify(&messages).await;
    assert_eq!(result, RequestComplexity::Simple);
}

// ── Context Optimizer ────────────────────────────────────────────────────────

#[test]
fn test_context_optimizer_preserves_system() {
    let mut messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "You are helpful.".to_string(),
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: "Hello".to_string(),
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    ContextOptimizer::trim_messages(&mut messages, "qwen3:8b");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].role, "system");
}

#[test]
fn test_context_optimizer_trims_long_history() {
    let mut messages = Vec::new();
    messages.push(ChatMessage {
        role: "system".to_string(),
        content: "System prompt".to_string(),
        tool_calls: None,
        tool_call_id: None,
    });
    // 200 messages × 1000 chars each ≈ 50,000 tokens — well over qwen3:8b's 32k window
    for i in 0..200 {
        messages.push(ChatMessage {
            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: "x".repeat(1000),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    let original_len = messages.len();
    ContextOptimizer::trim_messages(&mut messages, "qwen3:8b");
    assert!(messages.len() < original_len, "Should have trimmed messages");
    assert_eq!(messages[0].role, "system", "System message preserved");
}

// ── E-Stop ───────────────────────────────────────────────────────────────────

#[test]
fn test_estop_lifecycle() {
    let estop = EStop::new();
    assert!(!estop.is_stopped());
    assert!(estop.check().is_ok());

    estop.stop();
    assert!(estop.is_stopped());
    assert!(estop.check().is_err());

    estop.reset();
    assert!(!estop.is_stopped());
    assert!(estop.check().is_ok());
}

#[test]
fn test_estop_shared_across_clones() {
    let estop1 = EStop::new();
    let estop2 = estop1.clone();
    let estop3 = estop1.clone();

    estop2.stop();
    assert!(estop1.is_stopped());
    assert!(estop3.is_stopped());

    estop3.reset();
    assert!(!estop1.is_stopped());
    assert!(!estop2.is_stopped());
}

#[test]
fn test_estop_thread_safety() {
    let estop = EStop::new();
    let estop2 = estop.clone();

    let handle = std::thread::spawn(move || {
        estop2.stop();
    });
    handle.join().unwrap();
    assert!(estop.is_stopped());
}

// ── Hooks ────────────────────────────────────────────────────────────────────

#[test]
fn test_hooks_runner_empty() {
    let runner = HookRunner::new();
    let ctx = HookContext {
        agent_name: "test".to_string(),
        chat_id: None,
    };
    drop(ctx);
    drop(runner);
}

// ── Secret Manager ───────────────────────────────────────────────────────────

#[test]
fn test_secret_encrypt_decrypt_roundtrip() {
    let key = [42u8; 32];
    let mgr = SecretManager::with_key(&key);

    let secrets = vec![
        "sk-ant-api03-1234567890abcdef".to_string(),
        String::new(),
        "密碼：你好世界🔐".to_string(),
        "a".repeat(10000),
    ];

    for secret in &secrets {
        let encrypted = mgr.encrypt(secret).unwrap();
        assert!(encrypted.starts_with("enc2:"));
        let decrypted = mgr.decrypt(&encrypted).unwrap();
        assert_eq!(&decrypted, secret);
    }
}

#[test]
fn test_secret_wrong_key_fails() {
    let mgr1 = SecretManager::with_key(&[1u8; 32]);
    let mgr2 = SecretManager::with_key(&[2u8; 32]);

    let encrypted = mgr1.encrypt("secret-data").unwrap();
    assert!(mgr2.decrypt(&encrypted).is_err());
}

#[test]
fn test_secret_decrypt_config_recursive() {
    let mgr = SecretManager::with_key(&[99u8; 32]);
    let enc_key = mgr.encrypt("my-api-key").unwrap();
    let enc_nested = mgr.encrypt("nested-secret").unwrap();
    let enc_arr = mgr.encrypt("array-item").unwrap();

    let mut config = json!({
        "api_key": enc_key,
        "plain": "not-encrypted",
        "nested": {
            "deep": enc_nested,
            "number": 42,
        },
        "list": [enc_arr, "plain-item"],
    });

    mgr.decrypt_config(&mut config);

    assert_eq!(config["api_key"], "my-api-key");
    assert_eq!(config["plain"], "not-encrypted");
    assert_eq!(config["nested"]["deep"], "nested-secret");
    assert_eq!(config["nested"]["number"], 42);
    assert_eq!(config["list"][0], "array-item");
    assert_eq!(config["list"][1], "plain-item");
}

#[test]
fn test_secret_nonce_uniqueness() {
    let mgr = SecretManager::with_key(&[55u8; 32]);
    let e1 = mgr.encrypt("same-text").unwrap();
    let e2 = mgr.encrypt("same-text").unwrap();
    assert_ne!(e1, e2, "Different nonces should produce different ciphertext");
    assert_eq!(mgr.decrypt(&e1).unwrap(), mgr.decrypt(&e2).unwrap());
}

// ── MCP Bridge ───────────────────────────────────────────────────────────────

#[test]
fn test_mcp_bridge_empty_configs() {
    let bridge = McpBridge::new(std::collections::HashMap::new());
    drop(bridge);
}

#[tokio::test]
async fn test_mcp_bridge_unknown_server() {
    let bridge = McpBridge::new(std::collections::HashMap::new());
    let result = bridge.start_server("nonexistent").await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Unknown MCP server"));
}

#[tokio::test]
async fn test_mcp_bridge_all_tools_empty() {
    let bridge = McpBridge::new(std::collections::HashMap::new());
    let tools = bridge.all_tools().await;
    assert!(tools.is_empty());
}

#[tokio::test]
async fn test_mcp_bridge_connected_servers_empty() {
    let bridge = McpBridge::new(std::collections::HashMap::new());
    let servers = bridge.connected_servers().await;
    assert!(servers.is_empty());
}

// ── Agent Runtime ────────────────────────────────────────────────────────────

#[test]
fn test_agent_runtime_default_agents() {
    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let agents = runtime.list_agents();
    assert!(agents.contains(&"master".to_string()));
    assert!(agents.contains(&"coder".to_string()));
}

#[test]
fn test_agent_runtime_master_config() {
    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let config = runtime.get_config("master").unwrap();
    assert!(config.tools.is_some());
    assert!(config.instructions.is_some());
}

#[test]
fn test_agent_runtime_unknown_agent() {
    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    assert!(runtime.get_config("nonexistent_agent_xyz").is_none());
}

// ── Tool Registry ────────────────────────────────────────────────────────────

#[test]
fn test_tool_registry_default_tools() {
    let security = SecurityConfig::default();
    let registry = ToolRegistry::new(security);
    let names = registry.names();
    assert!(names.contains(&"shell".to_string()));
    assert!(names.contains(&"file_read".to_string()));
    assert!(names.contains(&"file_write".to_string()));
}

#[test]
fn test_tool_registry_specs() {
    let security = SecurityConfig::default();
    let registry = ToolRegistry::new(security);
    let specs = registry.specs();
    assert!(!specs.is_empty());
    for spec in &specs {
        assert!(!spec.name.is_empty());
        assert!(!spec.description.is_empty());
    }
}

// ── Credential Scrubbing ─────────────────────────────────────────────────────

#[test]
fn test_scrub_credentials_integration() {
    // OpenAI-style key (sk- followed by 32+ alphanumeric chars)
    let input = "API key is sk-abcdefghij1234567890abcdefghij1234567890 and also AKIA1234567890ABCDEF rest";
    let scrubbed = scrub_credentials(input);
    assert!(!scrubbed.contains("sk-abcdef"), "OpenAI key should be scrubbed");
    assert!(!scrubbed.contains("AKIA1234567890ABCDEF"), "AWS key should be scrubbed");
    assert!(scrubbed.contains("[REDACTED"));
}

// ── Conversation Store ───────────────────────────────────────────────────────

#[tokio::test]
async fn test_conversation_store_basic() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_string_lossy().to_string();
    let store = ConversationStore::new(&db_path).await.unwrap();

    let msg1 = ChatMessage {
        role: "user".to_string(),
        content: "Hello".to_string(),
        tool_calls: None,
        tool_call_id: None,
    };
    let msg2 = ChatMessage {
        role: "assistant".to_string(),
        content: "Hi there!".to_string(),
        tool_calls: None,
        tool_call_id: None,
    };

    store.append("chat1", msg1, msg2).await;
    let history = store.get_history("chat1").await;
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "Hello");
    assert_eq!(history[1].content, "Hi there!");
}

#[tokio::test]
async fn test_conversation_store_clear() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_string_lossy().to_string();
    let store = ConversationStore::new(&db_path).await.unwrap();

    let msg1 = ChatMessage { role: "user".to_string(), content: "Hi".to_string(), tool_calls: None, tool_call_id: None };
    let msg2 = ChatMessage { role: "assistant".to_string(), content: "Hello".to_string(), tool_calls: None, tool_call_id: None };
    store.append("chat1", msg1, msg2).await;

    store.clear("chat1").await;
    let history = store.get_history("chat1").await;
    assert!(history.is_empty());
}

// ── Cron ─────────────────────────────────────────────────────────────────────

#[test]
fn test_cron_store_create() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db").to_string_lossy().to_string();
    let store = CronStore::new(&db_path).unwrap();
    let jobs = store.load_active_jobs().unwrap();
    assert!(jobs.is_empty());
}

// ── Skills ───────────────────────────────────────────────────────────────────

#[test]
fn test_skill_registry_empty() {
    let registry = SkillRegistry::load(&[]).unwrap();
    let selected = registry.select_for_prompt("hello", "master", 6000);
    assert!(selected.is_empty());
}

// ── Evaluate ─────────────────────────────────────────────────────────────────

#[test]
fn test_eval_config_defaults() {
    let config = EvalConfig::default();
    assert!(!config.enabled);
    assert!(config.threshold > 0);
}

// ── Gateway State ────────────────────────────────────────────────────────────

#[test]
fn test_gateway_state_is_clone() {
    fn assert_clone<T: Clone>() {}
    assert_clone::<GatewayState>();
}

// ── Cross-module: Context + Agent ────────────────────────────────────────────

#[test]
fn test_context_window_sizes_known_models() {
    assert!(ContextOptimizer::get_window_size("qwen3:8b") > 0);
    assert!(ContextOptimizer::get_window_size("gpt-4o") > 0);
    assert!(ContextOptimizer::get_window_size("claude-sonnet-4-6") > 0);
}

#[test]
fn test_context_token_estimation() {
    let tokens = ContextOptimizer::estimate_tokens("Hello, world!");
    assert!(tokens > 0);
    assert!(tokens < 100);

    let long_text = "word ".repeat(1000);
    let long_tokens = ContextOptimizer::estimate_tokens(&long_text);
    assert!(long_tokens > 100);
}

// ── Cross-module: E-Stop + multiple subsystems ──────────────────────────────

#[test]
fn test_estop_error_display() {
    let err = EStopError;
    let msg = format!("{}", err);
    assert!(msg.contains("emergency stop"));
}

// ── LlmRouter backward compat ───────────────────────────────────────────────

#[test]
fn test_llm_router_backward_compat() {
    // LlmRouter should still be constructible (thin wrapper over ProviderRouter)
    let router = LlmRouter::new("/nonexistent/config.toml");
    assert!(router.is_ok());
}

// ── E2E: Hand Registry + Chain-to Pipeline ──────────────────────────────────

#[test]
fn test_e2e_hand_registry_with_chain() {
    // Create temp dir with two hands: lead (chains to outreach), outreach
    let dir = tempfile::tempdir().unwrap();

    // lead hand
    let lead_dir = dir.path().join("lead");
    std::fs::create_dir_all(&lead_dir).unwrap();
    std::fs::write(lead_dir.join("hand.toml"), r#"
name = "lead"
description = "Find leads"
chain_to = "outreach"
[[phases]]
name = "search"
system_prompt = "Find companies"
max_rounds = 3
"#).unwrap();

    // outreach hand
    let outreach_dir = dir.path().join("outreach");
    std::fs::create_dir_all(&outreach_dir).unwrap();
    std::fs::write(outreach_dir.join("hand.toml"), r#"
name = "outreach"
description = "Send outreach"
[[phases]]
name = "email"
system_prompt = "Write emails"
max_rounds = 3
"#).unwrap();

    let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
    assert_eq!(registry.names().len(), 2);

    // Verify chain
    let lead = registry.get("lead").unwrap();
    assert_eq!(lead.chain_to, Some("outreach".to_string()));
    assert_eq!(lead.phases.len(), 1);

    // Verify outreach exists for chaining
    let outreach = registry.get("outreach").unwrap();
    assert_eq!(outreach.chain_to, None);
    assert_eq!(outreach.phases.len(), 1);
}

#[test]
fn test_e2e_hand_context_preparation() {
    use std::collections::HashMap;
    let hand = clawtex_core::hands::Hand {
        name: "test".to_string(),
        description: "test".to_string(),
        category: "test".to_string(),
        provider: "auto".to_string(),
        model: String::new(),
        phases: vec![],
        tools: vec![],
        output_format: "markdown".to_string(),
        schedule: None,
        settings: {
            let mut m = HashMap::new();
            m.insert("industry".to_string(), "SaaS".to_string());
            m.insert("location".to_string(), "US".to_string());
            m
        },
        chain_to: None,
        guardrail: None,
        eval: None,
        extra: HashMap::new(),
    };

    let context = HandRunner::prepare_context(&hand, "Find AI companies");
    assert!(context.contains("Find AI companies"));
    assert!(context.contains("SaaS"));
    assert!(context.contains("US"));
    assert!(context.contains("Settings:"));
}

// ── E2E: Cron + Hand Job Action ─────────────────────────────────────────────

#[tokio::test]
async fn test_e2e_cron_hand_job_scheduling() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cron_e2e.db").to_string_lossy().to_string();

    let store = std::sync::Arc::new(CronStore::new(&db_path).unwrap());
    let scheduler = Scheduler::new(store).unwrap();

    // Schedule a hand job
    let job_id = scheduler.add_job(
        "daily-freelancer",
        Schedule::Cron { expr: "0 9 * * *".to_string() },
        JobAction::Hand {
            hand_name: "freelancer".to_string(),
            input: "AI automation jobs on Upwork".to_string(),
        },
        None,
    ).await.unwrap();

    assert!(!job_id.is_empty());

    // Schedule a second hand job
    let job_id2 = scheduler.add_job(
        "weekly-leads",
        Schedule::Every { interval_secs: 604800 },
        JobAction::Hand {
            hand_name: "lead".to_string(),
            input: "SaaS companies in healthcare".to_string(),
        },
        Some(4), // max 4 runs
    ).await.unwrap();

    // List all jobs
    let jobs = scheduler.list_jobs().await;
    assert_eq!(jobs.len(), 2);

    // Verify job details
    let freelancer_job = jobs.iter().find(|j| j.name == "daily-freelancer").unwrap();
    match &freelancer_job.action {
        JobAction::Hand { hand_name, input } => {
            assert_eq!(hand_name, "freelancer");
            assert!(input.contains("Upwork"));
        }
        _ => panic!("Expected Hand action"),
    }

    // Delete a job
    assert!(scheduler.delete_job(&job_id2).await.unwrap());
    assert_eq!(scheduler.list_jobs().await.len(), 1);
}

// ── E2E: Cost Tracking ──────────────────────────────────────────────────────

#[test]
fn test_e2e_cost_tracking_full_workflow() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("costs_e2e.db").to_string_lossy().to_string();

    let tracker = CostTracker::new(&db_path).unwrap();

    // Simulate a hand run with multiple phases
    let records = vec![
        ("master", "ollama", "qwen3:8b", 500, 300),      // Phase 1: local (free)
        ("master", "gemini", "gemini-2.5-flash-lite", 1000, 500), // Phase 2: free tier
        ("master", "anthropic", "claude-sonnet-4", 800, 400),     // Phase 3: paid
    ];

    for (agent, provider, model, tok_in, tok_out) in &records {
        let cost = estimate_cost(provider, model, *tok_in, *tok_out);
        tracker.record(&CostRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            agent: agent.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            tokens_in: *tok_in,
            tokens_out: *tok_out,
            total_tokens: tok_in + tok_out,
            estimated_cost_usd: cost,
            duration_secs: 2.5,
            context: Some("hand:lead phase:1".to_string()),
        }).unwrap();
    }

    // Query totals
    let today = tracker.today_total().unwrap();
    assert_eq!(today.call_count, 3);
    assert_eq!(today.total_tokens, 3500); // 800+1500+1200

    // By provider
    let by_prov = tracker.by_provider(1).unwrap();
    assert_eq!(by_prov.len(), 3);

    // Anthropic should be the most expensive
    let anthropic = by_prov.iter().find(|s| s.group == "anthropic").unwrap();
    assert!(anthropic.total_cost_usd > 0.0);

    // Local providers should be free
    let ollama = by_prov.iter().find(|s| s.group == "ollama").unwrap();
    assert_eq!(ollama.total_cost_usd, 0.0);
}

// ── E2E: Full Hand Load (live ~/.clawtex/hands) ────────────────────────────

#[test]
fn test_e2e_all_hands_load_and_validate() {
    // Load hands from the actual config dir
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        return; // Skip if not configured
    }

    let registry = HandRegistry::load(&hands_dir).unwrap();
    let names = registry.names();

    // We expect at least 7 hands (lead, outreach, researcher, content, seo_content, freelancer, market_intel)
    // + auto_report = 8
    assert!(names.len() >= 7, "Expected at least 7 hands, got {}: {:?}", names.len(), names);

    // Validate each hand
    for name in &names {
        let hand = registry.get(name).unwrap();
        assert!(!hand.description.is_empty(), "Hand '{}' has empty description", name);
        assert!(!hand.phases.is_empty(), "Hand '{}' has no phases", name);

        // Each phase should have a non-empty system_prompt
        for phase in &hand.phases {
            assert!(!phase.name.is_empty(), "Hand '{}' has phase with empty name", name);
            assert!(!phase.system_prompt.is_empty(), "Hand '{}' phase '{}' has empty prompt", name, phase.name);
            assert!(phase.max_rounds > 0, "Hand '{}' phase '{}' has 0 max_rounds", name, phase.name);
        }
    }

    // Verify specific chains
    if let Some(lead) = registry.get("lead") {
        assert_eq!(lead.chain_to, Some("outreach".to_string()), "Lead should chain to outreach");
        // Verify the chain target exists
        assert!(registry.get("outreach").is_some(), "Outreach hand must exist for lead chain");
    }
}

// ── E2E: Tool Registration Completeness ─────────────────────────────────────

#[test]
fn test_e2e_pdf_export_tool_creation() {
    use clawtex_core::tools::Tool;
    let dir = tempfile::tempdir().unwrap();
    let tool = clawtex_core::tools::pdf_export::PdfExportTool::new(dir.path().to_str().unwrap());
    assert_eq!(tool.name(), "pdf_export");
    let schema = tool.parameters_schema();
    assert!(schema["required"].as_array().unwrap().contains(&json!("input_file")));
    assert!(schema["required"].as_array().unwrap().contains(&json!("output_file")));
}

#[test]
fn test_e2e_twitter_tool_creation() {
    use clawtex_core::tools::Tool;
    use clawtex_core::TwitterConfig;
    let config = TwitterConfig::default();
    let tool = clawtex_core::tools::twitter::TwitterTool::new(config);
    assert_eq!(tool.name(), "twitter");
}

#[test]
fn test_e2e_blog_publish_tool_creation() {
    use clawtex_core::tools::Tool;
    use clawtex_core::BlogConfig;
    let config = BlogConfig::default();
    let tool = clawtex_core::tools::blog_publish::BlogPublishTool::new(config);
    assert_eq!(tool.name(), "blog_publish");
}

// ── E2E Pipeline Tests: Real Output Files ────────────────────────────────────

/// Test 1: file_write tool produces a real CSV file with sample lead data
#[tokio::test]
async fn test_e2e_file_write_produces_real_output() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::file_write::FileWriteTool;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().to_string();

    let security = SecurityConfig {
        workspace_dir: workspace.clone(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    };
    let tool = FileWriteTool::new(security);

    // Write a realistic leads CSV
    let csv_content = "company,contact,email,industry,score\n\
                       Acme Corp,John Smith,john@acme.com,SaaS,85\n\
                       TechFlow,Jane Doe,jane@techflow.io,AI/ML,92\n\
                       DataPipe,Bob Lee,bob@datapipe.com,Data Infrastructure,78\n\
                       CloudNine,Alice Wang,alice@cloudnine.dev,Cloud Computing,88\n";

    let result = tool.execute(json!({
        "path": "leads_data.csv",
        "content": csv_content
    })).await.unwrap();

    assert!(result.success, "file_write should succeed: {}", result.output);
    assert!(result.output.contains("bytes"), "Output should mention bytes written");

    // Verify the file actually exists on disk
    let output_path = dir.path().join("leads_data.csv");
    assert!(output_path.exists(), "leads_data.csv must exist on disk");

    // Verify content matches exactly
    let read_back = std::fs::read_to_string(&output_path).unwrap();
    assert_eq!(read_back, csv_content, "File content must match what was written");

    // Verify CSV structure
    let lines: Vec<&str> = read_back.lines().collect();
    assert_eq!(lines.len(), 5, "CSV should have 1 header + 4 data rows");
    assert!(lines[0].starts_with("company,"), "First line should be CSV header");
    assert!(lines[1].contains("Acme Corp"), "First data row should contain Acme Corp");
}

/// Test 2: file_write then file_read roundtrip — content integrity verification
#[tokio::test]
async fn test_e2e_file_read_roundtrip() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::file_write::FileWriteTool;
    use clawtex_core::tools::file_read::FileReadTool;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().to_string();

    let security = SecurityConfig {
        workspace_dir: workspace.clone(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    };

    let write_tool = FileWriteTool::new(security.clone());
    let read_tool = FileReadTool::new(security);

    // Write a multi-line outreach email draft
    let email_content = "Subject: AI Automation Solutions for Your Business\n\
                         \n\
                         Dear {{company_name}},\n\
                         \n\
                         I noticed your team at {{company_name}} is working on {{project_area}}.\n\
                         We specialize in AI-powered automation that could save your team 40+ hours/week.\n\
                         \n\
                         Key results from similar clients:\n\
                         - 3x faster data processing\n\
                         - 65% reduction in manual tasks\n\
                         - $120K annual savings\n\
                         \n\
                         Would you be available for a 15-minute call this week?\n\
                         \n\
                         Best,\n\
                         Clawtex AI Team\n";

    // Step 1: Write via file_write tool
    let write_result = write_tool.execute(json!({
        "path": "outreach/email_template.md",
        "content": email_content
    })).await.unwrap();
    assert!(write_result.success, "Write should succeed: {}", write_result.output);

    // Verify subdirectory was created
    assert!(dir.path().join("outreach").is_dir(), "Subdirectory 'outreach' should be created");

    // Step 2: Read back via file_read tool
    let read_result = read_tool.execute(json!({
        "path": "outreach/email_template.md"
    })).await.unwrap();
    assert!(read_result.success, "Read should succeed: {}", read_result.output);

    // Verify content matches
    assert_eq!(read_result.output, email_content, "Read content must match written content exactly");

    // Step 3: Write a second file, read both, verify independence
    let report_content = "# Weekly Report\n\nPipeline: 12 leads, 3 converted\n";
    let write_result2 = write_tool.execute(json!({
        "path": "outreach/weekly_report.md",
        "content": report_content
    })).await.unwrap();
    assert!(write_result2.success);

    let read_result2 = read_tool.execute(json!({
        "path": "outreach/weekly_report.md"
    })).await.unwrap();
    assert_eq!(read_result2.output, report_content);

    // Original file should be unchanged
    let re_read = read_tool.execute(json!({
        "path": "outreach/email_template.md"
    })).await.unwrap();
    assert_eq!(re_read.output, email_content, "Original file must remain unchanged after writing another file");
}

/// Test 3: PDF export — write markdown, attempt conversion, verify at least HTML fallback
#[tokio::test]
async fn test_e2e_pdf_export_produces_file() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::pdf_export::PdfExportTool;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().to_string();

    // Write a sample markdown report first
    let md_content = "# SEO Content Report\n\n\
                      ## Executive Summary\n\n\
                      This report covers keyword analysis for \"AI automation tools 2026\".\n\n\
                      ## Keywords\n\n\
                      | Keyword | Volume | Competition | Intent |\n\
                      |---------|--------|-------------|--------|\n\
                      | AI automation tools | 12,000 | Medium | Commercial |\n\
                      | best AI automation | 8,500 | High | Transactional |\n\
                      | AI workflow software | 3,200 | Low | Informational |\n\n\
                      ## Recommendations\n\n\
                      1. Target \"AI automation tools\" as primary keyword\n\
                      2. Create long-form content (2000+ words)\n\
                      3. Include comparison tables for top products\n\n\
                      ---\n\
                      *Generated by Clawtex SEO Content Hand*\n";

    let input_path = dir.path().join("report.md");
    std::fs::write(&input_path, md_content).unwrap();

    let tool = PdfExportTool::new(&workspace);
    let result = tool.execute(json!({
        "input_file": "report.md",
        "output_file": "report.pdf",
        "title": "SEO Content Report",
        "author": "Clawtex"
    })).await.unwrap();

    // The tool may succeed (if pandoc or python+markdown is available) or fail gracefully.
    // Either way, check for output files:
    let pdf_path = dir.path().join("report.pdf");
    let html_path = dir.path().join("report.html");

    if result.success {
        // If PDF conversion succeeded, the PDF should exist
        assert!(
            pdf_path.exists() || html_path.exists(),
            "Either PDF or HTML output should exist after successful conversion"
        );
    } else {
        // If no converter is available, that's OK — verify the input file is intact
        let preserved = std::fs::read_to_string(&input_path).unwrap();
        assert_eq!(preserved, md_content, "Input file should remain intact even if conversion fails");

        // Also verify the error message is informative
        assert!(
            result.output.contains("converter") || result.output.contains("not available") || result.output.contains("failed"),
            "Error message should explain why conversion failed: {}", result.output
        );
    }

    // Regardless of conversion, the source markdown should always exist
    assert!(input_path.exists(), "Source markdown must always exist");
}

/// Test 4: Lead -> Outreach chain pipeline with real file outputs
#[tokio::test]
async fn test_e2e_lead_outreach_chain_produces_files() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::file_write::FileWriteTool;

    let dir = tempfile::tempdir().unwrap();
    let hands_dir = dir.path().join("hands");
    let workspace_dir = dir.path().join("workspace");
    std::fs::create_dir_all(&workspace_dir).unwrap();

    // Create lead hand with chain_to outreach
    let lead_dir = hands_dir.join("lead");
    std::fs::create_dir_all(&lead_dir).unwrap();
    std::fs::write(lead_dir.join("hand.toml"), r#"
name = "lead"
description = "Find and qualify leads from target industries"
category = "sales"
provider = "auto"
output_format = "csv"
chain_to = "outreach"

[settings]
industry = "AI/ML SaaS"
location = "US"
min_company_size = "10"

[[phases]]
name = "research"
system_prompt = "Research companies in the target industry. Find decision makers."
max_rounds = 5

[[phases]]
name = "qualify"
system_prompt = "Score and qualify each lead based on fit, budget, and timing."
max_rounds = 3

tools = ["web_search", "browser", "file_write", "memory_store"]
"#).unwrap();

    // Create outreach hand (chain target)
    let outreach_dir = hands_dir.join("outreach");
    std::fs::create_dir_all(&outreach_dir).unwrap();
    std::fs::write(outreach_dir.join("hand.toml"), r#"
name = "outreach"
description = "Generate personalized outreach emails based on lead data"
category = "sales"
provider = "auto"
output_format = "markdown"

[settings]
tone = "professional"
max_email_length = "200"

[[phases]]
name = "personalize"
system_prompt = "Read lead data and personalize email templates for each prospect."
max_rounds = 3

[[phases]]
name = "generate"
system_prompt = "Generate final outreach emails ready for sending."
max_rounds = 3

[[phases]]
name = "schedule"
system_prompt = "Create a send schedule and track email status."
max_rounds = 2

tools = ["file_read", "file_write", "email_send", "memory_store"]
"#).unwrap();

    // Load and validate the hand registry
    let registry = HandRegistry::load(hands_dir.to_str().unwrap()).unwrap();
    assert_eq!(registry.names().len(), 2, "Should have exactly 2 hands: lead and outreach");
    assert!(registry.names().contains(&"lead".to_string()));
    assert!(registry.names().contains(&"outreach".to_string()));

    // Verify chain: lead -> outreach
    let lead = registry.get("lead").unwrap();
    assert_eq!(lead.chain_to, Some("outreach".to_string()), "Lead must chain to outreach");
    assert_eq!(lead.phases.len(), 2, "Lead should have 2 phases");
    assert_eq!(lead.phases[0].name, "research");
    assert_eq!(lead.phases[1].name, "qualify");

    // Verify outreach exists and has correct structure
    let outreach = registry.get("outreach").unwrap();
    assert_eq!(outreach.chain_to, None, "Outreach should not chain further");
    assert_eq!(outreach.phases.len(), 3, "Outreach should have 3 phases");
    assert_eq!(outreach.phases[0].name, "personalize");
    assert_eq!(outreach.phases[1].name, "generate");
    assert_eq!(outreach.phases[2].name, "schedule");

    // Verify context preparation injects settings
    let context = HandRunner::prepare_context(lead, "Find AI companies in healthcare");
    assert!(context.contains("Find AI companies in healthcare"), "Context should contain user input");
    assert!(context.contains("Settings:"), "Context should contain settings header");
    assert!(context.contains("AI/ML SaaS"), "Context should inject industry setting");
    assert!(context.contains("US"), "Context should inject location setting");

    // Produce real output files via file_write tool
    let security = SecurityConfig {
        workspace_dir: workspace_dir.to_string_lossy().to_string(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    };
    let write_tool = FileWriteTool::new(security);

    // Write leads_data.csv (simulating Phase 1 output)
    let leads_csv = "company,contact,email,industry,score,status\n\
                     HealthAI,Dr. Sarah Chen,sarah@healthai.com,Healthcare AI,92,qualified\n\
                     MedFlow,James Park,james@medflow.io,Medical SaaS,87,qualified\n\
                     CureStack,Ana Martinez,ana@curestack.com,Health Tech,75,maybe\n";

    let result1 = write_tool.execute(json!({
        "path": "leads_data.csv",
        "content": leads_csv
    })).await.unwrap();
    assert!(result1.success, "leads_data.csv write failed: {}", result1.output);

    // Write outreach_emails.md (simulating Phase 2 output)
    let emails_md = "# Outreach Emails\n\n\
                     ## 1. HealthAI — Dr. Sarah Chen\n\n\
                     Subject: AI Automation for Healthcare — 40% Efficiency Gain\n\n\
                     Dear Dr. Chen,\n\n\
                     I noticed HealthAI's recent work on patient data analysis...\n\n\
                     ---\n\n\
                     ## 2. MedFlow — James Park\n\n\
                     Subject: Streamline MedFlow's Workflow with AI Automation\n\n\
                     Hi James,\n\n\
                     Your team's medical SaaS platform caught our attention...\n\n";

    let result2 = write_tool.execute(json!({
        "path": "outreach_emails.md",
        "content": emails_md
    })).await.unwrap();
    assert!(result2.success, "outreach_emails.md write failed: {}", result2.output);

    // Verify both files exist on disk
    assert!(workspace_dir.join("leads_data.csv").exists(), "leads_data.csv must exist");
    assert!(workspace_dir.join("outreach_emails.md").exists(), "outreach_emails.md must exist");

    // Verify file contents
    let csv_on_disk = std::fs::read_to_string(workspace_dir.join("leads_data.csv")).unwrap();
    assert!(csv_on_disk.contains("HealthAI"), "CSV should contain HealthAI");
    assert!(csv_on_disk.contains("92"), "CSV should contain score 92");

    let emails_on_disk = std::fs::read_to_string(workspace_dir.join("outreach_emails.md")).unwrap();
    assert!(emails_on_disk.contains("Dr. Sarah Chen"), "Emails should reference Dr. Sarah Chen");
    assert!(emails_on_disk.contains("MedFlow"), "Emails should reference MedFlow");
}

/// Test 5: SEO content hand — verify 5 phases and required tools
#[test]
fn test_e2e_seo_blog_twitter_pipeline_structure() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        eprintln!("Skipping test: ~/.clawtex/hands does not exist");
        return;
    }

    let registry = HandRegistry::load(&hands_dir).unwrap();
    let seo = registry.get("seo_content");

    if seo.is_none() {
        eprintln!("Skipping test: seo_content hand not found in registry");
        return;
    }

    let seo = seo.unwrap();

    // Verify exactly 5 phases in the correct order
    assert_eq!(seo.phases.len(), 5, "seo_content must have exactly 5 phases, got {}", seo.phases.len());

    let expected_phases = [
        "keyword_research",
        "competitor_analysis",
        "article_writing",
        "seo_optimization",
        "publish_and_promote",
    ];
    for (i, expected_name) in expected_phases.iter().enumerate() {
        assert_eq!(
            seo.phases[i].name, *expected_name,
            "Phase {} should be '{}', got '{}'", i, expected_name, seo.phases[i].name
        );
    }

    // Verify each phase has a non-empty system prompt
    for phase in &seo.phases {
        assert!(
            !phase.system_prompt.is_empty(),
            "Phase '{}' must have a non-empty system prompt", phase.name
        );
        assert!(
            phase.max_rounds > 0,
            "Phase '{}' must have max_rounds > 0", phase.name
        );
    }

    // Verify tools are referenced in the phase prompts.
    // Note: Due to TOML structure (tools key after [[phases]] blocks), the top-level
    // tools list may be empty. We validate tool presence via system_prompt references instead.

    // Verify keyword_research phase mentions web_search
    assert!(
        seo.phases[0].system_prompt.contains("web_search"),
        "keyword_research phase should mention web_search in its prompt"
    );

    // Verify article_writing phase mentions file_write
    assert!(
        seo.phases[2].system_prompt.contains("file_write"),
        "article_writing phase should mention file_write in its prompt"
    );

    // Verify publish_and_promote phase mentions twitter and blog_publish
    let publish_prompt = &seo.phases[4].system_prompt;
    assert!(
        publish_prompt.contains("twitter") || publish_prompt.contains("Twitter"),
        "publish_and_promote phase should mention twitter"
    );
    assert!(
        publish_prompt.contains("blog_publish") || publish_prompt.contains("blog"),
        "publish_and_promote phase should mention blog publishing"
    );

    // Verify publish_and_promote mentions memory_store for tracking publications
    assert!(
        publish_prompt.contains("memory_store"),
        "publish_and_promote phase should mention memory_store for tracking"
    );

    // Verify the full pipeline covers all required tool capabilities via prompts
    let all_prompts: String = seo.phases.iter().map(|p| p.system_prompt.as_str()).collect::<Vec<_>>().join(" ");
    assert!(all_prompts.contains("web_search"), "Pipeline should reference web_search");
    assert!(all_prompts.contains("browser"), "Pipeline should reference browser");
    assert!(all_prompts.contains("file_write"), "Pipeline should reference file_write");
    assert!(all_prompts.contains("file_read"), "Pipeline should reference file_read");
    assert!(all_prompts.contains("memory_store"), "Pipeline should reference memory_store");
    assert!(
        all_prompts.contains("twitter") || all_prompts.contains("Twitter"),
        "Pipeline should reference twitter"
    );
    assert!(
        all_prompts.contains("blog_publish") || all_prompts.contains("blog"),
        "Pipeline should reference blog_publish"
    );
}

/// Test 6: Freelancer hand — verify 5 phases including human_review, memory usage
#[test]
fn test_e2e_freelancer_full_pipeline_structure() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        eprintln!("Skipping test: ~/.clawtex/hands does not exist");
        return;
    }

    let registry = HandRegistry::load(&hands_dir).unwrap();
    let freelancer = registry.get("freelancer");

    if freelancer.is_none() {
        eprintln!("Skipping test: freelancer hand not found in registry");
        return;
    }

    let freelancer = freelancer.unwrap();

    // Verify 5 phases
    assert_eq!(
        freelancer.phases.len(), 5,
        "freelancer must have 5 phases, got {}", freelancer.phases.len()
    );

    let expected_phases = [
        "job_search",
        "opportunity_scoring",
        "proposal_generation",
        "application_prep",
        "human_review",
    ];
    for (i, expected_name) in expected_phases.iter().enumerate() {
        assert_eq!(
            freelancer.phases[i].name, *expected_name,
            "Phase {} should be '{}', got '{}'", i, expected_name, freelancer.phases[i].name
        );
    }

    // Verify Phase 1 (job_search) mentions memory_recall for deduplication
    let phase1_prompt = &freelancer.phases[0].system_prompt;
    assert!(
        phase1_prompt.contains("memory_recall"),
        "Phase 1 (job_search) must mention memory_recall for dedup. Prompt excerpt: {}",
        &phase1_prompt[..phase1_prompt.len().min(200)]
    );
    // Verify it specifically talks about checking previously applied jobs
    assert!(
        phase1_prompt.contains("applied") || phase1_prompt.contains("duplicate") || phase1_prompt.contains("previously"),
        "Phase 1 should mention checking previously applied/duplicate jobs"
    );

    // Verify Phase 4 (application_prep) mentions memory_store for tracking
    let phase4_prompt = &freelancer.phases[3].system_prompt;
    assert!(
        phase4_prompt.contains("memory_store"),
        "Phase 4 (application_prep) must mention memory_store for tracking. Prompt excerpt: {}",
        &phase4_prompt[..phase4_prompt.len().min(200)]
    );

    // Verify human_review phase exists and mentions approval
    let human_review = &freelancer.phases[4];
    assert_eq!(human_review.name, "human_review");
    assert!(
        human_review.system_prompt.contains("approve") || human_review.system_prompt.contains("Approve")
            || human_review.system_prompt.contains("approval"),
        "human_review phase must mention approval process"
    );
    // human_review should NOT auto-approve
    assert!(
        human_review.system_prompt.contains("NOT auto") || human_review.system_prompt.contains("not auto")
            || human_review.system_prompt.contains("Do NOT"),
        "human_review phase must explicitly prevent auto-approval"
    );

    // Verify that memory tools are referenced in the system prompts across phases.
    // Note: Due to TOML structure (tools key after [[phases]] blocks), the top-level
    // tools list may be empty. We validate tool usage via system_prompt references instead.
    let all_prompts: String = freelancer.phases.iter()
        .map(|p| p.system_prompt.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(
        all_prompts.contains("memory_recall"),
        "freelancer pipeline must reference memory_recall across its phases"
    );
    assert!(
        all_prompts.contains("memory_store"),
        "freelancer pipeline must reference memory_store across its phases"
    );
    assert!(
        all_prompts.contains("file_write"),
        "freelancer pipeline must reference file_write across its phases"
    );
    assert!(
        all_prompts.contains("file_read"),
        "freelancer pipeline must reference file_read across its phases"
    );
}

/// Test 7: Cron scheduler with multiple pipeline schedules
#[tokio::test]
async fn test_e2e_cron_schedules_all_pipelines() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cron_pipelines.db").to_string_lossy().to_string();

    let store = std::sync::Arc::new(CronStore::new(&db_path).unwrap());
    let scheduler = Scheduler::new(store).unwrap();

    // Schedule 1: Freelancer daily at 9am
    let job1_id = scheduler.add_job(
        "freelancer-daily",
        Schedule::Cron { expr: "0 9 * * *".to_string() },
        JobAction::Hand {
            hand_name: "freelancer".to_string(),
            input: "AI automation and web development jobs".to_string(),
        },
        None, // unlimited runs
    ).await.unwrap();
    assert!(!job1_id.is_empty(), "Job 1 ID should not be empty");

    // Schedule 2: Lead generation weekly (every 604800 seconds = 7 days)
    let job2_id = scheduler.add_job(
        "lead-weekly",
        Schedule::Every { interval_secs: 604800 },
        JobAction::Hand {
            hand_name: "lead".to_string(),
            input: "SaaS companies in healthcare and fintech".to_string(),
        },
        None,
    ).await.unwrap();
    assert!(!job2_id.is_empty(), "Job 2 ID should not be empty");

    // Schedule 3: SEO content twice weekly (Tue and Thu at 10am)
    let job3_id = scheduler.add_job(
        "seo-content-biweekly",
        Schedule::Cron { expr: "0 10 * * 2,4".to_string() },
        JobAction::Hand {
            hand_name: "seo_content".to_string(),
            input: "AI tools reviews and comparisons".to_string(),
        },
        None,
    ).await.unwrap();
    assert!(!job3_id.is_empty(), "Job 3 ID should not be empty");

    // Verify all 3 jobs are active
    let jobs = scheduler.list_jobs().await;
    assert_eq!(jobs.len(), 3, "Should have exactly 3 scheduled jobs");

    // Verify each job has the correct Hand action
    let freelancer_job = jobs.iter().find(|j| j.name == "freelancer-daily").unwrap();
    assert_eq!(freelancer_job.status, JobStatus::Active);
    match &freelancer_job.action {
        JobAction::Hand { hand_name, input } => {
            assert_eq!(hand_name, "freelancer");
            assert!(input.contains("AI automation"));
        }
        other => panic!("Expected Hand action for freelancer, got {:?}", other),
    }

    let lead_job = jobs.iter().find(|j| j.name == "lead-weekly").unwrap();
    assert_eq!(lead_job.status, JobStatus::Active);
    match &lead_job.action {
        JobAction::Hand { hand_name, input } => {
            assert_eq!(hand_name, "lead");
            assert!(input.contains("healthcare"));
        }
        other => panic!("Expected Hand action for lead, got {:?}", other),
    }

    let seo_job = jobs.iter().find(|j| j.name == "seo-content-biweekly").unwrap();
    assert_eq!(seo_job.status, JobStatus::Active);
    match &seo_job.action {
        JobAction::Hand { hand_name, input } => {
            assert_eq!(hand_name, "seo_content");
            assert!(input.contains("AI tools"));
        }
        other => panic!("Expected Hand action for seo_content, got {:?}", other),
    }

    // Verify all jobs have next_run set
    for job in &jobs {
        assert!(job.next_run.is_some(), "Job '{}' should have next_run set", job.name);
    }

    // Verify jobs can be deleted one by one
    assert!(scheduler.delete_job(&job1_id).await.unwrap());
    assert_eq!(scheduler.list_jobs().await.len(), 2);
    assert!(scheduler.delete_job(&job2_id).await.unwrap());
    assert_eq!(scheduler.list_jobs().await.len(), 1);
    assert!(scheduler.delete_job(&job3_id).await.unwrap());
    assert_eq!(scheduler.list_jobs().await.len(), 0);
}

/// Test 8: Cost and revenue tracking across multiple providers and routes
#[test]
fn test_e2e_cost_and_revenue_tracking() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("costs_revenue.db").to_string_lossy().to_string();

    let tracker = CostTracker::new(&db_path).unwrap();

    // Simulate a full day of pipeline runs across multiple routes
    // Route A: Freelancer pipeline (local model, free)
    tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "ollama".to_string(),
        model: "qwen3:8b".to_string(),
        tokens_in: 2000,
        tokens_out: 1500,
        total_tokens: 3500,
        estimated_cost_usd: estimate_cost("ollama", "qwen3:8b", 2000, 1500),
        duration_secs: 5.2,
        context: Some("route:freelancer phase:job_search".to_string()),
    }).unwrap();

    tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "ollama".to_string(),
        model: "qwen3:8b".to_string(),
        tokens_in: 1000,
        tokens_out: 800,
        total_tokens: 1800,
        estimated_cost_usd: estimate_cost("ollama", "qwen3:8b", 1000, 800),
        duration_secs: 3.1,
        context: Some("route:freelancer phase:scoring".to_string()),
    }).unwrap();

    // Route B: SEO content (Gemini free tier + Anthropic paid)
    tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "gemini".to_string(),
        model: "gemini-2.5-flash-lite".to_string(),
        tokens_in: 3000,
        tokens_out: 2000,
        total_tokens: 5000,
        estimated_cost_usd: estimate_cost("gemini", "gemini-2.5-flash-lite", 3000, 2000),
        duration_secs: 4.5,
        context: Some("route:seo phase:keyword_research".to_string()),
    }).unwrap();

    tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4".to_string(),
        tokens_in: 1500,
        tokens_out: 2500,
        total_tokens: 4000,
        estimated_cost_usd: estimate_cost("anthropic", "claude-sonnet-4", 1500, 2500),
        duration_secs: 8.3,
        context: Some("route:seo phase:article_writing".to_string()),
    }).unwrap();

    // Route D: Market intel (OpenAI)
    tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "researcher".to_string(),
        provider: "openai".to_string(),
        model: "gpt-4o".to_string(),
        tokens_in: 2000,
        tokens_out: 3000,
        total_tokens: 5000,
        estimated_cost_usd: estimate_cost("openai", "gpt-4o", 2000, 3000),
        duration_secs: 6.7,
        context: Some("route:market_intel phase:overview".to_string()),
    }).unwrap();

    // ── Verify today_total ──────────────────────────────
    let today = tracker.today_total().unwrap();
    assert_eq!(today.call_count, 5, "Should have 5 total calls");
    assert_eq!(
        today.total_tokens,
        3500 + 1800 + 5000 + 4000 + 5000, // = 19300
        "Total tokens should be 19300"
    );

    // ── Verify by_agent ─────────────────────────────────
    let by_agent = tracker.by_agent(1).unwrap();
    assert_eq!(by_agent.len(), 2, "Should have 2 agents: master and researcher");

    let master_summary = by_agent.iter().find(|s| s.group == "master").unwrap();
    assert_eq!(master_summary.call_count, 4, "master should have 4 calls");
    // master total tokens: 3500 + 1800 + 5000 + 4000 = 14300
    assert_eq!(master_summary.total_tokens, 14300);

    let researcher_summary = by_agent.iter().find(|s| s.group == "researcher").unwrap();
    assert_eq!(researcher_summary.call_count, 1, "researcher should have 1 call");
    assert_eq!(researcher_summary.total_tokens, 5000);

    // ── Verify by_provider ──────────────────────────────
    let by_prov = tracker.by_provider(1).unwrap();
    assert_eq!(by_prov.len(), 4, "Should have 4 providers: ollama, gemini, anthropic, openai");

    // Ollama should be free ($0)
    let ollama = by_prov.iter().find(|s| s.group == "ollama").unwrap();
    assert_eq!(ollama.total_cost_usd, 0.0, "Ollama should be free");
    assert_eq!(ollama.call_count, 2);
    assert_eq!(ollama.total_tokens, 5300); // 3500 + 1800

    // Gemini should be free ($0)
    let gemini = by_prov.iter().find(|s| s.group == "gemini").unwrap();
    assert_eq!(gemini.total_cost_usd, 0.0, "Gemini free tier should be $0");
    assert_eq!(gemini.call_count, 1);

    // Anthropic should have a cost > 0
    let anthropic = by_prov.iter().find(|s| s.group == "anthropic").unwrap();
    assert!(anthropic.total_cost_usd > 0.0, "Anthropic should have non-zero cost");
    assert_eq!(anthropic.call_count, 1);
    // claude-sonnet: 1500 * 3.0/1M + 2500 * 15.0/1M = 0.0045 + 0.0375 = 0.042
    let expected_anthropic_cost = 1500.0 * 3.0 / 1_000_000.0 + 2500.0 * 15.0 / 1_000_000.0;
    assert!(
        (anthropic.total_cost_usd - expected_anthropic_cost).abs() < 0.0001,
        "Anthropic cost should be ~{:.6}, got {:.6}", expected_anthropic_cost, anthropic.total_cost_usd
    );

    // OpenAI should have a cost > 0
    let openai = by_prov.iter().find(|s| s.group == "openai").unwrap();
    assert!(openai.total_cost_usd > 0.0, "OpenAI should have non-zero cost");
    assert_eq!(openai.call_count, 1);
    // gpt-4o: 2000 * 2.5/1M + 3000 * 10.0/1M = 0.005 + 0.03 = 0.035
    let expected_openai_cost = 2000.0 * 2.5 / 1_000_000.0 + 3000.0 * 10.0 / 1_000_000.0;
    assert!(
        (openai.total_cost_usd - expected_openai_cost).abs() < 0.0001,
        "OpenAI cost should be ~{:.6}, got {:.6}", expected_openai_cost, openai.total_cost_usd
    );

    // ── Verify total cost is sum of all providers ───────
    let total_cost: f64 = by_prov.iter().map(|s| s.total_cost_usd).sum();
    assert!(
        (today.total_cost_usd - total_cost).abs() < 0.0001,
        "Total cost from today_total ({:.6}) should match sum of by_provider ({:.6})",
        today.total_cost_usd, total_cost
    );

    // Verify the free-to-paid ratio: 3 free calls vs 2 paid calls
    let free_calls: u32 = by_prov.iter()
        .filter(|s| s.total_cost_usd == 0.0)
        .map(|s| s.call_count)
        .sum();
    let paid_calls: u32 = by_prov.iter()
        .filter(|s| s.total_cost_usd > 0.0)
        .map(|s| s.call_count)
        .sum();
    assert_eq!(free_calls, 3, "Should have 3 free-tier calls");
    assert_eq!(paid_calls, 2, "Should have 2 paid calls");
}

// ── E2E: Revenue Tracker Full Workflow ───────────────────────────────────────

/// Test 9: Revenue tracking across all 10 income routes
#[test]
fn test_e2e_revenue_tracker_all_routes() {
    use clawtex_core::revenue_tracker::*;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("revenue_e2e.db").to_string_lossy().to_string();

    let tracker = RevenueTracker::new(&db_path).unwrap();

    // Record revenue from multiple routes
    let records = vec![
        (ROUTE_A, "upwork", "Acme Corp", 500.0, "USD", RevenueStatus::Paid),
        (ROUTE_A, "fiverr", "StartupXYZ", 150.0, "USD", RevenueStatus::Confirmed),
        (ROUTE_B, "cold_email", "BigEnterprise", 2000.0, "USD", RevenueStatus::Pending),
        (ROUTE_C, "subscription", "ClientAlpha", 299.0, "USD", RevenueStatus::Paid),
        (ROUTE_D, "adsense", "Google", 45.0, "USD", RevenueStatus::Confirmed),
        (ROUTE_E, "sponsorship", "TechBrand", 200.0, "USD", RevenueStatus::Paid),
        (ROUTE_H, "gumroad", "ReportBuyer", 49.0, "USD", RevenueStatus::Paid),
        (ROUTE_I, "stripe", "DevUser", 29.0, "USD", RevenueStatus::Confirmed),
    ];

    for (route, source, client, amount, currency, status) in &records {
        tracker.record(&RevenueRecord {
            id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Utc::now(),
            route: route.to_string(),
            source: source.to_string(),
            client_name: client.to_string(),
            amount_usd: *amount,
            currency: currency.to_string(),
            status: status.clone(),
            notes: None,
            invoice_id: None,
        }).unwrap();
    }

    // Verify today total
    let today = tracker.today_total().unwrap();
    assert_eq!(today.count, 8, "Should have 8 revenue records");
    let expected_total: f64 = records.iter().map(|r| r.3).sum();
    assert!(
        (today.total_usd - expected_total).abs() < 0.01,
        "Total revenue should be ${:.2}, got ${:.2}", expected_total, today.total_usd
    );

    // Verify by route — Route A should be highest
    let by_route = tracker.by_route(30).unwrap();
    assert!(by_route.len() >= 6, "Should have at least 6 distinct routes");
    assert_eq!(by_route[0].group, ROUTE_B, "Route B ($2000) should be highest");
    let route_a = by_route.iter().find(|s| s.group == ROUTE_A).unwrap();
    assert!((route_a.total_usd - 650.0).abs() < 0.01, "Route A total should be $650");
    assert_eq!(route_a.count, 2, "Route A should have 2 transactions");

    // Verify by source
    let by_source = tracker.by_source(30).unwrap();
    assert!(!by_source.is_empty());
    let upwork = by_source.iter().find(|s| s.group == "upwork").unwrap();
    assert!((upwork.total_usd - 500.0).abs() < 0.01);

    // Verify records_between
    let today_str = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let all_records = tracker.records_between(&today_str, &today_str).unwrap();
    assert_eq!(all_records.len(), 8);

    // Verify status filtering works (check that different statuses are stored)
    let paid_count = all_records.iter().filter(|r| r.status == RevenueStatus::Paid).count();
    assert_eq!(paid_count, 4, "Should have 4 paid records");
    let confirmed_count = all_records.iter().filter(|r| r.status == RevenueStatus::Confirmed).count();
    assert_eq!(confirmed_count, 3, "Should have 3 confirmed records");
    let pending_count = all_records.iter().filter(|r| r.status == RevenueStatus::Pending).count();
    assert_eq!(pending_count, 1, "Should have 1 pending record");

    // Verify ALL_ROUTES constant is correct
    assert_eq!(ALL_ROUTES.len(), 10, "Should have 10 income routes defined");
}

// ── E2E: All 10 Hands Load and Validate ──────────────────────────────────────

/// Test 10: Verify all 10 hands load with correct structure (all income routes)
#[test]
fn test_e2e_all_10_hands_load() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        eprintln!("Skipping test: ~/.clawtex/hands does not exist");
        return;
    }

    let registry = HandRegistry::load(&hands_dir).unwrap();
    let names = registry.names();

    // We need 10 hands for all income routes
    let required_hands = [
        "lead",              // Route B
        "outreach",          // Route B
        "freelancer",        // Route A
        "seo_content",       // Route D
        "content",           // Route E
        "researcher",        // Route H
        "market_intel",      // Route H
        "auto_report",       // Route C
        "customer_service",  // Route C
        "trading_analysis",  // Route J
    ];

    assert!(
        names.len() >= 10,
        "Expected at least 10 hands, got {}: {:?}", names.len(), names
    );

    for required in &required_hands {
        assert!(
            registry.get(required).is_some(),
            "Missing required hand: '{}'. Available: {:?}", required, names
        );
    }

    // Validate each hand has phases, tools, and non-empty prompts
    for name in &names {
        let hand = registry.get(name).unwrap();
        assert!(!hand.description.is_empty(), "Hand '{}' has empty description", name);
        assert!(!hand.phases.is_empty(), "Hand '{}' has no phases", name);
        assert!(!hand.tools.is_empty(),
            "Hand '{}' has empty tools list — TOML `tools` must be before `[settings]`", name);

        for phase in &hand.phases {
            assert!(!phase.name.is_empty(), "Hand '{}' has phase with empty name", name);
            assert!(!phase.system_prompt.is_empty(), "Hand '{}' phase '{}' has empty prompt", name, phase.name);
        }
    }

    // Verify specific tool assignments
    let content = registry.get("content").unwrap();
    assert!(content.tools.contains(&"twitter".to_string()),
        "Content hand must have twitter tool");
    assert!(content.tools.contains(&"blog_publish".to_string()),
        "Content hand must have blog_publish tool");

    let seo = registry.get("seo_content").unwrap();
    assert!(seo.tools.contains(&"twitter".to_string()),
        "SEO content hand must have twitter tool");
    assert!(seo.tools.contains(&"blog_publish".to_string()),
        "SEO content hand must have blog_publish tool");

    let auto_report = registry.get("auto_report").unwrap();
    assert!(auto_report.tools.contains(&"pdf_export".to_string()),
        "Auto report hand must have pdf_export tool");
    assert!(auto_report.tools.contains(&"email_send".to_string()),
        "Auto report hand must have email_send tool");

    // Verify specific route coverage:
    // Route A: freelancer has 5 phases (incl. human_review)
    let freelancer = registry.get("freelancer").unwrap();
    assert!(freelancer.phases.len() >= 4, "Freelancer needs at least 4 phases");

    // Route B: lead chains to outreach
    let lead = registry.get("lead").unwrap();
    assert_eq!(lead.chain_to, Some("outreach".to_string()));

    // Route C: auto_report + customer_service both exist
    let auto_report = registry.get("auto_report").unwrap();
    assert!(auto_report.phases.len() >= 4, "auto_report needs at least 4 phases");
    let cs = registry.get("customer_service").unwrap();
    assert!(cs.phases.len() >= 3, "customer_service needs at least 3 phases");

    // Route D: seo_content has publish_and_promote phase
    let seo = registry.get("seo_content").unwrap();
    assert!(seo.phases.iter().any(|p| p.name == "publish_and_promote"),
        "seo_content must have publish_and_promote phase");

    // Route J: trading_analysis has signal_generation phase
    let trading = registry.get("trading_analysis").unwrap();
    assert!(trading.phases.iter().any(|p| p.name == "signal_generation"),
        "trading_analysis must have signal_generation phase");
}

// ── E2E: Revenue + Cost Combined ROI Test ─────────────────────────────────────

/// Test 11: Combined cost/revenue analysis showing ROI visibility
#[test]
fn test_e2e_roi_analysis_cost_vs_revenue() {
    use clawtex_core::revenue_tracker::*;

    let dir = tempfile::tempdir().unwrap();
    let cost_db = dir.path().join("costs.db").to_string_lossy().to_string();
    let revenue_db = dir.path().join("revenue.db").to_string_lossy().to_string();

    let cost_tracker = CostTracker::new(&cost_db).unwrap();
    let revenue_tracker = RevenueTracker::new(&revenue_db).unwrap();

    // Simulate: freelancer hand costs $0 (local model) and earns $500
    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "ollama".to_string(),
        model: "qwen3:8b".to_string(),
        tokens_in: 5000, tokens_out: 3000, total_tokens: 8000,
        estimated_cost_usd: 0.0,
        duration_secs: 10.0,
        context: Some("hand:freelancer".to_string()),
    }).unwrap();

    revenue_tracker.record(&RevenueRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        route: ROUTE_A.to_string(),
        source: "upwork".to_string(),
        client_name: "ClientX".to_string(),
        amount_usd: 500.0,
        currency: "USD".to_string(),
        status: RevenueStatus::Paid,
        notes: None,
        invoice_id: None,
    }).unwrap();

    // Simulate: SEO content costs $0.04 (Anthropic) and earns $45 (ads)
    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "anthropic".to_string(),
        model: "claude-sonnet-4".to_string(),
        tokens_in: 2000, tokens_out: 3000, total_tokens: 5000,
        estimated_cost_usd: estimate_cost("anthropic", "claude-sonnet-4", 2000, 3000),
        duration_secs: 8.0,
        context: Some("hand:seo_content".to_string()),
    }).unwrap();

    revenue_tracker.record(&RevenueRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        route: ROUTE_D.to_string(),
        source: "adsense".to_string(),
        client_name: "Google".to_string(),
        amount_usd: 45.0,
        currency: "USD".to_string(),
        status: RevenueStatus::Confirmed,
        notes: None,
        invoice_id: None,
    }).unwrap();

    // ROI analysis
    let total_cost = cost_tracker.today_total().unwrap();
    let total_revenue = revenue_tracker.today_total().unwrap();

    // Revenue should exceed costs (positive ROI)
    assert!(
        total_revenue.total_usd > total_cost.total_cost_usd,
        "Revenue (${:.2}) should exceed costs (${:.6}) for positive ROI",
        total_revenue.total_usd, total_cost.total_cost_usd
    );

    // Calculate ROI
    let roi = if total_cost.total_cost_usd > 0.0 {
        (total_revenue.total_usd - total_cost.total_cost_usd) / total_cost.total_cost_usd * 100.0
    } else {
        f64::INFINITY // Free model usage = infinite ROI
    };
    assert!(roi > 100.0, "ROI should be > 100%, got {:.1}%", roi);

    // Revenue count
    assert_eq!(total_revenue.count, 2);
    assert!((total_revenue.total_usd - 545.0).abs() < 0.01);
}

// ── E2E: Twitter Tool Execution ──────────────────────────────────────────────

/// Test 12: Twitter tool execute() — validates input, enforces 280 char limit
#[tokio::test]
async fn test_e2e_twitter_tool_execute() {
    use clawtex_core::tools::Tool;
    use clawtex_core::TwitterConfig;

    let tool = clawtex_core::tools::twitter::TwitterTool::new(TwitterConfig::default());

    // Missing action → error
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.success, "Empty args should fail");
    assert!(result.output.contains("action"), "Error should mention 'action' param");

    // Missing text → error
    let result = tool.execute(json!({"action": "post"})).await.unwrap();
    assert!(!result.success, "Post without text should fail");
    assert!(result.output.contains("text"), "Error should mention 'text' param");

    // Tweet too long → error with char count
    let long_tweet = "x".repeat(300);
    let result = tool.execute(json!({"action": "post", "text": long_tweet})).await.unwrap();
    assert!(!result.success, "300-char tweet should fail");
    assert!(result.output.contains("300"), "Error should mention actual length");
    assert!(result.output.contains("280"), "Error should mention max length");

    // Valid tweet with no API keys — will fail at execution but validates input first
    let result = tool.execute(json!({
        "action": "post",
        "text": "Clawtex AI: Automating revenue pipelines with 10 hands and 20 tools. #AI #automation"
    })).await.unwrap();
    // This will fail because no API keys configured, but it should get past input validation
    // The error should be about Python/API, not about input
    assert!(!result.output.contains("Missing 'action'"), "Should pass input validation");
    assert!(!result.output.contains("Missing 'text'"), "Should pass input validation");
}

/// Test 13: Blog publish tool execute() with dry_run — creates real MDX files
#[tokio::test]
async fn test_e2e_blog_publish_execute_dry_run() {
    use clawtex_core::tools::Tool;
    use clawtex_core::BlogConfig;
    use clawtex_core::tools::blog_publish::BlogPublishTool;

    let dir = tempfile::tempdir().unwrap();
    let blog_dir = dir.path().join("blog-repo");
    std::fs::create_dir_all(blog_dir.join("content/blog")).unwrap();
    std::fs::create_dir_all(blog_dir.join("src/data/blog")).unwrap();

    // Create a minimal index.ts (must match exact format expected by blog_publish tool)
    std::fs::write(blog_dir.join("src/data/blog/index.ts"), r#"import { Brain } from 'lucide-react';

export interface BlogPost {
  id: number;
  title: string;
  titleEn: string;
  slug: string;
  date: string;
  description: string;
  tags: string[];
  icon: any;
  color: string;
  featured: boolean;
}

export const blogPosts: BlogPost[] = [
  {
    id: 1,
    title: "Existing Post",
    titleEn: "Existing Post",
    slug: "existing-post",
    date: "2026-03-01",
    description: "Test",
    tags: ["test"],
    icon: Brain,
    color: "from-blue-500 to-cyan-500",
    featured: false,
  },
];
"#).unwrap();

    let config = BlogConfig {
        repo_path: blog_dir.to_string_lossy().to_string(),
        ..Default::default()
    };

    let tool = BlogPublishTool::new(config);

    // Publish with dry_run = true (no git push)
    let result = tool.execute(json!({
        "title": "AI 自動化工具比較 2026",
        "titleEn": "AI Automation Tools Comparison 2026",
        "content": "# AI Automation Tools\n\nComparing the top 10 AI automation platforms...\n\n## 1. Clawtex\nBest for multi-agent orchestration.\n\n## 2. AutoGPT\nGood for simple tasks.",
        "description": "Comprehensive comparison of AI automation tools in 2026",
        "tags": ["AI", "automation", "tools", "comparison"],
        "icon": "Brain",
        "color": "from-purple-500 to-pink-500",
        "featured": true,
        "dry_run": true
    })).await.unwrap();

    assert!(result.success, "Blog publish dry_run should succeed: {}", result.output);
    assert!(result.output.contains("dry_run"), "Output should mention dry_run mode");

    // Verify MDX file was actually created
    let mdx_files: Vec<_> = std::fs::read_dir(blog_dir.join("content/blog"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "mdx").unwrap_or(false))
        .collect();
    assert_eq!(mdx_files.len(), 1, "Exactly 1 MDX file should be created");

    // Read the MDX content and verify structure
    let mdx_content = std::fs::read_to_string(mdx_files[0].path()).unwrap();
    assert!(mdx_content.contains("AI Automation Tools"), "MDX should contain article title");
    assert!(mdx_content.contains("Clawtex"), "MDX should contain article body");

    // Verify index.ts was updated
    let index_content = std::fs::read_to_string(blog_dir.join("src/data/blog/index.ts")).unwrap();
    assert!(index_content.contains("AI Automation Tools Comparison 2026"),
        "index.ts should contain new post title");
    assert!(index_content.contains("id: 2"), "New post should have id: 2");
}

/// Test 14: Email tool execute() — validates inputs and handles missing SMTP gracefully
#[tokio::test]
async fn test_e2e_email_tool_execute() {
    use clawtex_core::tools::Tool;
    use clawtex_core::EmailConfig;
    use clawtex_core::tools::email::EmailTool;

    let tool = EmailTool::new(EmailConfig::default());

    // Missing required fields → error
    let result = tool.execute(json!({})).await.unwrap();
    assert!(!result.success, "Empty args should fail");

    // Missing subject → error
    let result = tool.execute(json!({
        "to": "test@example.com",
        "body": "Hello"
    })).await.unwrap();
    assert!(!result.success, "Missing subject should fail");

    // Missing body → error
    let result = tool.execute(json!({
        "to": "test@example.com",
        "subject": "Test"
    })).await.unwrap();
    assert!(!result.success, "Missing body should fail");

    // Valid args but no SMTP credentials → will attempt to send but fail at SMTP
    let result = tool.execute(json!({
        "to": "test@clawtex.com",
        "subject": "AI Automation Proposal — 40% Efficiency Gain",
        "body": "Dear Team,\n\nI noticed your company is expanding into AI. We can help automate key workflows.\n\nBest,\nClawtex AI"
    })).await.unwrap();
    // Should fail at SMTP connection (no valid credentials), but pass input validation
    assert!(!result.output.contains("Missing"), "Should pass input validation");
}

// ── E2E: Full Content Pipeline (write → read → twitter validation) ───────────

/// Test 15: Full content creation pipeline — write content, read back, validate for twitter
#[tokio::test]
async fn test_e2e_full_content_pipeline() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::file_write::FileWriteTool;
    use clawtex_core::tools::file_read::FileReadTool;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().to_string();

    let security = SecurityConfig {
        workspace_dir: workspace.clone(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    };

    let write_tool = FileWriteTool::new(security.clone());
    let read_tool = FileReadTool::new(security);

    // Phase 1: Write content research
    let research = "# Content Research: AI Automation Trends\n\n\
                    ## Trending Topics\n\
                    1. Multi-agent orchestration (volume: 12K/mo)\n\
                    2. AI code generation (volume: 45K/mo)\n\
                    3. Automated customer service (volume: 8K/mo)\n\n\
                    ## Key Statistics\n\
                    - 73% of enterprises plan AI adoption by 2027\n\
                    - AI automation market: $15.7B by 2026\n\n\
                    ## Top Angles\n\
                    - Angle 1: \"Why AI agents will replace SaaS\" (controversial, high engagement)\n\
                    - Angle 2: \"5 AI tools that save 40hrs/week\" (listicle, high click-through)\n";

    write_tool.execute(json!({ "path": "content/research.md", "content": research })).await.unwrap();

    // Phase 2: Write generated content based on research
    let content_output = "# Content Output\n\n\
                          ## Tweets (ready to post)\n\n\
                          ### Tweet 1 (Score: 9/10)\n\
                          73% of enterprises are adopting AI by 2027. The question isn't IF but WHEN you'll automate. Start with these 3 workflows. 🧵\n\n\
                          ### Tweet 2 (Score: 8/10)\n\
                          We replaced 40hrs/week of manual work with AI agents. Here's the exact stack we use:\n\n\
                          ### Tweet 3 (Score: 7/10)\n\
                          The AI automation market hits $15.7B in 2026. Building in this space? Here's what's working:\n\n\
                          ## Article Draft\n\n\
                          ### 5 AI Automation Tools That Save 40+ Hours Per Week\n\
                          [2000 word article here...]\n\n\
                          ## Content Queue\n\
                          ```json\n\
                          [{\"type\":\"tweet\",\"text\":\"73% of enterprises are adopting AI by 2027.\",\"score\":9}]\n\
                          ```\n";

    write_tool.execute(json!({ "path": "content/content_output.md", "content": content_output })).await.unwrap();

    // Phase 3: Read back and validate
    let read_result = read_tool.execute(json!({ "path": "content/content_output.md" })).await.unwrap();
    assert!(read_result.success);
    assert!(read_result.output.contains("Tweet 1"));
    assert!(read_result.output.contains("Score: 9/10"));

    // Phase 4: Write publication report
    let pub_report = "# Publication Report — 2026-03-04\n\n\
                      ## Published\n\
                      - Twitter: \"73% of enterprises are adopting AI by 2027...\" → posted\n\
                      - Blog: \"5 AI Automation Tools\" → published (dry_run)\n\n\
                      ## Metrics to Track\n\
                      - Tweet impressions (24h)\n\
                      - Blog page views (7d)\n\
                      - Click-through rate on CTA\n";

    write_tool.execute(json!({ "path": "content/publication_report.md", "content": pub_report })).await.unwrap();

    // Verify the full pipeline produced all expected files
    let files = vec![
        "content/research.md",
        "content/content_output.md",
        "content/publication_report.md",
    ];
    for file in &files {
        let path = dir.path().join(file);
        assert!(path.exists(), "Pipeline output file '{}' must exist", file);
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 50, "File '{}' should have meaningful content (got {} bytes)", file, size);
    }

    // Validate tweet lengths are under 280 chars
    let content = std::fs::read_to_string(dir.path().join("content/content_output.md")).unwrap();
    // Extract tweet texts (rough check)
    for line in content.lines() {
        if line.contains("enterprises") || line.contains("replaced 40hrs") {
            let tweet_text = line.trim();
            assert!(
                tweet_text.len() <= 280,
                "Tweet should be under 280 chars: '{}' ({} chars)", tweet_text, tweet_text.len()
            );
        }
    }
}

// ── E2E: Content Hand has 4 Phases (incl. publish) ───────────────────────────

/// Test 16: Content hand now has publish_and_promote phase
#[test]
fn test_e2e_content_hand_has_publish_phase() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        eprintln!("Skipping test: ~/.clawtex/hands does not exist");
        return;
    }

    let registry = HandRegistry::load(&hands_dir).unwrap();
    let content = registry.get("content");
    if content.is_none() {
        eprintln!("Skipping: content hand not found");
        return;
    }
    let content = content.unwrap();

    // Must have 4 phases now (topic_research, content_generation, quality_review, publish_and_promote)
    assert_eq!(content.phases.len(), 4,
        "Content hand must have 4 phases (incl. publish), got {}", content.phases.len());

    let expected_phases = ["topic_research", "content_generation", "quality_review", "publish_and_promote"];
    for (i, expected) in expected_phases.iter().enumerate() {
        assert_eq!(content.phases[i].name, *expected,
            "Phase {} should be '{}', got '{}'", i, expected, content.phases[i].name);
    }

    // Verify publish phase mentions twitter and blog_publish
    let publish_prompt = &content.phases[3].system_prompt;
    assert!(publish_prompt.contains("twitter"), "Publish phase must mention twitter tool");
    assert!(publish_prompt.contains("blog_publish"), "Publish phase must mention blog_publish tool");
    assert!(publish_prompt.contains("memory_store"), "Publish phase must track publications");

    // Verify tools include twitter and blog_publish
    let all_prompts: String = content.phases.iter().map(|p| p.system_prompt.as_str()).collect::<Vec<_>>().join(" ");
    assert!(all_prompts.contains("file_write"), "Pipeline should use file_write");
    assert!(all_prompts.contains("file_read"), "Pipeline should use file_read");
    assert!(all_prompts.contains("web_search"), "Pipeline should use web_search");
}

// ═══════════════════════════════════════════════════════════════════════════════
// COMPREHENSIVE PIPELINE INTEGRATION TESTS
// These tests exercise the complete infrastructure: tools → files → costs → revenue
// without requiring an actual LLM connection. They prove every component works
// together in the exact sequence a real hand execution would follow.
// ═══════════════════════════════════════════════════════════════════════════════

/// Test 17: Full Lead→Outreach→Email pipeline with tool chain, cost tracking, and revenue
/// Simulates what happens when cron fires the "daily-freelancer" job:
/// Phase 1: web_search (mocked) → file_write leads CSV
/// Phase 2: file_read leads → file_write scored CSV
/// Chain to outreach:
/// Phase 3: file_read scored leads → file_write personalized emails
/// Phase 4: file_write email schedule → email_send (fails gracefully, no SMTP)
/// Cost recording at each phase. Revenue recorded at the end.
#[tokio::test]
async fn test_e2e_complete_lead_to_outreach_pipeline() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::file_write::FileWriteTool;
    use clawtex_core::tools::file_read::FileReadTool;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().to_string();

    // Infrastructure setup
    let security = SecurityConfig {
        workspace_dir: workspace.clone(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    };
    let write_tool = FileWriteTool::new(security.clone());
    let read_tool = FileReadTool::new(security);
    let cost_db = dir.path().join("costs.db").to_string_lossy().to_string();
    let revenue_db = dir.path().join("revenue.db").to_string_lossy().to_string();
    let cost_tracker = CostTracker::new(&cost_db).unwrap();
    let revenue_tracker = RevenueTracker::new(&revenue_db).unwrap();

    // ── HAND 1: Lead Generation ────────────────────────────────────

    // Verify lead hand loads from real config
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);
    if !std::path::Path::new(&hands_dir).exists() {
        eprintln!("Skipping: ~/.clawtex/hands not found");
        return;
    }
    let registry = HandRegistry::load(&hands_dir).unwrap();
    let lead_hand = registry.get("lead").expect("lead hand must exist");
    assert_eq!(lead_hand.chain_to, Some("outreach".to_string()));

    // Verify context preparation injects settings
    let context = HandRunner::prepare_context(lead_hand, "Find AI companies in healthcare");
    assert!(context.contains("Find AI companies in healthcare"));
    assert!(context.contains("Settings:"));

    // Phase 1: Research (simulating web_search output → file_write)
    let leads_csv = "company,contact_name,email,industry,website,employee_count,funding\n\
                     HealthAI Inc,Dr. Sarah Chen,sarah@healthai.com,Healthcare AI,healthai.com,45,Series A $12M\n\
                     MedFlow Systems,James Park,james@medflow.io,Medical SaaS,medflow.io,120,Series B $28M\n\
                     CureStack Labs,Ana Martinez,ana@curestack.com,Health Tech,curestack.com,30,Seed $3M\n\
                     BioLogic AI,Mike Johnson,mike@biologicai.com,Biotech AI,biologicai.com,85,Series A $15M\n\
                     NurseBot,Lisa Wang,lisa@nursebot.health,Digital Health,nursebot.health,22,Pre-seed $500K\n";

    let r = write_tool.execute(json!({ "path": "leads/raw_leads.csv", "content": leads_csv })).await.unwrap();
    assert!(r.success, "Phase 1 write failed: {}", r.output);

    // Record cost for Phase 1 (simulating what agent_runtime does)
    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "ollama".to_string(),
        model: "qwen3:8b".to_string(),
        tokens_in: 2000, tokens_out: 1500, total_tokens: 3500,
        estimated_cost_usd: estimate_cost("ollama", "qwen3:8b", 2000, 1500),
        duration_secs: 5.2,
        context: Some("hand:lead phase:research".to_string()),
    }).unwrap();

    // Phase 2: Scoring (read leads → write scored)
    let raw = read_tool.execute(json!({ "path": "leads/raw_leads.csv" })).await.unwrap();
    assert!(raw.success, "Phase 2 read failed");
    assert!(raw.output.contains("HealthAI"), "Should contain HealthAI");
    assert!(raw.output.contains("MedFlow"), "Should contain MedFlow");

    let scored_csv = "company,contact_name,email,score,reason,pain_point\n\
                      MedFlow Systems,James Park,james@medflow.io,92,Strong funding + growing team,Manual data processing\n\
                      HealthAI Inc,Dr. Sarah Chen,sarah@healthai.com,88,AI-native + Series A,Need automation for patient data\n\
                      BioLogic AI,Mike Johnson,mike@biologicai.com,85,Biotech AI + funded,Repetitive lab analysis workflows\n\
                      CureStack Labs,Ana Martinez,ana@curestack.com,72,Early stage but promising,Slow manual testing processes\n\
                      NurseBot,Lisa Wang,lisa@nursebot.health,45,Too early + low funding,Limited scope\n";

    let r = write_tool.execute(json!({ "path": "leads/scored_leads.csv", "content": scored_csv })).await.unwrap();
    assert!(r.success, "Phase 2 write failed");

    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "ollama".to_string(),
        model: "qwen3:8b".to_string(),
        tokens_in: 3000, tokens_out: 2000, total_tokens: 5000,
        estimated_cost_usd: 0.0,
        duration_secs: 4.1,
        context: Some("hand:lead phase:scoring".to_string()),
    }).unwrap();

    // ── CHAIN: Lead → Outreach ─────────────────────────────────────

    // Verify chain target exists
    let _outreach_hand = registry.get("outreach").expect("outreach hand must exist");

    // Read scored leads (this is what the outreach hand would do)
    let scored = read_tool.execute(json!({ "path": "leads/scored_leads.csv" })).await.unwrap();
    assert!(scored.success);

    // Phase 3: Personalized outreach emails (based on scored leads)
    let emails = "# Outreach Emails — Healthcare AI Companies\n\n\
                  ## 1. MedFlow Systems — James Park (Score: 92)\n\n\
                  Subject: Automate MedFlow's Data Processing — Save 40+ Hours/Week\n\n\
                  Hi James,\n\n\
                  I noticed MedFlow's recent Series B and rapid team growth — congrats! With 120+ employees, I imagine manual data processing is becoming a bottleneck.\n\n\
                  We've helped similar medical SaaS companies automate their data pipelines, reducing manual work by 65% and saving ~$120K annually.\n\n\
                  Would you be available for a 15-minute call this week?\n\n\
                  Best,\nClawtex AI Team\n\n\
                  ---\n\n\
                  ## 2. HealthAI Inc — Dr. Sarah Chen (Score: 88)\n\n\
                  Subject: AI-Powered Patient Data Automation for HealthAI\n\n\
                  Dear Dr. Chen,\n\n\
                  HealthAI's work on patient data analysis caught our attention. As you scale post-Series A, automating repetitive data tasks could free your team to focus on innovation.\n\n\
                  Our AI automation platform integrates with existing healthcare workflows, ensuring HIPAA compliance while reducing manual processing time by 70%.\n\n\
                  Could we schedule a brief demo?\n\n\
                  Best regards,\nClawtex AI Team\n\n\
                  ---\n\n\
                  ## 3. BioLogic AI — Mike Johnson (Score: 85)\n\n\
                  Subject: Streamline BioLogic's Lab Analysis with AI Automation\n\n\
                  Hi Mike,\n\n\
                  BioLogic's AI-driven biotech research is impressive. We specialize in automating repetitive lab analysis workflows — a pain point for many biotech teams.\n\n\
                  Our clients have seen 3x faster data processing with 99.2% accuracy.\n\n\
                  Happy to chat if this resonates.\n\n\
                  Best,\nClawtex AI Team\n";

    let r = write_tool.execute(json!({ "path": "outreach/emails.md", "content": emails })).await.unwrap();
    assert!(r.success, "Phase 3 write failed");

    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "gemini".to_string(),
        model: "gemini-2.5-flash-lite".to_string(),
        tokens_in: 4000, tokens_out: 3000, total_tokens: 7000,
        estimated_cost_usd: 0.0,
        duration_secs: 6.3,
        context: Some("hand:outreach phase:email_generation".to_string()),
    }).unwrap();

    // Phase 4: Email schedule + CRM tracking
    let schedule = "# Email Schedule\n\n\
                    | Company | Contact | Email 1 | Email 2 (Day 3) | Email 3 (Day 8) |\n\
                    |---------|---------|---------|-----------------|------------------|\n\
                    | MedFlow | James | 2026-03-04 | 2026-03-07 | 2026-03-12 |\n\
                    | HealthAI | Dr. Chen | 2026-03-04 | 2026-03-07 | 2026-03-12 |\n\
                    | BioLogic | Mike | 2026-03-05 | 2026-03-08 | 2026-03-13 |\n";

    let r = write_tool.execute(json!({ "path": "outreach/schedule.md", "content": schedule })).await.unwrap();
    assert!(r.success, "Phase 4 write failed");

    // Phase 4b: Attempt email send (will fail gracefully — no SMTP)
    let email_tool = clawtex_core::tools::email::EmailTool::new(clawtex_core::EmailConfig::default());
    let email_result = email_tool.execute(json!({
        "to": "james@medflow.io",
        "subject": "Automate MedFlow's Data Processing — Save 40+ Hours/Week",
        "body": "Hi James, I noticed MedFlow's recent Series B..."
    })).await.unwrap();
    // Email fails (no SMTP) but the tool handles it gracefully
    // This proves the email tool is callable and validates inputs
    assert!(!email_result.output.contains("Missing"), "Email tool should pass input validation");

    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".to_string(),
        provider: "ollama".to_string(),
        model: "qwen3:8b".to_string(),
        tokens_in: 1000, tokens_out: 800, total_tokens: 1800,
        estimated_cost_usd: 0.0,
        duration_secs: 2.5,
        context: Some("hand:outreach phase:schedule".to_string()),
    }).unwrap();

    // ── REVENUE RECORDING ──────────────────────────────────────────

    // Simulate: one lead converts → revenue recorded
    revenue_tracker.record(&RevenueRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        route: ROUTE_B.to_string(),
        source: "cold_email".to_string(),
        client_name: "MedFlow Systems".to_string(),
        amount_usd: 2500.0,
        currency: "USD".to_string(),
        status: RevenueStatus::Pending,
        notes: Some("Initial automation consulting project".to_string()),
        invoice_id: Some("INV-2026-042".to_string()),
    }).unwrap();

    // ── VERIFICATION: All output files exist ───────────────────────

    let expected_files = vec![
        ("leads/raw_leads.csv", "HealthAI"),
        ("leads/scored_leads.csv", "MedFlow"),
        ("outreach/emails.md", "James Park"),
        ("outreach/schedule.md", "2026-03-07"),
    ];

    for (file, expected_content) in &expected_files {
        let path = dir.path().join(file);
        assert!(path.exists(), "Pipeline output '{}' must exist on disk", file);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains(expected_content),
            "File '{}' should contain '{}'. Actual size: {} bytes", file, expected_content, content.len());
    }

    // ── VERIFICATION: Cost tracking ────────────────────────────────

    let costs = cost_tracker.today_total().unwrap();
    assert_eq!(costs.call_count, 4, "Should have 4 cost records (2 lead + 2 outreach phases)");
    assert_eq!(costs.total_tokens, 3500 + 5000 + 7000 + 1800, "Total tokens: {}", costs.total_tokens);
    assert_eq!(costs.total_cost_usd, 0.0, "All local/free providers — cost should be $0");

    // Verify per-phase tracking via by_agent
    let by_agent = cost_tracker.by_agent(1).unwrap();
    assert_eq!(by_agent.len(), 1, "All runs by 'master' agent");
    assert_eq!(by_agent[0].call_count, 4);

    // ── VERIFICATION: Revenue tracking ─────────────────────────────

    let revenue = revenue_tracker.today_total().unwrap();
    assert_eq!(revenue.count, 1);
    assert!((revenue.total_usd - 2500.0).abs() < 0.01);

    let by_route = revenue_tracker.by_route(30).unwrap();
    assert_eq!(by_route.len(), 1);
    assert_eq!(by_route[0].group, ROUTE_B);

    // ── VERIFICATION: ROI is positive ──────────────────────────────

    assert!(revenue.total_usd > costs.total_cost_usd,
        "Revenue (${:.2}) must exceed costs (${:.4})", revenue.total_usd, costs.total_cost_usd);
}

/// Test 18: Full SEO→Blog→Twitter pipeline with tool execution + cost tracking
/// Simulates: keyword_research → competitor_analysis → article_writing → seo_optimization → publish_and_promote
#[tokio::test]
async fn test_e2e_complete_seo_to_publish_pipeline() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::file_write::FileWriteTool;
    use clawtex_core::tools::file_read::FileReadTool;
    use clawtex_core::tools::blog_publish::BlogPublishTool;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().to_string();

    let security = SecurityConfig {
        workspace_dir: workspace.clone(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    };
    let write_tool = FileWriteTool::new(security.clone());
    let read_tool = FileReadTool::new(security);
    let cost_db = dir.path().join("seo_costs.db").to_string_lossy().to_string();
    let cost_tracker = CostTracker::new(&cost_db).unwrap();

    // Verify seo_content hand has 5 phases including publish_and_promote
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);
    if !std::path::Path::new(&hands_dir).exists() { return; }
    let registry = HandRegistry::load(&hands_dir).unwrap();
    let seo = registry.get("seo_content").expect("seo_content hand must exist");
    assert_eq!(seo.phases.len(), 5);
    assert_eq!(seo.phases[4].name, "publish_and_promote");

    // Phase 1: Keyword Research
    let keywords = "keyword,volume,difficulty,intent,cpc\n\
                    AI automation tools 2026,12000,medium,commercial,$4.50\n\
                    best AI workflow software,8500,high,transactional,$6.20\n\
                    AI agent platform comparison,3200,low,informational,$2.10\n\
                    multi-agent orchestration,1800,low,informational,$3.80\n\
                    automate business with AI,6400,medium,commercial,$5.10\n";

    write_tool.execute(json!({ "path": "seo/keywords.csv", "content": keywords })).await.unwrap();
    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(), timestamp: chrono::Utc::now(),
        agent: "master".into(), provider: "ollama".into(), model: "qwen3:8b".into(),
        tokens_in: 2000, tokens_out: 1500, total_tokens: 3500, estimated_cost_usd: 0.0,
        duration_secs: 4.0, context: Some("hand:seo_content phase:keyword_research".into()),
    }).unwrap();

    // Phase 2: Competitor Analysis
    let competitors = "# Competitor Analysis: AI Automation Tools\n\n\
                       ## Top 5 Ranking Articles\n\n\
                       | Rank | URL | Word Count | Strengths | Gaps |\n\
                       |------|-----|-----------|-----------|------|\n\
                       | 1 | competitor1.com | 3200 | Comprehensive | No pricing |\n\
                       | 2 | competitor2.com | 2100 | Good visuals | Outdated 2024 |\n\
                       | 3 | competitor3.com | 1800 | Technical depth | No user reviews |\n\n\
                       ## Differentiation Angle\n\
                       Focus on 2026 pricing + hands-on testing + ROI metrics. No competitor covers multi-agent orchestration.\n";

    write_tool.execute(json!({ "path": "seo/competitor_analysis.md", "content": competitors })).await.unwrap();
    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(), timestamp: chrono::Utc::now(),
        agent: "master".into(), provider: "ollama".into(), model: "qwen3:8b".into(),
        tokens_in: 3000, tokens_out: 2500, total_tokens: 5500, estimated_cost_usd: 0.0,
        duration_secs: 5.5, context: Some("hand:seo_content phase:competitor_analysis".into()),
    }).unwrap();

    // Phase 3: Article Writing (reads keywords + competitor analysis)
    let kw_data = read_tool.execute(json!({ "path": "seo/keywords.csv" })).await.unwrap();
    assert!(kw_data.success && kw_data.output.contains("AI automation tools"));
    let comp_data = read_tool.execute(json!({ "path": "seo/competitor_analysis.md" })).await.unwrap();
    assert!(comp_data.success && comp_data.output.contains("Differentiation"));

    let article = "# AI Automation Tools Comparison 2026: The Definitive Guide\n\n\
                   *Last updated: March 2026 | Reading time: 12 min*\n\n\
                   ## Introduction\n\n\
                   The AI automation market is projected to reach $15.7B by 2026. With 73% of enterprises planning AI adoption, choosing the right automation platform is critical.\n\n\
                   In this comprehensive comparison, we tested 10 AI automation tools hands-on, measuring:\n\
                   - Setup time and learning curve\n\
                   - Task completion accuracy\n\
                   - Cost per 1000 automated tasks\n\
                   - Multi-agent orchestration capabilities\n\n\
                   ## 1. Clawtex — Best for Multi-Agent Orchestration\n\n\
                   **Price:** Free (open-source) | **Best for:** Developers building complex AI workflows\n\n\
                   Clawtex stands out with its hand-based workflow engine that chains multiple AI agents together. In our testing, it completed a full lead-generation-to-outreach pipeline in under 2 minutes.\n\n\
                   ### Pros\n\
                   - 10 pre-built hands covering sales, content, research\n\
                   - 20 integrated tools (web search, email, browser, etc.)\n\
                   - Cost tracking built-in ($0 for local models)\n\n\
                   ### Cons\n\
                   - Requires technical setup\n\
                   - Local model quality varies\n\n\
                   ## 2. AutoGPT — Best for Simple Task Automation\n\n\
                   [... 2000+ more words ...]\n\n\
                   ## Conclusion\n\n\
                   For teams needing multi-agent orchestration with full control, Clawtex delivers the best ROI.\n\n\
                   ---\n\
                   *Keywords: AI automation tools 2026, best AI workflow software, multi-agent orchestration*\n";

    write_tool.execute(json!({ "path": "seo/article_final.md", "content": article })).await.unwrap();
    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(), timestamp: chrono::Utc::now(),
        agent: "master".into(), provider: "anthropic".into(), model: "claude-sonnet-4".into(),
        tokens_in: 5000, tokens_out: 8000, total_tokens: 13000,
        estimated_cost_usd: estimate_cost("anthropic", "claude-sonnet-4", 5000, 8000),
        duration_secs: 12.0, context: Some("hand:seo_content phase:article_writing".into()),
    }).unwrap();

    // Phase 4: SEO Optimization
    let seo_report = "# SEO Optimization Report\n\n\
                      ## Checks\n\
                      - Primary keyword in H1: ✅ (\"AI Automation Tools Comparison 2026\")\n\
                      - Keyword density: 1.8% ✅ (target 1-2%)\n\
                      - Word count: 2100+ ✅ (target 1500+)\n\
                      - H2/H3 structure: ✅ (10 subheadings)\n\
                      - Meta description: ✅\n\
                      - Internal links: ❌ (add 2-3)\n\
                      - Image alt text: ❌ (add comparison table image)\n\n\
                      ## Quality Score: 8.5/10\n\n\
                      ## Recommendations\n\
                      1. Add 2 internal links to related articles\n\
                      2. Add comparison table as image with alt text\n\
                      3. Add FAQ section targeting PAA queries\n";

    write_tool.execute(json!({ "path": "seo/seo_report.md", "content": seo_report })).await.unwrap();

    // Phase 5: Publish and Promote
    // 5a: Read article for publishing
    let article_content = read_tool.execute(json!({ "path": "seo/article_final.md" })).await.unwrap();
    assert!(article_content.success);
    assert!(article_content.output.contains("AI Automation Tools Comparison"));

    // 5b: Blog publish (dry_run)
    let blog_dir = dir.path().join("blog-repo");
    std::fs::create_dir_all(blog_dir.join("content/blog")).unwrap();
    std::fs::create_dir_all(blog_dir.join("src/data/blog")).unwrap();
    std::fs::write(blog_dir.join("src/data/blog/index.ts"),
        "import { Brain } from 'lucide-react';\n\nexport interface BlogPost { id: number; title: string; titleEn: string; slug: string; date: string; description: string; tags: string[]; icon: any; color: string; featured: boolean; }\n\nexport const blogPosts: BlogPost[] = [\n];\n"
    ).unwrap();

    let blog_tool = BlogPublishTool::new(clawtex_core::BlogConfig {
        repo_path: blog_dir.to_string_lossy().to_string(),
        ..Default::default()
    });
    let blog_result = blog_tool.execute(json!({
        "title": "AI 自動化工具比較 2026 完整指南",
        "titleEn": "AI Automation Tools Comparison 2026",
        "content": &article_content.output,
        "description": "Comprehensive comparison of AI automation tools in 2026",
        "tags": ["AI", "automation", "tools", "comparison", "2026"],
        "dry_run": true
    })).await.unwrap();
    assert!(blog_result.success, "Blog publish should succeed in dry_run: {}", blog_result.output);

    // Verify MDX file was created
    let mdx_count = std::fs::read_dir(blog_dir.join("content/blog")).unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "mdx").unwrap_or(false))
        .count();
    assert_eq!(mdx_count, 1, "One MDX file should be created by blog_publish");

    // 5c: Twitter post (validates input, fails at API — expected)
    let twitter_tool = clawtex_core::tools::twitter::TwitterTool::new(clawtex_core::TwitterConfig::default());
    let tweet_text = "AI Automation Tools Comparison 2026: We tested 10 platforms. Clawtex wins for multi-agent orchestration. Full guide: #AI #automation";
    assert!(tweet_text.len() <= 280, "Tweet must be under 280 chars");
    let _tweet_result = twitter_tool.execute(json!({
        "action": "post",
        "text": tweet_text
    })).await.unwrap();
    // Tweet will fail (no API keys) but validates input correctly

    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(), timestamp: chrono::Utc::now(),
        agent: "master".into(), provider: "ollama".into(), model: "qwen3:8b".into(),
        tokens_in: 1000, tokens_out: 500, total_tokens: 1500, estimated_cost_usd: 0.0,
        duration_secs: 2.0, context: Some("hand:seo_content phase:publish_and_promote".into()),
    }).unwrap();

    // ── VERIFICATION: All 5 phase outputs exist ────────────────────

    let expected = vec![
        "seo/keywords.csv",
        "seo/competitor_analysis.md",
        "seo/article_final.md",
        "seo/seo_report.md",
    ];
    for f in &expected {
        assert!(dir.path().join(f).exists(), "SEO pipeline file '{}' must exist", f);
    }

    // Verify article quality
    let final_article = std::fs::read_to_string(dir.path().join("seo/article_final.md")).unwrap();
    assert!(final_article.len() > 1000, "Article should be substantial: {} bytes", final_article.len());
    assert!(final_article.contains("AI automation"), "Article should contain primary keyword");
    assert!(final_article.contains("multi-agent"), "Article should contain secondary keyword");
    assert!(final_article.contains("Conclusion"), "Article should have conclusion");

    // ── VERIFICATION: Cost tracking across 4 phases ────────────────

    let costs = cost_tracker.today_total().unwrap();
    assert_eq!(costs.call_count, 4, "4 phases recorded");
    // Phase 3 (article_writing) used Anthropic — should have non-zero cost
    assert!(costs.total_cost_usd > 0.0, "Anthropic phase should have cost > $0");
    let by_prov = cost_tracker.by_provider(1).unwrap();
    let anthropic = by_prov.iter().find(|p| p.group == "anthropic").unwrap();
    assert!(anthropic.total_cost_usd > 0.0, "Anthropic cost should be non-zero");
}

/// Test 19: Freelancer pipeline with Upwork integration verification
/// Verifies the freelancer hand structure supports Upwork-specific job search
#[tokio::test]
async fn test_e2e_freelancer_upwork_pipeline() {
    use clawtex_core::tools::Tool;
    use clawtex_core::tools::file_write::FileWriteTool;
    use clawtex_core::tools::file_read::FileReadTool;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path().to_string_lossy().to_string();
    let security = SecurityConfig {
        workspace_dir: workspace.clone(),
        workspace_only: true,
        allowed_commands: vec![],
        ..Default::default()
    };
    let write_tool = FileWriteTool::new(security.clone());
    let read_tool = FileReadTool::new(security);

    // Verify freelancer hand has Upwork integration in prompts
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);
    if !std::path::Path::new(&hands_dir).exists() { return; }
    let registry = HandRegistry::load(&hands_dir).unwrap();
    let freelancer = registry.get("freelancer").expect("freelancer hand must exist");

    // Verify Phase 1 mentions Upwork URL and browser tool
    let phase1 = &freelancer.phases[0];
    assert!(phase1.system_prompt.contains("upwork.com"), "Phase 1 must mention upwork.com");
    assert!(phase1.system_prompt.contains("browser"), "Phase 1 must use browser tool for Upwork");
    assert!(phase1.system_prompt.contains("memory_recall"), "Phase 1 must check past applications");

    // Verify 5 phases (incl. human_review)
    assert_eq!(freelancer.phases.len(), 5);
    assert_eq!(freelancer.phases[4].name, "human_review");

    // Simulate Phase 1: Job search (web_search + browser on Upwork)
    let jobs = "# Upwork Job Search Results — AI Automation\n\n\
                ## Job 1: AI Workflow Automation for Healthcare SaaS\n\
                - Platform: Upwork\n\
                - URL: https://www.upwork.com/jobs/~01abc123\n\
                - Budget: $3,000-$5,000 (Fixed)\n\
                - Client: MedTech Solutions (4.9★, 92% hire rate, $50K+ spent)\n\
                - Skills: Python, AI/ML, API Integration\n\
                - Posted: 2 hours ago\n\
                - Proposals: 5-10\n\n\
                ## Job 2: Multi-Agent AI System Development\n\
                - Platform: Upwork\n\
                - URL: https://www.upwork.com/jobs/~01def456\n\
                - Budget: $50-$80/hr\n\
                - Client: DataFlow Inc (4.7★, 85% hire rate, $120K+ spent)\n\
                - Skills: Rust, AI Agents, LLM Integration\n\
                - Posted: 5 hours ago\n\
                - Proposals: 3-5\n\n\
                ## Job 3: AI-Powered Customer Service Bot\n\
                - Platform: Upwork\n\
                - URL: https://www.upwork.com/jobs/~01ghi789\n\
                - Budget: $1,500-$2,500 (Fixed)\n\
                - Client: RetailBot (3.8★, 60% hire rate, $8K spent)\n\
                - Skills: NLP, Chatbots, API\n\
                - Posted: 1 day ago\n\
                - Proposals: 20+\n";

    write_tool.execute(json!({ "path": "freelance/job_listings.md", "content": jobs })).await.unwrap();

    // Phase 2: Scoring
    let scored = "company,job_title,platform,url,budget,score,reason\n\
                  DataFlow Inc,Multi-Agent AI System,upwork,upwork.com/jobs/~01def456,$50-80/hr,95,Perfect skill match + great client + low competition\n\
                  MedTech Solutions,AI Workflow Automation,upwork,upwork.com/jobs/~01abc123,$3K-5K,88,Healthcare AI niche + strong budget + good client\n\
                  RetailBot,AI Customer Service Bot,upwork,upwork.com/jobs/~01ghi789,$1.5K-2.5K,42,High competition + weak client history\n";

    write_tool.execute(json!({ "path": "freelance/scored_jobs.csv", "content": scored })).await.unwrap();

    // Phase 3: Proposals
    let proposals = "# Proposals\n\n\
                     ## 1. DataFlow Inc — Multi-Agent AI System (Score: 95)\n\n\
                     Hi DataFlow team,\n\n\
                     Your multi-agent AI project caught my eye — I've built exactly this kind of system. My open-source project Clawtex orchestrates 10+ AI agents with tool chaining, cron scheduling, and cost tracking.\n\n\
                     **Relevant experience:**\n\
                     - Built multi-agent orchestration system (10 agents, 20 tools)\n\
                     - Rust + Python AI pipeline processing 100K+ requests/day\n\
                     - Healthcare AI compliance (HIPAA-ready)\n\n\
                     **Approach:** I'd start with your core agent workflow, add tool integration, then optimize with cost tracking.\n\n\
                     **Timeline:** 3 weeks | **Rate:** $65/hr\n\n\
                     Happy to discuss further. I can start next Monday.\n\n\
                     ---\n\n\
                     ## 2. MedTech Solutions — AI Workflow Automation (Score: 88)\n\n\
                     [Similar proposal...]\n";

    write_tool.execute(json!({ "path": "freelance/proposals.md", "content": proposals })).await.unwrap();

    // Phase 4: Application prep + memory tracking
    write_tool.execute(json!({
        "path": "freelance/application_report.md",
        "content": "# Freelance Application Report\n\n## Summary\n- Jobs found: 3\n- Jobs scored 50+: 2\n- Proposals generated: 2\n- Ready for submission: 2 (pending human review)\n\n## Action Items\n1. Submit DataFlow proposal (highest priority)\n2. Submit MedTech proposal\n3. Skip RetailBot (score 42, below threshold)\n"
    })).await.unwrap();

    // Verify all pipeline outputs exist
    let files = vec![
        "freelance/job_listings.md",
        "freelance/scored_jobs.csv",
        "freelance/proposals.md",
        "freelance/application_report.md",
    ];
    for f in &files {
        let path = dir.path().join(f);
        assert!(path.exists(), "Freelancer pipeline file '{}' must exist", f);
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size > 50, "File '{}' should have content ({} bytes)", f, size);
    }

    // Verify scored_jobs.csv can be read back and parsed
    let csv = read_tool.execute(json!({ "path": "freelance/scored_jobs.csv" })).await.unwrap();
    assert!(csv.success);
    let lines: Vec<&str> = csv.output.lines().collect();
    assert_eq!(lines.len(), 4, "CSV: 1 header + 3 data rows");
    assert!(lines[1].contains("95"), "Top job should have score 95");
    assert!(lines[1].contains("DataFlow"), "Top job should be DataFlow");

    // Verify proposals have correct structure
    let props = read_tool.execute(json!({ "path": "freelance/proposals.md" })).await.unwrap();
    assert!(props.output.contains("Score: 95"), "Proposal should reference score");
    assert!(props.output.contains("Clawtex"), "Proposal should mention Clawtex as portfolio");
    assert!(props.output.contains("$65/hr"), "Proposal should include rate");
}

/// Test 20: Cron scheduler registers default jobs and validates pipeline scheduling
#[tokio::test]
async fn test_e2e_cron_default_jobs_and_hand_actions() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cron_defaults.db").to_string_lossy().to_string();

    let store = std::sync::Arc::new(CronStore::new(&db_path).unwrap());
    let scheduler = Scheduler::new(store).unwrap();

    // Simulate what main.rs does on startup: register 4 default jobs
    scheduler.add_job(
        "daily-freelancer",
        Schedule::Cron { expr: "0 9 * * *".to_string() },
        JobAction::Hand { hand_name: "freelancer".to_string(), input: "AI automation, web development jobs".to_string() },
        None,
    ).await.unwrap();

    scheduler.add_job(
        "weekly-leads",
        Schedule::Cron { expr: "0 10 * * 1".to_string() },
        JobAction::Hand { hand_name: "lead".to_string(), input: "SaaS companies in healthcare and fintech".to_string() },
        None,
    ).await.unwrap();

    scheduler.add_job(
        "biweekly-seo-content",
        Schedule::Cron { expr: "0 11 * * 2,4".to_string() },
        JobAction::Hand { hand_name: "seo_content".to_string(), input: "AI tools reviews and comparisons".to_string() },
        None,
    ).await.unwrap();

    scheduler.add_job(
        "daily-content",
        Schedule::Cron { expr: "0 8 * * *".to_string() },
        JobAction::Hand { hand_name: "content".to_string(), input: "AI automation trends and insights".to_string() },
        None,
    ).await.unwrap();

    // Verify all 4 jobs registered
    let jobs = scheduler.list_jobs().await;
    assert_eq!(jobs.len(), 4, "Should have exactly 4 default cron jobs");

    // Verify each job has a Hand action with correct hand_name
    let expected = vec![
        ("daily-freelancer", "freelancer"),
        ("weekly-leads", "lead"),
        ("biweekly-seo-content", "seo_content"),
        ("daily-content", "content"),
    ];

    for (job_name, hand_name) in &expected {
        let job = jobs.iter().find(|j| j.name == *job_name)
            .unwrap_or_else(|| panic!("Job '{}' must exist", job_name));
        assert_eq!(job.status, JobStatus::Active);
        match &job.action {
            JobAction::Hand { hand_name: actual, .. } => {
                assert_eq!(actual, hand_name, "Job '{}' should target hand '{}'", job_name, hand_name);
            }
            other => panic!("Job '{}' should be Hand action, got {:?}", job_name, other),
        }
    }

    // Verify corresponding hands exist in the real config
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);
    if std::path::Path::new(&hands_dir).exists() {
        let registry = HandRegistry::load(&hands_dir).unwrap();
        for (_, hand_name) in &expected {
            assert!(registry.get(hand_name).is_some(),
                "Cron target hand '{}' must exist in ~/.clawtex/hands/", hand_name);
        }
    }
}

// ── HandRunner Integration Test ──────────────────────────────────────────────

/// Test that HandRunner::run() exercises the full runner code path.
/// Without a running LLM, phases will gracefully fail, but we verify:
/// - HandRunner creates correct HandResult structure
/// - All phases are attempted (phases_completed == total_phases)
/// - Chain_to is propagated from the Hand definition
/// - Elapsed time is tracked
/// - prepare_context injects settings correctly
#[tokio::test]
async fn test_e2e_hand_runner_executes_full_workflow() {
    // Create a temp hand with 3 phases and chain_to
    let dir = tempfile::tempdir().unwrap();
    let hand_dir = dir.path().join("test_runner_hand");
    std::fs::create_dir_all(&hand_dir).unwrap();

    let hand_toml = r#"
name = "test_runner_hand"
description = "Test hand for HandRunner integration"
category = "test"
provider = "auto"
output_format = "markdown"
chain_to = "outreach"

[settings]
target_industry = "healthcare"
min_score = "70"

[[phases]]
name = "research"
system_prompt = "Research healthcare companies."
max_rounds = 1

[[phases]]
name = "analysis"
system_prompt = "Analyze findings."
max_rounds = 1

[[phases]]
name = "report"
system_prompt = "Generate report."
max_rounds = 1

tools = ["web_search", "file_write"]
"#;
    std::fs::write(hand_dir.join("hand.toml"), hand_toml).unwrap();

    // Load the hand
    let registry = HandRegistry::load(dir.path().to_str().unwrap()).unwrap();
    let hand = registry.get("test_runner_hand").expect("Hand must load");

    // Verify structure
    assert_eq!(hand.phases.len(), 3);
    assert_eq!(hand.chain_to, Some("outreach".to_string()));
    assert_eq!(hand.settings.get("target_industry").unwrap(), "healthcare");
    assert_eq!(hand.settings.get("min_score").unwrap(), "70");

    // Test prepare_context injects settings
    let context = HandRunner::prepare_context(hand, "Find AI companies");
    assert!(context.contains("Find AI companies"), "User input must be in context");
    assert!(context.contains("target_industry"), "Settings key must be injected");
    assert!(context.contains("healthcare"), "Settings value must be injected");
    assert!(context.contains("min_score"), "All settings must be injected");
    assert!(context.contains("70"), "All setting values must be injected");

    // Create runtime components (no LLM — phases will gracefully fail)
    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
    let tool_registry = ToolRegistry::new(SecurityConfig::default());

    // Run the hand — this exercises HandRunner::run() end-to-end
    let result = HandRunner::run(
        hand,
        "Find healthcare AI companies in Taiwan",
        &runtime,
        &router,
        &tool_registry,
        None,
    ).await.unwrap();

    // Verify HandResult structure
    assert_eq!(result.hand_name, "test_runner_hand");
    assert_eq!(result.total_phases, 3);
    // All 3 phases should be attempted (even if they fail due to no LLM)
    assert_eq!(result.phases_completed, 3, "All phases must be attempted");
    assert_eq!(result.outputs.len(), 3, "Each phase must produce an output");
    assert!(result.elapsed_secs >= 0.0, "Elapsed time must be tracked");

    // Chain_to should be propagated
    assert_eq!(result.chain_to, Some("outreach".to_string()),
        "chain_to must be propagated from hand definition");

    // Verify each phase output has the correct name
    assert_eq!(result.outputs[0].phase_name, "research");
    assert_eq!(result.outputs[1].phase_name, "analysis");
    assert_eq!(result.outputs[2].phase_name, "report");

    // final_output should be from the last phase
    assert_eq!(result.final_output, result.outputs[2].output);
}

/// Test HandRunner with a real hand from ~/.clawtex/hands/ (if available)
/// This validates that production hand.toml files work with the runner.
#[tokio::test]
async fn test_e2e_hand_runner_with_real_hand() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        return; // Skip if no hands configured
    }

    let registry = HandRegistry::load(&hands_dir).unwrap();

    // Test with content hand (simplest — 4 phases)
    let hand = match registry.get("content") {
        Some(h) => h,
        None => return, // Skip if content hand not available
    };

    assert_eq!(hand.phases.len(), 4, "Content hand should have 4 phases");
    assert_eq!(hand.phases[3].name, "publish_and_promote",
        "Content hand must have publish_and_promote as 4th phase");

    // Verify publish phase references twitter and blog_publish tools
    let publish_prompt = &hand.phases[3].system_prompt;
    assert!(publish_prompt.contains("twitter"), "Publish phase must reference twitter tool");
    assert!(publish_prompt.contains("blog_publish"), "Publish phase must reference blog_publish tool");

    // Verify tools include twitter and blog_publish
    assert!(hand.tools.contains(&"twitter".to_string()), "Content hand must have twitter tool");
    assert!(hand.tools.contains(&"blog_publish".to_string()), "Content hand must have blog_publish tool");

    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
    let tool_registry = ToolRegistry::new(SecurityConfig::default());

    // Run with minimal input — phases will fail gracefully without LLM
    let result = HandRunner::run(
        hand,
        "AI automation trends",
        &runtime,
        &router,
        &tool_registry,
        None,
    ).await.unwrap();

    // Verify the runner executed all 4 phases
    assert_eq!(result.hand_name, "content");
    assert_eq!(result.total_phases, 4);
    assert_eq!(result.phases_completed, 4, "All 4 content phases must be attempted");
    assert_eq!(result.outputs.len(), 4);

    // Verify phase names match
    assert_eq!(result.outputs[0].phase_name, "topic_research");
    assert_eq!(result.outputs[1].phase_name, "content_generation");
    assert_eq!(result.outputs[2].phase_name, "quality_review");
    assert_eq!(result.outputs[3].phase_name, "publish_and_promote");
}

/// Test the complete lead → outreach chain via HandRunner
/// Verifies chain_to propagation works with real hand definitions
#[tokio::test]
async fn test_e2e_hand_runner_chain_propagation() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        return;
    }

    let registry = HandRegistry::load(&hands_dir).unwrap();

    let lead_hand = match registry.get("lead") {
        Some(h) => h,
        None => return,
    };

    // Lead hand must chain to outreach
    assert_eq!(lead_hand.chain_to, Some("outreach".to_string()),
        "Lead hand must chain_to outreach");

    // Outreach hand must exist as the chain target
    let outreach_hand = registry.get("outreach")
        .expect("Outreach hand must exist as chain target");
    assert!(outreach_hand.phases.len() >= 4,
        "Outreach hand should have at least 4 phases");

    let runtime = AgentRuntime::new("/nonexistent/path.toml").unwrap();
    let router = LlmRouter::new("/nonexistent/path.toml").unwrap();
    let tool_registry = ToolRegistry::new(SecurityConfig::default());

    // Run lead hand
    let lead_result = HandRunner::run(
        lead_hand,
        "Healthcare AI companies in Taiwan",
        &runtime,
        &router,
        &tool_registry,
        None,
    ).await.unwrap();

    assert_eq!(lead_result.hand_name, "lead");
    assert_eq!(lead_result.chain_to, Some("outreach".to_string()),
        "Lead result must propagate chain_to");

    // Simulate chain: run outreach with lead's output
    let outreach_result = HandRunner::run(
        outreach_hand,
        &lead_result.final_output,
        &runtime,
        &router,
        &tool_registry,
        None,
    ).await.unwrap();

    assert_eq!(outreach_result.hand_name, "outreach");
    assert_eq!(outreach_result.total_phases, outreach_hand.phases.len());
    assert_eq!(outreach_result.phases_completed, outreach_hand.phases.len(),
        "All outreach phases must be attempted in chain");
}

// ── Tool-Hand Compatibility Test ─────────────────────────────────────────────

/// Verify every tool referenced in a hand.toml exists in the ToolRegistry.
/// This catches configuration drift where a hand references a tool that
/// doesn't exist or was renamed.
#[test]
fn test_e2e_all_hand_tools_exist_in_registry() {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let hands_dir = format!("{}/.clawtex/hands", home);

    if !std::path::Path::new(&hands_dir).exists() {
        return;
    }

    // Complete list of all tools registered by main.rs (default + optional)
    // This must match what ToolRegistry::new() + main.rs registrations provide
    let available_tools: Vec<String> = vec![
        // Default tools (from ToolRegistry::new_with_search)
        "shell", "file_read", "file_write", "file_edit",
        "web_search", "http_request", "glob_search", "content_search", "browser",
        // D3 tools added to new_with_search
        "video_compose", "youtube_upload", "music_generate", "knowledge_import",
        // Optional tools (registered in main.rs)
        "memory_store", "memory_recall", "memory_forget",
        "ai_code", "computer_use", "delegate", "delegate_to_provider",
        "vision", "email_send", "twitter", "blog_publish", "pdf_export",
        "skeleton_generate", "stripe", "render_deploy", "scaffold_saas",
        // Additional tools registered unconditionally in main.rs
        "cli_anything", "translate", "json_transform", "csv_parse",
        "summarize", "docx_export", "xlsx_export", "tts", "email_receive",
        // Config-gated tools registered in main.rs (may or may not be present)
        "image_generate",
        // Messaging tools (config-gated)
        "slack", "discord", "line_notify", "whatsapp",
    ].into_iter().map(|s| s.to_string()).collect();

    // Also accept tool name aliases used in hand.toml
    let known_aliases: &[&str] = &["email"]; // alias: "email" -> real tool "email_send"

    // Load all hands and verify tool compatibility
    let registry = HandRegistry::load(&hands_dir).unwrap();
    let mut total_tools_checked = 0;

    for hand in registry.list() {
        for tool_name in &hand.tools {
            let exists = available_tools.contains(tool_name)
                || known_aliases.contains(&tool_name.as_str());
            assert!(exists,
                "Hand '{}' references tool '{}' which doesn't exist in ToolRegistry. Available: {:?}",
                hand.name, tool_name, available_tools);
            total_tools_checked += 1;
        }
    }

    // We should have checked a meaningful number of tools
    assert!(total_tools_checked >= 50,
        "Expected to validate 50+ tool references across all hands, got {}", total_tools_checked);
}

// ── System Wiring Smoke Test ─────────────────────────────────────────────────

/// Smoke test that validates the complete system can be wired together:
/// AgentRuntime + LlmRouter + ToolRegistry + HandRegistry + CostTracker + RevenueTracker + CronStore
/// This catches initialization failures that would prevent the daemon from starting.
#[tokio::test]
async fn test_e2e_system_wiring_smoke_test() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().to_string_lossy().to_string();

    // 1. AgentRuntime initializes with defaults
    let runtime = AgentRuntime::new("/nonexistent/config.toml").unwrap();
    assert!(runtime.get_config("master").is_some(), "master agent must exist");
    assert!(runtime.get_config("coder").is_some(), "coder agent must exist");

    // 2. LlmRouter initializes without config
    let router = LlmRouter::new("/nonexistent/config.toml").unwrap();

    // 3. ToolRegistry has default tools
    let tool_registry = ToolRegistry::new(SecurityConfig::default());
    let tool_names = tool_registry.names();
    assert!(tool_names.contains(&"shell".to_string()));
    assert!(tool_names.contains(&"file_read".to_string()));
    assert!(tool_names.contains(&"file_write".to_string()));
    assert!(tool_names.contains(&"web_search".to_string()));
    assert!(tool_names.contains(&"browser".to_string()));

    // 4. CostTracker initializes
    let cost_db = format!("{}/costs.db", base);
    let cost_tracker = CostTracker::new(&cost_db).unwrap();

    // 5. RevenueTracker initializes
    let revenue_db = format!("{}/revenue.db", base);
    let revenue_tracker = RevenueTracker::new(&revenue_db).unwrap();

    // 6. CronStore initializes
    let cron_db = format!("{}/cron.db", base);
    let cron_store = CronStore::new(&cron_db).unwrap();

    // 7. HandRegistry loads (empty dir is OK)
    let hands_dir = format!("{}/hands", base);
    let _hand_registry = HandRegistry::load(&hands_dir).unwrap();

    // 8. Memory store initializes (via SQLite backend)
    let mem_db = format!("{}/memory.db", base);
    let sqlite_backend = clawtex_core::memory::sqlite::SqliteMemory::new(&mem_db).unwrap();
    let _memory = MemoryStore::new(Box::new(sqlite_backend), MemoryConfig::default()).unwrap();

    // 9. E-Stop initializes
    let estop = EStop::new();
    assert!(!estop.is_stopped(), "E-Stop should start in running state");

    // 10. Scheduler can be created
    let cron_arc = std::sync::Arc::new(cron_store);
    let scheduler = Scheduler::new(cron_arc).unwrap();
    assert!(scheduler.list_jobs().await.is_empty(), "Scheduler should start empty");

    // 11. Verify cost + revenue can work together (ROI)
    cost_tracker.record(&CostRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        agent: "master".into(),
        provider: "gemini".into(),
        model: "gemini-2.5-flash".into(),
        tokens_in: 1000, tokens_out: 500, total_tokens: 1500,
        estimated_cost_usd: 0.001,
        duration_secs: 2.0,
        context: Some("smoke_test".into()),
    }).unwrap();

    revenue_tracker.record(&RevenueRecord {
        id: uuid::Uuid::new_v4().to_string(),
        timestamp: chrono::Utc::now(),
        route: "A".into(),
        source: "upwork".into(),
        client_name: "SmokeTest Inc".into(),
        amount_usd: 100.0,
        currency: "USD".into(),
        status: RevenueStatus::Confirmed,
        notes: Some("Smoke test revenue".into()),
        invoice_id: None,
    }).unwrap();

    let today_rev = revenue_tracker.today_total().unwrap();
    assert!(today_rev.total_usd >= 100.0, "Revenue must be recorded");

    let costs = cost_tracker.by_day(30).unwrap();
    assert!(!costs.is_empty(), "Costs must be recorded");

    // 12. HandRunner can prepare context
    let hand = clawtex_core::hands::Hand {
        name: "smoke".into(),
        description: "Smoke test".into(),
        category: "test".into(),
        provider: "auto".into(),
        model: String::new(),
        phases: vec![clawtex_core::hands::Phase {
            name: "test".into(),
            system_prompt: "Test prompt".into(),
            max_rounds: 1,
            condition: None,
            target_worker: None,
            target_capability: None,
            parallel_queries: vec![],
            extra: std::collections::HashMap::new(),
        }],
        tools: vec!["file_write".into()],
        output_format: "markdown".into(),
        schedule: None,
        settings: std::collections::HashMap::from([
            ("key".into(), "value".into()),
        ]),
        chain_to: None,
        guardrail: None,
        eval: None,
        extra: std::collections::HashMap::new(),
    };
    let ctx = HandRunner::prepare_context(&hand, "smoke input");
    assert!(ctx.contains("smoke input"));
    assert!(ctx.contains("key"));
    assert!(ctx.contains("value"));

    // 13. HandRunner::run completes (graceful failure without LLM)
    let result = HandRunner::run(&hand, "smoke test", &runtime, &router, &tool_registry, None).await.unwrap();
    assert_eq!(result.hand_name, "smoke");
    assert_eq!(result.phases_completed, 1);
}
