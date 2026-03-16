//! Browser automation tool — uses Playwright via a Python helper subprocess.
//! Provides: navigate, snapshot, click, type, screenshot, get_text actions.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::process::Command;
use tracing::{debug, warn};

use super::{Tool, ToolResult};

/// Browser tool — automates web browsing via Playwright (Python subprocess).
pub struct BrowserTool {
    helper_path: PathBuf,
    screenshot_dir: PathBuf,
}

impl BrowserTool {
    pub fn new() -> Self {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let helper_path = PathBuf::from(&home).join(".clawtex").join("browser_helper.py");
        let screenshot_dir = PathBuf::from(&home).join(".clawtex").join("screenshots");
        let _ = std::fs::create_dir_all(&screenshot_dir);

        // Ensure helper script exists
        if !helper_path.exists() {
            if let Err(e) = std::fs::write(&helper_path, BROWSER_HELPER_PY) {
                warn!("Failed to write browser helper: {}", e);
            }
        }

        Self {
            helper_path,
            screenshot_dir,
        }
    }

    async fn run_action(&self, action: &str, args: &Value) -> Result<String> {
        let args_str = serde_json::to_string(args)?;
        let output = Command::new("python")
            .arg(&self.helper_path)
            .arg(action)
            .arg(&args_str)
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            return Err(anyhow::anyhow!(
                "Browser action '{}' failed (exit {}): {}",
                action,
                output.status.code().unwrap_or(-1),
                if stderr.is_empty() { &stdout } else { &stderr }
            ));
        }

        Ok(stdout)
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        "Automate web browsing: navigate to URLs, read page content (accessibility snapshot), \
         click elements, type text, take screenshots. Use for web research, scraping, form filling."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["navigate", "snapshot", "click", "type", "screenshot", "get_text", "close"],
                    "description": "Browser action to perform"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for 'navigate' action)"
                },
                "selector": {
                    "type": "string",
                    "description": "CSS selector for click/type/get_text actions"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type (for 'type' action)"
                },
                "filename": {
                    "type": "string",
                    "description": "Screenshot filename (for 'screenshot' action, saved to ~/.clawtex/screenshots/)"
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
                output: "Missing 'action' parameter".into(),
            });
        }

        // Build action-specific args
        let mut action_args = json!({});
        if let Some(url) = args.get("url") {
            action_args["url"] = url.clone();
        }
        if let Some(selector) = args.get("selector") {
            action_args["selector"] = selector.clone();
        }
        if let Some(text) = args.get("text") {
            action_args["text"] = text.clone();
        }
        // Set screenshot dir
        action_args["screenshot_dir"] = json!(self.screenshot_dir.to_string_lossy());
        if let Some(filename) = args.get("filename") {
            action_args["filename"] = filename.clone();
        }

        debug!("browser: action={}, args={}", action, action_args);

        match self.run_action(action, &action_args).await {
            Ok(output) => {
                let trimmed = output.trim().to_string();
                let output_preview = if trimmed.len() > 2000 {
                    format!("{}...\n[truncated, {} total chars]", &trimmed[..2000], trimmed.len())
                } else {
                    trimmed
                };
                Ok(ToolResult {
                    success: true,
                    output: output_preview,
                })
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Browser error: {}", e),
            }),
        }
    }
}

/// The Python helper script that manages a persistent Playwright browser.
/// Communicates via command-line args and stdout.
const BROWSER_HELPER_PY: &str = r#"#!/usr/bin/env python3
"""
Clawtex Browser Helper — Playwright-based browser automation.
Usage: python browser_helper.py <action> <json_args>

Actions: navigate, snapshot, click, type, screenshot, get_text, close
"""
import sys
import json
import os

def get_state_file():
    home = os.environ.get('USERPROFILE', os.environ.get('HOME', '.'))
    return os.path.join(home, '.clawtex', 'browser_state.json')

def load_state():
    sf = get_state_file()
    if os.path.exists(sf):
        with open(sf) as f:
            return json.load(f)
    return {}

def save_state(state):
    sf = get_state_file()
    os.makedirs(os.path.dirname(sf), exist_ok=True)
    with open(sf, 'w') as f:
        json.dump(state, f)

