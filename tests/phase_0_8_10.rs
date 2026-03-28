use phantom_mesh::providers::cluster_fallback::{ProviderAvailability, nodes_with_provider};
use phantom_mesh::data::cross_node::*;
use phantom_mesh::config::{ClusterConfig, NodeConfig};

#[test]
fn test_nodes_with_provider() {
    let availability = vec![
        ProviderAvailability { node_id: "a".into(), providers: vec!["anthropic".into(), "openai".into()] },
        ProviderAvailability { node_id: "b".into(), providers: vec!["openai".into()] },
        ProviderAvailability { node_id: "c".into(), providers: vec!["gemini".into()] },
    ];
    let result = nodes_with_provider(&availability, "openai");
    assert_eq!(result, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn test_nodes_with_provider_none() {
    let availability = vec![
        ProviderAvailability { node_id: "a".into(), providers: vec!["anthropic".into()] },
    ];
    assert!(nodes_with_provider(&availability, "openai").is_empty());
}

#[test]
fn test_remote_conversation_request() {
    let (method, params) = get_remote_conversation_request("chat-123");
    assert_eq!(method, "rpc.conversations.get");
    assert_eq!(params["chat_id"], "chat-123");
}

#[test]
fn test_remote_task_dispatch() {
    let (method, params) = dispatch_remote_task_request("Research AI", "Find papers", "high");
    assert_eq!(method, "rpc.tasks.dispatch");
    assert_eq!(params["title"], "Research AI");
    assert_eq!(params["priority"], "high");
}

#[test]
fn test_cluster_config_merge_newer() {
    let mut config = ClusterConfig { version: 1, ..Default::default() };
    let newer = ClusterConfig { version: 2, daily_budget_usd: Some(10.0), ..Default::default() };
    assert!(config.merge(&newer));
    assert_eq!(config.version, 2);
    assert_eq!(config.daily_budget_usd, Some(10.0));
}

#[test]
fn test_cluster_config_merge_older_noop() {
    let mut config = ClusterConfig { version: 3, ..Default::default() };
    let older = ClusterConfig { version: 1, daily_budget_usd: Some(99.0), ..Default::default() };
    assert!(!config.merge(&older));
    assert_eq!(config.version, 3);
}

#[test]
fn test_node_config_default_port() {
    let config: NodeConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(config.port, 7878);
}
