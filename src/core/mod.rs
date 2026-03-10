/// Declarative command metadata used for help and policy resolution.
pub mod command_def;
/// Visibility and access-policy evaluation for commands.
pub mod command_policy;
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
