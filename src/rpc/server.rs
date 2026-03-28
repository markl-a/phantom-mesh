use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use serde_json::{json, Value};

use super::protocol::*;

/// State shared across RPC handlers.
#[derive(Clone)]
pub struct RpcState {
    pub node_id: String,
    // We'll add more fields later (AgentRuntime, ToolRegistry, etc.)
    // For now, just enough to handle basic RPC.
}

/// Build the RPC router. Mount at `/rpc` on the main Axum app.
pub fn rpc_router(state: RpcState) -> Router {
    Router::new()
        .route("/rpc", post(handle_rpc))
        .with_state(state)
}

/// Main RPC dispatcher — routes by method name.
async fn handle_rpc(
    State(state): State<RpcState>,
    _headers: HeaderMap,
    Json(request): Json<RpcRequest>,
) -> Json<RpcResponse> {
    // Validate JSON-RPC version
    if request.jsonrpc != "2.0" {
        return Json(RpcResponse::error(
            &request.id,
            ERR_INVALID_REQUEST,
            "Expected jsonrpc 2.0",
        ));
    }

    // Route by method
    let result = match request.method.as_str() {
        "rpc.ping" => handle_ping(&state, &request.params),
        "rpc.status" => handle_status(&state, &request.params),
        "rpc.capabilities" => handle_capabilities(&state, &request.params),
        "rpc.shell" => handle_shell(&state, &request.params).await,
        "rpc.file.read" => handle_file_read(&state, &request.params).await,
        _ => Err(RpcResponse::error(
            &request.id,
            ERR_METHOD_NOT_FOUND,
            &format!("Unknown method: {}", request.method),
        )),
    };

    match result {
        Ok(value) => Json(RpcResponse::success(&request.id, value)),
        Err(err_resp) => Json(err_resp),
    }
}

fn handle_ping(_state: &RpcState, _params: &Value) -> Result<Value, RpcResponse> {
    Ok(json!({ "pong": true }))
}

fn handle_status(state: &RpcState, _params: &Value) -> Result<Value, RpcResponse> {
    Ok(json!({
        "node_id": state.node_id,
        "status": "online",
    }))
}

fn handle_capabilities(state: &RpcState, _params: &Value) -> Result<Value, RpcResponse> {
    // Placeholder — will be filled with real capabilities in Phase 0.6
    Ok(json!({
        "node_id": state.node_id,
        "capabilities": ["shell", "browser", "network"],
    }))
}

async fn handle_shell(_state: &RpcState, params: &Value) -> Result<Value, RpcResponse> {
    let command = params
        .get("command")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RpcResponse::error("", ERR_INVALID_PARAMS, "Missing 'command' param"))?;

    // Execute shell command (basic — will be enhanced with proper tool integration)
    let output = tokio::process::Command::new(if cfg!(windows) { "cmd" } else { "sh" })
        .args(if cfg!(windows) {
            vec!["/C", command]
        } else {
            vec!["-c", command]
        })
        .output()
        .await
        .map_err(|e| RpcResponse::error("", ERR_INTERNAL, &format!("Shell exec failed: {}", e)))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(json!({
        "exit_code": output.status.code().unwrap_or(-1),
        "stdout": stdout,
        "stderr": stderr,
    }))
}

async fn handle_file_read(_state: &RpcState, params: &Value) -> Result<Value, RpcResponse> {
    let path = params
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RpcResponse::error("", ERR_INVALID_PARAMS, "Missing 'path' param"))?;

    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| RpcResponse::error("", ERR_INTERNAL, &format!("Read failed: {}", e)))?;

    Ok(json!({
        "path": path,
        "content": content,
        "size": content.len(),
    }))
}
