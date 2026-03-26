// GitHub integration tool — create issues, list issues, create PRs, list repos, get repo status
// Uses GitHub REST API v3 with GITHUB_TOKEN env var for authentication
// Follows the same pattern as stripe.rs for action-based HTTP API tools

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, info};

use super::{Tool, ToolResult};

const GITHUB_API_BASE: &str = "https://api.github.com";

pub struct GitHubTool {
    client: Client,
    token: String,
}

impl GitHubTool {
    pub fn new(token: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to build HTTP client");
        Self { client, token }
    }

    /// Create from GITHUB_TOKEN env var (returns empty token if not set)
    pub fn from_env() -> Self {
        let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
        Self::new(token)
    }

    /// Send a GET request to the GitHub API
    async fn github_get(&self, endpoint: &str, query: &[(String, String)]) -> Result<Value> {
        let url = format!("{}{}", GITHUB_API_BASE, endpoint);
        debug!("github GET {}", url);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "phantom-mesh")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .query(query)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("GitHub API error ({}): {}", status, msg);
        }
        Ok(body)
    }

    /// Send a POST request to the GitHub API with JSON body
    async fn github_post(&self, endpoint: &str, body: &Value) -> Result<Value> {
        let url = format!("{}{}", GITHUB_API_BASE, endpoint);
        debug!("github POST {}", url);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("User-Agent", "phantom-mesh")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(body)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await?;
        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("GitHub API error ({}): {}", status, msg);
        }
        Ok(body)
    }

    /// Create an issue on a repository
    async fn create_issue(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
        assignees: &[String],
    ) -> Result<String> {
        let endpoint = format!("/repos/{}/{}/issues", owner, repo);
        let mut payload = json!({
            "title": title,
            "body": body,
        });
        if !labels.is_empty() {
            payload["labels"] = json!(labels);
        }
        if !assignees.is_empty() {
            payload["assignees"] = json!(assignees);
        }

        let resp = self.github_post(&endpoint, &payload).await?;
        let number = resp["number"].as_u64().unwrap_or(0);
        let html_url = resp["html_url"].as_str().unwrap_or("");
        info!("github: created issue #{} on {}/{}", number, owner, repo);
        Ok(format!(
            "Issue created:\n  number: #{}\n  title: {}\n  url: {}",
            number, title, html_url
        ))
    }

    /// List issues on a repository
    async fn list_issues(
        &self,
        owner: &str,
        repo: &str,
        state: &str,
        limit: u64,
    ) -> Result<String> {
        let endpoint = format!("/repos/{}/{}/issues", owner, repo);
        let params = vec![
            ("state".into(), state.to_string()),
            ("per_page".into(), limit.to_string()),
            ("sort".into(), "updated".to_string()),
            ("direction".into(), "desc".to_string()),
        ];
        let resp = self.github_get(&endpoint, &params).await?;
        let issues = resp.as_array().map(|a| a.len()).unwrap_or(0);
        let mut output = format!("Issues for {}/{} (state={}, {} results):\n", owner, repo, state, issues);
        if let Some(arr) = resp.as_array() {
            for issue in arr {
                let number = issue["number"].as_u64().unwrap_or(0);
                let title = issue["title"].as_str().unwrap_or("");
                let state = issue["state"].as_str().unwrap_or("");
                let user = issue["user"]["login"].as_str().unwrap_or("");
                let labels: Vec<&str> = issue["labels"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|l| l["name"].as_str())
                            .collect()
                    })
                    .unwrap_or_default();
                let label_str = if labels.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", labels.join(", "))
                };
                output.push_str(&format!(
                    "  #{} ({}) — {} @{}{}\n",
                    number, state, title, user, label_str
                ));
            }
        }
        Ok(output)
    }

    /// Create a pull request
    async fn create_pr(
        &self,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        head: &str,
        base: &str,
        draft: bool,
    ) -> Result<String> {
        let endpoint = format!("/repos/{}/{}/pulls", owner, repo);
        let payload = json!({
            "title": title,
            "body": body,
            "head": head,
            "base": base,
            "draft": draft,
        });
        let resp = self.github_post(&endpoint, &payload).await?;
        let number = resp["number"].as_u64().unwrap_or(0);
        let html_url = resp["html_url"].as_str().unwrap_or("");
        info!(
            "github: created PR #{} on {}/{} ({} -> {})",
            number, owner, repo, head, base
        );
        Ok(format!(
            "Pull request created:\n  number: #{}\n  title: {}\n  head: {} -> base: {}\n  url: {}",
            number, title, head, base, html_url
        ))
    }

    /// List repositories for the authenticated user (or a specific user/org)
    async fn list_repos(&self, owner: &str, repo_type: &str, limit: u64) -> Result<String> {
        let (endpoint, params) = if owner.is_empty() {
            // Authenticated user's repos
            (
                "/user/repos".to_string(),
                vec![
                    ("type".into(), repo_type.to_string()),
                    ("per_page".into(), limit.to_string()),
                    ("sort".into(), "updated".to_string()),
                    ("direction".into(), "desc".to_string()),
                ],
            )
        } else {
            // Specific user's repos
            (
                format!("/users/{}/repos", owner),
                vec![
                    ("type".into(), repo_type.to_string()),
                    ("per_page".into(), limit.to_string()),
                    ("sort".into(), "updated".to_string()),
                    ("direction".into(), "desc".to_string()),
                ],
            )
        };
        let resp = self.github_get(&endpoint, &params).await?;
        let repos = resp.as_array().map(|a| a.len()).unwrap_or(0);
        let who = if owner.is_empty() {
            "authenticated user"
        } else {
            owner
        };
        let mut output = format!("Repositories for {} ({} results):\n", who, repos);
        if let Some(arr) = resp.as_array() {
            for repo in arr {
                let full_name = repo["full_name"].as_str().unwrap_or("");
                let description = repo["description"].as_str().unwrap_or("(no description)");
                let private = repo["private"].as_bool().unwrap_or(false);
                let stars = repo["stargazers_count"].as_u64().unwrap_or(0);
                let language = repo["language"].as_str().unwrap_or("?");
                let visibility = if private { "private" } else { "public" };
                output.push_str(&format!(
                    "  {} ({}, {}, {} stars) — {}\n",
                    full_name, visibility, language, stars, description
                ));
            }
        }
        Ok(output)
    }

    /// Get repository status: latest commit, open issues/PRs count, branch info
    async fn get_repo_status(&self, owner: &str, repo: &str) -> Result<String> {
        // Fetch repo info
        let repo_endpoint = format!("/repos/{}/{}", owner, repo);
        let repo_info = self.github_get(&repo_endpoint, &[]).await?;

        let full_name = repo_info["full_name"].as_str().unwrap_or("");
        let description = repo_info["description"].as_str().unwrap_or("(none)");
        let default_branch = repo_info["default_branch"].as_str().unwrap_or("main");
        let open_issues = repo_info["open_issues_count"].as_u64().unwrap_or(0);
        let stars = repo_info["stargazers_count"].as_u64().unwrap_or(0);
        let forks = repo_info["forks_count"].as_u64().unwrap_or(0);
        let language = repo_info["language"].as_str().unwrap_or("?");
        let private = repo_info["private"].as_bool().unwrap_or(false);
        let visibility = if private { "private" } else { "public" };

        let mut output = format!(
            "Repository: {}\n  description: {}\n  visibility: {}\n  language: {}\n  \
             default_branch: {}\n  open_issues: {}\n  stars: {}\n  forks: {}\n",
            full_name, description, visibility, language, default_branch, open_issues, stars, forks
        );

        // Try to fetch latest commit on default branch
        let commits_endpoint = format!("/repos/{}/{}/commits", owner, repo);
        let commits_params = vec![
            ("sha".into(), default_branch.to_string()),
            ("per_page".into(), "1".to_string()),
        ];
        match self.github_get(&commits_endpoint, &commits_params).await {
            Ok(commits) => {
                if let Some(arr) = commits.as_array() {
                    if let Some(latest) = arr.first() {
                        let sha = latest["sha"].as_str().unwrap_or("").chars().take(7).collect::<String>();
                        let message = latest["commit"]["message"]
                            .as_str()
                            .unwrap_or("")
                            .lines()
                            .next()
                            .unwrap_or("");
                        let author = latest["commit"]["author"]["name"]
                            .as_str()
                            .unwrap_or("");
                        let date = latest["commit"]["author"]["date"]
                            .as_str()
                            .unwrap_or("");
                        output.push_str(&format!(
                            "\n  Latest commit ({}):\n    {} — {} by {} ({})\n",
                            default_branch, sha, message, author, date
                        ));
                    }
                }
            }
            Err(e) => {
                output.push_str(&format!("\n  (Could not fetch latest commit: {})\n", e));
            }
        }

        // Try to fetch open PR count
        let prs_endpoint = format!("/repos/{}/{}/pulls", owner, repo);
        let prs_params = vec![
            ("state".into(), "open".to_string()),
            ("per_page".into(), "1".to_string()),
        ];
        match self.github_get(&prs_endpoint, &prs_params).await {
            Ok(prs) => {
                let count = prs.as_array().map(|a| a.len()).unwrap_or(0);
                // The open_issues count includes PRs; we show PRs separately if available
                output.push_str(&format!("  open_prs: {}+\n", count));
            }
            Err(_) => {}
        }

        Ok(output)
    }
}

