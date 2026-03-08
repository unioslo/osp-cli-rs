//! Plugin discovery, dispatch, and catalog management.

pub mod discovery;
pub mod dispatch;
pub mod manager;

pub use discovery::{
    CommandCatalogEntry, DiscoveredPlugin, DoctorReport, PluginSource, PluginSummary,
};
pub use dispatch::{
    PluginDispatchContext, PluginDispatchError, DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS,
};
pub use manager::PluginManager;
