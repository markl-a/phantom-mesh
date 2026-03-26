//! Email receive tool — reads emails via IMAP using Python subprocess.
//! Uses Python's built-in `imaplib` and `email` modules (no pip install needed).
//! Actions: check (list folders + unread), read (single email by UID), search (by subject/from/date).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use tracing::{debug, warn};

use super::{Tool, ToolResult};

/// IMAP configuration (from agents.toml [imap] section or env vars)
#[derive(Debug, Clone, serde::Deserialize)]
pub struct ImapConfig {
    #[serde(default = "default_imap_host")]
    pub host: String,
    #[serde(default = "default_imap_port")]
    pub port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    /// Use SSL (default: true)
    #[serde(default = "default_true")]
    pub use_ssl: bool,
}

fn default_imap_host() -> String { "imap.gmail.com".to_string() }
fn default_imap_port() -> u16 { 993 }
fn default_true() -> bool { true }

impl Default for ImapConfig {
    fn default() -> Self {
        Self {
            host: default_imap_host(),
            port: default_imap_port(),
            username: String::new(),
            password: String::new(),
            use_ssl: true,
        }
    }
}

impl ImapConfig {
    /// Build config by merging: explicit config < env vars < args override.
    /// Env vars: IMAP_HOST, IMAP_PORT, IMAP_USER, IMAP_PASS
    pub fn resolve(&self, args: &Value) -> ImapConfig {
        let host = args.get("imap_host").and_then(|v| v.as_str()).map(String::from)
            .or_else(|| std::env::var("IMAP_HOST").ok())
            .unwrap_or_else(|| self.host.clone());

        let port = args.get("imap_port").and_then(|v| v.as_u64()).map(|p| p as u16)
            .or_else(|| std::env::var("IMAP_PORT").ok().and_then(|s| s.parse().ok()))
            .unwrap_or(self.port);

        let username = args.get("imap_user").and_then(|v| v.as_str()).map(String::from)
            .or_else(|| std::env::var("IMAP_USER").ok())
            .unwrap_or_else(|| self.username.clone());

        let password = args.get("imap_pass").and_then(|v| v.as_str()).map(String::from)
            .or_else(|| std::env::var("IMAP_PASS").ok())
            .unwrap_or_else(|| self.password.clone());

        let use_ssl = args.get("use_ssl").and_then(|v| v.as_bool())
            .unwrap_or(self.use_ssl);

        ImapConfig { host, port, username, password, use_ssl }
    }

    /// Check if minimum IMAP config is available (host + username + password).
    pub fn is_configured(&self) -> bool {
        !self.username.is_empty() && !self.password.is_empty() && !self.host.is_empty()
    }
}

pub struct EmailReceiveTool {
    config: ImapConfig,
}

impl EmailReceiveTool {
    pub fn new(config: ImapConfig) -> Self {
        Self { config }
    }

    /// Generate the inline Python IMAP helper script.
    fn python_script() -> &'static str {
        r#"#!/usr/bin/env python3
"""Phantom Mesh IMAP helper - check/read/search emails via imaplib."""
import sys, json, imaplib, email
from email.header import decode_header
from email.utils import parsedate_to_datetime

def decode_hdr(raw):
    if raw is None:
        return ""
    parts = decode_header(raw)
    result = []
    for data, charset in parts:
        if isinstance(data, bytes):
            result.append(data.decode(charset or "utf-8", errors="replace"))
        else:
            result.append(data)
    return " ".join(result)

def connect(cfg):
    if cfg.get("use_ssl", True):
        m = imaplib.IMAP4_SSL(cfg["host"], cfg.get("port", 993))
    else:
        m = imaplib.IMAP4(cfg["host"], cfg.get("port", 143))
    m.login(cfg["username"], cfg["password"])
    return m

def action_check(m, cfg):
    """List folders with unread counts."""
    status, folders_raw = m.list()
    folders = []
    for f in (folders_raw or []):
        if isinstance(f, bytes):
            parts = f.decode("utf-8", errors="replace").split(' "/" ')
            if len(parts) >= 2:
                name = parts[-1].strip().strip('"')
            else:
                name = parts[0].strip().strip('"')
        else:
            name = str(f)
        # Get unread count for this folder
        try:
            m.select(name, readonly=True)
            st, data = m.search(None, "UNSEEN")
            unread = len(data[0].split()) if st == "OK" and data[0] else 0
            folders.append({"name": name, "unread": unread})
        except Exception:
            folders.append({"name": name, "unread": -1})
    return {"success": True, "folders": folders}

def action_read(m, cfg):
    """Read a single email by UID."""
    folder = cfg.get("folder", "INBOX")
    uid = cfg.get("message_id", "")
    if not uid:
        return {"success": False, "error": "message_id (UID) is required for read action"}

