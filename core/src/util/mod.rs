// core/src/util/mod.rs
//
// Cross-module utilities. Small, dependency-free helpers that multiple
// modules need but don't deserve their own top-level home.
//
// PF-2d (this commit): consolidates `colored()` / `is_colored()` ANSI
// helpers that PF-2a / PF-2b had as private duplicates in
// `service/{windows,linux}.rs` and `bin/phantom.rs`.

pub mod term;
