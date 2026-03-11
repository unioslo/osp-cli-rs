//! Configuration exists so the app can answer three questions consistently:
//! what keys are legal, which source wins, and what file edits are allowed.
//!
//! The mental model is:
//!
//! - [`crate::config::LoaderPipeline`] materializes source layers from files,
//!   environment, and in-memory overrides.
//! - [`crate::config::ConfigResolver`] applies source precedence, scope
//!   precedence, interpolation, and schema adaptation.
//! - [`crate::config::RuntimeConfig`] lowers the resolved config into the
//!   smaller runtime view used by the app and REPL.
//! - [`crate::config::set_scoped_value_in_toml`] edits TOML-backed config files
//!   while preserving the same schema and scope rules used at runtime.
//!
//! Read this module when you need to answer "where did this config value come
//! from?", "why did this value win?", or "what writes are legal for this key?".
//!
//! Contract:
//!
//! - source precedence and scope precedence are defined here, not in callers
//! - schema validation and config-store editing should stay aligned
//! - other modules should not hand-roll config merging or scope filtering

mod bootstrap;
mod core;
mod error;
mod explain;
mod interpolate;
mod loader;
mod resolver;
mod runtime;
mod selector;
mod store;

pub use core::*;
pub use error::*;
pub use loader::*;
pub use resolver::*;
pub use runtime::*;
pub use store::*;
