//! Multi-channel notification bridge (P15).
//!
//! `NotificationDispatcher` routes `Notification` records to one or more
//! `NotificationChannel` impls (OS, Telegram, …). Priority drives delivery:
//!
//! * P0 — immediate, parallel fire-and-forget across every channel.
//! * P1 — buffered; flushed as a single summary every 30 minutes.
//! * P2 — `tracing::debug!` only.
//!
//! A dedupe cache suppresses repeated notifications sharing the same
//! `dedup_key` within a 5-minute window (e.g. if a task transition fires twice).
//! Per-channel consecutive-failure counters surface a warning after 3 strikes.

pub mod channels;
pub mod dispatcher;

pub use dispatcher::NotificationDispatcher;
pub use pm_types::{classify_priority, Notification, NotificationAction, NotificationPriority};
