// Stripe payment integration tool
// Supports: create_product, create_price, create_checkout_link, list_customers, get_balance
// Uses Stripe REST API with STRIPE_SECRET_KEY env var

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, info};

use super::{Tool, ToolResult};

const STRIPE_API_BASE: &str = "https://api.stripe.com/v1";

pub struct StripeTool {
    client: Client,
    api_key: String,
}

impl StripeTool {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, api_key }
    }

    async fn stripe_post(&self, endpoint: &str, params: &[(String, String)]) -> Result<Value> {
        let url = format!("{}/{}", STRIPE_API_BASE, endpoint);
        debug!("stripe POST {}", url);
        let resp = self.client
            .post(&url)
            .basic_auth(&self.api_key, None::<&str>)
            .form(params)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body.pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Stripe API error ({}): {}", status, msg);
        }
        Ok(body)
    }

    async fn stripe_get(&self, endpoint: &str, params: &[(String, String)]) -> Result<Value> {
        let url = format!("{}/{}", STRIPE_API_BASE, endpoint);
        debug!("stripe GET {}", url);
        let resp = self.client
            .get(&url)
            .basic_auth(&self.api_key, None::<&str>)
            .query(params)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body.pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Stripe API error ({}): {}", status, msg);
        }
        Ok(body)
    }

    async fn create_product(&self, name: &str, description: &str) -> Result<String> {
        let params = vec![
            ("name".into(), name.to_string()),
            ("description".into(), description.to_string()),
        ];
        let resp = self.stripe_post("products", &params).await?;
        let id = resp["id"].as_str().unwrap_or("").to_string();
        info!("stripe: created product '{}' → {}", name, id);
        Ok(format!("Product created:\n  id: {}\n  name: {}\n  description: {}", id, name, description))
    }

    async fn create_price(&self, product_id: &str, amount_cents: u64, currency: &str, recurring_interval: &str) -> Result<String> {
        let mut params = vec![
            ("product".into(), product_id.to_string()),
            ("currency".into(), currency.to_string()),
        ];

        if recurring_interval.is_empty() || recurring_interval == "one_time" {
            // One-time price
            params.push(("unit_amount".into(), amount_cents.to_string()));
        } else {
            // Recurring price
            params.push(("unit_amount".into(), amount_cents.to_string()));
            params.push(("recurring[interval]".into(), recurring_interval.to_string()));
        }

        let resp = self.stripe_post("prices", &params).await?;
        let id = resp["id"].as_str().unwrap_or("").to_string();
        let price_display = format!("${:.2}/{}", amount_cents as f64 / 100.0,
            if recurring_interval.is_empty() || recurring_interval == "one_time" { "once" } else { recurring_interval });
        info!("stripe: created price {} → {}", price_display, id);
        Ok(format!("Price created:\n  id: {}\n  amount: {}\n  product: {}", id, price_display, product_id))
    }

    async fn create_checkout_link(&self, price_id: &str, success_url: &str, cancel_url: &str) -> Result<String> {
        let params = vec![
            ("line_items[0][price]".into(), price_id.to_string()),
            ("line_items[0][quantity]".into(), "1".to_string()),
            ("mode".into(), if price_id.contains("recurring") { "subscription" } else { "payment" }.to_string()),
            ("success_url".into(), success_url.to_string()),
            ("cancel_url".into(), cancel_url.to_string()),
        ];
        let resp = self.stripe_post("checkout/sessions", &params).await?;
        let url = resp["url"].as_str().unwrap_or("").to_string();
        let id = resp["id"].as_str().unwrap_or("").to_string();
        info!("stripe: created checkout session → {}", id);
        Ok(format!("Checkout session created:\n  id: {}\n  url: {}", id, url))
    }

    async fn create_payment_link(&self, price_id: &str) -> Result<String> {
        let params = vec![
            ("line_items[0][price]".into(), price_id.to_string()),
            ("line_items[0][quantity]".into(), "1".to_string()),
        ];
        let resp = self.stripe_post("payment_links", &params).await?;
        let url = resp["url"].as_str().unwrap_or("").to_string();
        let id = resp["id"].as_str().unwrap_or("").to_string();
        info!("stripe: created payment link → {}", url);
        Ok(format!("Payment link created:\n  id: {}\n  url: {}\n\nShare this URL with customers to collect payments.", id, url))
    }

    async fn list_customers(&self, limit: u64) -> Result<String> {
        let params = vec![("limit".into(), limit.to_string())];
        let resp = self.stripe_get("customers", &params).await?;
        let customers = resp["data"].as_array().map(|arr| arr.len()).unwrap_or(0);
        let mut output = format!("Customers ({}):\n", customers);
        if let Some(arr) = resp["data"].as_array() {
            for c in arr {
                let email = c["email"].as_str().unwrap_or("(no email)");
                let name = c["name"].as_str().unwrap_or("(unnamed)");
                let id = c["id"].as_str().unwrap_or("");
                output.push_str(&format!("  {} — {} ({})\n", id, name, email));
            }
        }
        Ok(output)
    }

    async fn get_balance(&self) -> Result<String> {
        let resp = self.stripe_get("balance", &[]).await?;
        let mut output = String::from("Stripe Balance:\n");
        if let Some(available) = resp["available"].as_array() {
            for b in available {
                let amount = b["amount"].as_i64().unwrap_or(0);
                let currency = b["currency"].as_str().unwrap_or("usd");
                output.push_str(&format!("  Available: ${:.2} {}\n", amount as f64 / 100.0, currency));
            }
        }
        if let Some(pending) = resp["pending"].as_array() {
            for b in pending {
                let amount = b["amount"].as_i64().unwrap_or(0);
                let currency = b["currency"].as_str().unwrap_or("usd");
                output.push_str(&format!("  Pending: ${:.2} {}\n", amount as f64 / 100.0, currency));
            }
        }
        Ok(output)
    }

    async fn list_products(&self, limit: u64) -> Result<String> {
        let params = vec![("limit".into(), limit.to_string())];
        let resp = self.stripe_get("products", &params).await?;
        let mut output = String::from("Products:\n");
        if let Some(arr) = resp["data"].as_array() {
            for p in arr {
                let id = p["id"].as_str().unwrap_or("");
                let name = p["name"].as_str().unwrap_or("(unnamed)");
                let active = p["active"].as_bool().unwrap_or(false);
                output.push_str(&format!("  {} — {} (active: {})\n", id, name, active));
            }
        }
        Ok(output)
    }
}

