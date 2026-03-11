//! Evaluation-time helpers for the canonical pipeline DSL.
//!
//! This module exists to keep selector resolution, flattening, and matching
//! rules separate from parsing and verb orchestration.
//!
//! Contract:
//!
//! - parsing belongs in `dsl::parse`
//! - high-level verb flow belongs in `dsl::engine` and `dsl::verbs`
//! - evaluator helpers should stay focused on data selection semantics

pub mod context;
pub mod flatten;
pub mod matchers;
pub mod resolve;
