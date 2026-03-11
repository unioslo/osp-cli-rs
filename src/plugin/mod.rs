//! The plugin module exists so external commands can participate in `osp`
//! without becoming a separate execution model everywhere else in the app.
//!
//! This module owns the boundary between the core app and external command
//! providers. Discovery and catalog building happen before dispatch so the rest
//! of the app can reason about plugins as ordinary command metadata.
//!
//! Contract:
//!
//! - plugin discovery and dispatch rules live here
//! - the rest of the app should consume plugin metadata/results, not spawn
//!   plugin processes ad hoc

pub(crate) mod active;
pub(crate) mod catalog;
pub(crate) mod config;
pub(crate) mod conversion;
pub(crate) mod discovery;
pub(crate) mod dispatch;
pub mod manager;
pub(crate) mod selection;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub use manager::{
    CommandCatalogEntry, CommandConflict, DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, DiscoveredPlugin,
    DoctorReport, PluginDispatchContext, PluginDispatchError, PluginManager, PluginSource,
    PluginSummary, RawPluginOutput,
};
