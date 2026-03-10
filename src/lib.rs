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

/// Test-friendly API adapters and mock implementations.
pub mod api;
/// Main host-facing entrypoints, runtime state, and session types.
pub mod app;
/// Command-line argument types and CLI parsing helpers.
pub mod cli;
/// Structured command and pipe completion types.
pub mod completion;
/// Layered configuration schema, loading, and resolution.
pub mod config;
/// Shared command, output, row, and protocol primitives.
pub mod core;
/// Row-oriented pipeline parsing and execution.
pub mod dsl;
/// Structured help/guide view models and conversions.
pub mod guide;
/// External plugin discovery, protocol, and dispatch support.
pub mod plugin;
/// Service-layer ports used by command execution.
pub mod ports;
pub mod prelude;
pub mod repl;
pub mod runtime;
/// Library-level service entrypoints built on the core ports.
pub mod services;
/// Rendering, theming, and structured output helpers.
pub mod ui;

pub use crate::app::{App, AppBuilder, AppRunner, run_from, run_process};
pub use crate::core::command_policy;
pub use crate::native::{
    NativeCommand, NativeCommandCatalogEntry, NativeCommandContext, NativeCommandOutcome,
    NativeCommandRegistry,
};

mod native;

#[cfg(test)]
mod tests;
