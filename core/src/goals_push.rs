//! Goals-push context formatting.
//!
//! Builds a compact, human-readable snapshot of the user's active goals so it
//! can be "pushed" into an agent's prompt context. The active goals are read
//! from the shared [`GoalsStore`], which keeps them behind a mutex so the
//! snapshot stays consistent while it is being rendered.
//!
//! The single entry point, [`goals_context`], returns an empty string when
//! there are no active goals (so callers can append the result unconditionally
//! without injecting noise), and otherwise an `"Active goals:"` header followed
//! by one bullet line per goal.

use crate::GoalsStore;

/// Render the active goals from `store` as a context string for prompt
/// injection.
///
/// Returns an empty `String` when there are no active goals. Otherwise the
/// result begins with an `"Active goals:"` header line, followed by one
/// `- {goal}` bullet line per goal in store order.
///
/// # Errors
///
/// Returns an error if the goals snapshot cannot be produced.
pub fn goals_context(store: &GoalsStore) -> anyhow::Result<String> {
    let goals = store.inner.lock().unwrap();
    if goals.is_empty() {
        return Ok(String::new());
    }
    let mut ctx = String::from("Active goals:\n");
    for g in goals.iter() {
        ctx.push_str(&format!("- {}\n", g));
    }
    Ok(ctx)
}
