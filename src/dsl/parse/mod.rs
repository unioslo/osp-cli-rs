//! Parsing helpers for the canonical pipeline DSL.
//!
//! This module exists to turn user-facing pipeline syntax into structured
//! tokens, selectors, and stage descriptions before evaluation begins.
//!
//! Contract:
//!
//! - parsing should stop at syntax and structural intent
//! - runtime selection and traversal rules belong in `dsl::eval`

pub mod key_spec;
pub mod lexer;
pub mod path;
pub mod pipeline;
pub mod quick;
