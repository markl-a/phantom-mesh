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

impl ErrorCode {
    /// Numeric error code string (E1xx=Provider, E2xx=Tool, E3xx=Cluster, E4xx=Config, E5xx=Agent)
    pub fn numeric_code(&self) -> &'static str {
        match self {
            // E1xx — Provider errors
            ErrorCode::ProviderUnavailable => "E101",
            ErrorCode::ProviderError => "E102",
            ErrorCode::ProviderTimeout => "E103",
            ErrorCode::ProvidersExhausted => "E104",

            // E2xx — Tool errors
            ErrorCode::ToolNotFound => "E201",
            ErrorCode::ToolExecutionFailed => "E202",
            ErrorCode::ApprovalDenied => "E203",
            ErrorCode::ApprovalTimeout => "E204",
            ErrorCode::ToolRateLimited => "E205",
            ErrorCode::ToolBlocked => "E206",

            // E3xx — Cluster errors
            ErrorCode::NoWorkersAvailable => "E301",
            ErrorCode::WorkerConnectionFailed => "E302",
            ErrorCode::DispatchFailed => "E303",

            // E4xx — Config / Budget / Auth errors
            ErrorCode::BudgetExceeded => "E401",
            ErrorCode::GlobalBudgetExceeded => "E402",
            ErrorCode::AuthFailed => "E403",
            ErrorCode::InvalidInput => "E404",

            // E5xx — Agent / Workflow errors
            ErrorCode::AgentNotFound => "E501",
            ErrorCode::LoopDetected => "E502",
            ErrorCode::MaxRoundsExceeded => "E503",
            ErrorCode::EmergencyStopped => "E504",
            ErrorCode::HandNotFound => "E505",
            ErrorCode::PhaseFailed => "E506",
            ErrorCode::GuardrailFailed => "E507",
            ErrorCode::QualityBelowThreshold => "E508",
            ErrorCode::PreflightFailed => "E509",

            // E9xx — General
            ErrorCode::InternalError => "E999",
        }
    }

    /// Suggested user action to resolve this error
    pub fn suggestion(&self) -> &'static str {
        match self {
            ErrorCode::ProviderUnavailable => "Check that the requested provider is configured in agents.toml",
            ErrorCode::ProviderError => "Check provider API key and rate limits",
            ErrorCode::ProviderTimeout => "Try again or switch to a faster provider",
            ErrorCode::ProvidersExhausted => "Wait for rate limit cooldown or add more providers",
            ErrorCode::BudgetExceeded => "Increase daily_budget_usd in agent config or wait until tomorrow",
            ErrorCode::GlobalBudgetExceeded => "Increase global budget limit or wait until tomorrow",
            ErrorCode::ToolNotFound => "Check tool name spelling or run /tools to see available tools",
            ErrorCode::ToolExecutionFailed => "Check tool parameters and try again",
            ErrorCode::ApprovalDenied => "Request was denied by the human operator",
            ErrorCode::ApprovalTimeout => "Operator did not respond in time; try again",
            ErrorCode::ToolRateLimited => "Too many tool calls; wait before retrying",
            ErrorCode::ToolBlocked => "This tool is blocked by security policy",
            ErrorCode::HandNotFound => "Check hand name or run /hands to see available workflows",
            ErrorCode::PhaseFailed => "Check phase configuration and provider availability",
            ErrorCode::GuardrailFailed => "Output did not pass safety checks; rephrase the request",
            ErrorCode::QualityBelowThreshold => "Output quality was too low; try with a better model",
            ErrorCode::PreflightFailed => "Pre-execution checks failed; verify tool dependencies",
            ErrorCode::AgentNotFound => "Check agent name in agents.toml",
            ErrorCode::LoopDetected => "Agent is repeating actions; adjust instructions",
            ErrorCode::MaxRoundsExceeded => "Agent used too many tool rounds; simplify the task",
            ErrorCode::EmergencyStopped => "System is in E-Stop mode; use /estop reset to resume",
            ErrorCode::NoWorkersAvailable => "No cluster workers are online; check worker status",
            ErrorCode::WorkerConnectionFailed => "Cannot reach worker; check network and SSH",
            ErrorCode::DispatchFailed => "Task dispatch failed; check cluster health",
            ErrorCode::AuthFailed => "Invalid API key or token",
            ErrorCode::InvalidInput => "Check request parameters",
            ErrorCode::InternalError => "Unexpected error; check logs for details",
        }
    }
}

