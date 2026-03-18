//! SSE and WebSocket gateway for streaming agent responses + event bus.
//! - `GET /stream/agent/:name?prompt=...` — Server-Sent Events
//! - `GET /events/agent/:name` — Agent event bus SSE stream
//! - `WS  /ws/agent/:name` — WebSocket full-duplex

use axum::{
    extract::{Path, Query, State, WebSocketUpgrade, ws},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, StreamExt, SinkExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};

use crate::agent_events::AgentEvent;
use crate::agent_runtime::AgentRuntime;
use crate::circuit_breaker::ProviderCircuitBreaker;
use crate::estop::EStop;
use crate::llm_router::{LlmRouter, ChatMessage};
use crate::providers::StreamChunk;
use crate::tools::ToolRegistry;
use crate::trajectory::{TrajectoryLogger, TrajectoryEntry};
use crate::watchdog::WorkerWatchdog;

/// Shared state needed by gateway handlers
#[derive(Clone)]
pub struct GatewayState {
    pub agent_runtime: Arc<AgentRuntime>,
    pub llm_router: Arc<LlmRouter>,
    pub tool_registry: Arc<ToolRegistry>,
    pub estop: Arc<EStop>,
    pub trajectory_logger: Option<Arc<TrajectoryLogger>>,
    pub circuit_breaker: Option<Arc<ProviderCircuitBreaker>>,
    pub watchdog: Option<Arc<tokio::sync::Mutex<WorkerWatchdog>>>,
    /// Rate limiter for /agent/think — per-worker call counts with timestamps
    pub agent_think_rate: Arc<Mutex<HashMap<String, Vec<std::time::Instant>>>>,
}

#[derive(Deserialize)]
pub struct StreamQuery {
    pub prompt: Option<String>,
}

// ── SSE endpoint ─────────────────────────────────────────────────────────────

/// `GET /stream/agent/:name?prompt=...`
/// Returns an SSE stream of agent responses.
pub async fn sse_agent(
    Path(agent_name): Path<String>,
    Query(query): Query<StreamQuery>,
    State(state): State<GatewayState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, axum::http::StatusCode> {
    let prompt = query.prompt.unwrap_or_default();
    if prompt.is_empty() {
        return Err(axum::http::StatusCode::BAD_REQUEST);
    }

    // Check E-Stop
    if state.estop.is_stopped() {
        return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    info!("SSE stream request: agent='{}', prompt='{}'", agent_name, truncate(&prompt, 60));

    let stream = AgentSseStream::new(
        state.agent_runtime,
        state.llm_router,
        state.tool_registry,
        state.estop,
        agent_name,
        prompt,
    );

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Wraps agent streaming into SSE Events
struct AgentSseStream {
    inner: Pin<Box<dyn Stream<Item = Result<StreamChunk, anyhow::Error>> + Send>>,
    finished: bool,
}

impl AgentSseStream {
    fn new(
        runtime: Arc<AgentRuntime>,
        router: Arc<LlmRouter>,
        tools: Arc<ToolRegistry>,
        estop: Arc<EStop>,
        agent_name: String,
        prompt: String,
    ) -> Self {
        // Create a channel-based stream
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<StreamChunk, anyhow::Error>>(32);

        tokio::spawn(async move {
            if estop.is_stopped() {
                let _ = tx.send(Err(anyhow::anyhow!("E-Stop active"))).await;
                return;
            }

            match runtime.run_streaming(
                &agent_name, &prompt, &[], &router, &tools, None,
            ).await {
                Ok(mut stream) => {
                    while let Some(chunk) = stream.next().await {
                        if estop.is_stopped() {
                            let _ = tx.send(Err(anyhow::anyhow!("E-Stop activated during stream"))).await;
                            break;
                        }
                        if tx.send(chunk).await.is_err() {
                            break; // Client disconnected
                        }
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e)).await;
                }
            }
        });

        Self {
            inner: Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)),
            finished: false,
        }
    }
}

