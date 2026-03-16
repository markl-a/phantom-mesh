//! Structured error codes for Clawtex operations.
//! Provides machine-readable error classification for API responses and internal diagnostics.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Structured error code enum for all Clawtex operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    // ── Provider errors ──
    /// No provider available for the requested model/task
    ProviderUnavailable,
    /// Provider returned an error (rate limit, auth, etc.)
    ProviderError,
    /// Provider response timeout
    ProviderTimeout,
    /// All providers in rotation exhausted
    ProvidersExhausted,

    // ── Budget errors ──
    /// Agent daily budget exceeded
    BudgetExceeded,
    /// Global daily budget exceeded
    GlobalBudgetExceeded,

    // ── Tool errors ──
    /// Tool not found in registry
    ToolNotFound,
    /// Tool execution failed
    ToolExecutionFailed,
    /// Tool requires approval that was denied
    ApprovalDenied,
    /// Tool approval timed out
    ApprovalTimeout,
    /// Tool rate limit exceeded
    ToolRateLimited,
    /// Tool blocked by security policy
    ToolBlocked,

    // ── Hand/Workflow errors ──
    /// Hand not found in registry
    HandNotFound,
    /// Hand phase failed
    PhaseFailed,
    /// Hand guardrail check failed
    GuardrailFailed,
    /// Hand quality score below threshold
    QualityBelowThreshold,
    /// Hand preflight check failed (missing tools, provider unreachable, etc.)
    PreflightFailed,

    // ── Agent errors ──
    /// Agent not found in config
    AgentNotFound,
    /// Agent loop detected
    LoopDetected,
    /// Agent max rounds exceeded
    MaxRoundsExceeded,
    /// Agent emergency stopped
    EmergencyStopped,

    // ── Cluster errors ──
    /// No workers available for dispatch
    NoWorkersAvailable,
    /// Worker connection failed
    WorkerConnectionFailed,
    /// Cluster dispatch failed
    DispatchFailed,

    // ── Auth errors ──
    /// Invalid or missing API key
    AuthFailed,

    // ── General errors ──
    /// Invalid input/parameters
    InvalidInput,
    /// Internal server error
    InternalError,
}

impl ErrorCode {
    /// HTTP status code suggestion for this error
    pub fn http_status(&self) -> u16 {
        match self {
            ErrorCode::BudgetExceeded | ErrorCode::GlobalBudgetExceeded => 429,
            ErrorCode::ToolRateLimited => 429,
            ErrorCode::AuthFailed => 401,
            ErrorCode::ToolBlocked => 403,
            ErrorCode::ApprovalDenied => 403,
            ErrorCode::ToolNotFound | ErrorCode::HandNotFound | ErrorCode::AgentNotFound => 404,
            ErrorCode::InvalidInput => 400,
            ErrorCode::ProviderTimeout | ErrorCode::ApprovalTimeout => 504,
            ErrorCode::EmergencyStopped => 503,
            ErrorCode::NoWorkersAvailable => 503,
            _ => 500,
        }
    }

    /// Human-readable description
    pub fn description(&self) -> &'static str {
        match self {
            ErrorCode::ProviderUnavailable => "No LLM provider available for this request",
            ErrorCode::ProviderError => "LLM provider returned an error",
            ErrorCode::ProviderTimeout => "LLM provider request timed out",
            ErrorCode::ProvidersExhausted => "All providers in the rotation have been tried",
            ErrorCode::BudgetExceeded => "Agent daily cost budget exceeded",
            ErrorCode::GlobalBudgetExceeded => "Global daily cost budget exceeded",
            ErrorCode::ToolNotFound => "Requested tool not found in registry",
            ErrorCode::ToolExecutionFailed => "Tool execution failed",
            ErrorCode::ApprovalDenied => "Human approval was denied",
            ErrorCode::ApprovalTimeout => "Human approval request timed out",
            ErrorCode::ToolRateLimited => "Tool rate limit exceeded",
            ErrorCode::ToolBlocked => "Tool blocked by security policy",
            ErrorCode::HandNotFound => "Workflow hand not found",
            ErrorCode::PhaseFailed => "Workflow phase execution failed",
            ErrorCode::GuardrailFailed => "Output failed guardrail validation",
            ErrorCode::QualityBelowThreshold => "Output quality score below threshold",
            ErrorCode::PreflightFailed => "Workflow preflight check failed",
            ErrorCode::AgentNotFound => "Agent not found in configuration",
            ErrorCode::LoopDetected => "Agent loop detected",
            ErrorCode::MaxRoundsExceeded => "Agent max tool-call rounds exceeded",
            ErrorCode::EmergencyStopped => "System is in emergency stop mode",
            ErrorCode::NoWorkersAvailable => "No cluster workers available",
            ErrorCode::WorkerConnectionFailed => "Failed to connect to cluster worker",
            ErrorCode::DispatchFailed => "Cluster task dispatch failed",
            ErrorCode::AuthFailed => "Authentication failed",
            ErrorCode::InvalidInput => "Invalid input parameters",
            ErrorCode::InternalError => "Internal server error",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.description())
    }
}

/// Structured error response for API endpoints
#[derive(Debug, Clone, Serialize)]
pub struct ClawtexError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ClawtexError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl fmt::Display for ClawtexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for ClawtexError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_code_http_status() {
        assert_eq!(ErrorCode::BudgetExceeded.http_status(), 429);
        assert_eq!(ErrorCode::AuthFailed.http_status(), 401);
        assert_eq!(ErrorCode::HandNotFound.http_status(), 404);
        assert_eq!(ErrorCode::InternalError.http_status(), 500);
        assert_eq!(ErrorCode::EmergencyStopped.http_status(), 503);
    }

    #[test]
    fn test_error_code_description() {
        assert!(!ErrorCode::BudgetExceeded.description().is_empty());
        assert!(!ErrorCode::ProviderUnavailable.description().is_empty());
    }

    #[test]
    fn test_clawtex_error_display() {
        let err = ClawtexError::new(ErrorCode::BudgetExceeded, "Agent 'master' exceeded $5.00 daily budget");
        let s = format!("{}", err);
        assert!(s.contains("budget"));
    }

    #[test]
    fn test_clawtex_error_serialize() {
        let err = ClawtexError::new(ErrorCode::ToolNotFound, "shell_exec")
            .with_detail("Did you mean 'shell'?");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("TOOL_NOT_FOUND"));
        assert!(json.contains("shell_exec"));
        assert!(json.contains("Did you mean"));
    }

    #[test]
    fn test_error_without_detail_no_null() {
        let err = ClawtexError::new(ErrorCode::AuthFailed, "bad key");
        let json = serde_json::to_string(&err).unwrap();
        assert!(!json.contains("detail"));
    }
}
