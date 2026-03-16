//! Twitter tool — post tweets via API (OAuth 1.0a) or Playwright browser fallback.
//! API posting uses zero external Python deps (pure stdlib).
//! Browser fallback uses Playwright with persistent session cookies.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing::{debug, warn};

use super::{Tool, ToolResult};

/// Twitter tool configuration (from agents.toml [twitter] section)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct TwitterConfig {
    #[serde(default)]
    pub consumer_key: String,
    #[serde(default)]
    pub consumer_secret: String,
    #[serde(default)]
    pub access_token: String,
    #[serde(default)]
    pub access_secret: String,
    #[serde(default)]
    pub screen_name: String,
    /// Optional password for automated browser login
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub oauth2_client_id: String,
    #[serde(default)]
    pub oauth2_client_secret: String,
}

impl Default for TwitterConfig {
    fn default() -> Self {
        Self {
            consumer_key: String::new(),
            consumer_secret: String::new(),
            access_token: String::new(),
            access_secret: String::new(),
            screen_name: String::new(),
            password: String::new(),
            oauth2_client_id: String::new(),
            oauth2_client_secret: String::new(),
        }
    }
}

pub struct TwitterTool {
    config: TwitterConfig,
    helper_path: PathBuf,
}

impl TwitterTool {
    pub fn new(config: TwitterConfig) -> Self {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let helper_path = PathBuf::from(&home).join(".clawtex").join("twitter_helper.py");

        // Deploy helper script
        if let Err(e) = std::fs::write(&helper_path, TWITTER_HELPER_PY) {
            warn!("Failed to write twitter helper: {}", e);
        }

        Self {
            config,
            helper_path,
        }
    }

    async fn run_action(&self, action: &str, args: &Value) -> Result<String> {
        let args_str = serde_json::to_string(args)?;
        let output = tokio::process::Command::new("python")
            .arg(&self.helper_path)
            .arg(action)
            .arg(&args_str)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() && stdout.trim().is_empty() {
            return Err(anyhow::anyhow!(
                "Twitter action '{}' failed (exit {}): {}",
                action,
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() { "unknown error" } else { stderr.trim() }
            ));
        }

        Ok(stdout)
    }
}

#[async_trait]
impl Tool for TwitterTool {
    fn name(&self) -> &str {
        "twitter"
    }

    fn description(&self) -> &str {
        "Post tweets to Twitter/X. Actions: 'post' (auto: API first, browser fallback), \
         'login' (open browser for manual login to save session). \
         Args: action, text, method (api/browser/auto)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["post", "login"],
                    "description": "Action: 'post' to send tweet, 'login' to open browser for manual login"
                },
                "text": {
                    "type": "string",
                    "description": "Tweet text (max 280 chars, required for 'post' action)"
                },
                "method": {
                    "type": "string",
                    "enum": ["auto", "api", "browser"],
                    "description": "Posting method: 'auto' (try API then browser), 'api' (API only), 'browser' (Playwright only). Default: auto"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if action.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Missing 'action' parameter. Use 'post' or 'login'.".into(),
            });
        }

        match action {
            "post" => {
                let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("");
                if text.is_empty() {
                    return Ok(ToolResult {
                        success: false,
                        output: "Missing 'text' parameter for post action.".into(),
                    });
                }
                if text.len() > 280 {
                    return Ok(ToolResult {
                        success: false,
                        output: format!("Tweet too long: {} chars (max 280)", text.len()),
                    });
                }

                let method = args.get("method").and_then(|v| v.as_str()).unwrap_or("auto");

                let action_args = json!({
                    "text": text,
                    "method": method,
                    "config": {
                        "consumer_key": self.config.consumer_key,
                        "consumer_secret": self.config.consumer_secret,
                        "access_token": self.config.access_token,
                        "access_secret": self.config.access_secret,
                        "screen_name": self.config.screen_name,
                        "password": self.config.password,
                    }
                });

                debug!("twitter: posting tweet ({} chars) via {}", text.len(), method);

                match self.run_action("post", &action_args).await {
                    Ok(output) => {
                        match serde_json::from_str::<Value>(&output.trim()) {
                            Ok(result) => {
                                let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                                let method_used = result.get("method").and_then(|v| v.as_str()).unwrap_or("unknown");
                                if success {
                                    let tweet_id = result.get("tweet_id").and_then(|v| v.as_str()).unwrap_or("");
                                    let msg = result.get("message").and_then(|v| v.as_str()).unwrap_or("Tweet posted");
                                    let output_msg = if !tweet_id.is_empty() {
                                        format!("{} (via {}, id: {})", msg, method_used, tweet_id)
                                    } else {
                                        format!("{} (via {})", msg, method_used)
                                    };
                                    Ok(ToolResult { success: true, output: output_msg })
                                } else {
                                    let error = result.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                                    Ok(ToolResult {
                                        success: false,
                                        output: format!("Tweet failed ({}): {}", method_used, error),
                                    })
                                }
                            }
                            Err(_) => Ok(ToolResult {
                                success: false,
                                output: format!("Unexpected helper output: {}", output.trim()),
                            }),
                        }
                    }
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Twitter error: {}", e),
                    }),
                }
            }
            "login" => {
                let action_args = json!({
                    "config": {
                        "screen_name": self.config.screen_name,
                    }
                });

                match self.run_action("login", &action_args).await {
                    Ok(output) => {
                        match serde_json::from_str::<Value>(&output.trim()) {
                            Ok(result) => {
                                let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                                let msg = if success {
                                    result.get("message").and_then(|v| v.as_str())
                                        .unwrap_or("Login session saved").to_string()
                                } else {
                                    result.get("error").and_then(|v| v.as_str())
                                        .unwrap_or("Login failed").to_string()
                                };
                                Ok(ToolResult { success, output: msg })
                            }
                            Err(_) => Ok(ToolResult {
                                success: false,
                                output: format!("Unexpected output: {}", output.trim()),
                            }),
                        }
                    }
                    Err(e) => Ok(ToolResult {
                        success: false,
                        output: format!("Login error: {}", e),
                    }),
                }
            }
            _ => Ok(ToolResult {
                success: false,
                output: format!("Unknown action: '{}'. Use 'post' or 'login'.", action),
            }),
        }
    }
}

