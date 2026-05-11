#![cfg(not(any(target_os = "android", target_os = "ios")))]

use async_trait::async_trait;
use pm_types::Notification;

use super::NotificationChannel;

/// OS-native desktop notification channel. Wraps the cross-platform `notify-rust`
/// crate: macOS → NSUserNotificationCenter, Linux → D-Bus, Windows → WinRT Toast.
/// Not available on Android (no dbus / NSUserNotificationCenter).
pub struct OsChannel;

#[async_trait]
impl NotificationChannel for OsChannel {
    fn name(&self) -> &str {
        "os"
    }

    async fn send(&self, n: &Notification) -> anyhow::Result<()> {
        let title = n.title.clone();
        let body = n.body.clone();
        // notify-rust's show() is synchronous; keep the async loop unblocked.
        tokio::task::spawn_blocking(move || {
            notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .timeout(notify_rust::Timeout::Milliseconds(10_000))
                .show()
        })
        .await?
        .map_err(|e| anyhow::anyhow!("notify-rust error: {}", e))?;
        Ok(())
    }

    async fn send_batch(&self, ns: &[Notification]) -> anyhow::Result<()> {
        if ns.is_empty() {
            return Ok(());
        }
        // OS notifications have no rich "batch" primitive — send a single summary
        // so the user isn't spammed with N popups.
        let title = format!("{} 個任務更新", ns.len());
        let lines: Vec<String> = ns
            .iter()
            .take(5)
            .map(|n| format!("• {}", n.title))
            .collect();
        let body = lines.join("\n");
        let summary = Notification {
            id: ns[0].id,
            dedup_key: format!("batch:{}", ns.len()),
            task_id: None,
            workspace_id: ns[0].workspace_id.clone(),
            priority: ns[0].priority,
            title,
            body,
            actions: vec![],
            timestamp: ns[0].timestamp,
        };
        self.send(&summary).await
    }
}
