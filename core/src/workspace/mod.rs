//! Workspace / context scoping (P9).
//!
//! A workspace is derived from the canonicalised current working directory. Its id
//! (`fnv1a_64(cwd)`) is used to scope every piece of phantom-mesh state: tasks,
//! memories, sessions, config, policies. Workspaces are created implicitly the
//! first time the daemon encounters a new cwd.

pub mod registry;
pub mod resolver;

pub use pm_types::{Workspace, WorkspaceId};
pub use registry::WorkspaceRegistry;
pub use resolver::WorkspaceResolver;