#[async_trait]
impl Tool for GitHubTool {
    fn name(&self) -> &str {
        "github"
    }

    fn description(&self) -> &str {
        "GitHub integration. Actions: create_issue, list_issues, create_pr, list_repos, \
         get_repo_status. Uses GitHub REST API with GITHUB_TOKEN for authentication."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["create_issue", "list_issues", "create_pr", "list_repos", "get_repo_status"],
                    "description": "The GitHub action to perform"
                },
                "owner": {
                    "type": "string",
                    "description": "Repository owner (user or org). For list_repos: leave empty for authenticated user."
                },
                "repo": {
                    "type": "string",
                    "description": "Repository name (e.g. 'phantom-mesh')"
                },
                "title": {
                    "type": "string",
                    "description": "Issue or PR title (for create_issue, create_pr)"
                },
                "body": {
                    "type": "string",
                    "description": "Issue or PR body/description (for create_issue, create_pr)"
                },
                "labels": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Labels to apply (for create_issue)"
                },
                "assignees": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "GitHub usernames to assign (for create_issue)"
                },
                "head": {
                    "type": "string",
                    "description": "Head branch for PR (for create_pr)"
                },
                "base": {
                    "type": "string",
                    "description": "Base branch for PR (default: main) (for create_pr)"
                },
                "draft": {
                    "type": "boolean",
                    "description": "Create PR as draft (default: false) (for create_pr)"
                },
                "state": {
                    "type": "string",
                    "enum": ["open", "closed", "all"],
                    "description": "Issue state filter (default: open) (for list_issues)"
                },
                "repo_type": {
                    "type": "string",
                    "enum": ["all", "owner", "public", "private", "member"],
                    "description": "Repository type filter (default: all) (for list_repos)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Number of items to return (default: 10, max: 100)"
                }
            },
            "required": ["action"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if action.is_empty() {
            anyhow::bail!("Missing required parameter 'action'");
        }

        let valid_actions = [
            "create_issue",
            "list_issues",
            "create_pr",
            "list_repos",
            "get_repo_status",
        ];
        if !valid_actions.contains(&action) {
            anyhow::bail!(
                "Invalid action '{}'. Valid actions: {}",
                action,
                valid_actions.join(", ")
            );
        }

        // Token is required for all actions
        if self.token.is_empty() {
            anyhow::bail!(
                "GITHUB_TOKEN is not set. Set the GITHUB_TOKEN environment variable to use the github tool."
            );
        }

        // Validate required args per action
        match action {
            "create_issue" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if owner.is_empty() || repo.is_empty() {
                    anyhow::bail!("create_issue requires 'owner' and 'repo'");
                }
                if title.is_empty() {
                    anyhow::bail!("create_issue requires 'title'");
                }
            }
            "list_issues" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                if owner.is_empty() || repo.is_empty() {
                    anyhow::bail!("list_issues requires 'owner' and 'repo'");
                }
            }
            "create_pr" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let head = args.get("head").and_then(|v| v.as_str()).unwrap_or("");
                if owner.is_empty() || repo.is_empty() {
                    anyhow::bail!("create_pr requires 'owner' and 'repo'");
                }
                if title.is_empty() {
                    anyhow::bail!("create_pr requires 'title'");
                }
                if head.is_empty() {
                    anyhow::bail!("create_pr requires 'head' branch");
                }
            }
            "get_repo_status" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                if owner.is_empty() || repo.is_empty() {
                    anyhow::bail!("get_repo_status requires 'owner' and 'repo'");
                }
            }
            // list_repos has no required args beyond action
            _ => {}
        }

        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        // Run preflight checks
        if let Err(e) = self.preflight(&args) {
            return Ok(ToolResult {
                success: false,
                output: format!("Preflight check failed: {}", e),
            });
        }

        let action = args
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let result = match action {
            "create_issue" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let labels: Vec<String> = args
                    .get("labels")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                let assignees: Vec<String> = args
                    .get("assignees")
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();
                self.create_issue(owner, repo, title, body, &labels, &assignees)
                    .await
            }
            "list_issues" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                let state = args
                    .get("state")
                    .and_then(|v| v.as_str())
                    .unwrap_or("open");
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10)
                    .min(100);
                self.list_issues(owner, repo, state, limit).await
            }
            "create_pr" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                let title = args
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let body = args.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let head = args.get("head").and_then(|v| v.as_str()).unwrap_or("");
                let base = args
                    .get("base")
                    .and_then(|v| v.as_str())
                    .unwrap_or("main");
                let draft = args
                    .get("draft")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                self.create_pr(owner, repo, title, body, head, base, draft)
                    .await
            }
            "list_repos" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo_type = args
                    .get("repo_type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("all");
                let limit = args
                    .get("limit")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(10)
                    .min(100);
                self.list_repos(owner, repo_type, limit).await
            }
            "get_repo_status" => {
                let owner = args
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let repo = args.get("repo").and_then(|v| v.as_str()).unwrap_or("");
                self.get_repo_status(owner, repo).await
            }
            _ => {
                return Ok(ToolResult {
                    success: false,
                    output: format!(
                        "Unknown action '{}'. Available: create_issue, list_issues, create_pr, list_repos, get_repo_status",
                        action
                    ),
                });
            }
        };

        match result {
            Ok(output) => Ok(ToolResult {
                success: true,
                output,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("GitHub error: {}", e),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> GitHubTool {
        GitHubTool::new("ghp_test_fake_token_1234567890".into())
    }

    fn make_tool_no_token() -> GitHubTool {
        GitHubTool::new(String::new())
    }

    // ── Basic trait tests ────────────────────────────────────────────────────

    #[test]
    fn test_tool_name() {
        let tool = make_tool();
        assert_eq!(tool.name(), "github");
    }

    #[test]
    fn test_tool_description_contains_actions() {
        let tool = make_tool();
        let desc = tool.description();
        assert!(desc.contains("create_issue"));
        assert!(desc.contains("list_issues"));
        assert!(desc.contains("create_pr"));
        assert!(desc.contains("list_repos"));
        assert!(desc.contains("get_repo_status"));
    }

    #[test]
    fn test_schema_has_all_properties() {
        let tool = make_tool();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("owner").is_some());
        assert!(props.get("repo").is_some());
        assert!(props.get("title").is_some());
        assert!(props.get("body").is_some());
        assert!(props.get("labels").is_some());
        assert!(props.get("assignees").is_some());
        assert!(props.get("head").is_some());
        assert!(props.get("base").is_some());
        assert!(props.get("draft").is_some());
        assert!(props.get("state").is_some());
        assert!(props.get("limit").is_some());
        assert!(props.get("repo_type").is_some());
    }

    #[test]
    fn test_schema_action_required() {
        let tool = make_tool();
        let schema = tool.parameters_schema();
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.contains(&json!("action")));
    }

    #[test]
    fn test_spec_generation() {
        let tool = make_tool();
        let spec = tool.spec();
        assert_eq!(spec.name, "github");
        assert!(!spec.description.is_empty());
        assert!(spec.parameters.get("properties").is_some());
    }

    // ── Preflight tests ─────────────────────────────────────────────────────

    #[test]
    fn test_preflight_missing_action() {
        let tool = make_tool();
        let result = tool.preflight(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("action"));
    }

    #[test]
    fn test_preflight_invalid_action() {
        let tool = make_tool();
        let result = tool.preflight(&json!({"action": "delete_repo"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid action"));
    }

    #[test]
    fn test_preflight_no_token() {
        let tool = make_tool_no_token();
        let result = tool.preflight(&json!({"action": "list_repos"}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("GITHUB_TOKEN"));
    }

    #[test]
    fn test_preflight_create_issue_missing_owner_repo() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "create_issue",
            "title": "Test"
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_preflight_create_issue_missing_title() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "create_issue",
            "owner": "markl-a",
            "repo": "Phantom Mesh"
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("title"));
    }

    #[test]
    fn test_preflight_create_issue_valid() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "create_issue",
            "owner": "markl-a",
            "repo": "Phantom Mesh",
            "title": "Test issue"
        }));
        assert!(result.is_ok());
    }

    #[test]
    fn test_preflight_list_issues_missing_repo() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "list_issues",
            "owner": "markl-a"
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owner"));
    }

    #[test]
    fn test_preflight_list_issues_valid() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "list_issues",
            "owner": "markl-a",
            "repo": "Phantom Mesh"
        }));
        assert!(result.is_ok());
    }

    #[test]
    fn test_preflight_create_pr_missing_head() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "create_pr",
            "owner": "markl-a",
            "repo": "Phantom Mesh",
            "title": "My PR"
        }));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("head"));
    }

    #[test]
    fn test_preflight_create_pr_valid() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "create_pr",
            "owner": "markl-a",
            "repo": "Phantom Mesh",
            "title": "My PR",
            "head": "feat/new-tool"
        }));
        assert!(result.is_ok());
    }

    #[test]
    fn test_preflight_get_repo_status_missing_repo() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "get_repo_status",
            "owner": "markl-a"
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_preflight_list_repos_valid_no_owner() {
        let tool = make_tool();
        let result = tool.preflight(&json!({
            "action": "list_repos"
        }));
        assert!(result.is_ok());
    }

    // ── Execute tests (arg validation, no network) ──────────────────────────

    #[tokio::test]
    async fn test_execute_missing_action() {
        let tool = make_tool();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Preflight"));
        assert!(result.output.contains("action"));
    }

    #[tokio::test]
    async fn test_execute_invalid_action() {
        let tool = make_tool();
        let result = tool.execute(json!({"action": "fork_repo"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Invalid action"));
    }

    #[tokio::test]
    async fn test_execute_no_token() {
        let tool = make_tool_no_token();
        let result = tool
            .execute(json!({"action": "list_repos"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("GITHUB_TOKEN"));
    }

    #[tokio::test]
    async fn test_execute_create_issue_missing_required() {
        let tool = make_tool();
        let result = tool
            .execute(json!({"action": "create_issue"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("owner"));
    }

    #[tokio::test]
    async fn test_execute_create_pr_missing_head() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "create_pr",
                "owner": "markl-a",
                "repo": "Phantom Mesh",
                "title": "test"
            }))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.output.contains("head"));
    }

    #[tokio::test]
    async fn test_execute_get_repo_status_missing_repo() {
        let tool = make_tool();
        let result = tool
            .execute(json!({
                "action": "get_repo_status",
                "owner": "markl-a"
            }))
            .await
            .unwrap();
        assert!(!result.success);
    }

    // ── from_env constructor test ───────────────────────────────────────────

    #[test]
    fn test_from_env_no_token() {
        // When GITHUB_TOKEN is not set, token should be empty
        // (We don't unset the env var in case it exists; just verify the constructor works)
        let tool = GitHubTool::from_env();
        assert_eq!(tool.name(), "github");
    }

    // ── Action enum validation in schema ────────────────────────────────────

    #[test]
    fn test_schema_action_enum_values() {
        let tool = make_tool();
        let schema = tool.parameters_schema();
        let action_enum = schema
            .pointer("/properties/action/enum")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(action_enum.len(), 5);
        assert!(action_enum.contains(&json!("create_issue")));
        assert!(action_enum.contains(&json!("list_issues")));
        assert!(action_enum.contains(&json!("create_pr")));
        assert!(action_enum.contains(&json!("list_repos")));
        assert!(action_enum.contains(&json!("get_repo_status")));
    }

    #[test]
    fn test_schema_state_enum_values() {
        let tool = make_tool();
        let schema = tool.parameters_schema();
        let state_enum = schema
            .pointer("/properties/state/enum")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(state_enum.len(), 3);
        assert!(state_enum.contains(&json!("open")));
        assert!(state_enum.contains(&json!("closed")));
        assert!(state_enum.contains(&json!("all")));
    }
}