impl Stream for AgentSseStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.finished {
            return Poll::Ready(None);
        }

        match self.inner.as_mut().poll_next(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                self.finished = true;
                // Send a final [DONE] event
                Poll::Ready(Some(Ok(Event::default().event("done").data("[DONE]"))))
            }
            Poll::Ready(Some(Ok(chunk))) => {
                let event = match &chunk {
                    StreamChunk::ContentDelta(text) => {
                        Event::default().event("content").data(text.clone())
                    }
                    StreamChunk::ToolCallStart { id, name } => {
                        Event::default().event("tool_start").data(
                            json!({"id": id, "name": name}).to_string()
                        )
                    }
                    StreamChunk::ToolCallArgumentsDelta { id, delta } => {
                        Event::default().event("tool_args").data(
                            json!({"id": id, "delta": delta}).to_string()
                        )
                    }
                    StreamChunk::Done { usage } => {
                        self.finished = true;
                        let data = if let Some(u) = usage {
                            json!({"total_tokens": u.total_tokens}).to_string()
                        } else {
                            "{}".to_string()
                        };
                        Event::default().event("done").data(data)
                    }
                };
                Poll::Ready(Some(Ok(event)))
            }
            Poll::Ready(Some(Err(e))) => {
                self.finished = true;
                Poll::Ready(Some(Ok(
                    Event::default().event("error").data(e.to_string())
                )))
            }
        }
    }
}

// ── WebSocket endpoint ───────────────────────────────────────────────────────

/// `GET /ws/agent/:name` — WebSocket upgrade
/// Client sends JSON: `{"prompt": "..."}`
/// Server sends JSON: `{"event": "content", "data": "..."}` / `{"event": "done"}`
pub async fn ws_agent(
    Path(agent_name): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<GatewayState>,
) -> axum::response::Response {
    info!("WebSocket upgrade request: agent='{}'", agent_name);
    ws.on_upgrade(move |socket| handle_ws(socket, agent_name, state))
}

async fn handle_ws(socket: ws::WebSocket, agent_name: String, state: GatewayState) {
    let (mut sender, mut receiver) = socket.split();

    // Read messages from client
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            ws::Message::Text(text) => {
                let parsed: Result<serde_json::Value, _> = serde_json::from_str(&text);
                let prompt = match parsed {
                    Ok(v) => v.get("prompt").and_then(|p| p.as_str()).unwrap_or("").to_string(),
                    Err(_) => {
                        // Treat raw text as prompt
                        text.to_string()
                    }
                };

                if prompt.is_empty() {
                    let _ = sender.send(ws::Message::Text(
                        json!({"event": "error", "data": "Empty prompt"}).to_string().into()
                    )).await;
                    continue;
                }

                // Check E-Stop
                if state.estop.is_stopped() {
                    let _ = sender.send(ws::Message::Text(
                        json!({"event": "error", "data": "E-Stop active"}).to_string().into()
                    )).await;
                    continue;
                }

                debug!("WS agent='{}', prompt='{}'", agent_name, truncate(&prompt, 60));

                // Run streaming agent
                match state.agent_runtime.run_streaming(
                    &agent_name, &prompt, &[], &state.llm_router, &state.tool_registry, None,
                ).await {
                    Ok(mut stream) => {
                        while let Some(chunk) = stream.next().await {
                            if state.estop.is_stopped() {
                                let _ = sender.send(ws::Message::Text(
                                    json!({"event": "error", "data": "E-Stop activated"}).to_string().into()
                                )).await;
                                break;
                            }

                            let ws_msg = match chunk {
                                Ok(StreamChunk::ContentDelta(text)) => {
                                    json!({"event": "content", "data": text})
                                }
                                Ok(StreamChunk::ToolCallStart { id, name }) => {
                                    json!({"event": "tool_start", "id": id, "name": name})
                                }
                                Ok(StreamChunk::ToolCallArgumentsDelta { id, delta }) => {
                                    json!({"event": "tool_args", "id": id, "delta": delta})
                                }
                                Ok(StreamChunk::Done { usage }) => {
                                    let tokens = usage.map(|u| u.total_tokens).unwrap_or(0);
                                    json!({"event": "done", "total_tokens": tokens})
                                }
                                Err(e) => {
                                    json!({"event": "error", "data": e.to_string()})
                                }
                            };

                            if sender.send(ws::Message::Text(ws_msg.to_string().into())).await.is_err() {
                                break; // Client disconnected
                            }
                        }
                    }
                    Err(e) => {
                        error!("WS agent error: {}", e);
                        let _ = sender.send(ws::Message::Text(
                            json!({"event": "error", "data": e.to_string()}).to_string().into()
                        )).await;
                    }
                }
            }
            ws::Message::Close(_) => break,
            _ => {} // Ignore binary, ping, pong
        }
    }

    debug!("WebSocket connection closed for agent '{}'", agent_name);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}...", &s[..max]) }
}

