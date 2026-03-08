//! CLI grammar and command-facing helpers still owned by the host layer.

pub(crate) mod commands;
pub(crate) mod invocation;
pub mod pipeline;

pub use crate::osp_cli::Cli;