/// Python helper script for Twitter operations.
/// Supports OAuth 1.0a API posting and Playwright browser posting.
const TWITTER_HELPER_PY: &str = r#"#!/usr/bin/env python3
"""
Clawtex Twitter Helper — post tweets via API (OAuth 1.0a) or Playwright browser.
Usage: python twitter_helper.py <action> <json_args>
Actions: post, login
"""
import sys, json, os, time, hmac, hashlib, base64, urllib.parse, urllib.request, uuid

def oauth1_header(method, url, consumer_key, consumer_secret, access_token, access_secret):
    """Generate OAuth 1.0a Authorization header (no external deps)."""
    oauth_params = {
        'oauth_consumer_key': consumer_key,
        'oauth_nonce': uuid.uuid4().hex,
        'oauth_signature_method': 'HMAC-SHA1',
        'oauth_timestamp': str(int(time.time())),
        'oauth_token': access_token,
        'oauth_version': '1.0',
    }
    # For JSON body endpoints, only OAuth params in signature base string
    sorted_params = '&'.join(
        f'{urllib.parse.quote(k, safe="")}'
        f'={urllib.parse.quote(str(v), safe="")}'
        for k, v in sorted(oauth_params.items())
    )
    base_string = (
        f'{method}'
        f'&{urllib.parse.quote(url, safe="")}'
        f'&{urllib.parse.quote(sorted_params, safe="")}'
    )
    signing_key = (
        f'{urllib.parse.quote(consumer_secret, safe="")}'
        f'&{urllib.parse.quote(access_secret, safe="")}'
    )
    signature = base64.b64encode(
        hmac.new(signing_key.encode(), base_string.encode(), hashlib.sha1).digest()
    ).decode()
    oauth_params['oauth_signature'] = signature

    auth_header = 'OAuth ' + ', '.join(
        f'{urllib.parse.quote(k, safe="")}="{urllib.parse.quote(v, safe="")}"'
        for k, v in sorted(oauth_params.items())
    )
    return auth_header


