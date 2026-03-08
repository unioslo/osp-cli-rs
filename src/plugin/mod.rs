//! Plugin discovery, dispatch, and catalog management.

pub(crate) mod config;
pub(crate) mod conversion;
pub mod discovery;
pub mod dispatch;
pub mod manager;
pub(crate) mod state;
#[cfg(test)]
mod tests;

pub use manager::{
    CommandCatalogEntry, CommandConflict, DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, DiscoveredPlugin,
    DoctorReport, PluginDispatchContext, PluginDispatchError, PluginManager, PluginSource,
    PluginSummary,
};
