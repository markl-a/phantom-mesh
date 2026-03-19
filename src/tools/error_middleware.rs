//! Tool Error Middleware — structured error types for tool execution failures.
//!
//! Provides ToolError with categorized error types and factory methods.
//! Replaces ad-hoc error strings with structured, classifiable errors.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Category of tool error — enables automated error handling strategies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolErrorCategory {
    /// Tool not found in registry
    NotFound,
    /// Preflight check failed (args validation, file existence, etc.)
    PreflightFailed,
    /// Rate limit exceeded
    RateLimited,
    /// Tool execution failed (runtime error)
    ExecutionFailed,
    /// Permission denied (autonomy level, service tier, etc.)
    PermissionDenied,
    /// Timeout during execution
    Timeout,
    /// Invalid arguments
    InvalidArgs,
    /// External service error (API down, network issue)
    ExternalServiceError,
}

impl fmt::Display for ToolErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolErrorCategory::NotFound => write!(f, "not_found"),
            ToolErrorCategory::PreflightFailed => write!(f, "preflight_failed"),
            ToolErrorCategory::RateLimited => write!(f, "rate_limited"),
            ToolErrorCategory::ExecutionFailed => write!(f, "execution_failed"),
            ToolErrorCategory::PermissionDenied => write!(f, "permission_denied"),
            ToolErrorCategory::Timeout => write!(f, "timeout"),
            ToolErrorCategory::InvalidArgs => write!(f, "invalid_args"),
            ToolErrorCategory::ExternalServiceError => write!(f, "external_service_error"),
        }
    }
}

/// Structured tool error with category, message, and optional details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolError {
    /// Error category for automated handling
    pub category: ToolErrorCategory,
    /// Tool name that errored
    pub tool_name: String,
    /// Human-readable error message
    pub message: String,
    /// Whether this error is retryable
    pub retryable: bool,
    /// Suggested retry delay in seconds (if retryable)
    pub retry_after_secs: Option<u64>,
    /// Original error string (for debugging)
    pub original_error: Option<String>,
}

impl ToolError {
    // ── Factory methods ──────────────────────────────────────────────────────

    /// Tool not found in registry.
    pub fn not_found(tool_name: &str) -> Self {
        Self {
            category: ToolErrorCategory::NotFound,
            tool_name: tool_name.to_string(),
            message: format!("Tool '{}' not found in registry", tool_name),
            retryable: false,
            retry_after_secs: None,
            original_error: None,
        }
    }

    /// Preflight check failed.
    pub fn preflight_failed(tool_name: &str, reason: &str) -> Self {
        Self {
            category: ToolErrorCategory::PreflightFailed,
            tool_name: tool_name.to_string(),
            message: format!("Preflight check failed for '{}': {}", tool_name, reason),
            retryable: false,
            retry_after_secs: None,
            original_error: Some(reason.to_string()),
        }
    }

    /// Rate limit exceeded.
    pub fn rate_limited(tool_name: &str, detail: &str) -> Self {
        Self {
            category: ToolErrorCategory::RateLimited,
            tool_name: tool_name.to_string(),
            message: format!("Rate limit exceeded for '{}': {}", tool_name, detail),
            retryable: true,
            retry_after_secs: Some(60),
            original_error: Some(detail.to_string()),
        }
    }

    /// Execution failed at runtime.
    pub fn execution_failed(tool_name: &str, error: &str) -> Self {
        Self {
            category: ToolErrorCategory::ExecutionFailed,
            tool_name: tool_name.to_string(),
            message: format!("Tool '{}' execution failed: {}", tool_name, error),
            retryable: true,
            retry_after_secs: Some(5),
            original_error: Some(error.to_string()),
        }
    }

    /// Permission denied.
    pub fn permission_denied(tool_name: &str, reason: &str) -> Self {
        Self {
            category: ToolErrorCategory::PermissionDenied,
            tool_name: tool_name.to_string(),
            message: format!("Permission denied for '{}': {}", tool_name, reason),
            retryable: false,
            retry_after_secs: None,
            original_error: Some(reason.to_string()),
        }
    }

