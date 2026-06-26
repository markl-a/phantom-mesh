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

    #[test]
    fn test_rpc_response_unknown_is_neither() {
        // The third state must report as neither success nor error so callers
        // route it through the "retry once then surface" path rather than
        // misclassifying it as a hard failure or a clean result.
        let resp = RpcResponse::Unknown { message: "peer dropped mid-stream".into() };
        assert!(!resp.is_success());
        assert!(!resp.is_error());
    }

    #[test]
    fn test_rpc_response_unknown_serde_roundtrip() {
        let resp = RpcResponse::Unknown { message: "timeout before headers".into() };
        let json = serde_json::to_string(&resp).unwrap();
        let back: RpcResponse = serde_json::from_str(&json).unwrap();
        assert!(!back.is_success());
        assert!(!back.is_error());
        match back {
            RpcResponse::Unknown { message } => assert_eq!(message, "timeout before headers"),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn test_epoch_serde_roundtrip() {
        let epoch = Epoch(7);
        let json = serde_json::to_string(&epoch).unwrap();
        assert_eq!(json, "7"); // newtype serializes transparently as the inner u64
        let back: Epoch = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Epoch(7));
    }

    #[test]
    fn test_term_serde_roundtrip() {
        let term = Term(3);
        let json = serde_json::to_string(&term).unwrap();
        assert_eq!(json, "3");
        let back: Term = serde_json::from_str(&json).unwrap();
        assert_eq!(back, term);
    }

    #[test]
    fn test_term_and_epoch_are_hashable() {
        // Both derive Hash so they can key split-brain / election bookkeeping
        // maps; exercise that the derive is actually usable as a map key.
        use std::collections::HashMap;
        let mut votes: HashMap<Term, u32> = HashMap::new();
        *votes.entry(Term(1)).or_insert(0) += 1;
        *votes.entry(Term(1)).or_insert(0) += 1;
        *votes.entry(Term(2)).or_insert(0) += 1;
        assert_eq!(votes.get(&Term(1)), Some(&2));
        assert_eq!(votes.get(&Term(2)), Some(&1));

        let mut seen: HashMap<Epoch, &str> = HashMap::new();
        seen.insert(Epoch(10), "a");
        seen.insert(Epoch(10), "b"); // overwrites — same key
        assert_eq!(seen.len(), 1);
        assert_eq!(seen.get(&Epoch(10)), Some(&"b"));
    }

    #[test]
    fn test_term_next_is_monotonic() {
        let mut t = Term(0);
        for expected in 1..=5 {
            t = t.next();
            assert_eq!(t, Term(expected));
        }
    }
}
