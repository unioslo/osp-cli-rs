mod app;
mod cli;
mod logging;
pub mod pipeline;
mod plugin_manager;
mod repl;
mod rows;
mod theme_loader;
pub mod state;

pub use app::run_from;