    m.select(folder, readonly=True)
    status, data = m.uid("FETCH", str(uid), "(RFC822)")
    if status != "OK" or not data or data[0] is None:
        return {"success": False, "error": f"Message UID {uid} not found in {folder}"}

    raw = data[0][1] if isinstance(data[0], tuple) else data[0]
    msg = email.message_from_bytes(raw)
    subject = decode_hdr(msg.get("Subject"))
    sender = decode_hdr(msg.get("From"))
    date = msg.get("Date", "")
    to = decode_hdr(msg.get("To"))
    cc = decode_hdr(msg.get("Cc", ""))

    body = ""
    if msg.is_multipart():
        for part in msg.walk():
            ct = part.get_content_type()
            if ct == "text/plain":
                payload = part.get_payload(decode=True)
                if payload:
                    charset = part.get_content_charset() or "utf-8"
                    body = payload.decode(charset, errors="replace")
                    break
        if not body:
            for part in msg.walk():
                ct = part.get_content_type()
                if ct == "text/html":
                    payload = part.get_payload(decode=True)
                    if payload:
                        charset = part.get_content_charset() or "utf-8"
                        body = payload.decode(charset, errors="replace")
                        break
    else:
        payload = msg.get_payload(decode=True)
        if payload:
            charset = msg.get_content_charset() or "utf-8"
            body = payload.decode(charset, errors="replace")

    # Truncate very long bodies
    if len(body) > 5000:
        body = body[:5000] + "\n...(truncated)"

    attachments = []
    if msg.is_multipart():
        for part in msg.walk():
            fn = part.get_filename()
            if fn:
                attachments.append(decode_hdr(fn))

    return {
        "success": True,
        "email": {
            "uid": str(uid),
            "subject": subject,
            "from": sender,
            "to": to,
            "cc": cc,
            "date": date,
            "body": body,
            "attachments": attachments,
        }
    }

def action_search(m, cfg):
    """Search emails by subject/from/date."""
    folder = cfg.get("folder", "INBOX")
    query = cfg.get("query", "")
    limit = int(cfg.get("limit", 10))

    if not query:
        return {"success": False, "error": "query is required for search action"}

    m.select(folder, readonly=True)

    # Build IMAP search criteria
    criteria = []
    # Check if query looks like a structured search (key:value)
    if ":" in query and any(query.lower().startswith(p) for p in ["from:", "subject:", "since:", "before:", "to:"]):
        for part in query.split(","):
            part = part.strip()
            if part.lower().startswith("from:"):
                criteria.append(f'FROM "{part[5:].strip()}"')
            elif part.lower().startswith("subject:"):
                criteria.append(f'SUBJECT "{part[8:].strip()}"')
            elif part.lower().startswith("to:"):
                criteria.append(f'TO "{part[3:].strip()}"')
            elif part.lower().startswith("since:"):
                criteria.append(f'SINCE "{part[6:].strip()}"')
            elif part.lower().startswith("before:"):
                criteria.append(f'BEFORE "{part[7:].strip()}"')
            else:
                criteria.append(f'SUBJECT "{part}"')
    else:
        # Treat entire query as subject search
        criteria.append(f'SUBJECT "{query}"')

    search_str = " ".join(criteria) if criteria else "ALL"
    status, data = m.search(None, search_str)

    if status != "OK":
        return {"success": False, "error": f"IMAP search failed: {status}"}

    uids_raw = data[0].split() if data[0] else []
    # Take the most recent N (last in list = newest)
    uids = uids_raw[-limit:] if len(uids_raw) > limit else uids_raw
    uids.reverse()  # newest first

