//! `osp-cli` is the canonical single-crate package for `osp`.
//!
//! - [`app`] for the main CLI entrypoints and stateful host surface
//! - [`config`] for configuration types and resolution
//! - [`core`] for shared output, row, and runtime types
//! - [`dsl`] for pipeline parsing and execution
//! - [`ui`] for rendering and message formatting
//! - [`repl`] for REPL engine types
//! - [`completion`] for command/completion tree types
//! - [`plugin`] for plugin discovery/dispatch management
//! - [`api`], [`ports`], and [`services`] for the service/client layer
//!
//! The old multi-crate mirror still lives under `workspace/`, but the root
//! crate's canonical implementation now lives in the module tree below.

pub mod api;
pub mod app;
pub mod cli;
pub mod completion;
pub mod config;
pub mod core;
pub mod dsl;
pub mod plugin;
pub mod ports;
pub mod prelude;
pub mod repl;
pub mod runtime;
pub mod services;
pub mod ui;

pub use crate::app::{App, AppBuilder, AppRunner, run_from, run_process};
pub use crate::core::command_policy;

#[cfg(test)]
mod tests;
