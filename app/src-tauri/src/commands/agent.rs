use serde_json::Value;
use tauri::State;

use crate::commands::settings::AppConfigState;
use crate::commands::HttpClient;
use crate::runtime_state::RuntimeState;
#[allow(unused_imports)]
use tauri::Manager;

#[tauri::command]
pub async fn run_agent(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    name: String,
    input: String,
    provider: Option<String>,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/agent/{}/run", config.hub_url, name);
    let mut body = serde_json::json!({ "prompt": input });
    if let Some(p) = provider {
        body["provider"] = Value::String(p);
    }
    let resp = http
        .0
        .post(&url)
        .bearer_auth(&config.auth_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn run_hand(
    config: State<'_, AppConfigState>,
    http: State<'_, HttpClient>,
    name: String,
    prompt: String,
) -> Result<Value, String> {
    let config = config.read().clone();
    let url = format!("{}/hand/{}/run", config.hub_url, name);
    let body = serde_json::json!({ "prompt": prompt });
    let resp = http
        .0
        .post(&url)
        .bearer_auth(&config.auth_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?;
    resp.json().await.map_err(|e| e.to_string())
}

/// Direct agent execution — calls PhantomMeshRuntime in-process without HTTP.
/// Emits `agent_event` Tauri events for real-time frontend updates.
#[tauri::command]
pub async fn send_message(
    app: tauri::AppHandle,
    prompt: String,
    agent: Option<String>,
) -> Result<Value, String> {
    use tauri::Emitter;
    use tauri::Manager;

    let agent_log = agent.clone().unwrap_or_else(|| "master".into());
    tracing::info!(target: "phantom-app", "send_message ENTER: agent={} prompt_len={}",
        agent_log, prompt.len());

    let runtime_state = app.try_state::<RuntimeState>()
        .ok_or_else(|| "Runtime 尚未就緒，請稍候再試。".to_string())?;
    let runtime = &runtime_state;

    let agent_name = agent.unwrap_or_else(|| "master".to_string());
    let rt = &runtime.runtime;
    let app_state = rt.app_state();

    // Emit "thinking" event so the frontend can show a spinner
    let _ = app.emit("agent_event", serde_json::json!({
        "type": "thinking",
        "agent": agent_name,
    }));

    // Inject user profile context (mirrors the HTTP gateway logic)
    let profile_ctx = {
        let profile = app_state.user_profile.read().unwrap_or_else(|p| p.into_inner());
        profile.system_prompt_context()
    };

    // Inject goals context for the master agent
    let goals_ctx = if agent_name == "master" {
        app_state.goals_store.as_ref()
            .and_then(|gs| phantom_mesh::goals_push::goals_context(gs).ok())
            .filter(|s| !s.is_empty())
    } else {
        None
    };

    // Combine profile + goals context
    let mut extra = profile_ctx;
    if let Some(goals) = &goals_ctx {
        extra.push_str("\n\n");
        extra.push_str(goals);
    }
    let extra_ref = if extra.is_empty() { None } else { Some(extra.as_str()) };

    // Load conversation history for continuity
    let chat_id = "app-default";
    let history = app_state.conversations.get_history(chat_id).await;

    // Call the agent runtime directly (no HTTP round-trip)
    let result = match app_state
        .agent_runtime
        .run_tracked(
            &agent_name,
            &prompt,
            &history,
            extra_ref,
            &app_state.cost_tracker,
        )
        .await
    {
        Ok(r) => {
            tracing::info!(target: "phantom-app",
                "send_message OK: agent={} elapsed={:.2}s output_len={}",
                agent_name, r.elapsed_secs, r.output.len());
            r
        }
        Err(e) => {
            tracing::error!(target: "phantom-app",
                "send_message FAIL: agent={} err={}", agent_name, e);
            return Err(format!("Agent call failed: {}", e));
        }
    };

    // Save to conversation history
    use phantom_mesh::providers::traits::ChatMessage;
    let user_msg = ChatMessage { role: "user".into(), content: prompt.clone(), tool_calls: None };
    let asst_msg = ChatMessage { role: "assistant".into(), content: result.output.clone(), tool_calls: None };
    app_state.conversations.append(chat_id, user_msg, asst_msg).await;

    // Emit "done" event
    let _ = app.emit("agent_event", serde_json::json!({
        "type": "done",
        "agent": agent_name,
    }));

    Ok(serde_json::json!({
        "agent": agent_name,
        "output": result.output,
        "tool_calls": result.tool_calls_made,
        "elapsed": result.elapsed_secs,
    }))
}

/// Get conversation store status (lightweight health check for the runtime).
#[tauri::command]
pub async fn get_conversations(
    app: tauri::AppHandle,
) -> Result<Value, String> {
    use tauri::Manager;
    let runtime = app.try_state::<RuntimeState>()
        .ok_or_else(|| "Runtime not ready".to_string())?;
    let app_state = runtime.runtime.app_state();
    let sessions = app_state.conversations.active_count().await;
    Ok(serde_json::json!({
        "active_sessions": sessions,
    }))
}

/// Import an agents.toml string fetched from a coordinator's /onboarding/config
/// endpoint. Writes it into the app's config directory and asks the user to
/// restart the app for the new config to take effect.
#[tauri::command]
pub async fn import_agents_toml(
    app: tauri::AppHandle,
    content: String,
) -> Result<String, String> {
    use std::fs;
    use tauri::Manager;

    if content.trim().is_empty() {
        return Err("config 內容是空的".into());
    }
    if !content.contains("[providers") && !content.contains("[cluster]") {
        return Err("不像 agents.toml 內容（缺 [providers] 或 [cluster] 段）".into());
    }

    // Pick a destination: prefer the package-private `files/` dir on Android.
    // Fall back to app_config_dir() on desktop.
    let candidates: Vec<std::path::PathBuf> = {
        let mut v: Vec<std::path::PathBuf> = Vec::new();
        if let Ok(config_dir) = app.path().app_config_dir() {
            v.push(config_dir.join("files").join("agents.toml"));
            v.push(config_dir.join("agents.toml"));
            v.push(config_dir.join(".phantom-mesh").join("agents.toml"));
        }
        if let Some(home) = dirs::home_dir() {
            v.push(home.join(".phantom-mesh").join("agents.toml"));
        }
        v
    };

    // Write to first candidate where parent dir is writable.
    let mut last_err: Option<String> = None;
    for path in &candidates {
        if let Some(parent) = path.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                last_err = Some(format!("create_dir_all {:?}: {}", parent, e));
                continue;
            }
        }
        match fs::write(path, &content) {
            Ok(_) => {
                tracing::info!("Imported agents.toml to {}", path.display());
                return Ok(format!("已寫入 {}，請關閉 app 重新開啟", path.display()));
            }
            Err(e) => {
                last_err = Some(format!("write {:?}: {}", path, e));
            }
        }
    }

    Err(format!("所有目標路徑都寫不進去。最後錯誤: {}",
        last_err.unwrap_or_else(|| "no writable path".into())))
}
