//! `osp-cli` exists to keep the canonical `osp` product surface in one crate.
//!
//! The point of this crate is not just packaging. It is the place where the
//! product's main concerns meet: CLI host orchestration, layered config
//! resolution, rendering, REPL integration, completion, plugins, and the
//! pipeline DSL. Keeping those boundaries visible in one crate makes rustdoc a
//! real architecture map instead of a thin wrapper over several mirrored
//! packages.
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
//! - [`api`] provides test-friendly fixtures and lightweight helper adapters
//!   rather than a second full integration surface.
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
//! For embedders:
//!
//! - use [`app::App`] or [`app::AppBuilder`] for the full CLI/REPL host
//! - use [`app::AppState::builder`] when you need a manual runtime/session
//!   snapshot
//! - use [`app::UiState::builder`] and [`app::LaunchContext::builder`] for the
//!   heavier host-building blocks
//! - use [`repl::ReplRunConfig::builder`] and [`repl::HistoryConfig::builder`]
//!   when embedding the REPL editor surface directly
//! - use [`guide::GuideView`] and [`completion::CompletionTreeBuilder`] for
//!   semantic payload generation
//! - use [`services`] and [`ports`] when you want narrower integrations instead
//!   of the whole host stack
//!
//! The root crate module tree is the only supported code path. Older mirrored
//! layouts have been removed so rustdoc and the source tree describe the same
//! architecture.

/// Test-friendly API adapters and mock implementations.
pub mod api;
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
mod dsl2;
/// Structured help/guide view models and conversions.
pub mod guide;
/// External plugin discovery, protocol, and dispatch support.
pub mod plugin;
/// Service-layer ports used by command execution.
pub mod ports;
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
