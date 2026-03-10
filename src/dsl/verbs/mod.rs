//! Canonical DSL verb surface.
//!
//! Architecture rule:
//! - selector verbs preserve/rewrite semantic document structure via addressed
//!   matches
//! - collection verbs operate on row/group collections via the bridge helpers
//!
//! Do not let each verb invent its own hybrid traversal again. If a new verb is
//! structurally selecting or rewriting descendants, it belongs on the selector
//! substrate. If it is sorting/grouping/aggregating row-like collections, it
//! belongs on the collection bridge.

pub(crate) mod aggregate;
pub(crate) mod collapse;
pub(crate) mod common;
pub(crate) mod copy;
pub(crate) mod filter;
pub(crate) mod group;
pub(crate) mod jq;
pub(crate) mod json;
pub(crate) mod limit;
pub(crate) mod project;
pub(crate) mod question;
pub(crate) mod quick;
pub(crate) mod selector;
pub(crate) mod sort;
pub(crate) mod unroll;
pub(crate) mod values;
