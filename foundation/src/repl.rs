//! REPL engine and prompt/history types.

pub use crate::osp_repl::{
    CompletionDebug, CompletionDebugFrame, CompletionDebugMatch,
    CompletionDebugOptions, DebugStep, HighlightDebugSpan, HistoryConfig,
    HistoryEntry, HistoryShellContext, LineProjection, LineProjector,
    OspHistoryStore, PromptRightRenderer, ReplAppearance, ReplInputMode,
    ReplLineResult, ReplPrompt, ReplReloadKind, ReplRunConfig, ReplRunResult,
    SharedHistory, color_from_style_spec, debug_completion, debug_completion_steps,
    debug_highlight, default_pipe_verbs, expand_history, run_repl,
};
