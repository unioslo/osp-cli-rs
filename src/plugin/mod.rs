//! The plugin module exists so external commands can participate in `osp`
//! without becoming a separate execution model everywhere else in the app.
//!
//! This module owns the boundary between the core app and external command
//! providers. Discovery and catalog building happen before dispatch so the rest
//! of the app can reason about plugins as ordinary command metadata.
//!
//! Broad-strokes flow:
//!
//! ```text
//! plugin executable
//!      │ emits `describe` JSON
//!      ▼
//! [ core::plugin ]  wire DTOs + validation
//!      ▼
//! [ plugin ]        discovery, catalog building, provider selection
//!      ▼
//! [ app ]           command dispatch and rendering
//!      │
//!      └── later invokes plugin command -> `ResponseV1`
//! ```
//!
//! Start here based on which side of the boundary you own:
//!
//! - host/application side: [`crate::plugin::PluginManager`]
//! - wire-format / protocol side: [`crate::core::plugin`]
//! - in-process built-ins that should behave like plugins without subprocesses:
//!   [`crate::native::NativeCommandRegistry`]
//!
//! Minimal host-side browse path:
//!
//! ```
//! use osp_cli::plugin::PluginManager;
//!
//! let manager = PluginManager::new(Vec::new()).with_path_discovery(false);
//! let plugins = manager.list_plugins();
//! let catalog = manager.command_catalog();
//! let doctor = manager.doctor();
//!
//! assert!(plugins.is_empty());
//! assert!(catalog.is_empty());
//! assert!(doctor.conflicts.is_empty());
//! ```
//!
//! Choose [`crate::plugin::PluginManager`] when you are building the host and
//! want discovery, catalog, and provider-selection behavior. Choose
//! [`crate::core::plugin`] when you are implementing the plugin executable or
//! need the stable wire DTOs directly.
//!
//! Contract:
//!
//! - plugin discovery and dispatch rules live here
//! - the rest of the app should consume plugin metadata/results, not spawn
//!   plugin processes ad hoc
//!
//! Public API shape:
//!
//! - [`crate::plugin::PluginManager`] is the host-side facade for discovery,
//!   browse surfaces, provider selection, and dispatch
//! - catalog and doctor payloads such as
//!   [`crate::plugin::CommandCatalogEntry`],
//!   [`crate::plugin::PluginSummary`], and [`crate::plugin::DoctorReport`]
//!   stay plain semantic data
//! - dispatch customization flows through
//!   [`crate::plugin::PluginDispatchContext`] and dispatch failures are
//!   surfaced as [`crate::plugin::PluginDispatchError`]
//! - discovery provenance is described by [`crate::plugin::PluginSource`]

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
