use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Three-state error type for Phantom Mesh.
///
/// - **Transient**: Temporary failure, safe to retry with backoff.
/// - **Permanent**: Unrecoverable failure, do not retry.
/// - **Unknown**: Unclear failure state, retry once then surface.
#[derive(thiserror::Error, Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PhantomError {
    #[error("transient: {message}")]
    Transient {
        message: String,
        retry_after_ms: Option<u64>,
    },
    #[error("permanent: {message}")]
    Permanent {
        message: String,
        suggestion: Option<String>,
    },
    #[error("unknown: {message}")]
    Unknown {
        message: String,
        trace_id: Option<Uuid>,
    },
}

impl PhantomError {
    pub fn transient(message: impl Into<String>) -> Self {
        Self::Transient { message: message.into(), retry_after_ms: None }
    }

    pub fn permanent(message: impl Into<String>) -> Self {
        Self::Permanent { message: message.into(), suggestion: None }
    }

    pub fn unknown(message: impl Into<String>) -> Self {
        Self::Unknown { message: message.into(), trace_id: None }
    }

    pub fn is_retryable(&self) -> bool {
        !matches!(self, Self::Permanent { .. })
    }

    pub fn kind(&self) -> ErrorKind {
        match self {
            Self::Transient { .. } => ErrorKind::Transient,
            Self::Permanent { .. } => ErrorKind::Permanent,
            Self::Unknown { .. } => ErrorKind::Unknown,
        }
    }
}

/// Error classification for RPC responses and routing decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ErrorKind {
    Transient,
    Permanent,
    Unknown,
}

/// Type alias for trace identifiers used across all public APIs.
pub type TraceId = Uuid;

/// Trace context for distributed tracing across nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceContext {
    pub trace_id: TraceId,
    pub span_id: String,
    pub parent_span_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phantom_error_transient() {
        let err = PhantomError::transient("connection timeout");
        assert!(err.is_retryable());
        assert_eq!(err.kind(), ErrorKind::Transient);
        assert_eq!(err.to_string(), "transient: connection timeout");
    }

    #[test]
    fn test_phantom_error_permanent() {
        let err = PhantomError::Permanent {
            message: "invalid API key".into(),
            suggestion: Some("Check your .env file".into()),
        };
        assert!(!err.is_retryable());
        assert_eq!(err.kind(), ErrorKind::Permanent);
    }

    #[test]
    fn test_phantom_error_unknown() {
        let trace = Uuid::new_v4();
        let err = PhantomError::Unknown {
            message: "unexpected state".into(),
            trace_id: Some(trace),
        };
        assert!(err.is_retryable());
        assert_eq!(err.kind(), ErrorKind::Unknown);
    }

    #[test]
    fn test_phantom_error_serde_roundtrip() {
        let err = PhantomError::transient("timeout");
        let json = serde_json::to_string(&err).unwrap();
        let back: PhantomError = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kind(), ErrorKind::Transient);
    }

    #[test]
    fn test_error_kind_values() {
        assert_ne!(ErrorKind::Transient, ErrorKind::Permanent);
        assert_ne!(ErrorKind::Permanent, ErrorKind::Unknown);
    }

    #[test]
    fn test_trace_context_creation() {
        let ctx = TraceContext {
            trace_id: Uuid::new_v4(),
            span_id: "span-1".into(),
            parent_span_id: None,
        };
        assert_eq!(ctx.span_id, "span-1");
    }
}
