use serde::{Deserialize, Serialize};

// ── Core message type ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<serde_json::Value>,
}

// ── Provider error classification ─────────────────────────────────────────

/// Structured error variants returned by provider interactions.
///
/// Use [`classify_error`] to map an HTTP status code and response body
/// into one of these variants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// HTTP 429 — the provider is throttling requests.
    RateLimit,
    /// HTTP 401 / 403 — invalid or missing API key.
    AuthError,
    /// Connection-level failure (DNS, TLS, timeout, etc.).
    NetworkError,
    /// The requested model ID does not exist on this provider.
    ModelNotFound,
    /// The prompt or context window exceeds the model's token limit.
    ContextTooLong,
    /// Any other error; the inner string contains a human-readable description.
    Unknown(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::RateLimit => write!(f, "rate limit exceeded"),
            ProviderError::AuthError => write!(f, "authentication error"),
            ProviderError::NetworkError => write!(f, "network error"),
            ProviderError::ModelNotFound => write!(f, "model not found"),
            ProviderError::ContextTooLong => write!(f, "context too long"),
            ProviderError::Unknown(msg) => write!(f, "unknown error: {}", msg),
        }
    }
}

impl std::error::Error for ProviderError {}

/// Map an HTTP `status` code and response `body` to a [`ProviderError`].
///
/// The body is inspected for well-known error strings from Anthropic and
/// OpenAI so that the caller can act on the specific failure kind without
/// parsing raw JSON.
pub fn classify_error(status: u16, body: &str) -> ProviderError {
    let body_lower = body.to_lowercase();
    match status {
        429 => ProviderError::RateLimit,
        401 | 403 => ProviderError::AuthError,
        404 => {
            // Distinguish "model not found" from generic 404.
            if body_lower.contains("model")
                && (body_lower.contains("not found") || body_lower.contains("does not exist"))
            {
                ProviderError::ModelNotFound
            } else {
                ProviderError::Unknown(format!(
                    "HTTP 404: {}",
                    crate::tools::floor_char_boundary(body, 200)
                ))
            }
        }
        400 => {
            // Two distinct ways providers phrase context overflows: either
            // "context too long / length / limit" (Anthropic, OpenAI flavor)
            // or "max_tokens / context_window" (Groq, Together flavor).
            // Both map to the same ProviderError; clippy flagged the doubled
            // branches, collapsed into one OR.
            let is_context_overflow = (body_lower.contains("context")
                && (body_lower.contains("too long")
                    || body_lower.contains("length")
                    || body_lower.contains("limit")))
                || body_lower.contains("max_tokens")
                || body_lower.contains("context_window");
            if is_context_overflow {
                ProviderError::ContextTooLong
            } else {
                ProviderError::Unknown(format!(
                    "HTTP 400: {}",
                    crate::tools::floor_char_boundary(body, 200)
                ))
            }
        }
        0 => ProviderError::NetworkError,
        _ => ProviderError::Unknown(format!(
            "HTTP {}: {}",
            status,
            crate::tools::floor_char_boundary(body, 200)
        )),
    }
}
