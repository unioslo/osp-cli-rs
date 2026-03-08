//! Plugin discovery, dispatch, and catalog management.

pub mod discovery;
pub mod dispatch;
pub mod manager;

pub use discovery::{
    CommandCatalogEntry, DiscoveredPlugin, DoctorReport, PluginSource, PluginSummary,
};
pub use dispatch::{DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, PluginDispatchContext, PluginDispatchError};
pub use manager::PluginManager;
