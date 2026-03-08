//! REPL engine plus the host-side interactive shell integration.

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
pub use engine::{
    CompletionDebug, CompletionDebugFrame, CompletionDebugMatch, CompletionDebugOptions, DebugStep,
    HighlightDebugSpan, HistoryConfig, HistoryEntry, HistoryShellContext, LineProjection,
    LineProjector, OspHistoryStore, PromptRightRenderer, ReplAppearance, ReplInputMode,
    ReplLineResult, ReplPrompt, ReplReloadKind, ReplRunConfig, ReplRunResult, SharedHistory,
    color_from_style_spec, debug_completion, debug_completion_steps, debug_highlight,
    default_pipe_verbs, expand_history, run_repl,
};
pub(crate) use engine::{
    CompletionTraceEvent, CompletionTraceMenuState, trace_completion, trace_completion_enabled,
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
