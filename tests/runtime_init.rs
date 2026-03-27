//! Integration tests for PhantomMeshRuntime initialization.

use phantom_mesh::runtime::{PhantomMeshRuntime, RuntimeConfig};

#[tokio::test]
async fn runtime_inits_with_defaults_temp_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = RuntimeConfig {
        data_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let rt = PhantomMeshRuntime::init(cfg).await.expect("runtime init should succeed");

    // AppState is accessible
    let state = rt.app_state();
    assert!(!state.dashboard_token.is_empty());
    assert!(state.started_at.elapsed().as_secs() < 10);
}

#[tokio::test]
async fn runtime_provides_conversation_store() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = RuntimeConfig {
        data_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let rt = PhantomMeshRuntime::init(cfg).await.unwrap();

    // ConversationStore should be usable — a fresh store has zero active sessions
    let convos = rt.app_state().conversations.clone();
    let count = convos.active_count().await;
    assert_eq!(count, 0, "fresh store should have no active conversations");
}

#[tokio::test]
async fn runtime_tool_registry_has_default_tools() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = RuntimeConfig {
        data_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let rt = PhantomMeshRuntime::init(cfg).await.unwrap();

    // ToolRegistry is created — the default constructor registers built-in tools
    let names = rt.app_state().tool_registry.names();
    // The ToolRegistry::new() with default SecurityConfig registers base tools
    // (file_read, file_edit, file_write, shell, etc.)
    assert!(names.len() >= 4, "should have at least 4 built-in tools, got {}: {:?}", names.len(), names);
}

#[tokio::test]
async fn runtime_agent_runtime_accessor() {
    let tmp = tempfile::TempDir::new().unwrap();
    let cfg = RuntimeConfig {
        data_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let rt = PhantomMeshRuntime::init(cfg).await.unwrap();

    // agent_runtime() convenience accessor should return the same Arc
    let a1 = rt.agent_runtime();
    let a2 = &rt.app_state().agent_runtime;
    assert!(std::sync::Arc::ptr_eq(a1, a2));
}