    results = []
    for uid_bytes in uids:
        uid = uid_bytes.decode() if isinstance(uid_bytes, bytes) else str(uid_bytes)
        st, msg_data = m.fetch(uid, "(BODY.PEEK[HEADER.FIELDS (SUBJECT FROM DATE)])")
        if st == "OK" and msg_data and msg_data[0] is not None:
            raw = msg_data[0][1] if isinstance(msg_data[0], tuple) else msg_data[0]
            msg = email.message_from_bytes(raw) if isinstance(raw, bytes) else email.message_from_string(str(raw))
            results.append({
                "uid": uid,
                "subject": decode_hdr(msg.get("Subject")),
                "from": decode_hdr(msg.get("From")),
                "date": msg.get("Date", ""),
            })

    return {"success": True, "total_matches": len(uids_raw), "showing": len(results), "emails": results}

def main():
    cfg = json.loads(sys.stdin.read())
    action = cfg.get("action", "check")
    try:
        m = connect(cfg)
        if action == "check":
            result = action_check(m, cfg)
        elif action == "read":
            result = action_read(m, cfg)
        elif action == "search":
            result = action_search(m, cfg)
        else:
            result = {"success": False, "error": f"Unknown action: {action}"}
        m.logout()
    except Exception as e:
        result = {"success": False, "error": str(e)}
    print(json.dumps(result, ensure_ascii=False))

if __name__ == "__main__":
    main()
"#
    }

    /// Write the Python helper script to a temp location and return its path.
    fn deploy_helper(&self) -> Result<String> {
        let home = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .unwrap_or_else(|_| ".".to_string());
        let helper_path = format!("{}/.phantom-mesh/imap_helper.py", home);
        std::fs::write(&helper_path, Self::python_script())?;
        Ok(helper_path)
    }
}

#[async_trait]
impl Tool for EmailReceiveTool {
    fn name(&self) -> &str {
        "email_receive"
    }

