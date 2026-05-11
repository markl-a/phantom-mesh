use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pm_types::{Notification, NotificationPriority};
use tokio::sync::Mutex;

use super::channels::NotificationChannel;

const DEDUPE_TTL: Duration = Duration::from_secs(5 * 60);
const DEDUPE_CAP: usize = 1_000;
const FAIL_STRIKE_LIMIT: u32 = 3;

struct Inner {
    channels: Vec<Arc<dyn NotificationChannel>>,
    p1_buffer: Vec<Notification>,
    dedupe_recent: VecDeque<(String, Instant)>,
    consecutive_failures: HashMap<String, u32>,
}

impl Inner {
    fn new() -> Self {
        Self {
            channels: vec![],
            p1_buffer: vec![],
            dedupe_recent: VecDeque::new(),
            consecutive_failures: HashMap::new(),
        }
    }

    fn is_duplicate(&mut self, key: &str) -> bool {
        self.prune_dedupe();
        self.dedupe_recent.iter().any(|(k, _)| k == key)
    }

    fn remember(&mut self, key: String) {
        self.dedupe_recent.push_back((key, Instant::now()));
        while self.dedupe_recent.len() > DEDUPE_CAP {
            self.dedupe_recent.pop_front();
        }
    }

    fn prune_dedupe(&mut self) {
        let cutoff = Instant::now() - DEDUPE_TTL;
        while let Some((_, ts)) = self.dedupe_recent.front() {
            if *ts < cutoff {
                self.dedupe_recent.pop_front();
            } else {
                break;
            }
        }
    }
}

/// Facade clone-able across handlers. All real state lives behind an async Mutex.
#[derive(Clone)]
pub struct NotificationDispatcher {
    inner: Arc<Mutex<Inner>>,
}

impl NotificationDispatcher {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(Inner::new())),
        }
    }

    pub async fn add_channel(&self, ch: Arc<dyn NotificationChannel>) {
        let mut inner = self.inner.lock().await;
        tracing::info!("notification channel registered: {}", ch.name());
        inner.channels.push(ch);
    }

    pub async fn channel_names(&self) -> Vec<String> {
        let inner = self.inner.lock().await;
        inner.channels.iter().map(|c| c.name().to_string()).collect()
    }

    /// Dispatch a notification. P0 goes out immediately via every channel
    /// (parallel, fire-and-forget). P1 is queued for the next batch flush.
    /// P2 is logged but not sent.
    pub async fn notify(&self, n: Notification) {
        let (channels, send_immediately) = {
            let mut inner = self.inner.lock().await;
            if inner.is_duplicate(&n.dedup_key) {
                tracing::debug!(dedup_key = %n.dedup_key, "suppressed duplicate notification");
                return;
            }
            inner.remember(n.dedup_key.clone());

            match n.priority {
                NotificationPriority::P0 => (inner.channels.clone(), true),
                NotificationPriority::P1 => {
                    inner.p1_buffer.push(n);
                    return;
                }
                NotificationPriority::P2 => {
                    tracing::debug!(?n.priority, title = %n.title, "P2 notification (silent)");
                    return;
                }
            }
        };

        if send_immediately {
            for ch in channels {
                let disp = self.clone();
                let notif = n.clone();
                tokio::spawn(async move {
                    disp.send_and_track(&ch, &notif).await;
                });
            }
        }
    }

    /// Drain the P1 buffer into a single summary per channel. Returns the
    /// number of notifications that were in the buffer (0 if nothing flushed).
    pub async fn flush_p1(&self) -> usize {
        let (channels, batch) = {
            let mut inner = self.inner.lock().await;
            if inner.p1_buffer.is_empty() {
                return 0;
            }
            let batch = std::mem::take(&mut inner.p1_buffer);
            (inner.channels.clone(), batch)
        };

        let count = batch.len();
        for ch in channels {
            let disp = self.clone();
            let batch = batch.clone();
            tokio::spawn(async move {
                disp.send_batch_and_track(&ch, &batch).await;
            });
        }
        count
    }

    /// Spawn the 30-minute flush loop. Returns a handle the caller can ignore
    /// (or hold to cancel on shutdown).
    pub fn spawn_flush_loop(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(30 * 60));
            ticker.tick().await; // skip the first immediate tick
            loop {
                ticker.tick().await;
                let flushed = self.flush_p1().await;
                if flushed > 0 {
                    tracing::info!(count = flushed, "flushed P1 notification batch");
                }
            }
        })
    }

    async fn send_and_track(&self, ch: &Arc<dyn NotificationChannel>, n: &Notification) {
        match ch.send(n).await {
            Ok(()) => {
                let mut inner = self.inner.lock().await;
                inner.consecutive_failures.remove(ch.name());
            }
            Err(e) => self.record_failure(ch.name(), &e).await,
        }
    }

    async fn send_batch_and_track(
        &self,
        ch: &Arc<dyn NotificationChannel>,
        batch: &[Notification],
    ) {
        match ch.send_batch(batch).await {
            Ok(()) => {
                let mut inner = self.inner.lock().await;
                inner.consecutive_failures.remove(ch.name());
            }
            Err(e) => self.record_failure(ch.name(), &e).await,
        }
    }

    async fn record_failure(&self, ch_name: &str, err: &anyhow::Error) {
        let mut inner = self.inner.lock().await;
        let counter = inner.consecutive_failures.entry(ch_name.to_string()).or_insert(0);
        *counter += 1;
        let count = *counter;
        drop(inner);

        if count >= FAIL_STRIKE_LIMIT {
            tracing::warn!(
                channel = ch_name,
                consecutive_failures = count,
                "notification channel degraded: {}",
                err
            );
        } else {
            tracing::debug!(channel = ch_name, "notification send failed: {}", err);
        }
    }
}

