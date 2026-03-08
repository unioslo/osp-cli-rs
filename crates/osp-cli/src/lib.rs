mod app;
mod cli;
mod invocation;
mod logging;
pub mod pipeline;
mod plugin_config;
mod plugin_manager;
mod repl;
mod rows;
pub mod state;
mod theme_loader;
mod ui_presentation;
mod ui_sink;

pub use app::{classify_exit_code, render_report_message, run_from};
pub use cli::Cli;