    fn description(&self) -> &str {
        "Receive and read emails via IMAP. Actions: 'check' (list folders + unread count), 'read' (read single email by UID), 'search' (search by subject/from/date)"
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "description": "Action to perform: 'check', 'read', or 'search'",
                    "enum": ["check", "read", "search"]
                },
                "folder": {
                    "type": "string",
                    "description": "IMAP folder to operate on (default: INBOX)"
                },
                "query": {
                    "type": "string",
                    "description": "Search query. Can be plain text (searches subject) or structured: 'from:user@example.com, subject:invoice, since:01-Mar-2026'"
                },
                "message_id": {
                    "type": "string",
                    "description": "Email UID to read (required for 'read' action)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Max results to return for search (default: 10)"
                },
                "imap_host": {
                    "type": "string",
                    "description": "Override IMAP host (default: from config or IMAP_HOST env var)"
                },
                "imap_port": {
                    "type": "integer",
                    "description": "Override IMAP port (default: 993 for SSL, 143 for plain)"
                },
                "imap_user": {
                    "type": "string",
                    "description": "Override IMAP username (default: from config or IMAP_USER env var)"
                },
                "imap_pass": {
                    "type": "string",
                    "description": "Override IMAP password (default: from config or IMAP_PASS env var)"
                },
                "use_ssl": {
                    "type": "boolean",
                    "description": "Use SSL/TLS connection (default: true)"
                }
            },
            "required": ["action"]
        })
    }

    fn preflight(&self, args: &Value) -> Result<()> {
        // Check that Python is available (try "python" first, then "python3" for macOS/Linux)
        let python_ok = std::process::Command::new("python")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !python_ok {
            let python3_ok = std::process::Command::new("python3")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !python3_ok {
                anyhow::bail!("Python is not available. email_receive requires Python with imaplib (built-in).");
            }
        }

        // Check IMAP config is available (from config, env vars, or args)
        let resolved = self.config.resolve(args);
        if !resolved.is_configured() {
            anyhow::bail!(
                "IMAP not configured. Set [imap] username/password in agents.toml, \
                 or set IMAP_HOST/IMAP_USER/IMAP_PASS env vars, \
                 or pass imap_host/imap_user/imap_pass in args."
            );
        }

        // Validate action
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if !action.is_empty() && !["check", "read", "search"].contains(&action) {
            anyhow::bail!("Invalid action '{}'. Must be one of: check, read, search", action);
        }

        Ok(())
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("check");
        let folder = args.get("folder").and_then(|v| v.as_str()).unwrap_or("INBOX");
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let message_id = args.get("message_id").and_then(|v| v.as_str()).unwrap_or("");
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(10);

        // Validate action
        if !["check", "read", "search"].contains(&action) {
            return Ok(ToolResult {
                success: false,
                output: format!("Invalid action '{}'. Use: check, read, or search", action),
            });
        }

        // Validate action-specific requirements
        if action == "read" && message_id.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Action 'read' requires 'message_id' parameter (email UID)".to_string(),
            });
        }

        if action == "search" && query.is_empty() {
            return Ok(ToolResult {
                success: false,
                output: "Action 'search' requires 'query' parameter".to_string(),
            });
        }

        // Resolve IMAP config (config file < env vars < args)
        let resolved = self.config.resolve(&args);
        if !resolved.is_configured() {
            return Ok(ToolResult {
                success: false,
                output: "IMAP not configured. Set [imap] username/password in agents.toml, or IMAP_USER/IMAP_PASS env vars.".to_string(),
            });
        }

        // Deploy helper script
        let helper_path = match self.deploy_helper() {
            Ok(p) => p,
            Err(e) => return Ok(ToolResult {
                success: false,
                output: format!("Failed to deploy IMAP helper: {}", e),
            }),
        };

        // Build config payload for Python helper
        let config_payload = json!({
            "host": resolved.host,
            "port": resolved.port,
            "username": resolved.username,
            "password": resolved.password,
            "use_ssl": resolved.use_ssl,
            "action": action,
            "folder": folder,
            "query": query,
            "message_id": message_id,
            "limit": limit,
        });

        debug!("email_receive: action={}, folder={}, host={}", action, folder, resolved.host);

        // Execute Python helper (try "python" first, fall back to "python3" for macOS/Linux)
        let child_result = tokio::process::Command::new("python")
            .arg(&helper_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();
        let mut child = match child_result {
            Ok(c) => c,
            Err(_) => tokio::process::Command::new("python3")
                .arg(&helper_path)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .map_err(|e| anyhow::anyhow!("Failed to spawn IMAP helper: {}", e))?,
        };

        // Write config to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let config_str = serde_json::to_string(&config_payload)?;
            stdin.write_all(config_str.as_bytes()).await?;
            drop(stdin);
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            child.wait_with_output(),
        ).await
            .map_err(|_| anyhow::anyhow!("IMAP operation timed out after 30s"))?
            .map_err(|e| anyhow::anyhow!("IMAP helper error: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            warn!("IMAP helper failed: {}", stderr);
            return Ok(ToolResult {
                success: false,
                output: format!("IMAP operation failed: {}", if stderr.is_empty() { &stdout } else { &stderr }),
            });
        }

        // Parse JSON result from helper
        match serde_json::from_str::<Value>(&stdout) {
            Ok(result) => {
                let success = result.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
                if success {
                    Ok(ToolResult {
                        success: true,
                        output: format_result(action, &result),
                    })
                } else {
                    let error = result.get("error").and_then(|v| v.as_str()).unwrap_or("Unknown error");
                    Ok(ToolResult {
                        success: false,
                        output: format!("IMAP error: {}", error),
                    })
                }
            }
            Err(_) => Ok(ToolResult {
                success: false,
                output: format!("Unexpected helper output: {}", stdout),
            }),
        }
    }
}