impl Default for NotificationDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    struct CountingChannel {
        name: &'static str,
        sent: Arc<AtomicUsize>,
        batches: Arc<AtomicUsize>,
        fail: bool,
    }

    #[async_trait]
    impl NotificationChannel for CountingChannel {
        fn name(&self) -> &str { self.name }
        async fn send(&self, _n: &Notification) -> anyhow::Result<()> {
            if self.fail {
                return Err(anyhow::anyhow!("simulated failure"));
            }
            self.sent.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn send_batch(&self, ns: &[Notification]) -> anyhow::Result<()> {
            if self.fail {
                return Err(anyhow::anyhow!("simulated failure"));
            }
            self.batches.fetch_add(1, Ordering::SeqCst);
            self.sent.fetch_add(ns.len(), Ordering::SeqCst);
            Ok(())
        }
    }

    fn mk_notification(kind: &str, priority: NotificationPriority) -> Notification {
        Notification {
            id: Uuid::new_v4(),
            dedup_key: format!("test:{}:{}", kind, Uuid::new_v4()),
            task_id: None,
            workspace_id: "ws1".into(),
            priority,
            title: format!("t-{}", kind),
            body: "b".into(),
            actions: vec![],
            timestamp: 0,
        }
    }

    fn counting(name: &'static str) -> (Arc<CountingChannel>, Arc<AtomicUsize>, Arc<AtomicUsize>) {
        let sent = Arc::new(AtomicUsize::new(0));
        let batches = Arc::new(AtomicUsize::new(0));
        let ch = Arc::new(CountingChannel {
            name,
            sent: sent.clone(),
            batches: batches.clone(),
            fail: false,
        });
        (ch, sent, batches)
    }

    #[tokio::test]
    async fn p0_dispatches_to_every_channel() {
        let disp = NotificationDispatcher::new();
        let (ch1, s1, _) = counting("a");
        let (ch2, s2, _) = counting("b");
        disp.add_channel(ch1).await;
        disp.add_channel(ch2).await;

        disp.notify(mk_notification("k1", NotificationPriority::P0)).await;
        // wait for spawned sends
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(s1.load(Ordering::SeqCst), 1);
        assert_eq!(s2.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn p1_buffers_until_flush() {
        let disp = NotificationDispatcher::new();
        let (ch, sent, batches) = counting("a");
        disp.add_channel(ch).await;

        for _ in 0..3 {
            disp.notify(mk_notification("k", NotificationPriority::P1)).await;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        // nothing sent yet
        assert_eq!(sent.load(Ordering::SeqCst), 0);
        assert_eq!(batches.load(Ordering::SeqCst), 0);

        let flushed = disp.flush_p1().await;
        assert_eq!(flushed, 3);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(batches.load(Ordering::SeqCst), 1);
        assert_eq!(sent.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn p2_is_silent() {
        let disp = NotificationDispatcher::new();
        let (ch, sent, _) = counting("a");
        disp.add_channel(ch).await;
        disp.notify(mk_notification("k", NotificationPriority::P2)).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(sent.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn dedupe_suppresses_same_key_twice() {
        let disp = NotificationDispatcher::new();
        let (ch, sent, _) = counting("a");
        disp.add_channel(ch).await;

        let mut n = mk_notification("k", NotificationPriority::P0);
        n.dedup_key = "task:abc:failed".into();
        disp.notify(n.clone()).await;
        disp.notify(n).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(sent.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failures_are_per_channel_and_do_not_panic() {
        let disp = NotificationDispatcher::new();
        let (good, good_sent, _) = counting("good");
        let failing = Arc::new(CountingChannel {
            name: "bad",
            sent: Arc::new(AtomicUsize::new(0)),
            batches: Arc::new(AtomicUsize::new(0)),
            fail: true,
        });
        disp.add_channel(good).await;
        disp.add_channel(failing).await;

        disp.notify(mk_notification("k", NotificationPriority::P0)).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        // good channel got the message even though bad failed
        assert_eq!(good_sent.load(Ordering::SeqCst), 1);
    }
}
