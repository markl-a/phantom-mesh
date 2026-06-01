//! Coach Node prompt templates + lint (E003).
//!
//! Big Goal Operational Principle #1: Coach tone is never judgmental.
//! Every coach-facing text path runs through `lint::check` to catch
//! shame patterns before they reach the user.

pub mod lint;
pub mod templates;
