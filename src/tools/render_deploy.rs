// Render deployment tool
// Supports: create_service, deploy, get_status, set_env, list_services, delete_service
// Uses Render REST API (api.render.com) with RENDER_API_KEY env var

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, info};

use super::{Tool, ToolResult};

const RENDER_API_BASE: &str = "https://api.render.com/v1";

pub struct RenderDeployTool {
    client: Client,
    api_key: String,
}

impl RenderDeployTool {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, api_key }
    }

    async fn render_post(&self, endpoint: &str, body: &Value) -> Result<Value> {
        let url = format!("{}/{}", RENDER_API_BASE, endpoint);
        debug!("render POST {}", url);
        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(json!({"error": "Failed to parse response"}));
        if !status.is_success() {
            let msg = body.pointer("/message")
                .or_else(|| body.pointer("/error"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Render API error ({}): {}", status, msg);
        }
        Ok(body)
    }

    async fn render_get(&self, endpoint: &str) -> Result<Value> {
        let url = format!("{}/{}", RENDER_API_BASE, endpoint);
        debug!("render GET {}", url);
        let resp = self.client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(json!({"error": "Failed to parse response"}));
        if !status.is_success() {
            let msg = body.pointer("/message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Render API error ({}): {}", status, msg);
        }
        Ok(body)
    }

    async fn render_delete(&self, endpoint: &str) -> Result<Value> {
        let url = format!("{}/{}", RENDER_API_BASE, endpoint);
        debug!("render DELETE {}", url);
        let resp = self.client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await?;
        let status = resp.status();
        if status.is_success() {
            Ok(json!({"deleted": true}))
        } else {
            let body: Value = resp.json().await.unwrap_or(json!({}));
            let msg = body.pointer("/message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            anyhow::bail!("Render API error ({}): {}", status, msg);
        }
    }

    async fn create_service(&self, name: &str, repo_url: &str, branch: &str, env: &str) -> Result<String> {
        // Determine runtime: Docker if Dockerfile exists, Node otherwise
        let body = json!({
            "type": "web_service",
            "name": name,
            "repo": repo_url,
            "branch": branch,
            "autoDeploy": "yes",
            "serviceDetails": {
                "env": env,
                "plan": "free",
                "region": "oregon",
                "numInstances": 1
            }
        });

        let resp = self.render_post("services", &body).await?;
        let service = &resp["service"];
        let id = service["id"].as_str().unwrap_or("");
        let slug = service["slug"].as_str().unwrap_or("");
        let service_url = format!("https://{}.onrender.com", slug);
        info!("render: created service '{}' → {} ({})", name, id, service_url);
        Ok(format!(
            "Service created:\n  id: {}\n  name: {}\n  url: {}\n  repo: {}\n  branch: {}\n  auto_deploy: yes",
            id, name, service_url, repo_url, branch
        ))
    }

    async fn deploy(&self, service_id: &str) -> Result<String> {
        let body = json!({"clearCache": "do_not_clear"});
        let resp = self.render_post(&format!("services/{}/deploys", service_id), &body).await?;
        let deploy = &resp["deploy"];
        let deploy_id = deploy["id"].as_str().unwrap_or("");
        let status = deploy["status"].as_str().unwrap_or("unknown");
        info!("render: triggered deploy {} for service {}", deploy_id, service_id);
        Ok(format!("Deploy triggered:\n  deploy_id: {}\n  status: {}\n  service: {}", deploy_id, status, service_id))
    }

    async fn get_status(&self, service_id: &str) -> Result<String> {
        let resp = self.render_get(&format!("services/{}", service_id)).await?;
        let name = resp["name"].as_str().unwrap_or("");
        let status = resp["suspended"].as_str().unwrap_or("active");
        let slug = resp["slug"].as_str().unwrap_or("");
        let url = format!("https://{}.onrender.com", slug);
        let updated = resp["updatedAt"].as_str().unwrap_or("");

        // Get latest deploy
        let deploys = self.render_get(&format!("services/{}/deploys?limit=1", service_id)).await.ok();
        let deploy_status = deploys.as_ref()
            .and_then(|d| d.as_array())
            .and_then(|arr| arr.first())
            .and_then(|d| d["deploy"]["status"].as_str())
            .unwrap_or("unknown");

        Ok(format!(
            "Service Status:\n  name: {}\n  id: {}\n  url: {}\n  status: {}\n  latest_deploy: {}\n  updated: {}",
            name, service_id, url, status, deploy_status, updated
        ))
    }

    async fn set_env(&self, service_id: &str, key: &str, value: &str) -> Result<String> {
        let body = json!([{
            "key": key,
            "value": value
        }]);
        self.render_post(&format!("services/{}/env-vars", service_id), &body).await?;
        info!("render: set env var {} on service {}", key, service_id);
        Ok(format!("Environment variable set:\n  service: {}\n  key: {}\n  value: [set]", service_id, key))
    }

    async fn list_services(&self, limit: u64) -> Result<String> {
        let resp = self.render_get(&format!("services?limit={}", limit)).await?;
        let mut output = String::from("Render Services:\n");
        if let Some(arr) = resp.as_array() {
            for item in arr {
                let svc = &item["service"];
                let id = svc["id"].as_str().unwrap_or("");
                let name = svc["name"].as_str().unwrap_or("(unnamed)");
                let slug = svc["slug"].as_str().unwrap_or("");
                let svc_type = svc["type"].as_str().unwrap_or("");
                output.push_str(&format!("  {} — {} (https://{}.onrender.com) [{}]\n", id, name, slug, svc_type));
            }
            if arr.is_empty() {
                output.push_str("  (no services found)\n");
            }
        }
        Ok(output)
    }

    async fn delete_service(&self, service_id: &str) -> Result<String> {
        self.render_delete(&format!("services/{}", service_id)).await?;
        info!("render: deleted service {}", service_id);
        Ok(format!("Service deleted: {}", service_id))
    }
}

#[async_trait]
impl Tool for RenderDeployTool {
    fn name(&self) -> &str { "render_deploy" }

    fn description(&self) -> &str {
        "Deploy services to Render cloud platform. Actions: create_service (from GitHub repo), \
         deploy (trigger new deploy), get_status, set_env (environment variables), \
         list_services, delete_service. Use this to deploy API services and web apps."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create_service", "deploy", "get_status", "set_env", "list_services", "delete_service"],
                    "description": "The Render action to perform"
                },
                "name": {
                    "type": "string",
                    "description": "Service name (for create_service)"
                },
                "repo_url": {
                    "type": "string",
                    "description": "GitHub repo URL (for create_service, e.g. https://github.com/user/repo)"
                },
                "branch": {
                    "type": "string",
                    "description": "Git branch to deploy (default: main)"
                },
                "env": {
                    "type": "string",
                    "enum": ["docker", "node", "python"],
                    "description": "Runtime environment (default: docker)"
                },
                "service_id": {
                    "type": "string",
                    "description": "Service ID (for deploy, get_status, set_env, delete_service)"
                },
                "key": {
                    "type": "string",
                    "description": "Environment variable name (for set_env)"
                },
                "value": {
                    "type": "string",
                    "description": "Environment variable value (for set_env)"
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
            "create_service" => {
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let repo_url = args.get("repo_url").and_then(|v| v.as_str()).unwrap_or("");
                let branch = args.get("branch").and_then(|v| v.as_str()).unwrap_or("main");
                let env = args.get("env").and_then(|v| v.as_str()).unwrap_or("docker");
                if name.is_empty() || repo_url.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'name' and 'repo_url' are required".into() });
                }
                self.create_service(name, repo_url, branch, env).await
            }
            "deploy" => {
                let service_id = args.get("service_id").and_then(|v| v.as_str()).unwrap_or("");
                if service_id.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'service_id' is required".into() });
                }
                self.deploy(service_id).await
            }
            "get_status" => {
                let service_id = args.get("service_id").and_then(|v| v.as_str()).unwrap_or("");
                if service_id.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'service_id' is required".into() });
                }
                self.get_status(service_id).await
            }
            "set_env" => {
                let service_id = args.get("service_id").and_then(|v| v.as_str()).unwrap_or("");
                let key = args.get("key").and_then(|v| v.as_str()).unwrap_or("");
                let value = args.get("value").and_then(|v| v.as_str()).unwrap_or("");
                if service_id.is_empty() || key.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'service_id' and 'key' are required".into() });
                }
                self.set_env(service_id, key, value).await
            }
            "list_services" => {
                let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);
                self.list_services(limit).await
            }
            "delete_service" => {
                let service_id = args.get("service_id").and_then(|v| v.as_str()).unwrap_or("");
                if service_id.is_empty() {
                    return Ok(ToolResult { success: false, output: "Error: 'service_id' is required".into() });
                }
                self.delete_service(service_id).await
            }
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: format!("Unknown action '{}'. Available: create_service, deploy, get_status, set_env, list_services, delete_service", action),
                });
            }
        };

        match result {
            Ok(output) => Ok(ToolResult { success: true, output }),
            Err(e) => Ok(ToolResult { success: false, output: format!("Render error: {}", e) }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_missing_action() {
        let tool = RenderDeployTool::new("rnd_fake_key".into());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_create_service_missing_fields() {
        let tool = RenderDeployTool::new("rnd_fake_key".into());
        let result = tool.execute(json!({"action": "create_service"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("name"));
    }

    #[tokio::test]
    async fn test_deploy_missing_service_id() {
        let tool = RenderDeployTool::new("rnd_fake_key".into());
        let result = tool.execute(json!({"action": "deploy"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("service_id"));
    }

    #[tokio::test]
    async fn test_set_env_missing_fields() {
        let tool = RenderDeployTool::new("rnd_fake_key".into());
        let result = tool.execute(json!({"action": "set_env"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("service_id"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let tool = RenderDeployTool::new("rnd_fake_key".into());
        let result = tool.execute(json!({"action": "invalid"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }
}
