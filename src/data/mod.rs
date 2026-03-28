//! Data layer — schema migrations and cross-node sync index.

pub mod migrations;
pub mod sync;
pub mod cross_node;
pub use cross_node::*;
