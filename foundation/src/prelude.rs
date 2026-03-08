//! Small convenience surface for embedding the app without importing the full module tree.

pub use crate::app::{App, AppBuilder, AppRunner, run_from, run_process};
pub use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
pub use crate::runtime::{AppState, RuntimeContext, UiState};
pub use crate::ui::RenderSettings;