/// Format the raw JSON result into a human-readable string for the LLM.
fn format_result(action: &str, result: &Value) -> String {
    match action {
        "check" => {
            let mut out = String::from("IMAP Folders:\n");
            if let Some(folders) = result.get("folders").and_then(|v| v.as_array()) {
                for f in folders {
                    let name = f.get("name").and_then(|v| v.as_str()).unwrap_or("?");
                    let unread = f.get("unread").and_then(|v| v.as_i64()).unwrap_or(-1);
                    if unread >= 0 {
                        out.push_str(&format!("  {} ({} unread)\n", name, unread));
                    } else {
                        out.push_str(&format!("  {} (unavailable)\n", name));
                    }
                }
                out.push_str(&format!("\nTotal: {} folders", folders.len()));
            }
            out
        }
        "read" => {
            if let Some(email) = result.get("email") {
                let subject = email.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
                let from = email.get("from").and_then(|v| v.as_str()).unwrap_or("?");
                let to = email.get("to").and_then(|v| v.as_str()).unwrap_or("?");
                let date = email.get("date").and_then(|v| v.as_str()).unwrap_or("?");
                let body = email.get("body").and_then(|v| v.as_str()).unwrap_or("");
                let cc = email.get("cc").and_then(|v| v.as_str()).unwrap_or("");
                let uid = email.get("uid").and_then(|v| v.as_str()).unwrap_or("?");

                let mut out = format!(
                    "Email UID: {}\nFrom: {}\nTo: {}\nDate: {}\nSubject: {}\n",
                    uid, from, to, date, subject
                );

                if !cc.is_empty() {
                    out.push_str(&format!("CC: {}\n", cc));
                }

                // Attachments
                if let Some(attachments) = email.get("attachments").and_then(|v| v.as_array()) {
                    if !attachments.is_empty() {
                        out.push_str(&format!("Attachments: {}\n",
                            attachments.iter()
                                .filter_map(|a| a.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                }

                out.push_str(&format!("\n---\n{}", body));
                out
            } else {
                "Email data missing from response".to_string()
            }
        }
        "search" => {
            let total = result.get("total_matches").and_then(|v| v.as_u64()).unwrap_or(0);
            let showing = result.get("showing").and_then(|v| v.as_u64()).unwrap_or(0);
            let mut out = format!("Search results: {} found, showing {}\n\n", total, showing);

            if let Some(emails) = result.get("emails").and_then(|v| v.as_array()) {
                for (i, e) in emails.iter().enumerate() {
                    let uid = e.get("uid").and_then(|v| v.as_str()).unwrap_or("?");
                    let subject = e.get("subject").and_then(|v| v.as_str()).unwrap_or("(no subject)");
                    let from = e.get("from").and_then(|v| v.as_str()).unwrap_or("?");
                    let date = e.get("date").and_then(|v| v.as_str()).unwrap_or("?");
                    out.push_str(&format!(
                        "{}. [UID {}] {} | {} | {}\n",
                        i + 1, uid, subject, from, date
                    ));
                }
            }

            out
        }
        _ => serde_json::to_string_pretty(result).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name() {
        let tool = EmailReceiveTool::new(ImapConfig::default());
        assert_eq!(tool.name(), "email_receive");
    }

    #[test]
    fn test_schema() {
        let tool = EmailReceiveTool::new(ImapConfig::default());
        let schema = tool.parameters_schema();
        let props = schema.get("properties").unwrap();
        assert!(props.get("action").is_some());
        assert!(props.get("folder").is_some());
        assert!(props.get("query").is_some());
        assert!(props.get("message_id").is_some());
        assert!(props.get("limit").is_some());
        assert_eq!(schema["required"][0], "action");
    }

    #[test]
    fn test_preflight_no_config() {
        let tool = EmailReceiveTool::new(ImapConfig::default());
        let args = json!({"action": "check"});
        // Default config has empty username/password, so preflight should fail
        // (unless IMAP_USER env var is set, which it typically won't be in tests)
        if std::env::var("IMAP_USER").is_err() {
            let result = tool.preflight(&args);
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("IMAP not configured"));
        }
    }

    #[tokio::test]
    async fn test_action_check_no_config() {
        let tool = EmailReceiveTool::new(ImapConfig::default());
        let result = tool.execute(json!({"action": "check"})).await.unwrap();
        // Should fail because IMAP is not configured
        if std::env::var("IMAP_USER").is_err() {
            assert!(!result.success);
            assert!(result.output.contains("IMAP not configured"));
        }
    }

    #[tokio::test]
    async fn test_action_read_missing_message_id() {
        let tool = EmailReceiveTool::new(ImapConfig {
            username: "test@example.com".to_string(),
            password: "testpass".to_string(),
            ..ImapConfig::default()
        });
        let result = tool.execute(json!({"action": "read"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("message_id"));
    }

    #[tokio::test]
    async fn test_action_search_missing_query() {
        let tool = EmailReceiveTool::new(ImapConfig {
            username: "test@example.com".to_string(),
            password: "testpass".to_string(),
            ..ImapConfig::default()
        });
        let result = tool.execute(json!({"action": "search"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("query"));
    }

    #[tokio::test]
    async fn test_invalid_action() {
        let tool = EmailReceiveTool::new(ImapConfig {
            username: "test@example.com".to_string(),
            password: "testpass".to_string(),
            ..ImapConfig::default()
        });
        let result = tool.execute(json!({"action": "delete"})).await.unwrap();
        assert!(!result.success);
        assert!(result.output.contains("Invalid action"));
    }

    #[test]
    fn test_default_folder() {
        let tool = EmailReceiveTool::new(ImapConfig::default());
        let schema = tool.parameters_schema();
        let folder_desc = schema["properties"]["folder"]["description"].as_str().unwrap();
        assert!(folder_desc.contains("INBOX"));
    }

    #[test]
    fn test_config_defaults() {
        let config = ImapConfig::default();
        assert_eq!(config.host, "imap.gmail.com");
        assert_eq!(config.port, 993);
        assert!(config.use_ssl);
        assert!(config.username.is_empty());
        assert!(config.password.is_empty());
    }

    #[test]
    fn test_config_resolve_from_args() {
        let config = ImapConfig::default();
        let args = json!({
            "imap_host": "mail.example.com",
            "imap_port": 995,
            "imap_user": "user@example.com",
            "imap_pass": "secret",
            "use_ssl": false
        });
        let resolved = config.resolve(&args);
        assert_eq!(resolved.host, "mail.example.com");
        assert_eq!(resolved.port, 995);
        assert_eq!(resolved.username, "user@example.com");
        assert_eq!(resolved.password, "secret");
        assert!(!resolved.use_ssl);
    }

    #[test]
    fn test_config_is_configured() {
        assert!(!ImapConfig::default().is_configured());
        assert!(ImapConfig {
            username: "user@example.com".to_string(),
            password: "pass".to_string(),
            ..ImapConfig::default()
        }.is_configured());
    }

    #[test]
    fn test_description() {
        let tool = EmailReceiveTool::new(ImapConfig::default());
        assert!(tool.description().contains("IMAP"));
        assert!(tool.description().contains("check"));
        assert!(tool.description().contains("read"));
        assert!(tool.description().contains("search"));
    }

    #[test]
    fn test_format_result_check() {
        let result = json!({
            "success": true,
            "folders": [
                {"name": "INBOX", "unread": 5},
                {"name": "Sent", "unread": 0},
                {"name": "Drafts", "unread": -1}
            ]
        });
        let formatted = format_result("check", &result);
        assert!(formatted.contains("INBOX"));
        assert!(formatted.contains("5 unread"));
        assert!(formatted.contains("0 unread"));
        assert!(formatted.contains("unavailable"));
        assert!(formatted.contains("3 folders"));
    }

    #[test]
    fn test_format_result_read() {
        let result = json!({
            "success": true,
            "email": {
                "uid": "123",
                "subject": "Test Email",
                "from": "sender@example.com",
                "to": "receiver@example.com",
                "cc": "",
                "date": "Mon, 17 Mar 2026 10:00:00 +0800",
                "body": "Hello, this is a test.",
                "attachments": ["file.pdf"]
            }
        });
        let formatted = format_result("read", &result);
        assert!(formatted.contains("Test Email"));
        assert!(formatted.contains("sender@example.com"));
        assert!(formatted.contains("Hello, this is a test."));
        assert!(formatted.contains("file.pdf"));
    }

    #[test]
    fn test_format_result_search() {
        let result = json!({
            "success": true,
            "total_matches": 25,
            "showing": 2,
            "emails": [
                {"uid": "100", "subject": "Invoice #1", "from": "billing@example.com", "date": "Mon, 17 Mar 2026"},
                {"uid": "99", "subject": "Invoice #2", "from": "billing@example.com", "date": "Sun, 16 Mar 2026"}
            ]
        });
        let formatted = format_result("search", &result);
        assert!(formatted.contains("25 found"));
        assert!(formatted.contains("showing 2"));
        assert!(formatted.contains("Invoice #1"));
        assert!(formatted.contains("UID 100"));
    }

    #[test]
    fn test_preflight_invalid_action() {
        let tool = EmailReceiveTool::new(ImapConfig {
            username: "test@example.com".to_string(),
            password: "testpass".to_string(),
            ..ImapConfig::default()
        });
        let args = json!({"action": "delete"});
        // Preflight checks Python availability first, then config, then action.
        // If Python is available and config is set, it should fail on invalid action.
        let result = tool.preflight(&args);
        // This may pass or fail depending on Python availability,
        // but if Python is available and config is configured, it should fail on invalid action
        if let Err(e) = result {
            let msg = e.to_string();
            assert!(msg.contains("Invalid action") || msg.contains("Python"));
        }
    }
}