// ── Agent Event Bus SSE endpoint ────────────────────────────────────────────

/// `GET /events/agent/:name` — Subscribe to agent event bus via SSE.
/// Streams JSON-serialized AgentEvent objects in real-time.
pub async fn sse_agent_events(
    Path(_agent_name): Path<String>,
    State(state): State<GatewayState>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, axum::http::StatusCode> {
    let bus = state.agent_runtime.event_bus()
        .ok_or(axum::http::StatusCode::SERVICE_UNAVAILABLE)?;

    let rx = bus.subscribe();
    info!("SSE event subscription started");

    let stream = EventBusStream { rx };
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Wraps a broadcast::Receiver into an SSE stream.
struct EventBusStream {
    rx: tokio::sync::broadcast::Receiver<AgentEvent>,
}

impl Stream for EventBusStream {
    type Item = Result<Event, Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match self.rx.try_recv() {
            Ok(event) => {
                let json = serde_json::to_string(&event).unwrap_or_default();
                let sse_event = Event::default()
                    .event("agent_event")
                    .data(json);
                Poll::Ready(Some(Ok(sse_event)))
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                // Register waker and return pending
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                // Missed some events, continue
                let event = Event::default()
                    .event("warning")
                    .data("events_lagged");
                Poll::Ready(Some(Ok(event)))
            }
            Err(tokio::sync::broadcast::error::TryRecvError::Closed) => {
                Poll::Ready(None)
            }
        }
    }
}

// ── Trajectory query params ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct TrajectoryQuery {
    pub days: Option<u32>,
    pub agent: Option<String>,
    pub hand: Option<String>,
    pub limit: Option<usize>,
}

// ── REST: Trajectory + Cluster Health endpoints ─────────────────────────────

