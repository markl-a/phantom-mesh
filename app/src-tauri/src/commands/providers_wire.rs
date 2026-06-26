// Wave H1.2 — Tauri command surface for SPEC-14 LLM provider routing + completion.
//
// Wraps `phantom_mesh::providers_wire` so the H2.2 React Conversation surface
// can pick a provider, validate config, and run synchronous or
// fire-and-forget completions through Tauri's invoke channel.
//
// The streaming variant `providers_complete_streaming` returns a `request_id`
// immediately and emits a `providers_complete_event` on the Tauri event bus
// with one of three shapes: `{kind:"started",request_id}` /
// `{kind:"done",request_id,response}` / `{kind:"error",request_id,error}`.
// Real per-token streaming requires a core API providers_wire does not yet
// expose; the event channel here keeps the frontend wire stable so when
// core grows token-stream support we widen the `kind` enum without churning
// the Tauri seam.

use phantom_mesh::providers_wire::{
    self, LatencyClass, ProviderClass, ProviderConfig, ProviderError, ProviderRequest,
    ProviderResponse,
};
use serde_json::json;
use tauri::{AppHandle, Emitter};

fn err_string(e: ProviderError) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn providers_select_provider(
    class: ProviderClass,
    latency: LatencyClass,
) -> Result<String, String> {
    providers_wire::select_provider(class, latency).map_err(err_string)
}

#[tauri::command]
pub async fn providers_validate_config(config: ProviderConfig) -> Result<(), String> {
    providers_wire::validate_config(&config).map_err(err_string)
}

#[tauri::command]
pub async fn providers_complete(req: ProviderRequest) -> Result<ProviderResponse, String> {
    tokio::task::spawn_blocking(move || providers_wire::complete(req))
        .await
        .map_err(|e| format!("provider task join failed: {e}"))?
        .map_err(err_string)
}

#[tauri::command]
pub async fn providers_complete_streaming(
    app: AppHandle,
    request_id: String,
    req: ProviderRequest,
) -> Result<String, String> {
    let _ = app.emit(
        "providers_complete_event",
        json!({"kind": "started", "request_id": request_id}),
    );

    let app_for_task = app.clone();
    let request_id_for_task = request_id.clone();
    tokio::task::spawn_blocking(move || {
        let result = providers_wire::complete(req);
        match result {
            Ok(response) => {
                let _ = app_for_task.emit(
                    "providers_complete_event",
                    json!({
                        "kind": "done",
                        "request_id": request_id_for_task,
                        "response": response,
                    }),
                );
            }
            Err(e) => {
                let _ = app_for_task.emit(
                    "providers_complete_event",
                    json!({
                        "kind": "error",
                        "request_id": request_id_for_task,
                        "error": e.to_string(),
                    }),
                );
            }
        }
    });

    Ok(request_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use phantom_mesh::providers_wire::{MessageRole, ResponseFormat};

    fn make_request(model: &str) -> ProviderRequest {
        ProviderRequest {
            model: model.to_string(),
            system_prompt: None,
            messages: vec![phantom_mesh::providers_wire::Message {
                role: MessageRole::User,
                content: "hi".to_string(),
                images: Vec::new(),
            }],
            max_tokens: Some(8),
            temperature: Some(0.0),
            response_format: ResponseFormat::PlainText,
            tools: Vec::new(),
        }
    }

    fn make_config(slug: &str) -> ProviderConfig {
        ProviderConfig {
            slug: slug.to_string(),
            api_key_ref: "vault#test".to_string(),
            default_model: "test-model".to_string(),
            base_url: None,
            timeout_ms: 5000,
        }
    }

    fn point_at_missing_toml() {
        // Test isolation: every test in this module pins the agents.toml path
        // to a guaranteed-missing file so `read_agents_toml` returns
        // `ProviderError::Unknown` deterministically. All tests pin to the
        // same path so parallel execution stays safe.
        // SAFETY: Rust 2024 made set_var unsafe; the value is a static
        // string and tests are single-process.
        unsafe {
            std::env::set_var(
                "PHANTOM_MESH_AGENTS_TOML",
                "/nonexistent/path/phantom-mesh-test-agents.toml",
            );
        }
    }

    #[tokio::test]
    async fn select_provider_errors_when_toml_missing() {
        point_at_missing_toml();
        let err = providers_select_provider(ProviderClass::Frontier, LatencyClass::Interactive)
            .await
            .unwrap_err();
        assert!(
            err.contains("agents.toml read failed") || err.starts_with("provider."),
            "got {err}"
        );
    }

    #[tokio::test]
    async fn complete_errors_when_toml_missing() {
        point_at_missing_toml();
        let err = providers_complete(make_request("nonexistent-model-xyz"))
            .await
            .unwrap_err();
        assert!(err.starts_with("provider."), "got {err}");
    }

    #[tokio::test]
    async fn validate_config_rejects_empty_slug() {
        let mut cfg = make_config("groq");
        cfg.slug = "".to_string();
        let err = providers_validate_config(cfg).await.unwrap_err();
        assert!(err.contains("slug"), "got {err}");
    }

    #[tokio::test]
    async fn validate_config_rejects_empty_api_key_ref() {
        let mut cfg = make_config("groq");
        cfg.api_key_ref = "".to_string();
        let err = providers_validate_config(cfg).await.unwrap_err();
        assert!(
            err.contains("api_key_ref") || err.contains("auth_error"),
            "got {err}"
        );
    }

    #[tokio::test]
    async fn validate_config_rejects_empty_default_model() {
        let mut cfg = make_config("groq");
        cfg.default_model = "".to_string();
        let err = providers_validate_config(cfg).await.unwrap_err();
        assert!(
            err.contains("default_model") || err.contains("model_not_found"),
            "got {err}"
        );
    }

    #[tokio::test]
    async fn validate_config_rejects_zero_timeout() {
        let mut cfg = make_config("groq");
        cfg.timeout_ms = 0;
        let err = providers_validate_config(cfg).await.unwrap_err();
        assert!(err.contains("timeout"), "got {err}");
    }
}
