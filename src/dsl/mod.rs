//! OSP's row-oriented DSL.
//!
//! The crate is intentionally split into a small number of phases:
//! - `parse`: turn pipeline text into explicit stage intent
//! - `eval`: apply that intent to rows or groups
//! - `stages`: keep per-verb behavior in focused modules
//!
//! The goal is not to be a generic query language. It is a pragmatic pipeline
//! for interactive CLI inspection, so we optimize for readable stage behavior
//! and stable user-facing semantics over clever parsing tricks.

pub mod eval;
pub mod model;
pub mod parse;
pub mod stages;
pub mod verbs;

pub use eval::engine::{
    apply_output_pipeline, apply_pipeline, execute_pipeline, execute_pipeline_streaming,
};
pub use parse::pipeline::{Pipeline, parse_pipeline};
pub use verbs::{
    VerbInfo, VerbStreaming, is_registered_explicit_verb, registered_verbs, render_streaming_badge,
    verb_info,
};

#[cfg(test)]
mod contract_tests;