def post_tweet_api(text, config):
    """Post tweet using Twitter API v2 with OAuth 1.0a."""
    url = 'https://api.twitter.com/2/tweets'
    auth = oauth1_header(
        'POST', url,
        config['consumer_key'], config['consumer_secret'],
        config['access_token'], config['access_secret'],
    )

    body = json.dumps({'text': text}).encode('utf-8')
    req = urllib.request.Request(url, data=body, method='POST')
    req.add_header('Authorization', auth)
    req.add_header('Content-Type', 'application/json')

    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            result = json.loads(resp.read().decode())
            tweet_id = result.get('data', {}).get('id', '')
            return {'success': True, 'method': 'api', 'tweet_id': tweet_id,
                    'message': f'Tweet posted successfully (id: {tweet_id})'}
    except urllib.error.HTTPError as e:
        error_body = e.read().decode()
        return {'success': False, 'method': 'api', 'error': f'HTTP {e.code}: {error_body}'}
    except Exception as e:
        return {'success': False, 'method': 'api', 'error': str(e)}


def post_tweet_browser(text, config):
    """Post tweet using Playwright browser automation with persistent session."""
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        return {'success': False, 'method': 'browser',
                'error': 'playwright not installed. Run: pip install playwright && playwright install chromium'}

    home = os.environ.get('USERPROFILE', os.environ.get('HOME', '.'))
    user_data = os.path.join(home, '.clawtex', 'twitter_session')
    os.makedirs(user_data, exist_ok=True)

    # Check if session exists (has cookies)
    has_session = os.path.exists(os.path.join(user_data, 'Default', 'Cookies')) or \
                  os.path.exists(os.path.join(user_data, 'Cookies'))

    with sync_playwright() as p:
        context = p.chromium.launch_persistent_context(
            user_data,
            headless=has_session,  # headless if session exists, visible if not
            viewport={'width': 1280, 'height': 720},
            user_agent=(
                'Mozilla/5.0 (Windows NT 10.0; Win64; x64) '
                'AppleWebKit/537.36 (KHTML, like Gecko) '
                'Chrome/130.0.0.0 Safari/537.36'
            ),
        )
        page = context.pages[0] if context.pages else context.new_page()

        try:
            # Navigate to Twitter home
            page.goto('https://x.com/home', timeout=30000)
            page.wait_for_load_state('domcontentloaded', timeout=15000)
            time.sleep(3)

            # Check if logged in
            compose = page.query_selector('[data-testid="tweetTextarea_0"]')
            if not compose:
                compose = page.query_selector('div[role="textbox"]')

            if not compose:
                # Not logged in
                username = config.get('screen_name', '')
                password = config.get('password', '')

                if not password:
                    context.close()
                    return {
                        'success': False, 'method': 'browser',
                        'error': (
                            'Not logged in. Either: '
                            '1) Add twitter.password to agents.toml, or '
                            '2) Run twitter(action="login") to log in manually first.'
                        )
                    }

                # Automate login
                page.goto('https://x.com/i/flow/login', timeout=30000)
                page.wait_for_load_state('domcontentloaded', timeout=15000)
                time.sleep(2)

                try:
                    # Enter username
                    username_input = page.wait_for_selector(
                        'input[autocomplete="username"]', timeout=10000
                    )
                    username_input.fill(username)
                    time.sleep(0.5)
                    page.keyboard.press('Enter')
                    time.sleep(2)

                    # Enter password
                    password_input = page.wait_for_selector(
                        'input[name="password"], input[type="password"]',
                        timeout=10000
                    )
                    password_input.fill(password)
                    time.sleep(0.5)
                    page.keyboard.press('Enter')
                    time.sleep(5)
                except Exception as e:
                    context.close()
                    return {'success': False, 'method': 'browser',
                            'error': f'Login interaction failed: {e}'}

                # Verify login
                page.goto('https://x.com/home', timeout=30000)
                time.sleep(3)
                compose = page.query_selector('[data-testid="tweetTextarea_0"]')
                if not compose:
                    compose = page.query_selector('div[role="textbox"]')
                if not compose:
                    context.close()
                    return {'success': False, 'method': 'browser',
                            'error': 'Login failed. Check credentials or run login action manually.'}

            # Type tweet
            compose.click()
            time.sleep(0.5)
            page.keyboard.type(text, delay=30)
            time.sleep(1)

            # Click Post button
            post_btn = page.query_selector('[data-testid="tweetButtonInline"]')
            if not post_btn:
                post_btn = page.query_selector('[data-testid="tweetButton"]')

            if post_btn:
                post_btn.click()
                time.sleep(3)
                context.close()
                return {'success': True, 'method': 'browser',
                        'message': 'Tweet posted via browser'}
            else:
                context.close()
                return {'success': False, 'method': 'browser',
                        'error': 'Post button not found on page'}

        except Exception as e:
            try:
                context.close()
            except Exception:
                pass
            return {'success': False, 'method': 'browser', 'error': str(e)}


