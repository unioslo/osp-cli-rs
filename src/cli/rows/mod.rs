//! Row/output normalization helpers used by CLI command handlers.
//!
//! This module exists so built-in commands and plugin responses can converge on
//! the same canonical output model without each call site re-implementing
//! normalization rules.
//!
//! In practice it owns two small but important bridges:
//!
//! - row literals and builders for terse command-side construction
//! - conversion between raw/plugin row data and [`crate::core::output_model::OutputResult`]
//!
//! Keep these rules centralized. If commands start hand-rolling scalar/object
//! normalization or grouped-row flattening in several places, the rendered
//! behavior will drift.

pub(crate) mod output;
pub(crate) mod row;