/// Map ErrorClass (from providers/reliable.rs) to ErrorCode
pub fn error_class_to_code(class: &str) -> ErrorCode {
    match class {
        "RateLimited" => ErrorCode::ProviderError,
        "NonRetryable" => ErrorCode::ProviderError,
        "Transient" => ErrorCode::ProviderTimeout,
        _ => ErrorCode::InternalError,
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.numeric_code(), self.description())
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

    // ── Numeric code tests ────────────────────────────────────────────

    #[test]
    fn test_numeric_codes_provider() {
        assert_eq!(ErrorCode::ProviderUnavailable.numeric_code(), "E101");
        assert_eq!(ErrorCode::ProviderError.numeric_code(), "E102");
        assert_eq!(ErrorCode::ProviderTimeout.numeric_code(), "E103");
        assert_eq!(ErrorCode::ProvidersExhausted.numeric_code(), "E104");
    }

    #[test]
    fn test_numeric_codes_tool() {
        assert_eq!(ErrorCode::ToolNotFound.numeric_code(), "E201");
        assert_eq!(ErrorCode::ToolExecutionFailed.numeric_code(), "E202");
        assert_eq!(ErrorCode::ApprovalDenied.numeric_code(), "E203");
        assert_eq!(ErrorCode::ApprovalTimeout.numeric_code(), "E204");
        assert_eq!(ErrorCode::ToolRateLimited.numeric_code(), "E205");
        assert_eq!(ErrorCode::ToolBlocked.numeric_code(), "E206");
    }

    #[test]
    fn test_numeric_codes_cluster() {
        assert_eq!(ErrorCode::NoWorkersAvailable.numeric_code(), "E301");
        assert_eq!(ErrorCode::WorkerConnectionFailed.numeric_code(), "E302");
        assert_eq!(ErrorCode::DispatchFailed.numeric_code(), "E303");
    }

    #[test]
    fn test_numeric_codes_config() {
        assert_eq!(ErrorCode::BudgetExceeded.numeric_code(), "E401");
        assert_eq!(ErrorCode::GlobalBudgetExceeded.numeric_code(), "E402");
        assert_eq!(ErrorCode::AuthFailed.numeric_code(), "E403");
        assert_eq!(ErrorCode::InvalidInput.numeric_code(), "E404");
    }

    #[test]
    fn test_numeric_codes_agent() {
        assert_eq!(ErrorCode::AgentNotFound.numeric_code(), "E501");
        assert_eq!(ErrorCode::LoopDetected.numeric_code(), "E502");
        assert_eq!(ErrorCode::MaxRoundsExceeded.numeric_code(), "E503");
        assert_eq!(ErrorCode::EmergencyStopped.numeric_code(), "E504");
        assert_eq!(ErrorCode::HandNotFound.numeric_code(), "E505");
        assert_eq!(ErrorCode::PhaseFailed.numeric_code(), "E506");
        assert_eq!(ErrorCode::GuardrailFailed.numeric_code(), "E507");
        assert_eq!(ErrorCode::QualityBelowThreshold.numeric_code(), "E508");
        assert_eq!(ErrorCode::PreflightFailed.numeric_code(), "E509");
    }

    #[test]
    fn test_numeric_codes_general() {
        assert_eq!(ErrorCode::InternalError.numeric_code(), "E999");
    }

    #[test]
    fn test_suggestion_not_empty() {
        // Every error code should have a non-empty suggestion
        let codes = vec![
            ErrorCode::ProviderUnavailable, ErrorCode::ProviderError,
            ErrorCode::ProviderTimeout, ErrorCode::ProvidersExhausted,
            ErrorCode::BudgetExceeded, ErrorCode::GlobalBudgetExceeded,
            ErrorCode::ToolNotFound, ErrorCode::ToolExecutionFailed,
            ErrorCode::ApprovalDenied, ErrorCode::ApprovalTimeout,
            ErrorCode::ToolRateLimited, ErrorCode::ToolBlocked,
            ErrorCode::HandNotFound, ErrorCode::PhaseFailed,
            ErrorCode::GuardrailFailed, ErrorCode::QualityBelowThreshold,
            ErrorCode::PreflightFailed, ErrorCode::AgentNotFound,
            ErrorCode::LoopDetected, ErrorCode::MaxRoundsExceeded,
            ErrorCode::EmergencyStopped, ErrorCode::NoWorkersAvailable,
            ErrorCode::WorkerConnectionFailed, ErrorCode::DispatchFailed,
            ErrorCode::AuthFailed, ErrorCode::InvalidInput,
            ErrorCode::InternalError,
        ];
        for code in codes {
            assert!(!code.suggestion().is_empty(), "Missing suggestion for {:?}", code);
        }
    }

    #[test]
    fn test_display_includes_numeric_code() {
        let s = format!("{}", ErrorCode::ToolNotFound);
        assert!(s.contains("E201"), "Display should include numeric code: {}", s);
        assert!(s.contains("not found"), "Display should include description: {}", s);
    }

    #[test]
    fn test_error_class_to_code() {
        assert_eq!(error_class_to_code("RateLimited"), ErrorCode::ProviderError);
        assert_eq!(error_class_to_code("NonRetryable"), ErrorCode::ProviderError);
        assert_eq!(error_class_to_code("Transient"), ErrorCode::ProviderTimeout);
        assert_eq!(error_class_to_code("Unknown"), ErrorCode::InternalError);
    }

    #[test]
    fn test_clawtex_error_with_numeric_display() {
        let err = ClawtexError::new(ErrorCode::ToolRateLimited, "shell rate limited");
        let s = format!("{}", err);
        assert!(s.contains("E205"));
    }

    #[test]
    fn test_numeric_code_uniqueness() {
        let codes = vec![
            ErrorCode::ProviderUnavailable, ErrorCode::ProviderError,
            ErrorCode::ProviderTimeout, ErrorCode::ProvidersExhausted,
            ErrorCode::ToolNotFound, ErrorCode::ToolExecutionFailed,
            ErrorCode::ApprovalDenied, ErrorCode::ApprovalTimeout,
            ErrorCode::ToolRateLimited, ErrorCode::ToolBlocked,
            ErrorCode::NoWorkersAvailable, ErrorCode::WorkerConnectionFailed,
            ErrorCode::DispatchFailed, ErrorCode::BudgetExceeded,
            ErrorCode::GlobalBudgetExceeded, ErrorCode::AuthFailed,
            ErrorCode::InvalidInput, ErrorCode::AgentNotFound,
            ErrorCode::LoopDetected, ErrorCode::MaxRoundsExceeded,
            ErrorCode::EmergencyStopped, ErrorCode::HandNotFound,
            ErrorCode::PhaseFailed, ErrorCode::GuardrailFailed,
            ErrorCode::QualityBelowThreshold, ErrorCode::PreflightFailed,
            ErrorCode::InternalError,
        ];
        let mut seen = std::collections::HashSet::new();
        for code in &codes {
            let numeric = code.numeric_code();
            assert!(seen.insert(numeric), "Duplicate numeric code: {}", numeric);
        }
    }

    #[test]
    fn test_error_code_serialize_json() {
        let err = ClawtexError::new(ErrorCode::DispatchFailed, "worker offline");
        let json = serde_json::to_string(&err).unwrap();
        assert!(json.contains("DISPATCH_FAILED"));
    }
}
