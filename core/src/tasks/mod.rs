//! Long-running task runtime (P5).
//!
//! Tracks TaskRecord state across the agent loop with SQLite persistence.
//! Replaces the in-memory JobStore. Designed to host checkpoint/resume and
//! OpenFang-style guards (see `docs/phase1-specs/P5-task-state-machine.md`).

pub mod session;
pub mod state;
pub mod store;

pub use pm_types::{SessionEntry, TaskRecord, TaskStatus};
pub use session::{load_and_repair, repair, session_path, RepairedSession, SessionWriter};
pub use state::TaskQueue;
pub use store::TaskStore;
