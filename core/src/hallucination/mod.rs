//! T22 — Anti-hallucination V1 scanner.
//!
//! Deterministic regex-based scanner that flags assistant replies which
//! assert completion of a side-effecting action ("created file at X",
//! "✅ 完成") when the round's tool-call set is empty. The scanner is a
//! pure function: pass it the reply text + the round's tool_calls +
//! the round's tool_results, get back a list of unbacked claims.
//!
//! V1 implements ONLY Shape 1 from
//! `docs/anti-hallucination-v1-design.md` — bare claim of file/script
//! creation with zero tool calls in the round. Shapes 2-6 are deferred.
//!
//! Entire module is gated behind `experimental-anti-hallucination`.
//! Default `cargo build` does not compile any of this.

mod scanner;

pub use scanner::{scan, ClaimRule, UnbackedClaim, CLAIM_SIGNATURES};
