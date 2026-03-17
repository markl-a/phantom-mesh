use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Mutex;
use super::{Tool, ToolResult};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Notification {
    id: String,
    title: String,
    message: String,
    priority: String,
    created_at: String,
    read: bool,
}

pub struct NotificationCenterTool {
    notifications: Mutex<Vec<Notification>>,
}

impl NotificationCenterTool {
    pub fn new() -> Self {
        Self { notifications: Mutex::new(Vec::new()) }
    }
}

#[async_trait]
impl Tool for NotificationCenterTool {
    fn name(&self) -> &str { "notification_center" }
    fn description(&self) -> &str { "Send, list, and manage notifications for agents and users" }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "operation": {"type": "string", "enum": ["send", "list", "read", "clear", "count"]},
                "title": {"type": "string"},
                "message": {"type": "string"},
                "priority": {"type": "string", "enum": ["low", "normal", "high", "urgent"]},
                "id": {"type": "string"},
                "unread_only": {"type": "boolean"}
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: Value) -> Result<ToolResult> {
        let operation = args["operation"].as_str().unwrap_or("list");
        match operation {
            "send" => {
                let title = args["title"].as_str().unwrap_or("Untitled").to_string();
                let message = args["message"].as_str().unwrap_or("").to_string();
                let priority = args["priority"].as_str().unwrap_or("normal").to_string();
                let id = format!("notif-{}", chrono::Utc::now().timestamp_millis());
                let notification = Notification {
                    id: id.clone(), title: title.clone(), message,
                    priority: priority.clone(), created_at: chrono::Utc::now().to_rfc3339(), read: false,
                };
                let mut notifs = self.notifications.lock().unwrap();
                notifs.push(notification);
                Ok(ToolResult {
                    output: json!({"id": id, "status": "sent", "title": title, "priority": priority, "total_notifications": notifs.len()}).to_string(),
                    success: true,
                })
            }
            "list" => {
                let unread_only = args["unread_only"].as_bool().unwrap_or(false);
                let notifs = self.notifications.lock().unwrap();
                let filtered: Vec<&Notification> = if unread_only {
                    notifs.iter().filter(|n| !n.read).collect()
                } else {
                    notifs.iter().collect()
                };
                Ok(ToolResult {
                    output: json!({"notifications": filtered, "count": filtered.len(), "filter": if unread_only {"unread"} else {"all"}}).to_string(),
                    success: true,
                })
            }
            "read" => {
                let id = args["id"].as_str().unwrap_or("");
                if id.is_empty() {
                    return Ok(ToolResult { output: "Missing notification ID".to_string(), success: false });
                }
                let mut notifs = self.notifications.lock().unwrap();
                if let Some(notif) = notifs.iter_mut().find(|n| n.id == id) {
                    notif.read = true;
                    Ok(ToolResult {
                        output: json!({"id": notif.id, "title": notif.title, "message": notif.message, "priority": notif.priority, "created_at": notif.created_at, "read": true}).to_string(),
                        success: true,
                    })
                } else {
                    Ok(ToolResult { output: format!("Notification not found: {}", id), success: false })
                }
            }
            "clear" => {
                let mut notifs = self.notifications.lock().unwrap();
                let count = notifs.len();
                notifs.clear();
                Ok(ToolResult { output: json!({"status": "cleared", "removed": count}).to_string(), success: true })
            }
            "count" => {
                let unread_only = args["unread_only"].as_bool().unwrap_or(false);
                let notifs = self.notifications.lock().unwrap();
                let count = if unread_only { notifs.iter().filter(|n| !n.read).count() } else { notifs.len() };
                Ok(ToolResult { output: json!({"count": count, "filter": if unread_only {"unread"} else {"all"}}).to_string(), success: true })
            }
            _ => Ok(ToolResult { output: format!("Unknown operation: {}", operation), success: false }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_send_notification() {
        let t = NotificationCenterTool::new();
        let r = t.execute(json!({"operation": "send", "title": "Alert", "message": "test", "priority": "high"})).await.unwrap();
        assert!(r.success);
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["status"], "sent");
        assert_eq!(p["priority"], "high");
    }

    #[tokio::test]
    async fn test_list_notifications() {
        let t = NotificationCenterTool::new();
        t.execute(json!({"operation": "send", "title": "A", "message": "m"})).await.unwrap();
        t.execute(json!({"operation": "send", "title": "B", "message": "m"})).await.unwrap();
        let r = t.execute(json!({"operation": "list"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["count"], 2);
    }

    #[tokio::test]
    async fn test_list_unread_only() {
        let t = NotificationCenterTool::new();
        t.execute(json!({"operation": "send", "title": "A", "message": "m"})).await.unwrap();
        t.execute(json!({"operation": "send", "title": "B", "message": "m"})).await.unwrap();
        let id = { let n = t.notifications.lock().unwrap(); n[0].id.clone() };
        t.execute(json!({"operation": "read", "id": id})).await.unwrap();
        let r = t.execute(json!({"operation": "list", "unread_only": true})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["count"], 1);
    }

    #[tokio::test]
    async fn test_read_notification() {
        let t = NotificationCenterTool::new();
        let s = t.execute(json!({"operation": "send", "title": "Important", "message": "Read me"})).await.unwrap();
        let p: Value = serde_json::from_str(&s.output).unwrap();
        let id = p["id"].as_str().unwrap();
        let r = t.execute(json!({"operation": "read", "id": id})).await.unwrap();
        let p2: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p2["read"], true);
    }

    #[tokio::test]
    async fn test_read_nonexistent() {
        let t = NotificationCenterTool::new();
        let r = t.execute(json!({"operation": "read", "id": "fake"})).await.unwrap();
        assert!(!r.success);
    }

    #[tokio::test]
    async fn test_clear_notifications() {
        let t = NotificationCenterTool::new();
        t.execute(json!({"operation": "send", "title": "A", "message": "m"})).await.unwrap();
        let r = t.execute(json!({"operation": "clear"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["removed"], 1);
    }

    #[tokio::test]
    async fn test_count() {
        let t = NotificationCenterTool::new();
        t.execute(json!({"operation": "send", "title": "A", "message": "m"})).await.unwrap();
        let r = t.execute(json!({"operation": "count"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["count"], 1);
    }

    #[tokio::test]
    async fn test_count_unread() {
        let t = NotificationCenterTool::new();
        t.execute(json!({"operation": "send", "title": "A", "message": "m"})).await.unwrap();
        t.execute(json!({"operation": "send", "title": "B", "message": "m"})).await.unwrap();
        let id = { let n = t.notifications.lock().unwrap(); n[0].id.clone() };
        t.execute(json!({"operation": "read", "id": id})).await.unwrap();
        let r = t.execute(json!({"operation": "count", "unread_only": true})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["count"], 1);
    }

    #[tokio::test]
    async fn test_default_priority() {
        let t = NotificationCenterTool::new();
        let r = t.execute(json!({"operation": "send", "title": "T", "message": "m"})).await.unwrap();
        let p: Value = serde_json::from_str(&r.output).unwrap();
        assert_eq!(p["priority"], "normal");
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let t = NotificationCenterTool::new();
        let r = t.execute(json!({"operation": "delete"})).await.unwrap();
        assert!(!r.success);
    }

    #[tokio::test]
    async fn test_read_missing_id() {
        let t = NotificationCenterTool::new();
        let r = t.execute(json!({"operation": "read"})).await.unwrap();
        assert!(!r.success);
    }

    #[test]
    fn test_name_and_schema() {
        let t = NotificationCenterTool::new();
        assert_eq!(t.name(), "notification_center");
        assert!(!t.description().is_empty());
        let schema = t.parameters_schema();
        assert!(schema["properties"]["operation"].is_object());
    }
}
