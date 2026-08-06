//! Conversation IPC commands — proxy the in-process SpectynMesh runtime's
//! session store so the desktop app's ConversationView / ConversationSelector
//! work without a separate daemon. The browser `httpFallback` in
//! `app/src/lib/tauri-compat.ts` mirrors these against the standalone daemon's
//! `/conversations/*` routes (core/src/main.rs); these commands give the Tauri
//! path the same shapes from the in-process `SessionStore`.

use serde_json::{json, Value};
use tauri::Manager;

use crate::runtime_state::RuntimeState;

/// Load a single conversation's message history.
/// Mirrors the daemon's `GET /conversations/:chat_id/history` shape:
/// `{ "chat_id": <id>, "messages": [{ role, content }, ...] }`.
///
/// `rename_all = "snake_case"`: every JS call site (api.ts, ConversationView,
/// MobileConversation) passes `{ chat_id }`, and the browser httpFallback in
/// tauri-compat.ts reads `args.chat_id`. Tauri 2's default expects camelCase
/// (`chatId`) from JS, so without this attribute the invoke fails at runtime
/// with "invalid args".
#[tauri::command(rename_all = "snake_case")]
pub async fn get_conversation_history(
    app: tauri::AppHandle,
    chat_id: String,
) -> Result<Value, String> {
    let runtime = app
        .try_state::<RuntimeState>()
        .ok_or_else(|| "Runtime 尚未就緒，請稍候再試。".to_string())?;
    let app_state = runtime.runtime.app_state();
    let history = app_state.conversations.get_history(&chat_id).await;
    let messages: Vec<Value> = history
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();
    Ok(json!({ "chat_id": chat_id, "messages": messages }))
}

/// List known conversations (id + message_count).
/// Mirrors the daemon's `GET /conversations/list` shape, enriched with the
/// per-session `message_count` the ConversationSelector renders:
/// `{ "conversations": [{ id, message_count }, ...] }`.
#[tauri::command]
pub async fn list_conversations(app: tauri::AppHandle) -> Result<Value, String> {
    let runtime = app
        .try_state::<RuntimeState>()
        .ok_or_else(|| "Runtime 尚未就緒，請稍候再試。".to_string())?;
    let app_state = runtime.runtime.app_state();
    let infos = app_state.conversations.list_with_info().await;
    let conversations: Vec<Value> = infos
        .iter()
        .map(|s| json!({ "id": s.id, "message_count": s.message_count }))
        .collect();
    Ok(json!({ "conversations": conversations }))
}

/// Reset (delete) a conversation's history.
/// Mirrors the daemon's `POST /conversations/:chat_id/reset` shape:
/// `{ "chat_id": <id>, "reset": <bool> }`.
///
/// `rename_all = "snake_case"`: JS call sites pass `{ chat_id }` — see
/// [`get_conversation_history`].
#[tauri::command(rename_all = "snake_case")]
pub async fn reset_conversation(
    app: tauri::AppHandle,
    chat_id: String,
) -> Result<Value, String> {
    let runtime = app
        .try_state::<RuntimeState>()
        .ok_or_else(|| "Runtime 尚未就緒，請稍候再試。".to_string())?;
    let app_state = runtime.runtime.app_state();
    let deleted = app_state.conversations.delete(&chat_id).await;
    Ok(json!({ "chat_id": chat_id, "reset": deleted }))
}
