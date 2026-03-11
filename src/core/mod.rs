//! Core primitives shared across the rest of the crate.
//!
//! This module exists to hold small, stable building blocks that many other
//! subsystems need: rows, output/result types, command metadata, shell token
//! handling, fuzzy matching, and a few protocol DTOs.
//!
//! Contract:
//!
//! - types here should stay broadly reusable and free of host-specific logic
//! - `core` can be depended on widely, but it should avoid depending on
//!   higher-level modules like `app`, `repl`, or `ui`

/// Declarative command metadata used for help and policy resolution.
pub mod command_def;
/// Visibility and access-policy evaluation for commands.
pub mod command_policy;
/// Shared Unicode-aware fuzzy matching helpers.
pub mod fuzzy;
/// Output-mode and presentation enums shared across the crate.
pub mod output;
/// Structured row/group/document output types.
pub mod output_model;
/// Plugin protocol DTOs shared across plugin boundaries.
pub mod plugin;
/// Shared row representation used by the DSL and services.
pub mod row;
/// Runtime-mode and verbosity primitives.
pub mod runtime;
/// Shell-like tokenization helpers.
pub mod shell_words;
