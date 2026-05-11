use async_trait::async_trait;
use pm_types::Notification;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub mod os;
pub mod telegram;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub use os::OsChannel;
pub use telegram::TelegramChannel;

/// Channel-independent delivery trait. Implementors must be Send + Sync so the
/// dispatcher can fan out concurrently via `tokio::spawn`.
#[async_trait]
pub trait NotificationChannel: Send + Sync {
    fn name(&self) -> &str;

    async fn send(&self, n: &Notification) -> anyhow::Result<()>;

    /// Delivery of a batched summary of notifications, used for P1 windows.
    /// Default implementation sends them one-by-one.
    async fn send_batch(&self, ns: &[Notification]) -> anyhow::Result<()> {
        for n in ns {
            self.send(n).await?;
        }
        Ok(())
    }
}
