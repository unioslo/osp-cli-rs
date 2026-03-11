//! The REPL module exists to own interactive shell behavior that the ordinary
//! CLI host should not know about.
//!
//! The layering here is intentional:
//!
//! - `engine` owns the line editor boundary, prompt rendering, history
//!   picker/completion adapters, and debug surfaces.
//! - `dispatch` owns command execution and shell-scope behavior once a line
//!   has been accepted.
//! - `completion` shapes the live command catalog into REPL-aware completion
//!   trees.
//! - `presentation` owns prompt appearance and intro/help material that is
//!   specific to interactive use.
//!
//! When debugging the REPL, first decide whether the issue is editor/runtime
//! state, dispatch semantics, or rendering. That is usually enough to choose
//! the right submodule.
//!
//! Contract:
//!
//! - this module may depend on editor/runtime adapters, completion, UI, and
//!   dispatch code
//! - it should not become the owner of generic command execution rules, config
//!   resolution, or non-interactive CLI parsing
//!
//! Public API shape:
//!
//! - debug snapshots and other semantic payloads stay direct and cheap to read
//! - host-style REPL configuration flows through concrete builders and
//!   factories such as [`crate::repl::ReplRunConfig::builder`],
//!   [`crate::repl::ReplAppearance::builder`], and
//!   [`crate::repl::HistoryConfig::builder`]
//! - guided REPL configuration follows the crate-wide naming rule:
//!   `new(...)` for exact constructors, `builder(...)` for staged
//!   configuration, `with_*` setters, and `build()` as the terminal step

pub(crate) mod completion;
pub(crate) mod dispatch;
mod engine;
pub(crate) mod help;
mod highlight;
pub(crate) mod history;
mod history_store;
mod host;
pub(crate) mod input;
pub(crate) mod lifecycle;
mod menu;
mod menu_core;
pub(crate) mod presentation;
pub(crate) mod surface;

#[cfg(test)]
pub(crate) use dispatch::apply_repl_shell_prefix;
#[cfg(test)]
pub(crate) use engine::ReplCompleter;
pub use engine::{
    CompletionDebug, CompletionDebugFrame, CompletionDebugMatch, CompletionDebugOptions, DebugStep,
    HighlightDebugSpan, HistoryConfig, HistoryConfigBuilder, HistoryEntry, HistoryShellContext,
    LineProjection, LineProjector, PromptRightRenderer, ReplAppearance, ReplAppearanceBuilder,
    ReplInputMode, ReplLineResult, ReplPrompt, ReplReloadKind, ReplRunConfig, ReplRunConfigBuilder,
    ReplRunResult, SharedHistory, color_from_style_spec, debug_completion, debug_completion_steps,
    debug_highlight, debug_history_menu, debug_history_menu_steps, default_pipe_verbs, run_repl,
};
pub(crate) use engine::{
    CompletionTraceEvent, CompletionTraceMenuState, expand_history, trace_completion,
    trace_completion_enabled,
};
pub(crate) use host::{
    ReplViewContext, repl_command_spec, run_plugin_repl, run_repl_debug_command_for,
};
#[cfg(test)]
pub(crate) use input::{ReplParsedLine, is_repl_shellable_command};
#[cfg(test)]
pub(crate) use presentation::{
    render_prompt_template, render_repl_prompt_right_for_test, theme_display_name,
};
