//! REPL engine and prompt/history types.

pub mod debug;
pub mod history;
pub mod prompt;
pub mod run;

pub use debug::{
    CompletionDebug, CompletionDebugFrame, CompletionDebugMatch,
    CompletionDebugOptions, DebugStep, HighlightDebugSpan, color_from_style_spec,
    debug_completion, debug_completion_steps, debug_highlight,
};
pub use history::{
    HistoryConfig, HistoryEntry, HistoryShellContext, OspHistoryStore,
    SharedHistory, expand_history,
};
pub use prompt::{
    LineProjection, LineProjector, PromptRightRenderer, ReplAppearance, ReplPrompt,
};
pub use run::{
    ReplInputMode, ReplLineResult, ReplReloadKind, ReplRunConfig, ReplRunResult,
    default_pipe_verbs, run_repl,
};
