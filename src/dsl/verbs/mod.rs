//! Row operators for the single partitioned RowSet execution substrate.
//! Nested selectors address row values; group keys and aggregates are metadata.

pub(crate) mod aggregate;
pub(crate) mod collapse;
pub(crate) mod common;
#[cfg(test)]
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