/// `GET /trajectories?days=7&hand=seo_content&limit=100`
///
/// Query trajectory entries filtered by hand name or recent days.
pub async fn get_trajectories(
    Query(query): Query<TrajectoryQuery>,
    State(state): State<GatewayState>,
) -> Result<axum::Json<serde_json::Value>, axum::http::StatusCode> {
    let logger = state.trajectory_logger.as_ref()
        .ok_or(axum::http::StatusCode::SERVICE_UNAVAILABLE)?;

    let days = query.days.unwrap_or(7);
    let limit = query.limit.unwrap_or(100);

    let entries = if let Some(hand) = &query.hand {
        logger.by_hand(hand, limit)
    } else {
        logger.recent(days, limit)
    };

    match entries {
        Ok(entries) => Ok(axum::Json(serde_json::json!({
            "count": entries.len(),
            "entries": entries,
        }))),
        Err(e) => {
            tracing::warn!("Failed to query trajectories: {}", e);
            Err(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// `GET /trajectories/stats`
///
/// Return aggregated quality statistics (per provider+model) and worker
/// efficiency statistics from the trajectory database.
pub async fn get_trajectory_stats(
    State(state): State<GatewayState>,
) -> Result<axum::Json<serde_json::Value>, axum::http::StatusCode> {
    let logger = state.trajectory_logger.as_ref()
        .ok_or(axum::http::StatusCode::SERVICE_UNAVAILABLE)?;

    let quality = logger.quality_stats().map_err(|e| {
        tracing::warn!("Failed to get quality stats: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let workers = logger.worker_stats().map_err(|e| {
        tracing::warn!("Failed to get worker stats: {}", e);
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(axum::Json(serde_json::json!({
        "quality_stats": quality,
        "worker_stats": workers,
    })))
}

/// `GET /cluster/health`
///
/// Return circuit breaker status for all providers and watchdog status for all
/// configured workers. Both sections are omitted when the corresponding
/// component is not initialized.
pub async fn get_cluster_health(
    State(state): State<GatewayState>,
) -> Result<axum::Json<serde_json::Value>, axum::http::StatusCode> {
    let circuit_status = state.circuit_breaker.as_ref().map(|cb| {
        let status_map = cb.status();
        let statuses: Vec<_> = status_map.into_values().collect();
        statuses
    });

    let watchdog_status = if let Some(wd) = &state.watchdog {
        let wd_guard = wd.lock().await;
        Some(wd_guard.status_snapshot().await)
    } else {
        None
    };

    let watchdog_events = if let Some(wd) = &state.watchdog {
        let wd_guard = wd.lock().await;
        Some(wd_guard.recent_events(20).await)
    } else {
        None
    };

    Ok(axum::Json(serde_json::json!({
        "circuit_breakers": circuit_status,
        "watchdog": watchdog_status,
        "watchdog_events": watchdog_events,
    })))
}

// ── Agent Think endpoint ─────────────────────────────────────────────────────

/// Request body for `POST /agent/think`
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentThinkRequest {
    /// Worker name (e.g. "rog6", "mipad")
    pub worker: String,
    /// The high-level goal / current prompt
    pub prompt: String,
    /// Conversation history so far
    #[serde(default)]
    pub history: Vec<AgentThinkMessage>,
    /// Tools available on this device
    #[serde(default)]
    pub available_tools: Vec<String>,
}

/// Simplified message for agent think history
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentThinkMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// Response from `POST /agent/think`
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AgentThinkResponse {
    /// The agent's reasoning about what to do next
    pub reasoning: String,
    /// Which tool to call next (None if done)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_tool: Option<String>,
    /// Arguments for the next tool call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_args: Option<serde_json::Value>,
    /// Whether the agent considers the task complete
    pub done: bool,
}

/// `POST /agent/think`
///
/// Mobile/tablet workers call this to get LLM reasoning from the Hub.
/// The device doesn't need a local model — Hub does the thinking.
/// Rate limited to 10 calls per worker per minute.
pub async fn agent_think(
    State(state): State<GatewayState>,
    axum::Json(req): axum::Json<AgentThinkRequest>,
) -> Result<axum::Json<AgentThinkResponse>, axum::http::StatusCode> {
    // Check E-Stop
    if state.estop.is_stopped() {
        return Err(axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    // Rate limit: 10 calls per worker per minute
    {
        let mut rates = state.agent_think_rate.lock().await;
        let calls = rates.entry(req.worker.clone()).or_default();
        let now = std::time::Instant::now();
        let one_min_ago = now.checked_sub(std::time::Duration::from_secs(60));
        if let Some(cutoff) = one_min_ago {
            calls.retain(|t| *t > cutoff);
        }
        if calls.len() >= 10 {
            warn!("Agent think rate limit exceeded for worker '{}'", req.worker);
            return Err(axum::http::StatusCode::TOO_MANY_REQUESTS);
        }
        calls.push(now);
    }

    info!("Agent think: worker='{}', prompt='{}', history={}, tools={:?}",
        req.worker, truncate(&req.prompt, 60), req.history.len(), req.available_tools);

    // Build messages for LLM
    let tools_list = if req.available_tools.is_empty() {
        "web_search, http_request".to_string()
    } else {
        req.available_tools.join(", ")
    };

    let system_prompt = format!(
        "You are an AI agent running on device '{}'. You have access to these tools: [{}].\n\
         Your goal is to complete the user's task by reasoning step-by-step and calling tools.\n\n\
         IMPORTANT: Respond ONLY with valid JSON in this exact format:\n\
         {{\"reasoning\": \"your thought process\", \"next_tool\": \"tool_name\", \"tool_args\": {{...}}, \"done\": false}}\n\n\
         When you have enough information to answer, set done=true and put the final answer in reasoning.\n\
         When done=true, omit next_tool and tool_args.\n\n\
         Tool argument formats:\n\
         - web_search: {{\"query\": \"search terms\"}}\n\
         - http_request: {{\"url\": \"https://...\", \"method\": \"GET\"}}\n\n\
         Be concise. Focus on the task. Use tools efficiently.",
        req.worker, tools_list
    );

    let mut messages = vec![
        ChatMessage {
            role: "system".to_string(),
            content: system_prompt,
            tool_calls: None,
            tool_call_id: None,
        },
        ChatMessage {
            role: "user".to_string(),
            content: req.prompt.clone(),
            tool_calls: None,
            tool_call_id: None,
        },
    ];

    // Add conversation history
    for msg in &req.history {
        messages.push(ChatMessage {
            role: msg.role.clone(),
            content: msg.content.clone(),
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Use smart_route to pick the cheapest provider
    let provider = state.llm_router.smart_route("ollama", 2.0).await;
    let start = std::time::Instant::now();

    match state.llm_router.route(&messages.last().map(|m| m.content.as_str()).unwrap_or(""), &provider).await {
        Ok(raw_response) => {
            let duration = start.elapsed().as_secs_f64();

            // Try to parse as JSON
            let response = parse_agent_think_response(&raw_response);

            // Log trajectory
            if let Some(ref logger) = state.trajectory_logger {
                let entry = TrajectoryEntry {
                    id: format!("at-{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap_or("0")),
                    session_id: None,
                    agent_name: format!("agent_{}", req.worker),
                    hand_name: None,
                    phase_name: None,
                    provider: provider.clone(),
                    model: "auto".to_string(),
                    prompt: truncate(&req.prompt, 2000),
                    output: truncate(&raw_response, 5000),
                    tool_calls: if response.done { 0 } else { 1 },
                    tool_names: response.next_tool.clone().into_iter().collect(),
                    total_tokens: 0,
                    duration_secs: duration,
                    estimated_cost_usd: 0.0,
                    quality_score: None,
                    guardrail_issues: vec![],
                    success: true,
                    error_message: None,
                    worker_name: Some(req.worker.clone()),
                    worker_latency_ms: Some((duration * 1000.0) as u64),
                    created_at: chrono::Utc::now().to_rfc3339(),
                    date_key: chrono::Utc::now().format("%Y-%m-%d").to_string(),
                };
                if let Err(e) = logger.log_run(&entry) {
                    warn!("Failed to log agent_think trajectory: {}", e);
                }
            }

            info!("Agent think response for '{}': done={}, next_tool={:?}, duration={:.1}s",
                req.worker, response.done, response.next_tool, duration);

            Ok(axum::Json(response))
        }
        Err(e) => {
            error!("Agent think LLM error for '{}': {}", req.worker, e);
            // Return a fallback response instead of 500
            Ok(axum::Json(AgentThinkResponse {
                reasoning: format!("LLM error: {}. Please retry.", e),
                next_tool: None,
                tool_args: None,
                done: true,
            }))
        }
    }
}

/// Parse LLM output into AgentThinkResponse, handling various response formats
fn parse_agent_think_response(raw: &str) -> AgentThinkResponse {
    // Try direct JSON parse
    if let Ok(resp) = serde_json::from_str::<AgentThinkResponse>(raw) {
        return resp;
    }

    // Try to extract JSON from markdown code blocks
    let trimmed = raw.trim();
    let json_str = if trimmed.starts_with("```json") {
        trimmed.strip_prefix("```json").and_then(|s| s.strip_suffix("```")).unwrap_or(trimmed)
    } else if trimmed.starts_with("```") {
        trimmed.strip_prefix("```").and_then(|s| s.strip_suffix("```")).unwrap_or(trimmed)
    } else if trimmed.contains('{') {
        // Find first { and last }
        let start = trimmed.find('{').unwrap_or(0);
        let end = trimmed.rfind('}').map(|i| i + 1).unwrap_or(trimmed.len());
        &trimmed[start..end]
    } else {
        trimmed
    };

    if let Ok(resp) = serde_json::from_str::<AgentThinkResponse>(json_str.trim()) {
        return resp;
    }

    // Fallback: treat entire response as reasoning, mark as done
    AgentThinkResponse {
        reasoning: raw.to_string(),
        next_tool: None,
        tool_args: None,
        done: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stream_query_deserialize() {
        let q: StreamQuery = serde_json::from_str(r#"{"prompt": "hello"}"#).unwrap();
        assert_eq!(q.prompt.unwrap(), "hello");
    }

    #[test]
    fn test_stream_query_empty() {
        let q: StreamQuery = serde_json::from_str("{}").unwrap();
        assert!(q.prompt.is_none());
    }

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn test_truncate_long() {
        let result = truncate("hello world this is a long string", 10);
        assert!(result.ends_with("..."));
        assert_eq!(result.len(), 13); // 10 + "..."
    }

    #[test]
    fn test_gateway_state_clone() {
        // GatewayState should be Clone (required by axum State)
        // This is a compile-time check — if GatewayState isn't Clone, this won't compile
        fn assert_clone<T: Clone>() {}
        assert_clone::<GatewayState>();
    }

    #[test]
    fn test_parse_agent_think_response_json() {
        let raw = r#"{"reasoning": "I should search", "next_tool": "web_search", "tool_args": {"query": "test"}, "done": false}"#;
        let resp = parse_agent_think_response(raw);
        assert_eq!(resp.reasoning, "I should search");
        assert_eq!(resp.next_tool.as_deref(), Some("web_search"));
        assert!(!resp.done);
    }

    #[test]
    fn test_parse_agent_think_response_done() {
        let raw = r#"{"reasoning": "Here is the answer", "done": true}"#;
        let resp = parse_agent_think_response(raw);
        assert_eq!(resp.reasoning, "Here is the answer");
        assert!(resp.done);
        assert!(resp.next_tool.is_none());
    }

    #[test]
    fn test_parse_agent_think_response_markdown() {
        let raw = "```json\n{\"reasoning\": \"test\", \"done\": true}\n```";
        let resp = parse_agent_think_response(raw);
        assert_eq!(resp.reasoning, "test");
        assert!(resp.done);
    }

    #[test]
    fn test_parse_agent_think_response_embedded_json() {
        let raw = "Sure! Here is my response: {\"reasoning\": \"found it\", \"done\": true} Hope this helps.";
        let resp = parse_agent_think_response(raw);
        assert_eq!(resp.reasoning, "found it");
        assert!(resp.done);
    }

    #[test]
    fn test_parse_agent_think_response_fallback() {
        let raw = "I cannot parse this as JSON at all";
        let resp = parse_agent_think_response(raw);
        assert_eq!(resp.reasoning, raw);
        assert!(resp.done);
        assert!(resp.next_tool.is_none());
    }

    #[test]
    fn test_agent_think_request_deserialize() {
        let json = r#"{
            "worker": "rog6",
            "prompt": "Search for AI trends",
            "history": [{"role": "assistant", "content": "thinking..."}],
            "available_tools": ["web_search"]
        }"#;
        let req: AgentThinkRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.worker, "rog6");
        assert_eq!(req.available_tools, vec!["web_search"]);
        assert_eq!(req.history.len(), 1);
    }

    #[test]
    fn test_agent_think_request_defaults() {
        let json = r#"{"worker": "ipad", "prompt": "hello"}"#;
        let req: AgentThinkRequest = serde_json::from_str(json).unwrap();
        assert!(req.history.is_empty());
        assert!(req.available_tools.is_empty());
    }
}
