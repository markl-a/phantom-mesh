//! Stripe Webhook Handler — automatic revenue recording from Stripe payment events.
//!
//! Verifies webhook signatures (HMAC-SHA256), parses Stripe event payloads, and
//! converts payment events into revenue recording actions.
//!
//! Supported event types:
//! - `checkout.session.completed`
//! - `invoice.paid`
//! - `payment_intent.succeeded`

use anyhow::{anyhow, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Tolerance window for timestamp verification (5 minutes).
const TIMESTAMP_TOLERANCE_SECS: u64 = 300;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Action to take after processing a webhook event.
#[derive(Debug, Clone, PartialEq)]
pub enum WebhookAction {
    /// Record revenue from a successful payment.
    RecordRevenue {
        amount_usd: f64,
        client: String,
        description: String,
    },
    /// Event type is not relevant; no action needed.
    Ignore,
    /// An error occurred while processing the event.
    Error(String),
}

/// Parsed Stripe event from a webhook payload.
#[derive(Debug, Clone, Deserialize)]
pub struct StripeEvent {
    pub id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    pub data: serde_json::Value,
    pub created: u64,
}

/// Stripe webhook handler with signature verification.
pub struct StripeWebhook {
    pub webhook_secret: String,
}

// ---------------------------------------------------------------------------
// HMAC-SHA256 (manual implementation using sha2 crate)
// ---------------------------------------------------------------------------

/// Compute HMAC-SHA256 using the raw SHA-256 primitive.
///
/// HMAC(K, m) = H((K' XOR opad) || H((K' XOR ipad) || m))
/// where K' is the key padded/hashed to block size (64 bytes for SHA-256).
fn hmac_sha256(key: &[u8], message: &[u8]) -> Vec<u8> {
    const BLOCK_SIZE: usize = 64;

    // Step 1: If key is longer than block size, hash it.
    let key_prime = if key.len() > BLOCK_SIZE {
        let mut hasher = Sha256::new();
        hasher.update(key);
        hasher.finalize().to_vec()
    } else {
        key.to_vec()
    };

    // Step 2: Pad key to block size with zeroes.
    let mut padded_key = vec![0u8; BLOCK_SIZE];
    padded_key[..key_prime.len()].copy_from_slice(&key_prime);

    // Step 3: Create inner and outer padded keys.
    let mut i_key_pad = vec![0u8; BLOCK_SIZE];
    let mut o_key_pad = vec![0u8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        i_key_pad[i] = padded_key[i] ^ 0x36;
        o_key_pad[i] = padded_key[i] ^ 0x5c;
    }

    // Step 4: Inner hash = H(i_key_pad || message)
    let mut inner_hasher = Sha256::new();
    inner_hasher.update(&i_key_pad);
    inner_hasher.update(message);
    let inner_hash = inner_hasher.finalize();

    // Step 5: Outer hash = H(o_key_pad || inner_hash)
    let mut outer_hasher = Sha256::new();
    outer_hasher.update(&o_key_pad);
    outer_hasher.update(inner_hash);
    outer_hasher.finalize().to_vec()
}

// ---------------------------------------------------------------------------
// Signature verification
// ---------------------------------------------------------------------------

/// Verify a Stripe webhook signature.
///
/// The `Stripe-Signature` header has the format: `t=<timestamp>,v1=<signature>`
/// The signed payload is `<timestamp>.<payload>`, signed with HMAC-SHA256 using
/// the webhook secret.
///
/// Returns `true` if the signature is valid.
pub fn verify_signature(payload: &str, signature: &str, secret: &str) -> bool {
    // Parse the Stripe-Signature header components.
    let mut timestamp_str: Option<&str> = None;
    let mut v1_signature: Option<&str> = None;

    for part in signature.split(',') {
        let part = part.trim();
        if let Some(ts) = part.strip_prefix("t=") {
            timestamp_str = Some(ts);
        } else if let Some(sig) = part.strip_prefix("v1=") {
            v1_signature = Some(sig);
        }
    }

    let (timestamp_str, expected_sig) = match (timestamp_str, v1_signature) {
        (Some(t), Some(s)) => (t, s),
        _ => {
            warn!("Stripe signature header missing t= or v1= component");
            return false;
        }
    };

    // Construct the signed payload: "<timestamp>.<payload>"
    let signed_payload = format!("{}.{}", timestamp_str, payload);

    // Compute HMAC-SHA256.
    let computed = hmac_sha256(secret.as_bytes(), signed_payload.as_bytes());
    let computed_hex = hex::encode(&computed);

    // Constant-time comparison to prevent timing attacks.
    constant_time_eq(computed_hex.as_bytes(), expected_sig.as_bytes())
}

/// Constant-time byte comparison to mitigate timing side-channels.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---------------------------------------------------------------------------
// Event parsing
// ---------------------------------------------------------------------------

/// Parse a raw JSON payload into a `StripeEvent`.
pub fn parse_event(payload: &str) -> Result<StripeEvent> {
    serde_json::from_str(payload).map_err(|e| anyhow!("Failed to parse Stripe event: {}", e))
}

// ---------------------------------------------------------------------------
// Amount extraction
// ---------------------------------------------------------------------------

/// Extract the payment amount from Stripe event data.
///
/// Stripe reports amounts in the smallest currency unit (e.g. cents for USD).
/// This function divides by 100 to return the amount in dollars.
///
/// Looks for `amount`, `amount_total`, or `amount_paid` fields inside `data.object`.
pub fn extract_amount(data: &serde_json::Value) -> Option<f64> {
    let object = data.get("object")?;

    // Try several known field names in priority order.
    let amount_cents = object
        .get("amount_total")
        .and_then(|v| v.as_i64())
        .or_else(|| object.get("amount_paid").and_then(|v| v.as_i64()))
        .or_else(|| object.get("amount").and_then(|v| v.as_i64()))?;

    if amount_cents < 0 {
        return None;
    }

    Some(amount_cents as f64 / 100.0)
}

/// Extract the customer/client identifier from Stripe event data.
///
/// Tries `customer_email`, `customer_details.email`, `customer`, in that order.
fn extract_client(data: &serde_json::Value) -> String {
    let object = match data.get("object") {
        Some(o) => o,
        None => return "unknown".to_string(),
    };

    // Try customer_email first (common in checkout.session.completed).
    if let Some(email) = object.get("customer_email").and_then(|v| v.as_str()) {
        if !email.is_empty() {
            return email.to_string();
        }
    }

    // Try nested customer_details.email.
    if let Some(details) = object.get("customer_details") {
        if let Some(email) = details.get("email").and_then(|v| v.as_str()) {
            if !email.is_empty() {
                return email.to_string();
            }
        }
    }

    // Try customer ID as fallback.
    if let Some(cust) = object.get("customer").and_then(|v| v.as_str()) {
        if !cust.is_empty() {
            return cust.to_string();
        }
    }

    "unknown".to_string()
}

/// Extract a human-readable description from the event data.
fn extract_description(event_type: &str, data: &serde_json::Value) -> String {
    let object = match data.get("object") {
        Some(o) => o,
        None => return format!("Stripe {}", event_type),
    };

    // Try description field.
    if let Some(desc) = object.get("description").and_then(|v| v.as_str()) {
        if !desc.is_empty() {
            return desc.to_string();
        }
    }

    // Try metadata.description or metadata.product.
    if let Some(meta) = object.get("metadata") {
        if let Some(desc) = meta.get("description").and_then(|v| v.as_str()) {
            if !desc.is_empty() {
                return desc.to_string();
            }
        }
        if let Some(product) = meta.get("product").and_then(|v| v.as_str()) {
            if !product.is_empty() {
                return format!("Payment for {}", product);
            }
        }
    }

    format!("Stripe {}", event_type)
}

// ---------------------------------------------------------------------------
// Event handling
// ---------------------------------------------------------------------------

impl StripeWebhook {
    /// Create a new webhook handler with the given signing secret.
    pub fn new(webhook_secret: &str) -> Self {
        Self {
            webhook_secret: webhook_secret.to_string(),
        }
    }

    /// Verify and handle an incoming webhook request.
    ///
    /// Returns a `WebhookAction` indicating what the caller should do.
    pub fn process(&self, payload: &str, signature: &str) -> WebhookAction {
        if !verify_signature(payload, signature, &self.webhook_secret) {
            return WebhookAction::Error("Invalid webhook signature".to_string());
        }

        match parse_event(payload) {
            Ok(event) => handle_event(&event),
            Err(e) => WebhookAction::Error(format!("Failed to parse event: {}", e)),
        }
    }
}

/// Determine the action to take for a given Stripe event.
///
/// Handles:
/// - `checkout.session.completed` — record revenue from completed checkout
/// - `invoice.paid` — record revenue from paid invoice
/// - `payment_intent.succeeded` — record revenue from successful payment intent
///
/// All other event types are ignored.
pub fn handle_event(event: &StripeEvent) -> WebhookAction {
    match event.event_type.as_str() {
        "checkout.session.completed" | "invoice.paid" | "payment_intent.succeeded" => {
            let amount = match extract_amount(&event.data) {
                Some(a) if a > 0.0 => a,
                Some(_) => {
                    debug!(event_id = %event.id, "Stripe event has zero amount, ignoring");
                    return WebhookAction::Ignore;
                }
                None => {
                    warn!(event_id = %event.id, event_type = %event.event_type, "Could not extract amount from Stripe event");
                    return WebhookAction::Error(format!(
                        "Could not extract amount from event {}",
                        event.id
                    ));
                }
            };

            let client = extract_client(&event.data);
            let description = extract_description(&event.event_type, &event.data);

            debug!(
                event_id = %event.id,
                event_type = %event.event_type,
                amount_usd = amount,
                client = %client,
                "Processing Stripe payment event"
            );

            WebhookAction::RecordRevenue {
                amount_usd: amount,
                client,
                description,
            }
        }
        _ => {
            debug!(event_type = %event.event_type, "Ignoring unhandled Stripe event type");
            WebhookAction::Ignore
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── Helper: compute a valid Stripe signature for testing ─────────────

    fn make_signature(payload: &str, secret: &str, timestamp: u64) -> String {
        let signed = format!("{}.{}", timestamp, payload);
        let mac = hmac_sha256(secret.as_bytes(), signed.as_bytes());
        format!("t={},v1={}", timestamp, hex::encode(mac))
    }

    // ── Signature verification tests ────────────────────────────────────

    #[test]
    fn test_verify_signature_valid() {
        let secret = "whsec_test_secret_key_12345";
        let payload = r#"{"id":"evt_1","type":"invoice.paid","data":{},"created":1000}"#;
        let timestamp = 1700000000u64;
        let sig = make_signature(payload, secret, timestamp);

        assert!(verify_signature(payload, &sig, secret));
    }

    #[test]
    fn test_verify_signature_wrong_secret() {
        let secret = "whsec_correct_secret";
        let wrong_secret = "whsec_wrong_secret";
        let payload = r#"{"id":"evt_2","type":"invoice.paid","data":{},"created":1000}"#;
        let timestamp = 1700000000u64;
        let sig = make_signature(payload, secret, timestamp);

        assert!(!verify_signature(payload, &sig, wrong_secret));
    }

    #[test]
    fn test_verify_signature_tampered_payload() {
        let secret = "whsec_test_secret";
        let payload = r#"{"id":"evt_3","type":"invoice.paid","data":{},"created":1000}"#;
        let tampered = r#"{"id":"evt_3","type":"invoice.paid","data":{},"created":9999}"#;
        let timestamp = 1700000000u64;
        let sig = make_signature(payload, secret, timestamp);

        assert!(!verify_signature(tampered, &sig, secret));
    }

    #[test]
    fn test_verify_signature_missing_v1() {
        let payload = r#"{"id":"evt_4"}"#;
        let sig = "t=1700000000";
        assert!(!verify_signature(payload, sig, "whsec_any"));
    }

    #[test]
    fn test_verify_signature_missing_timestamp() {
        let payload = r#"{"id":"evt_5"}"#;
        let sig = "v1=abcdef1234567890";
        assert!(!verify_signature(payload, sig, "whsec_any"));
    }

    #[test]
    fn test_verify_signature_empty_header() {
        let payload = r#"{"id":"evt_6"}"#;
        assert!(!verify_signature(payload, "", "whsec_any"));
    }

    // ── Event parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_event_valid() {
        let payload = json!({
            "id": "evt_123",
            "type": "invoice.paid",
            "data": {
                "object": {
                    "amount_paid": 5000,
                    "customer_email": "alice@example.com"
                }
            },
            "created": 1700000000u64
        })
        .to_string();

        let event = parse_event(&payload).unwrap();
        assert_eq!(event.id, "evt_123");
        assert_eq!(event.event_type, "invoice.paid");
        assert_eq!(event.created, 1700000000);
        assert!(event.data.get("object").is_some());
    }

    #[test]
    fn test_parse_event_malformed_json() {
        let result = parse_event("{not valid json!!!");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_event_missing_required_fields() {
        // Missing "type" field
        let payload = json!({
            "id": "evt_456",
            "data": {},
            "created": 1000
        })
        .to_string();

        let result = parse_event(&payload);
        assert!(result.is_err());
    }

    // ── Amount extraction tests ─────────────────────────────────────────

    #[test]
    fn test_extract_amount_from_amount_total() {
        let data = json!({
            "object": { "amount_total": 2999 }
        });
        let amount = extract_amount(&data).unwrap();
        assert!((amount - 29.99).abs() < 0.001);
    }

    #[test]
    fn test_extract_amount_from_amount_paid() {
        let data = json!({
            "object": { "amount_paid": 10000 }
        });
        let amount = extract_amount(&data).unwrap();
        assert!((amount - 100.0).abs() < 0.001);
    }

    #[test]
    fn test_extract_amount_from_amount() {
        let data = json!({
            "object": { "amount": 4250 }
        });
        let amount = extract_amount(&data).unwrap();
        assert!((amount - 42.50).abs() < 0.001);
    }

    #[test]
    fn test_extract_amount_priority_order() {
        // amount_total takes precedence over amount
        let data = json!({
            "object": {
                "amount_total": 5000,
                "amount": 3000
            }
        });
        let amount = extract_amount(&data).unwrap();
        assert!((amount - 50.0).abs() < 0.001);
    }

    #[test]
    fn test_extract_amount_missing() {
        let data = json!({ "object": { "currency": "usd" } });
        assert!(extract_amount(&data).is_none());
    }

    #[test]
    fn test_extract_amount_no_object() {
        let data = json!({ "something_else": 42 });
        assert!(extract_amount(&data).is_none());
    }

    #[test]
    fn test_extract_amount_negative_rejected() {
        let data = json!({
            "object": { "amount": -500 }
        });
        assert!(extract_amount(&data).is_none());
    }

    // ── Event handling tests ────────────────────────────────────────────

    #[test]
    fn test_handle_checkout_session_completed() {
        let event = StripeEvent {
            id: "evt_checkout_1".to_string(),
            event_type: "checkout.session.completed".to_string(),
            data: json!({
                "object": {
                    "amount_total": 9900,
                    "customer_email": "buyer@example.com",
                    "description": "Pro Plan Subscription"
                }
            }),
            created: 1700000000,
        };

        match handle_event(&event) {
            WebhookAction::RecordRevenue {
                amount_usd,
                client,
                description,
            } => {
                assert!((amount_usd - 99.0).abs() < 0.001);
                assert_eq!(client, "buyer@example.com");
                assert_eq!(description, "Pro Plan Subscription");
            }
            other => panic!("Expected RecordRevenue, got {:?}", other),
        }
    }

    #[test]
    fn test_handle_invoice_paid() {
        let event = StripeEvent {
            id: "evt_invoice_1".to_string(),
            event_type: "invoice.paid".to_string(),
            data: json!({
                "object": {
                    "amount_paid": 4900,
                    "customer": "cus_ABC123",
                    "metadata": {
                        "product": "Clawtex API"
                    }
                }
            }),
            created: 1700000000,
        };

        match handle_event(&event) {
            WebhookAction::RecordRevenue {
                amount_usd,
                client,
                description,
            } => {
                assert!((amount_usd - 49.0).abs() < 0.001);
                assert_eq!(client, "cus_ABC123");
                assert_eq!(description, "Payment for Clawtex API");
            }
            other => panic!("Expected RecordRevenue, got {:?}", other),
        }
    }

    #[test]
    fn test_handle_payment_intent_succeeded() {
        let event = StripeEvent {
            id: "evt_pi_1".to_string(),
            event_type: "payment_intent.succeeded".to_string(),
            data: json!({
                "object": {
                    "amount": 25000,
                    "customer_email": "enterprise@corp.com",
                    "description": "Enterprise consulting"
                }
            }),
            created: 1700000000,
        };

        match handle_event(&event) {
            WebhookAction::RecordRevenue {
                amount_usd,
                client,
                description,
            } => {
                assert!((amount_usd - 250.0).abs() < 0.001);
                assert_eq!(client, "enterprise@corp.com");
                assert_eq!(description, "Enterprise consulting");
            }
            other => panic!("Expected RecordRevenue, got {:?}", other),
        }
    }

    #[test]
    fn test_handle_unknown_event_type() {
        let event = StripeEvent {
            id: "evt_unknown".to_string(),
            event_type: "customer.subscription.created".to_string(),
            data: json!({ "object": {} }),
            created: 1700000000,
        };

        assert_eq!(handle_event(&event), WebhookAction::Ignore);
    }

    #[test]
    fn test_handle_event_zero_amount_ignored() {
        let event = StripeEvent {
            id: "evt_zero".to_string(),
            event_type: "checkout.session.completed".to_string(),
            data: json!({
                "object": { "amount_total": 0 }
            }),
            created: 1700000000,
        };

        assert_eq!(handle_event(&event), WebhookAction::Ignore);
    }

    #[test]
    fn test_handle_event_missing_amount_is_error() {
        let event = StripeEvent {
            id: "evt_noamt".to_string(),
            event_type: "invoice.paid".to_string(),
            data: json!({ "object": { "currency": "usd" } }),
            created: 1700000000,
        };

        match handle_event(&event) {
            WebhookAction::Error(msg) => {
                assert!(msg.contains("evt_noamt"));
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    // ── Client extraction tests ─────────────────────────────────────────

    #[test]
    fn test_extract_client_from_customer_email() {
        let data = json!({
            "object": {
                "customer_email": "test@example.com",
                "customer": "cus_fallback"
            }
        });
        assert_eq!(extract_client(&data), "test@example.com");
    }

    #[test]
    fn test_extract_client_from_customer_details() {
        let data = json!({
            "object": {
                "customer_details": { "email": "nested@example.com" }
            }
        });
        assert_eq!(extract_client(&data), "nested@example.com");
    }

    #[test]
    fn test_extract_client_fallback_to_customer_id() {
        let data = json!({
            "object": { "customer": "cus_XYZ789" }
        });
        assert_eq!(extract_client(&data), "cus_XYZ789");
    }

    #[test]
    fn test_extract_client_unknown() {
        let data = json!({ "object": {} });
        assert_eq!(extract_client(&data), "unknown");
    }

    // ── StripeWebhook.process integration tests ─────────────────────────

    #[test]
    fn test_webhook_process_valid_event() {
        let secret = "whsec_integration_test";
        let webhook = StripeWebhook::new(secret);
        let payload = json!({
            "id": "evt_proc_1",
            "type": "invoice.paid",
            "data": {
                "object": {
                    "amount_paid": 7500,
                    "customer_email": "paid@example.com"
                }
            },
            "created": 1700000000u64
        })
        .to_string();

        let sig = make_signature(&payload, secret, 1700000000);

        match webhook.process(&payload, &sig) {
            WebhookAction::RecordRevenue {
                amount_usd, client, ..
            } => {
                assert!((amount_usd - 75.0).abs() < 0.001);
                assert_eq!(client, "paid@example.com");
            }
            other => panic!("Expected RecordRevenue, got {:?}", other),
        }
    }

    #[test]
    fn test_webhook_process_invalid_signature() {
        let webhook = StripeWebhook::new("whsec_real_secret");
        let payload = r#"{"id":"evt_bad","type":"invoice.paid","data":{"object":{"amount_paid":100}},"created":1000}"#;
        let sig = "t=1700000000,v1=0000000000000000000000000000000000000000000000000000000000000000";

        match webhook.process(payload, sig) {
            WebhookAction::Error(msg) => {
                assert!(msg.contains("Invalid webhook signature"));
            }
            other => panic!("Expected Error, got {:?}", other),
        }
    }

    // ── HMAC-SHA256 correctness test ────────────────────────────────────

    #[test]
    fn test_hmac_sha256_known_vector() {
        // RFC 4231 Test Case 2: "what do ya want for nothing?" with key "Jefe"
        let key = b"Jefe";
        let msg = b"what do ya want for nothing?";
        let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";

        let result = hmac_sha256(key, msg);
        assert_eq!(hex::encode(&result), expected);
    }

    #[test]
    fn test_hmac_sha256_empty_message() {
        let key = b"secret";
        let msg = b"";
        // Just verify it produces a 32-byte (256-bit) output without panicking.
        let result = hmac_sha256(key, msg);
        assert_eq!(result.len(), 32);
    }

    // ── Description extraction tests ────────────────────────────────────

    #[test]
    fn test_extract_description_from_object() {
        let data = json!({
            "object": { "description": "Monthly subscription" }
        });
        assert_eq!(
            extract_description("invoice.paid", &data),
            "Monthly subscription"
        );
    }

    #[test]
    fn test_extract_description_from_metadata() {
        let data = json!({
            "object": {
                "metadata": { "product": "API Credits" }
            }
        });
        assert_eq!(
            extract_description("checkout.session.completed", &data),
            "Payment for API Credits"
        );
    }

    #[test]
    fn test_extract_description_fallback() {
        let data = json!({ "object": {} });
        assert_eq!(
            extract_description("payment_intent.succeeded", &data),
            "Stripe payment_intent.succeeded"
        );
    }

    // ── Constant-time comparison tests ──────────────────────────────────

    #[test]
    fn test_constant_time_eq_identical() {
        assert!(constant_time_eq(b"hello", b"hello"));
    }

    #[test]
    fn test_constant_time_eq_different() {
        assert!(!constant_time_eq(b"hello", b"world"));
    }

    #[test]
    fn test_constant_time_eq_different_lengths() {
        assert!(!constant_time_eq(b"short", b"longer_string"));
    }
}
