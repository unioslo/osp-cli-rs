//! Plugin discovery, dispatch, and catalog management.

pub mod manager;

pub use manager::{
    CommandCatalogEntry, DiscoveredPlugin, DoctorReport, PluginDispatchContext,
    PluginDispatchError, PluginManager, PluginSource, PluginSummary,
    DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS,
};
