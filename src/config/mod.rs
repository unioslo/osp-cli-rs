//! Configuration exists so the app can answer three questions consistently:
//! what keys are legal, which source wins, and what file edits are allowed.
//!
//! The mental model is:
//!
//! - [`crate::config::LoaderPipeline`] materializes source layers from files,
//!   environment, and in-memory overrides.
//! - [`crate::config::ConfigResolver`] applies source precedence, scope
//!   precedence, interpolation, and schema adaptation.
//! - [`crate::config::RuntimeConfig`] lowers the resolved config into the
//!   smaller runtime view used by the app and REPL.
//! - [`crate::config::set_scoped_value_in_toml`] edits TOML-backed config files
//!   while preserving the same schema and scope rules used at runtime.
//!
//! Quick starts:
//!
//! In-memory or test-only config resolution:
//!
//! ```
//! use osp_cli::config::{ConfigLayer, LoaderPipeline, ResolveOptions, StaticLayerLoader};
//!
//! let mut defaults = ConfigLayer::default();
//! defaults.set("profile.default", "default");
//! defaults.set("theme.name", "dracula");
//!
//! let resolved = LoaderPipeline::new(StaticLayerLoader::new(defaults))
//!     .resolve(ResolveOptions::new().with_terminal("cli"))
//!     .unwrap();
//!
//! assert_eq!(resolved.terminal(), Some("cli"));
//! assert_eq!(resolved.get_string("theme.name"), Some("dracula"));
//! ```
//!
//! Host-style bootstrap with runtime defaults plus standard path discovery:
//!
//! ```no_run
//! use osp_cli::config::{
//!     ResolveOptions, RuntimeConfig, RuntimeConfigPaths, RuntimeDefaults,
//!     RuntimeLoadOptions, build_runtime_pipeline,
//! };
//!
//! let defaults = RuntimeDefaults::from_process_env("dracula", "osp> ").to_layer();
//! let paths = RuntimeConfigPaths::discover();
//! let presentation = None;
//! let cli = None;
//! let session = None;
//!
//! let resolved = build_runtime_pipeline(
//!     defaults,
//!     presentation,
//!     &paths,
//!     RuntimeLoadOptions::default(),
//!     cli,
//!     session,
//! )
//! .resolve(ResolveOptions::new().with_terminal("cli"))?;
//!
//! let runtime = RuntimeConfig::from_resolved(&resolved);
//! assert_eq!(runtime.active_profile, resolved.active_profile());
//! # Ok::<(), osp_cli::config::ConfigError>(())
//! ```
//!
//! On-disk config mutation:
//!
//! - use [`crate::config::set_scoped_value_in_toml`] and
//!   [`crate::config::unset_scoped_value_in_toml`] instead of editing TOML by
//!   hand
//! - use [`crate::config::ConfigResolver::explain_key`] when the main question
//!   is "why did this value win?"
//!
//! Broad-strokes flow:
//!
//! ```text
//! files / env / session overrides
//!        │
//!        ▼ [ LoaderPipeline ]
//!    ConfigLayer values + scope metadata
//!        │
//!        ▼ [ ConfigResolver ]
//!    precedence + interpolation + type adaptation
//!        │
//!        ├── ResolvedConfig  (full provenance-aware map)
//!        ├── RuntimeConfig   (smaller runtime view used by the host)
//!        └── config explain  (why this winner won)
//! ```
//!
//! Read this module when you need to answer "where did this config value come
//! from?", "why did this value win?", or "what writes are legal for this key?".
//!
//! Most callers should not be assembling bespoke config logic. The normal path
//! is:
//!
//! - load layers through [`crate::config::LoaderPipeline`]
//! - resolve once through [`crate::config::ConfigResolver`]
//! - consume the result as [`crate::config::ResolvedConfig`] or
//!   [`crate::config::RuntimeConfig`]
//! - for host-style startup, prefer [`crate::config::RuntimeDefaults`],
//!   [`crate::config::RuntimeConfigPaths`], and
//!   [`crate::config::build_runtime_pipeline`] over hand-rolled file/env
//!   discovery
//!
//! The same discipline should hold for writes. File edits belong through the
//! store helpers here so `config set`, runtime resolution, and `config explain`
//! stay aligned. A hand-rolled file edit path almost always creates drift.
//!
//! Contract:
//!
//! - source precedence and scope precedence are defined here, not in callers
//! - schema validation and config-store editing should stay aligned
//! - other modules should not hand-roll config merging or scope filtering

mod bootstrap;
mod core;
mod error;
mod explain;
mod interpolate;
mod loader;
mod resolver;
mod runtime;
mod selector;
mod store;

pub use core::*;
pub use error::*;
pub use loader::*;
pub use resolver::*;
pub use runtime::*;
pub use store::*;
