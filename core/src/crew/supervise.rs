//! The conductor-facing supervision hooks: a `RunObserver` sink (a live mirror
//! of each blackboard post, so a governed run is watchable) and `ControlState`
//! (the shared abort / steer signals the conductor reads at round boundaries).
//! Ported from the `ensemble` project (Apache-2.0) §6 crew port — ONLY the
//! conductor-facing subset is carried. ensemble's `FeedObserver` / `ControlCmd`
//! / `drain_control` + the `watch` CLI live behind its own `ndjson` feed + serve
//! layer, which spectyn does not need (it has its own serve + the governed-run
//! flight recorder for live observability).

use super::blackboard::Message;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// A live sink the conductor mirrors each blackboard post into, so a governed run is watchable in
/// real time. Best-effort by contract: an implementation must never let a write failure surface —
/// it cannot be allowed to change a run's outcome (mirrors the journal's discipline).
pub trait RunObserver: Send + Sync {
    fn post(&self, m: &Message);
}

impl RunObserver for Vec<Box<dyn RunObserver>> {
    fn post(&self, m: &Message) {
        for observer in self {
            observer.post(m);
        }
    }
}

/// Shared signals a control watcher feeds and the conductor reads at round boundaries: an abort
/// request (`hard` ⇒ kill the running CLI now), plus a queue of steer prompts to inject into the
/// next round. The abort flag is the SAME `Arc<AtomicBool>` the conductor checks, so a control-feed
/// abort and a Ctrl-C converge.
#[derive(Default)]
pub struct ControlState {
    abort: Arc<AtomicBool>,
    hard: Arc<AtomicBool>,
    steers: Mutex<Vec<String>>,
}

impl ControlState {
    /// The abort flag to share with the conductor (so a control abort == the conductor's own abort).
    pub fn abort_flag(&self) -> Arc<AtomicBool> {
        self.abort.clone()
    }
    /// The HARD-abort flag to hand to a running adapter: when set (an `abort --hard`), the adapter
    /// kills its child mid-turn instead of waiting for the round boundary. A clean abort never sets it.
    pub fn hard_flag(&self) -> Arc<AtomicBool> {
        self.hard.clone()
    }
    pub fn aborted(&self) -> bool {
        self.abort.load(Ordering::Relaxed)
    }
    pub fn hard(&self) -> bool {
        self.hard.load(Ordering::Relaxed)
    }
    /// Drain and return the queued steer prompts (each consumed once, injected into the next round).
    pub fn take_steers(&self) -> Vec<String> {
        std::mem::take(&mut self.steers.lock().unwrap())
    }
    /// Queue a steer prompt (the conductor injects it into the next round's implementer prompt).
    pub fn push_steer(&self, prompt: &str) {
        self.steers.lock().unwrap().push(prompt.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_observer_vec_fans_out_to_every_sink() {
        use std::sync::{Arc, Mutex};

        struct Rec(Arc<Mutex<Vec<String>>>);
        impl RunObserver for Rec {
            fn post(&self, m: &Message) {
                self.0.lock().unwrap().push(m.kind.clone());
            }
        }

        let one = Arc::new(Mutex::new(Vec::new()));
        let two = Arc::new(Mutex::new(Vec::new()));
        let observers: Vec<Box<dyn RunObserver>> =
            vec![Box::new(Rec(one.clone())), Box::new(Rec(two.clone()))];

        observers.post(&Message {
            from: "conductor".to_string(),
            kind: "decision".to_string(),
            body: "LANDED".to_string(),
        });

        assert_eq!(one.lock().unwrap().as_slice(), ["decision"]);
        assert_eq!(two.lock().unwrap().as_slice(), ["decision"]);
    }

    #[test]
    fn control_state_queues_steers_and_shares_abort_flag() {
        let st = ControlState::default();
        assert!(!st.aborted());

        // Steers queue in order and drain exactly once.
        st.push_steer("skip the UI");
        st.push_steer("focus on the parser");
        assert_eq!(
            st.take_steers(),
            vec!["skip the UI".to_string(), "focus on the parser".to_string()]
        );
        assert!(st.take_steers().is_empty(), "steers drain once");

        // The handed-out abort flag is the SAME Arc the conductor checks — setting it through the
        // shared handle is visible via aborted() (a control abort and a Ctrl-C converge).
        let flag = st.abort_flag();
        flag.store(true, Ordering::Relaxed);
        assert!(st.aborted(), "abort flag is the same Arc the conductor checks");

        // The hard flag is independent of the clean-abort flag.
        assert!(!st.hard());
        st.hard_flag().store(true, Ordering::Relaxed);
        assert!(st.hard());
    }
}
