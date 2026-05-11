use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::ErrorKind;

/// An RPC request between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcRequest {
    pub request_id: Uuid,
    pub timestamp: i64,
    pub trace_id: Uuid,
    /// Ed25519 signature of the request body.
    pub signature: Vec<u8>,
    /// Call depth for preventing infinite delegation loops.
    pub depth: u32,
    /// Chain of node_ids this request has traversed.
    pub chain: Vec<String>,
    pub method: String,
    pub params: serde_json::Value,
}

/// An RPC response — three-state: Success, Error, or Unknown.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum RpcResponse {
    Success(serde_json::Value),
    Error(ErrorKind),
    Unknown { message: String },
}

impl RpcResponse {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Success(_))
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Self::Error(_))
    }
}

/// Election term number for Bully-variant coordinator election.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Term(pub u64);

/// Epoch for split-brain detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Epoch(pub u64);

impl Term {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl Epoch {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rpc_request_creation() {
        let req = RpcRequest {
            request_id: Uuid::new_v4(),
            timestamp: 1234567890,
            trace_id: Uuid::new_v4(),
            signature: vec![0u8; 64],
            depth: 0,
            chain: vec!["node-1".into()],
            method: "execute_tool".into(),
            params: serde_json::json!({"tool": "shell", "command": "ls"}),
        };
        assert_eq!(req.depth, 0);
        assert_eq!(req.chain.len(), 1);
    }

    #[test]
    fn test_rpc_response_success() {
        let resp = RpcResponse::Success(serde_json::json!({"result": "ok"}));
        assert!(resp.is_success());
        assert!(!resp.is_error());
    }

    #[test]
    fn test_rpc_response_error() {
        let resp = RpcResponse::Error(ErrorKind::Transient);
        assert!(!resp.is_success());
        assert!(resp.is_error());
    }

    #[test]
    fn test_term_ordering() {
        let t1 = Term(1);
        let t2 = Term(2);
        assert!(t2 > t1);
        assert_eq!(t1.next(), t2);
    }

    #[test]
    fn test_epoch_ordering() {
        let e1 = Epoch(5);
        let e2 = e1.next();
        assert_eq!(e2, Epoch(6));
    }

    #[test]
    fn test_rpc_response_serde() {
        let resp = RpcResponse::Success(serde_json::json!(42));
        let json = serde_json::to_string(&resp).unwrap();
        let back: RpcResponse = serde_json::from_str(&json).unwrap();
        assert!(back.is_success());
    }
}
