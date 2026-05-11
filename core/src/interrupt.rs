//! Cooperative interrupt mechanism for in-flight agent turns.
//!
//! Modeled on Hermes Agent's `_interrupt_requested` + `_interrupt_message`
//! pair (references/hermes-agent/run_agent.py:1178, 4416, 6460): callers
//! flip a flag, the agent loop polls it at safe points (before each
//! round, on every streaming chunk) and unwinds, optionally returning a
//! follow-up user message that the caller can feed straight into the
//! next turn — so a second Enter while the model is still talking
//! doesn't lose the new prompt.
//!
//! Rust port uses tokio's `CancellationToken` for the wakeup primitive,
//! which is cheaper than Hermes' 300 ms thread poll: streaming sites can
//! `tokio::select!` on `cancelled()` and react the moment the flag flips
//! instead of waiting for the next chunk.

use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug, Default)]
pub struct InterruptHandle {
    token: CancellationToken,
    message: Arc<Mutex<Option<String>>>,
}

impl InterruptHandle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel the in-flight turn. `message`, if supplied, is stashed so
    /// the orchestrator can retrieve it via [`take_message`] and feed it
    /// in as the next user turn — this is what lets a second Enter
    /// "redirect" the agent without losing the new prompt.
    pub fn interrupt(&self, message: Option<String>) {
        if let Some(m) = message {
            if let Ok(mut slot) = self.message.lock() {
                *slot = Some(m);
            }
        }
        self.token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Borrow-tied future that resolves when [`interrupt`] is called.
    /// Use inside `tokio::select!` to race against streaming reads.
    pub fn cancelled(&self) -> tokio_util::sync::WaitForCancellationFuture<'_> {
        self.token.cancelled()
    }

    /// Pop the queued follow-up message (if any). The orchestrator
    /// calls this *once* after a turn unwinds via interrupt; any later
    /// call returns `None`.
    pub fn take_message(&self) -> Option<String> {
        self.message.lock().ok()?.take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_handle_is_not_cancelled() {
        let h = InterruptHandle::new();
        assert!(!h.is_cancelled());
        assert!(h.take_message().is_none());
    }

    #[test]
    fn interrupt_with_message_flips_flag_and_queues() {
        let h = InterruptHandle::new();
        h.interrupt(Some("redirect to do X instead".into()));
        assert!(h.is_cancelled());
        assert_eq!(h.take_message().as_deref(), Some("redirect to do X instead"));
        // Message is one-shot.
        assert!(h.take_message().is_none());
    }

    #[test]
    fn interrupt_without_message_still_cancels() {
        let h = InterruptHandle::new();
        h.interrupt(None);
        assert!(h.is_cancelled());
        assert!(h.take_message().is_none());
    }

    #[test]
    fn clones_share_state() {
        let h1 = InterruptHandle::new();
        let h2 = h1.clone();
        h2.interrupt(Some("hi".into()));
        assert!(h1.is_cancelled());
        assert_eq!(h1.take_message().as_deref(), Some("hi"));
    }

    #[tokio::test]
    async fn cancelled_future_resolves_after_interrupt() {
        let h = InterruptHandle::new();
        let h2 = h.clone();
        // Fire interrupt from a background task and race the cancellation
        // future against a 1 s deadline. Without working wakeup the
        // sleep wins and the test fails — exactly the failure mode we
        // want to guard against if someone swaps the impl.
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            h2.interrupt(None);
        });
        let cancelled = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            h.cancelled(),
        )
        .await;
        assert!(cancelled.is_ok(), "cancelled() did not resolve after interrupt()");
    }
}