    /// Timeout during execution.
    pub fn timeout(tool_name: &str, duration_secs: u64) -> Self {
        Self {
            category: ToolErrorCategory::Timeout,
            tool_name: tool_name.to_string(),
            message: format!("Tool '{}' timed out after {}s", tool_name, duration_secs),
            retryable: true,
            retry_after_secs: Some(10),
            original_error: None,
        }
    }

    /// Invalid arguments.
    pub fn invalid_args(tool_name: &str, detail: &str) -> Self {
        Self {
            category: ToolErrorCategory::InvalidArgs,
            tool_name: tool_name.to_string(),
            message: format!("Invalid arguments for '{}': {}", tool_name, detail),
            retryable: false,
            retry_after_secs: None,
            original_error: Some(detail.to_string()),
        }
    }

    /// External service error.
    pub fn external_service(tool_name: &str, service: &str, error: &str) -> Self {
        Self {
            category: ToolErrorCategory::ExternalServiceError,
            tool_name: tool_name.to_string(),
            message: format!(
                "External service '{}' error for tool '{}': {}",
                service, tool_name, error
            ),
            retryable: true,
            retry_after_secs: Some(30),
            original_error: Some(error.to_string()),
        }
    }

    /// Whether this error should be reported to the user vs. handled internally.
    pub fn is_user_facing(&self) -> bool {
        matches!(
            self.category,
            ToolErrorCategory::NotFound
                | ToolErrorCategory::PermissionDenied
                | ToolErrorCategory::InvalidArgs
        )
    }

    /// Get a short error code string for structured responses.
    pub fn error_code(&self) -> String {
        format!("TOOL_ERR_{}", self.category.to_string().to_uppercase())
    }
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.category, self.message)
    }
}

impl std::error::Error for ToolError {}

