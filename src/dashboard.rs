//! Legacy dashboard re-export module.
//!
//! All dashboard functionality has been consolidated into [`crate::web_dashboard`].
//! This module re-exports the v1 server-rendered Kanban dashboard for backward
//! compatibility with existing call sites (e.g. `phantom_mesh::dashboard::render(...)`).

pub use crate::web_dashboard::{render, format_uptime, html_escape};
