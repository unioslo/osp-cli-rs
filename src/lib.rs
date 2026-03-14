#![cfg_attr(
    not(test),
    warn(clippy::expect_used, clippy::panic, clippy::unwrap_used)
)]
#![deny(missing_docs)]

//! `osp-cli` is the library behind the `osp` CLI and REPL.
//!
//! Use it when you want one of these jobs:
//!
//! - run the full `osp` host in-process
//! - build a wrapper crate with site-specific native commands and defaults
//! - execute the small LDAP service surface without the full host
//! - render rows or run DSL pipelines in-process
//!
//! Most readers only need one of those lanes. You do not need to understand
//! the whole crate before using it.
//!
//! The crate also keeps the full `osp` product surface in one place so the
//! main concerns stay visible together: host orchestration, config resolution,
//! rendering, REPL integration, completion, plugins, and the pipeline DSL.
//! That makes rustdoc a useful architecture map after you have picked the
//! smallest surface that fits your job.
//!
//! Quick starts for the three most common library shapes:
//!
//! Full `osp`-style host with captured output:
//!
//! ```
//! use osp_cli::App;
//! use osp_cli::app::BufferedUiSink;
//!
//! let mut sink = BufferedUiSink::default();
//! let exit = App::new().run_with_sink(["osp", "--help"], &mut sink)?;
//!
//! assert_eq!(exit, 0);
//! assert!(!sink.stdout.is_empty());
//! assert!(sink.stderr.is_empty());
//! # Ok::<(), miette::Report>(())
//! ```
//!
//! Lightweight LDAP command execution plus DSL stages:
//!
//! ```
//! use osp_cli::config::RuntimeConfig;
//! use osp_cli::ports::mock::MockLdapClient;
//! use osp_cli::services::{ServiceContext, execute_line};
//!
//! let ctx = ServiceContext::new(
//!     Some("oistes".to_string()),
//!     MockLdapClient::default(),
//!     RuntimeConfig::default(),
//! );
//! let output = execute_line(&ctx, "ldap user oistes | P uid cn")
//!     .expect("service command should run");
//! let rows = output.as_rows().expect("expected row output");
//!
//! assert_eq!(rows.len(), 1);
//! assert_eq!(rows[0].get("uid").and_then(|value| value.as_str()), Some("oistes"));
//! assert!(rows[0].contains_key("cn"));
//! ```
//!
//! Rendering existing rows without bootstrapping the full host:
//!
//! ```
//! use osp_cli::core::output::OutputFormat;
//! use osp_cli::row;
//! use osp_cli::ui::{RenderSettings, render_rows};
//!
//! let rendered = render_rows(
//!     &[row! { "uid" => "alice", "mail" => "alice@example.com" }],
//!     &RenderSettings::test_plain(OutputFormat::Json),
//! );
//!
//! assert!(rendered.contains("\"uid\": \"alice\""));
//! assert!(rendered.contains("\"mail\": \"alice@example.com\""));
//! ```
//!
//! Building a product-specific wrapper crate:
//!
//! - keep site-specific auth, policy, and domain integrations in the wrapper
//!   crate
//! - extend the command surface with [`App::with_native_commands`] or
//!   [`AppBuilder::with_native_commands`]
//! - keep runtime config bootstrap aligned with
//!   [`config::RuntimeDefaults`], [`config::RuntimeConfigPaths`], and
//!   [`config::build_runtime_pipeline`]
//! - expose a thin product-level `run_process` or builder API on top of this
//!   crate instead of forking generic host behavior
//!
//! Minimal wrapper shape:
//!
//! ```
//! use std::ffi::OsString;
//!
//! use anyhow::Result;
//! use clap::Command;
//! use osp_cli::app::BufferedUiSink;
//! use osp_cli::config::ConfigLayer;
//! use osp_cli::{
//!     App, AppBuilder, NativeCommand, NativeCommandContext, NativeCommandOutcome,
//!     NativeCommandRegistry,
//! };
//!
//! struct SiteStatusCommand;
//!
//! impl NativeCommand for SiteStatusCommand {
//!     fn command(&self) -> Command {
//!         Command::new("site-status").about("Show site-specific status")
//!     }
//!
//!     fn execute(
//!         &self,
//!         _args: &[String],
//!         _context: &NativeCommandContext<'_>,
//!     ) -> Result<NativeCommandOutcome> {
//!         Ok(NativeCommandOutcome::Exit(0))
//!     }
//! }
//!
//! fn site_registry() -> NativeCommandRegistry {
//!     NativeCommandRegistry::new().with_command(SiteStatusCommand)
//! }
//!
//! fn site_defaults() -> ConfigLayer {
//!     let mut defaults = ConfigLayer::default();
//!     defaults.set("extensions.site.enabled", true);
//!     defaults
//! }
//!
//! #[derive(Clone)]
//! struct SiteApp {
//!     inner: App,
//! }
//!
//! impl SiteApp {
//!     fn builder() -> AppBuilder {
//!         App::builder()
//!             .with_native_commands(site_registry())
//!             .with_product_defaults(site_defaults())
//!     }
//!
//!     fn new() -> Self {
//!         Self {
//!             inner: Self::builder().build(),
//!         }
//!     }
//!
//!     fn run_process<I, T>(&self, args: I) -> i32
//!     where
//!         I: IntoIterator<Item = T>,
//!         T: Into<OsString> + Clone,
//!     {
//!         self.inner.run_process(args)
//!     }
//! }
//!
//! let app = SiteApp::new();
//! let mut sink = BufferedUiSink::default();
//! let exit = app.inner.run_process_with_sink(["osp", "--help"], &mut sink);
//!
//! assert_eq!(exit, 0);
//! assert!(sink.stdout.contains("site-status"));
//! assert_eq!(app.run_process(["osp", "--help"]), 0);
//! ```
//!
//! If you are new here, start with one of these:
//!
//! - wrapper crate / downstream product →
//!   [embedding guide](https://github.com/unioslo/osp-cli-rs/blob/main/docs/EMBEDDING.md)
//!   and [`App::builder`]
//! - full in-process host → [`app`]
//! - smaller service-only integration → [`services`]
//! - rendering / formatting only → [`ui`]
//!
//! Start here depending on what you need:
//!
//! - [`app`] exists to turn the lower-level pieces into a running CLI or REPL
//!   process.
//! - [`cli`] exists to model the public command-line grammar.
//! - [`config`] exists to answer what values are legal, where they came from,
//!   and what finally wins.
//! - [`completion`] exists to rank suggestions without depending on terminal
//!   state or editor code.
//! - [`repl`] exists to own the interactive shell boundary.
//! - [`dsl`] exists to provide the canonical document-first pipeline language.
//! - [`ui`] exists to lower structured output into terminal-facing text.
//! - [`plugin`] exists to treat external command providers as part of the same
//!   command surface.
//! - [`services`] and [`ports`] exist for smaller embeddable integrations that
//!   do not want the whole host stack.
//!
//! # Feature Flags
//!
//! - `clap` (enabled by default): exposes the clap conversion helpers such as
//!   [`crate::core::command_def::CommandDef::from_clap`],
//!   [`crate::core::plugin::DescribeCommandV1::from_clap`], and
//!   [`crate::core::plugin::DescribeV1::from_clap_command`].
//!
//! At runtime, data flows roughly like this:
//!
//! ```text
//! argv / REPL line
//!      │
//!      ▼ [ cli ]     parse grammar and flags
//!      ▼ [ config ]  resolve layered settings (builtin → file → env → cli)
//!      ▼ [ app ]     dispatch to plugin or native command  ──►  Vec<Row>
//!      ▼ [ dsl ]     apply pipeline stages to rows         ──►  OutputResult
//!      ▼ [ ui ]      render structured output to terminal or UiSink
//! ```
//!
//! Architecture contracts worth keeping stable:
//!
//! - lower-level modules should not depend on [`app`]
//! - [`completion`] stays pure and should not start doing network, plugin
//!   discovery, or terminal I/O
//! - [`ui`] renders structured input but should not become a config-resolver or
//!   service-execution layer
//! - [`cli`] describes the grammar of the program but does not execute it
//! - [`config`] owns precedence and legality rules so callers do not invent
//!   their own merge semantics
//!
//! Public API shape:
//!
//! - semantic payload modules such as [`guide`] and most of [`completion`]
//!   stay intentionally cheap to compose and inspect
//! - host machinery such as [`app::App`], [`app::AppBuilder`], and runtime
//!   state is guided through constructors/builders/accessors rather than
//!   compatibility shims or open-ended assembly
//! - each public concept should have one canonical home; duplicate aliases and
//!   mirrored module paths are treated as API debt
//!
//! Guided construction naming:
//!
//! - `Type::new(...)` is the exact constructor when the caller already knows
//!   the required inputs
//! - `Type::builder(...)` starts guided construction for heavier host/runtime
//!   objects and returns a concrete `TypeBuilder`
//! - builder setters use `with_*` and the terminal step is always `build()`
//! - `Type::from_*` and `Type::detect()` are reserved for derived/probing
//!   factories
//! - semantic DSLs may keep domain verbs such as `arg`, `flag`, or
//!   `subcommand`; the `with_*` rule is for guided host configuration, not for
//!   every fluent API
//! - avoid abstract "factory builder" layers in the public API; callers should
//!   see concrete type-named builders and factories directly
//!
//! For embedders, choose the smallest surface that solves the problem you
//! actually have:
//!
//! - "I want a full `osp`-style binary or custom `main`" →
//!   [`app::App::builder`], [`app::AppBuilder::build`], or
//!   [`app::App::run_from`]
//! - "I want to capture rendered stdout/stderr in tests or another host" →
//!   [`app::App::with_sink`] or [`app::AppBuilder::build_with_sink`]
//! - "I want parser + service execution + DSL, but not the full host" →
//!   [`services::ServiceContext`] and [`services::execute_line`]
//! - "I already have rows and only want pipeline transforms" →
//!   [`dsl::apply_pipeline`] or [`dsl::apply_output_pipeline`]
//! - "I need plugin discovery and catalog/policy integration" →
//!   [`plugin::PluginManager`] on the host side, or [`core::plugin`] when
//!   implementing the wire protocol itself
//! - "I need manual runtime/session state" → [`app::AppStateBuilder::new`],
//!   [`app::UiState::new`], [`app::UiState::from_resolved_config`], and direct
//!   [`app::LaunchContext`] setters
//! - "I want to embed the interactive editor loop directly" →
//!   [`repl::ReplRunConfig::builder`] and [`repl::HistoryConfig::builder`]
//! - "I need semantic payload generation for help/completion surfaces" →
//!   [`guide::GuideView`] and [`completion::CompletionTreeBuilder`]
//!
//! The root crate module tree is the only supported code path. Older mirrored
//! layouts have been removed so rustdoc and the source tree describe the same
//! architecture.

/// Main host-facing entrypoints, runtime state, and session types.
pub mod app;
/// Command-line argument types and CLI parsing helpers.
pub mod cli;
/// Structured command and pipe completion types.
pub mod completion;
/// Layered configuration schema, loading, and resolution.
pub mod config;
/// Shared command, output, row, and protocol primitives.
pub mod core;
/// Canonical pipeline parsing and execution.
pub mod dsl;
/// Structured help/guide view models and conversions.
pub mod guide;
/// External plugin discovery, protocol, and dispatch support.
pub mod plugin;
/// Service-layer ports used by command execution.
pub mod ports;
/// Interactive REPL editor, prompt, history, and completion surface.
pub mod repl;
/// Library-level service entrypoints built on the core ports.
pub mod services;
/// Rendering, theming, and structured output helpers.
pub mod ui;

pub use crate::app::{App, AppBuilder, AppRunner, run_from, run_process};
pub use crate::core::command_policy;
pub use crate::native::{
    NativeCommand, NativeCommandCatalogEntry, NativeCommandContext, NativeCommandOutcome,
    NativeCommandRegistry,
};

mod native;

#[cfg(test)]
mod tests;