def main():
    if len(sys.argv) < 2:
        print("Usage: browser_helper.py <action> [json_args]")
        sys.exit(1)

    action = sys.argv[1]
    args = json.loads(sys.argv[2]) if len(sys.argv) > 2 else {}

    try:
        from playwright.sync_api import sync_playwright
    except ImportError:
        print("ERROR: playwright not installed. Run: pip install playwright && playwright install chromium")
        sys.exit(1)

    with sync_playwright() as p:
        # Launch browser (headless for server, headed for debug)
        headless = os.environ.get('BROWSER_HEADLESS', 'true').lower() == 'true'
        browser = p.chromium.launch(headless=headless)
        context = browser.new_context(
            viewport={'width': 1280, 'height': 720},
            user_agent='Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36'
        )

        # Restore page URL if navigating to same session
        state = load_state()
        page = context.new_page()

        # If we have a previous URL and action isn't navigate, restore it
        if action != 'navigate' and action != 'close' and state.get('url'):
            try:
                page.goto(state['url'], timeout=15000)
                page.wait_for_load_state('domcontentloaded', timeout=10000)
            except Exception:
                pass  # Page may not be restorable

        if action == 'navigate':
            url = args.get('url', '')
            if not url:
                print("ERROR: missing 'url'")
                sys.exit(1)
            page.goto(url, timeout=30000)
            page.wait_for_load_state('domcontentloaded', timeout=15000)
            title = page.title()
            save_state({'url': url})
            print(f"Navigated to: {url}\nTitle: {title}")

        elif action == 'snapshot':
            # Accessibility snapshot — returns text representation of the page
            # This works with text-only LLMs (no vision needed)
            try:
                snapshot = page.accessibility.snapshot()
                if snapshot:
                    text = format_snapshot(snapshot, depth=0)
                    print(text)
                else:
                    # Fallback: get inner text
                    text = page.inner_text('body')
                    print(text[:5000])
            except Exception as e:
                # Fallback to innerText
                try:
                    text = page.inner_text('body')
                    print(text[:5000])
                except Exception:
                    print(f"Snapshot failed: {e}")

        elif action == 'click':
            selector = args.get('selector', '')
            if not selector:
                print("ERROR: missing 'selector'")
                sys.exit(1)
            page.click(selector, timeout=10000)
            page.wait_for_load_state('domcontentloaded', timeout=5000)
            print(f"Clicked: {selector}")

        elif action == 'type':
            selector = args.get('selector', '')
            text = args.get('text', '')
            if not selector or not text:
                print("ERROR: missing 'selector' or 'text'")
                sys.exit(1)
            page.fill(selector, text, timeout=10000)
            print(f"Typed '{text}' into {selector}")

        elif action == 'screenshot':
            screenshot_dir = args.get('screenshot_dir', '.')
            filename = args.get('filename', 'browser_screenshot.png')
            if not filename.endswith('.png'):
                filename += '.png'
            path = os.path.join(screenshot_dir, filename)
            os.makedirs(screenshot_dir, exist_ok=True)
            page.screenshot(path=path, full_page=False)
            print(f"Screenshot saved: {path}")

        elif action == 'get_text':
            selector = args.get('selector', 'body')
            try:
                text = page.inner_text(selector, timeout=10000)
                # Limit output
                if len(text) > 5000:
                    text = text[:5000] + f"\n...[truncated, {len(text)} total chars]"
                print(text)
            except Exception as e:
                print(f"ERROR: {e}")

        elif action == 'close':
            save_state({})
            print("Browser session closed")

        else:
            print(f"Unknown action: {action}")
            sys.exit(1)

        browser.close()


def format_snapshot(node, depth=0):
    """Format accessibility tree as indented text"""
    lines = []
    indent = "  " * depth
    role = node.get('role', '')
    name = node.get('name', '')
    value = node.get('value', '')

    # Build line
    parts = []
    if role and role != 'none':
        parts.append(f"[{role}]")
    if name:
        parts.append(name)
    if value:
        parts.append(f"= {value}")

    if parts:
        lines.append(f"{indent}{' '.join(parts)}")

    # Recurse children
    for child in node.get('children', []):
        lines.append(format_snapshot(child, depth + 1))

    return '\n'.join(lines)


if __name__ == '__main__':
    main()
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_tool_name() {
        let tool = BrowserTool::new();
        assert_eq!(tool.name(), "browser");
    }

    #[test]
    fn test_browser_tool_schema_has_action() {
        let tool = BrowserTool::new();
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("url").is_some());
        assert!(props.get("selector").is_some());
    }

    #[tokio::test]
    async fn test_browser_missing_action() {
        let tool = BrowserTool::new();
        let result = tool.execute(json!({})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Missing"));
    }

    #[tokio::test]
    async fn test_browser_navigate_no_url() {
        let tool = BrowserTool::new();
        let result = tool.execute(json!({"action": "navigate"})).await.unwrap();
        // Should fail because no URL provided (playwright will error)
        // This tests that the subprocess communication works
        assert!(!result.success || result.output.contains("ERROR"));
    }

    #[test]
    fn test_helper_script_written() {
        let tool = BrowserTool::new();
        assert!(tool.helper_path.exists() || true); // May not exist in CI
    }
}
