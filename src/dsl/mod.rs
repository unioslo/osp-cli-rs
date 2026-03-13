//! Canonical document-first pipeline DSL.
//!
//! A pipeline is a command followed by zero or more transformation stages
//! separated by `|`. The stages transform the rows returned by the command:
//!
//! ```text
//! "orch task list | F status=running | S created | L 10"
//!  ─────────────────  ─────────────────────────────────
//!     command              pipeline stages
//! ```
//!
//! Data flow:
//!
//! ```text
//! raw line
//!   │
//!   ▼  parse_pipeline(line)
//!   Pipeline { command: "orch task list",
//!              stages:  ["F status=running", "S created", "L 10"] }
//!   │
//!   │  caller dispatches the command and gets rows back
//!   │
//!   ▼  apply_pipeline(rows, &stages)
//!   OutputResult  ← filtered · sorted · limited rows
//! ```
//!
//! The main public surface:
//!
//! - [`crate::dsl::parse_pipeline`] turns a raw line into a [`crate::dsl::Pipeline`] value
//! - [`crate::dsl::parse_stage`] parses one stage when the command has already been split away
//! - [`crate::dsl::apply_pipeline`] / [`crate::dsl::apply_output_pipeline`] apply stages to
//!   existing rows
//! - [`crate::dsl::registered_verbs`] returns metadata for all supported stage verbs
//! - [`crate::dsl::verb_info`] looks up a single verb by name
//!
//! Choose the smallest entrypoint that matches your starting point:
//!
//! - if you only need to split `"command | stages"` into structured pieces, use
//!   [`crate::dsl::parse_pipeline`]
//! - if you already have one raw stage like `"F uid=alice"` and need to inspect
//!   its verb/spec shape, use [`crate::dsl::parse_stage`]
//! - if your command already produced `Vec<Row>`, use [`crate::dsl::apply_pipeline`]
//! - if you already have an [`crate::core::output_model::OutputResult`] and want
//!   to preserve its semantic document or render metadata, use
//!   [`crate::dsl::apply_output_pipeline`]
//! - if your rows come from an iterator and you want streamable stages to stay
//!   streamable for as long as possible, use [`crate::dsl::execute_pipeline_streaming`]
//!
//! Common verbs: `F` (filter), `P` (project), `S` (sort), `G` (group),
//! `A` (aggregate), `L` (limit), `V`/`K` (quick search), `U` (unroll),
//! `JQ` (jq expression). See [`crate::dsl::registered_verbs`] for the full list
//! with streaming notes in [`crate::dsl::VerbStreaming`].

pub(crate) mod compiled;
pub(crate) mod eval;
pub(crate) mod model;
pub(crate) mod parse;
pub(crate) mod verb_info;

mod engine;
mod value;
mod verbs;

pub use engine::{
    apply_output_pipeline, apply_pipeline, execute_pipeline, execute_pipeline_streaming,
};
pub use parse::pipeline::{Pipeline, parse_pipeline, parse_stage};
pub use verb_info::{
    VerbInfo, VerbStreaming, is_registered_explicit_verb, registered_verbs, render_streaming_badge,
    verb_info,
};

#[cfg(test)]
mod tests;
