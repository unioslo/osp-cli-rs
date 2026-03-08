//! Plugin manager surface staged as a first-class top-level module.

pub use crate::osp_cli::plugin_manager::{
    CommandCatalogEntry, DiscoveredPlugin, DoctorReport, PluginDispatchContext,
    PluginDispatchError, PluginManager, PluginSource, PluginSummary,
    DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS,
};
