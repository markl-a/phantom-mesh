use phantom_mesh::rpc::protocol::*;
use phantom_mesh::rpc::server::{rpc_router, RpcState};
use phantom_mesh::rpc::client::RpcClient;
use serde_json::json;

// ---------------------------------------------------------------------------
// 1. test_rpc_request_new
// ---------------------------------------------------------------------------
#[test]
fn test_rpc_request_new() {
    let req = RpcRequest::new("rpc.ping", json!({}));
    assert_eq!(req.jsonrpc, "2.0");
    assert_eq!(req.method, "rpc.ping");
    // id should be a valid UUID (36 chars with hyphens)
    assert_eq!(req.id.len(), 36);
    assert!(uuid::Uuid::parse_str(&req.id).is_ok());
}

// ---------------------------------------------------------------------------
// 2. test_rpc_response_success
// ---------------------------------------------------------------------------
#[test]
fn test_rpc_response_success() {
    let resp = RpcResponse::success("abc-123", json!({"pong": true}));
    assert_eq!(resp.jsonrpc, "2.0");
    assert_eq!(resp.id, "abc-123");
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());
    assert_eq!(resp.result.unwrap()["pong"], true);
}

// ---------------------------------------------------------------------------
// 3. test_rpc_response_error
// ---------------------------------------------------------------------------
#[test]
fn test_rpc_response_error() {
    let resp = RpcResponse::error("abc-123", ERR_METHOD_NOT_FOUND, "no such method");
    assert_eq!(resp.jsonrpc, "2.0");
    assert_eq!(resp.id, "abc-123");
    assert!(resp.result.is_none());
    assert!(resp.error.is_some());
    let err = resp.error.unwrap();
    assert_eq!(err.code, ERR_METHOD_NOT_FOUND);
    assert_eq!(err.message, "no such method");
}

// ---------------------------------------------------------------------------
// 4. test_rpc_request_serialize_roundtrip
// ---------------------------------------------------------------------------
#[test]
fn test_rpc_request_serialize_roundtrip() {
    let req = RpcRequest::new("rpc.status", json!({"verbose": true}));
    let serialized = serde_json::to_string(&req).unwrap();
    let deserialized: RpcRequest = serde_json::from_str(&serialized).unwrap();
    assert_eq!(deserialized.jsonrpc, req.jsonrpc);
    assert_eq!(deserialized.id, req.id);
    assert_eq!(deserialized.method, req.method);
    assert_eq!(deserialized.params, req.params);
}

// ---------------------------------------------------------------------------
// Helper: start a test server on a random port, return the address string.
// ---------------------------------------------------------------------------
async fn start_test_server() -> String {
    let state = RpcState {
        node_id: "test-node".to_string(),
    };
    let app = rpc_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr.to_string()
}

// ---------------------------------------------------------------------------
// 5. test_rpc_server_ping
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rpc_server_ping() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();
    let req = RpcRequest::new("rpc.ping", json!({}));

    let resp: RpcResponse = client
        .post(format!("http://{}/rpc", addr))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(resp.jsonrpc, "2.0");
    assert_eq!(resp.id, req.id);
    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    assert_eq!(result["pong"], true);
}

// ---------------------------------------------------------------------------
// 6. test_rpc_server_status
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rpc_server_status() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();
    let req = RpcRequest::new("rpc.status", json!({}));

    let resp: RpcResponse = client
        .post(format!("http://{}/rpc", addr))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(resp.error.is_none());
    let result = resp.result.unwrap();
    assert_eq!(result["node_id"], "test-node");
    assert_eq!(result["status"], "online");
}

// ---------------------------------------------------------------------------
// 7. test_rpc_server_unknown_method
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rpc_server_unknown_method() {
    let addr = start_test_server().await;
    let client = reqwest::Client::new();
    let req = RpcRequest::new("rpc.nonexistent", json!({}));

    let resp: RpcResponse = client
        .post(format!("http://{}/rpc", addr))
        .json(&req)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(resp.result.is_none());
    let err = resp.error.unwrap();
    assert_eq!(err.code, ERR_METHOD_NOT_FOUND);
    assert!(err.message.contains("rpc.nonexistent"));
}

// ---------------------------------------------------------------------------
// 8. test_rpc_client_call
// ---------------------------------------------------------------------------
#[tokio::test]
async fn test_rpc_client_call() {
    let addr = start_test_server().await;
    let rpc_client = RpcClient::new();

    let pong = rpc_client.ping(&addr).await.unwrap();
    assert!(pong);
}
