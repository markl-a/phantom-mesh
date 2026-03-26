//! Phantom Mesh 效能基準測試
//!
//! 用 criterion 測量關鍵路徑的效能，確保沒有回歸。
//! 執行：cargo bench
//! 報告：target/criterion/report/index.html

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId};
use phantom_mesh::*;
use serde_json::json;

// ── Context Optimizer 基準 ──────────────────────────────────────────────────

fn bench_context_trim(c: &mut Criterion) {
    let mut group = c.benchmark_group("context_optimizer");

    for msg_count in [10, 50, 100, 500].iter() {
        group.bench_with_input(
            BenchmarkId::new("trim_messages", msg_count),
            msg_count,
            |b, &count| {
                b.iter(|| {
                    let mut messages: Vec<ChatMessage> = Vec::with_capacity(count + 1);
                    messages.push(ChatMessage {
                        role: "system".to_string(),
                        content: "You are a helpful assistant.".to_string(),
                        tool_calls: None,
                        tool_call_id: None,
                    });
                    for i in 0..count {
                        messages.push(ChatMessage {
                            role: if i % 2 == 0 { "user" } else { "assistant" }.to_string(),
                            content: "x".repeat(200),
                            tool_calls: None,
                            tool_call_id: None,
                        });
                    }
                    ContextOptimizer::trim_messages(black_box(&mut messages), "qwen3:8b");
                    messages.len()
                });
            },
        );
    }

    group.finish();
}

// ── Credential Scrubbing 基準 ───────────────────────────────────────────────

fn bench_scrub_credentials(c: &mut Criterion) {
    let mut group = c.benchmark_group("credential_scrubbing");

    let test_cases = vec![
        ("short_clean", "Hello world, no secrets here."),
        ("short_with_key", "My API key is sk-1234567890abcdef1234567890abcdef"),
        ("long_clean", &"The quick brown fox. ".repeat(100)),
        ("long_mixed", &format!(
            "Some text {} more text {} end",
            "password=SuperSecret123!",
            "AKIA1234567890ABCDEF"
        )),
    ];

    for (name, text) in test_cases {
        group.bench_with_input(
            BenchmarkId::new("scrub", name),
            &text.to_string(),
            |b, text| {
                b.iter(|| scrub_credentials(black_box(text)));
            },
        );
    }

    group.finish();
}

// ── Tool Dispatch 基準 ──────────────────────────────────────────────────────

fn bench_tool_dispatch_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("tool_dispatch");

    // XML tool call 解析
    let xml_response = r#"I'll help you with that.
<tool_call>
{"name": "shell", "arguments": {"command": "ls -la"}}
</tool_call>
Let me check the results."#;

    group.bench_function("parse_xml_tool_call", |b| {
        b.iter(|| {
            parse_tool_calls(black_box(xml_response), DispatchMode::Xml)
        });
    });

    // JSON tool call 解析
    let json_response = json!([{
        "id": "call_1",
        "function": {
            "name": "shell",
            "arguments": "{\"command\": \"ls -la\"}"
        }
    }]);

    group.bench_function("parse_json_tool_calls", |b| {
        b.iter(|| {
            // 模擬解析 JSON tool calls
            let _calls: Vec<ToolCall> = serde_json::from_value(black_box(json_response.clone()))
                .unwrap_or_default();
        });
    });

    group.finish();
}

// ── Memory Store 基準 ──────────────────────────────────────────────────────

fn bench_memory_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_store");

    group.bench_function("store_and_recall", |b| {
        let rt = tokio::runtime::Runtime::new().unwrap();
        b.iter(|| {
            rt.block_on(async {
                let tmp = tempfile::tempdir().unwrap();
                let db_path = tmp.path().join("bench_memory.db");
                let config = MemoryConfig {
                    db_path: db_path.to_string_lossy().to_string(),
                    max_entries: 10000,
                    ..Default::default()
                };
                let store = MemoryStore::new(config).unwrap();

                // 存 10 筆
                for i in 0..10 {
                    let _ = store.store(
                        &format!("bench_key_{}", i),
                        &format!("value_{}", i),
                        MemoryCategory::Core,
                    ).await;
                }

                // 回想
                let _ = store.recall("bench_key_5").await;
            });
        });
    });

    group.finish();
}

// ── Conversation Store 基準 ─────────────────────────────────────────────────

fn bench_conversation(c: &mut Criterion) {
    let mut group = c.benchmark_group("conversation_store");

    group.bench_function("add_and_get_100_messages", |b| {
        b.iter(|| {
            let tmp = tempfile::tempdir().unwrap();
            let db_path = tmp.path().join("bench_conv.db");
            let store = ConversationStore::new(db_path.to_string_lossy().as_ref()).unwrap();

            let chat_id = 12345i64;
            for i in 0..100 {
                let role = if i % 2 == 0 { "user" } else { "assistant" };
                store.add_message(chat_id, role, &format!("Message {}", i)).unwrap();
            }

            let _messages = store.get_messages(chat_id, 50).unwrap();
        });
    });

    group.finish();
}

// ── EStop 基準 ──────────────────────────────────────────────────────────────

fn bench_estop(c: &mut Criterion) {
    let mut group = c.benchmark_group("estop");

    group.bench_function("check_not_stopped", |b| {
        let estop = EStop::new();
        b.iter(|| {
            black_box(estop.check()).unwrap();
        });
    });

    group.bench_function("check_stopped", |b| {
        let estop = EStop::new();
        estop.stop();
        b.iter(|| {
            let _ = black_box(estop.check());
        });
    });

    group.finish();
}

// ── 組合執行 ────────────────────────────────────────────────────────────────

criterion_group!(
    benches,
    bench_context_trim,
    bench_scrub_credentials,
    bench_tool_dispatch_parsing,
    bench_memory_operations,
    bench_conversation,
    bench_estop,
);
criterion_main!(benches);