def login_browser(config):
    """Open browser for manual Twitter login to save session cookies."""
    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        return {'success': False,
                'error': 'playwright not installed. Run: pip install playwright && playwright install chromium'}

    home = os.environ.get('USERPROFILE', os.environ.get('HOME', '.'))
    user_data = os.path.join(home, '.clawtex', 'twitter_session')
    os.makedirs(user_data, exist_ok=True)

    with sync_playwright() as p:
        context = p.chromium.launch_persistent_context(
            user_data,
            headless=False,
            viewport={'width': 1280, 'height': 720},
            user_agent=(
                'Mozilla/5.0 (Windows NT 10.0; Win64; x64) '
                'AppleWebKit/537.36 (KHTML, like Gecko) '
                'Chrome/130.0.0.0 Safari/537.36'
            ),
        )
        page = context.pages[0] if context.pages else context.new_page()
        page.goto('https://x.com/i/flow/login', timeout=30000)

        # Wait for user to complete login (up to 120 seconds)
        print(json.dumps({
            'status': 'waiting',
            'message': 'Browser opened. Please log in to Twitter/X. Waiting up to 120 seconds...'
        }), flush=True)

        for _ in range(24):
            time.sleep(5)
            current_url = page.url
            if 'home' in current_url and 'login' not in current_url:
                context.close()
                return {'success': True,
                        'message': 'Login session saved. Future posts will use this session.'}

        context.close()
        return {'success': False,
                'error': 'Login timed out after 120 seconds. Please try again.'}


def main():
    if len(sys.argv) < 2:
        print(json.dumps({'success': False, 'error': 'Usage: twitter_helper.py <action> [json_args]'}))
        sys.exit(1)

    action = sys.argv[1]
    args = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}

    if action == 'post':
        text = args.get('text', '')
        if not text:
            print(json.dumps({'success': False, 'error': 'Missing tweet text'}))
            sys.exit(1)

        method = args.get('method', 'auto')
        config = args.get('config', {})

        if method in ('api', 'auto'):
            # Try API first
            result = post_tweet_api(text, config)
            if result['success'] or method == 'api':
                print(json.dumps(result))
                return
            # API failed, fall through to browser if auto

        if method in ('browser', 'auto'):
            result = post_tweet_browser(text, config)
            print(json.dumps(result))
        else:
            print(json.dumps({'success': False, 'error': f'Unknown method: {method}'}))

    elif action == 'login':
        config = args.get('config', {})
        result = login_browser(config)
        print(json.dumps(result))

    else:
        print(json.dumps({'success': False, 'error': f'Unknown action: {action}'}))


if __name__ == '__main__':
    main()
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_twitter_tool_name() {
        let tool = TwitterTool::new(TwitterConfig::default());
        assert_eq!(tool.name(), "twitter");
    }

    #[test]
    fn test_twitter_config_defaults() {
        let config = TwitterConfig::default();
        assert!(config.consumer_key.is_empty());
        assert!(config.access_token.is_empty());
        assert!(config.screen_name.is_empty());
    }

    #[test]
    fn test_twitter_tool_schema() {
        let tool = TwitterTool::new(TwitterConfig::default());
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("text").is_some());
        assert!(props.get("method").is_some());
    }

    #[tokio::test]
    async fn test_twitter_missing_action() {
        let tool = TwitterTool::new(TwitterConfig::default());
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_twitter_post_missing_text() {
        let tool = TwitterTool::new(TwitterConfig::default());
        let result = tool.execute(json!({"action": "post"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing 'text'"));
    }

    #[tokio::test]
    async fn test_twitter_post_too_long() {
        let tool = TwitterTool::new(TwitterConfig::default());
        let long_text = "a".repeat(281);
        let result = tool.execute(json!({"action": "post", "text": long_text})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("too long"));
    }

    #[tokio::test]
    async fn test_twitter_unknown_action() {
        let tool = TwitterTool::new(TwitterConfig::default());
        let result = tool.execute(json!({"action": "delete"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Unknown action"));
    }
}