#[async_trait]
impl Tool for StripeTool {
    fn name(&self) -> &str { "stripe" }

    fn description(&self) -> &str {
        "Stripe payment integration. Actions: create_product, create_price, create_payment_link, \
         create_checkout, list_customers, list_products, get_balance. \
         Use this to set up billing for SaaS products."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create_product", "create_price", "create_payment_link", "create_checkout", "list_customers", "list_products", "get_balance"],
                    "description": "The Stripe action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Product name (for create_product)"
                },
                "description": {
                    "type": "string",
                    "description": "Product description (for create_product)"
                },
                "product_id": {
                    "type": "string",
                    "description": "Product ID (for create_price, e.g. prod_xxx)"
                },
                "price_id": {
                    "type": "string",
                    "description": "Price ID (for create_payment_link/create_checkout, e.g. price_xxx)"
                },
                "amount_cents": {
                    "type": "integer",
                    "description": "Price in cents (e.g. 999 = $9.99) (for create_price)"
                },
                "currency": {
                    "type": "string",
                    "description": "Currency code (default: usd) (for create_price)"
                },
                "recurring_interval": {
                    "type": "string",
                    "enum": ["one_time", "month", "year"],
                    "description": "Billing interval (for create_price, default: one_time)"
                },
                "success_url": {
                    "type": "string",
                    "description": "URL to redirect after successful payment (for create_checkout)"
                },
                "cancel_url": {
                    "type": "string",
                    "description": "URL to redirect if payment is cancelled (for create_checkout)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of items to list (default: 10)"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");

        let result = match action {
            "create_product" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let desc = args.get("description").and_then(|v| v.as_str()).unwrap_or("");
                if name.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'name' is required for create_product".into() });
                }
                self.create_product(name, desc).await
            }
            "create_price" => {
                let product_id = args.get("product_id").and_then(|v| v.as_str()).unwrap_or("");
                let amount = args.get("amount_cents").and_then(|v| v.as_u64()).unwrap_or(0);
                let currency = args.get("currency").and_then(|v| v.as_str()).unwrap_or("usd");
                let interval = args.get("recurring_interval").and_then(|v| v.as_str()).unwrap_or("one_time");
                if product_id.is_empty() || amount == 0 {
                    return Ok(ToolResult { success: false, output: "Error: 'product_id' and 'amount_cents' are required".into() });
                }
                self.create_price(product_id, amount, currency, interval).await
            }
            "create_payment_link" => {
                let price_id = args.get("price_id").and_then(|v| v.as_str()).unwrap_or("");
                if price_id.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'price_id' is required".into() });
                }
                self.create_payment_link(price_id).await
            }
            "create_checkout" => {
                let price_id = args.get("price_id").and_then(|v| v.as_str()).unwrap_or("");
                let success_url = args.get("success_url").and_then(|v| v.as_str()).unwrap_or("https://example.com/success");
                let cancel_url = args.get("cancel_url").and_then(|v| v.as_str()).unwrap_or("https://example.com/cancel");
                if price_id.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'price_id' is required".into() });
                }
                self.create_checkout_link(price_id, success_url, cancel_url).await
            }
            "list_customers" => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
                self.list_customers(limit).await
            }
            "list_products" => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
                self.list_products(limit).await
            }
            "get_balance" => {
                self.get_balance().await
            }
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Unknown action '{}'. Available: create_product, create_price, create_payment_link, create_checkout, list_customers, list_products, get_balance", action),
                });
            }
        };

        match result {
            Ok(output) => Ok(ToolResult { success: true, output }),
            Err(e) => Ok(ToolResult { success: false, output: format!("Stripe error: {}", e) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_missing_action() {
        let tool = StripeTool::new("sk_test_fake".into());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_create_product_missing_name() {
        let tool = StripeTool::new("sk_test_fake".into());
        let result = tool.execute(json!({"action": "create_product"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("name"));
    }

    #[tokio::test]
    async fn test_create_price_missing_fields() {
        let tool = StripeTool::new("sk_test_fake".into());
        let result = tool.execute(json!({"action": "create_price"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("product_id"));
    }

    #[tokio::test]
    async fn test_create_payment_link_missing_price() {
        let tool = StripeTool::new("sk_test_fake".into());
        let result = tool.execute(json!({"action": "create_payment_link"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("price_id"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = StripeTool::new("sk_test_fake".into());
        let result = tool.execute(json!({"action": "invalid"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }
}
