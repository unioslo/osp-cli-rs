//! `osp-cli` is the canonical single-crate package for `osp`.
//!
//! The internal implementation still carries some compatibility-oriented
//! `osp_*` module names, but consumers should prefer the stable top-level
//! modules exported here:
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
//! Most `osp_*` modules remain public for now as compatibility shims while the
//! old multi-crate mirror lives under `workspace/`.

#[doc(hidden)]
pub mod osp_api;
#[doc(hidden)]
pub mod osp_cli;
#[doc(hidden)]
pub mod osp_completion;
#[doc(hidden)]
pub mod osp_config;
#[doc(hidden)]
pub mod osp_core;
#[doc(hidden)]
pub mod osp_dsl;
#[doc(hidden)]
pub mod osp_ports;
#[doc(hidden)]
pub mod osp_repl;
#[doc(hidden)]
pub mod osp_services;
mod osp_ui;

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

#[cfg(test)]
mod tests;