/// Classify a raw error string into a ToolError with appropriate category.
/// Heuristic-based: looks for keywords in the error message.
pub fn classify_tool_error(tool_name: &str, error: &str) -> ToolError {
    let lower = error.to_lowercase();

    if lower.contains("not found") || lower.contains("unknown tool") {
        ToolError::not_found(tool_name)
    } else if lower.contains("preflight") || lower.contains("validation") {
        ToolError::preflight_failed(tool_name, error)
    } else if lower.contains("rate limit") || lower.contains("too many requests") || lower.contains("429") {
        ToolError::rate_limited(tool_name, error)
    } else if lower.contains("timeout") || lower.contains("timed out") || lower.contains("deadline") {
        ToolError::timeout(tool_name, 0)
    } else if lower.contains("permission") || lower.contains("denied") || lower.contains("forbidden") || lower.contains("403") {
        ToolError::permission_denied(tool_name, error)
    } else if lower.contains("invalid") || lower.contains("missing") || lower.contains("required") {
        ToolError::invalid_args(tool_name, error)
    } else if lower.contains("connection") || lower.contains("network") || lower.contains("dns") || lower.contains("502") || lower.contains("503") {
        ToolError::external_service(tool_name, "unknown", error)
    } else {
        ToolError::execution_failed(tool_name, error)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Factory method tests ──

    #[test]
    fn test_not_found() {
        let err = ToolError::not_found("nonexistent");
        assert_eq!(err.category, ToolErrorCategory::NotFound);
        assert_eq!(err.tool_name, "nonexistent");
        assert!(!err.retryable);
        assert!(err.message.contains("not found"));
    }

    #[test]
    fn test_preflight_failed() {
        let err = ToolError::preflight_failed("shell", "command not allowed");
        assert_eq!(err.category, ToolErrorCategory::PreflightFailed);
        assert!(!err.retryable);
        assert!(err.message.contains("Preflight"));
    }

    #[test]
    fn test_rate_limited() {
        let err = ToolError::rate_limited("web_search", "30/hour exceeded");
        assert_eq!(err.category, ToolErrorCategory::RateLimited);
        assert!(err.retryable);
        assert_eq!(err.retry_after_secs, Some(60));
    }

    #[test]
    fn test_execution_failed() {
        let err = ToolError::execution_failed("shell", "exit code 1");
        assert_eq!(err.category, ToolErrorCategory::ExecutionFailed);
        assert!(err.retryable);
        assert_eq!(err.retry_after_secs, Some(5));
    }

    #[test]
    fn test_permission_denied() {
        let err = ToolError::permission_denied("file_write", "autonomy level: readonly");
        assert_eq!(err.category, ToolErrorCategory::PermissionDenied);
        assert!(!err.retryable);
        assert!(err.is_user_facing());
    }

    #[test]
    fn test_timeout() {
        let err = ToolError::timeout("http_request", 30);
        assert_eq!(err.category, ToolErrorCategory::Timeout);
        assert!(err.retryable);
        assert!(err.message.contains("30s"));
    }

    #[test]
    fn test_invalid_args() {
        let err = ToolError::invalid_args("file_read", "missing 'path' field");
        assert_eq!(err.category, ToolErrorCategory::InvalidArgs);
        assert!(!err.retryable);
        assert!(err.is_user_facing());
    }

    #[test]
    fn test_external_service() {
        let err = ToolError::external_service("web_search", "Google", "503 Service Unavailable");
        assert_eq!(err.category, ToolErrorCategory::ExternalServiceError);
        assert!(err.retryable);
        assert!(err.message.contains("Google"));
    }

    // ── Display / error code tests ──

    #[test]
    fn test_display() {
        let err = ToolError::not_found("test");
        let s = format!("{}", err);
        assert!(s.contains("[not_found]"));
        assert!(s.contains("test"));
    }

    #[test]
    fn test_error_code() {
        let err = ToolError::rate_limited("x", "y");
        assert_eq!(err.error_code(), "TOOL_ERR_RATE_LIMITED");

        let err2 = ToolError::not_found("x");
        assert_eq!(err2.error_code(), "TOOL_ERR_NOT_FOUND");
    }

    #[test]
    fn test_is_user_facing() {
        assert!(ToolError::not_found("x").is_user_facing());
        assert!(ToolError::permission_denied("x", "y").is_user_facing());
        assert!(ToolError::invalid_args("x", "y").is_user_facing());
        assert!(!ToolError::execution_failed("x", "y").is_user_facing());
        assert!(!ToolError::rate_limited("x", "y").is_user_facing());
    }

    // ── classify_tool_error tests ──

    #[test]
    fn test_classify_not_found() {
        let err = classify_tool_error("foo", "Unknown tool: foo not found");
        assert_eq!(err.category, ToolErrorCategory::NotFound);
    }

    #[test]
    fn test_classify_rate_limit() {
        let err = classify_tool_error("web_search", "Rate limit exceeded: 429 Too Many Requests");
        assert_eq!(err.category, ToolErrorCategory::RateLimited);
    }

    #[test]
    fn test_classify_timeout() {
        let err = classify_tool_error("http_request", "Connection timed out after 30s");
        assert_eq!(err.category, ToolErrorCategory::Timeout);
    }

    #[test]
    fn test_classify_permission() {
        let err = classify_tool_error("shell", "Permission denied: not in allowed list");
        assert_eq!(err.category, ToolErrorCategory::PermissionDenied);
    }

    #[test]
    fn test_classify_invalid_args() {
        let err = classify_tool_error("file_read", "Missing required field: path");
        assert_eq!(err.category, ToolErrorCategory::InvalidArgs);
    }

    #[test]
    fn test_classify_network() {
        let err = classify_tool_error("http_request", "Connection refused: network unreachable");
        assert_eq!(err.category, ToolErrorCategory::ExternalServiceError);
    }

    #[test]
    fn test_classify_fallback() {
        let err = classify_tool_error("shell", "Some random error occurred");
        assert_eq!(err.category, ToolErrorCategory::ExecutionFailed);
    }

    // ── Serialization tests ──

    #[test]
    fn test_tool_error_serialization() {
        let err = ToolError::rate_limited("web_search", "exceeded");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("RateLimited"));
        let back: ToolError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.category, ToolErrorCategory::RateLimited);
        assert_eq!(back.tool_name, "web_search");
    }

    #[test]
    fn test_category_display() {
        assert_eq!(format!("{}", ToolErrorCategory::NotFound), "not_found");
        assert_eq!(format!("{}", ToolErrorCategory::RateLimited), "rate_limited");
        assert_eq!(
            format!("{}", ToolErrorCategory::ExternalServiceError),
            "external_service_error"
        );
    }
}
