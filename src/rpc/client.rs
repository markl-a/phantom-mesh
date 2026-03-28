use reqwest::Client;
use serde_json::Value;

use super::protocol::*;

/// RPC client for calling other nodes.
pub struct RpcClient {
    http: Client,
    timeout: std::time::Duration,
}

impl RpcClient {
    pub fn new() -> Self {
        Self {
            http: Client::new(),
            timeout: std::time::Duration::from_secs(30),
        }
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Send an RPC request to a node.
    pub async fn call(
        &self,
        addr: &str,
        method: &str,
        params: Value,
    ) -> Result<Value, RpcCallError> {
        let request = RpcRequest::new(method, params);
        let url = format!("http://{}/rpc", addr);

        let response = self
            .http
            .post(&url)
            .json(&request)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e| RpcCallError::Network(e.to_string()))?;

        let rpc_response: RpcResponse = response
            .json()
            .await
            .map_err(|e| RpcCallError::Deserialization(e.to_string()))?;

        if let Some(err) = rpc_response.error {
            return Err(RpcCallError::Remote(err.code, err.message));
        }

        rpc_response.result.ok_or(RpcCallError::EmptyResult)
    }

    /// Ping a node.
    pub async fn ping(&self, addr: &str) -> Result<bool, RpcCallError> {
        let result = self.call(addr, "rpc.ping", serde_json::json!({})).await?;
        Ok(result
            .get("pong")
            .and_then(|v| v.as_bool())
            .unwrap_or(false))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RpcCallError {
    #[error("network error: {0}")]
    Network(String),
    #[error("deserialization error: {0}")]
    Deserialization(String),
    #[error("remote error ({0}): {1}")]
    Remote(i32, String),
    #[error("empty result")]
    EmptyResult,
}
