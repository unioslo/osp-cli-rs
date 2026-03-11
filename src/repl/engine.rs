//! The REPL engine exists to own the editor/runtime boundary of interactive
//! `osp`.
//!
//! High-level flow:
//!
//! - configure the line editor or basic fallback based on terminal capability
//! - render prompts and prompt-right state for the current REPL frame
//! - adapt completion, history search, and highlighting into editor-facing
//!   menus and callbacks
//! - expose debug/trace surfaces so host commands can inspect the live editor
//!   state without reimplementing it
//!
//! Higher-level orchestration lives in [`super::host`] and
//! [`super::lifecycle`]. Actual command execution lives in [`super::dispatch`].
//!
//! Contract:
//!
//! - this module may depend on editor adapters, completion, history, and UI
//!   presentation helpers
//! - it should not become the owner of generic command execution, config
//!   precedence, or product-level restart policy
//!
//! Public API shape:
//!
//! - debug snapshots stay direct semantic payloads
//! - host-facing REPL prompts, appearance, and run configuration live in the
//!   dedicated [`config`] surface instead of inside the editor mechanics
//! - host-style REPL runtime configuration uses concrete builders and
//!   constructors such as [`ReplRunConfig::builder`],
//!   [`ReplAppearance::builder`], and [`CompletionDebugOptions::new`]

pub use super::highlight::{HighlightDebugSpan, debug_highlight};
pub(crate) use super::history_store::expand_history;
pub use super::history_store::{
    HistoryConfig, HistoryConfigBuilder, HistoryEntry, HistoryShellContext, SharedHistory,
};
use anyhow::Result;

mod adapter;
mod config;
mod debug;
mod editor;
mod overlay;
mod session;

pub(crate) use adapter::{
    CompletionTraceEvent, CompletionTraceMenuState, trace_completion, trace_completion_enabled,
};
#[cfg(test)]
use adapter::{
    ReplCompleter, ReplHistoryCompleter, build_repl_highlighter, expand_home, path_suggestions,
    split_path_stub,
};
pub use adapter::{color_from_style_spec, default_pipe_verbs};
pub use config::{
    LineProjection, LineProjector, PromptRightRenderer, ReplAppearance, ReplAppearanceBuilder,
    ReplInputMode, ReplLineResult, ReplPrompt, ReplReloadKind, ReplRunConfig, ReplRunConfigBuilder,
    ReplRunResult,
};
pub use debug::{
    CompletionDebug, CompletionDebugFrame, CompletionDebugMatch, CompletionDebugOptions, DebugStep,
    debug_completion, debug_completion_steps, debug_history_menu, debug_history_menu_steps,
};
#[cfg(test)]
use editor::{
    AutoCompleteEmacs, contains_cursor_position_report, is_cursor_position_error,
    parse_cursor_position_report,
};
pub(crate) use editor::{BasicInputReason, OspPrompt, basic_input_reason};
#[cfg(test)]
use overlay::{build_history_menu, build_history_picker_options, history_picker_items};
use session::{InteractiveLoopConfig, SubmissionContext, run_repl_basic, run_repl_interactive};
#[cfg(test)]
use session::{SubmissionResult, process_submission};

const COMPLETION_MENU_NAME: &str = "completion_menu";
const HISTORY_MENU_NAME: &str = "history_menu";
const HOST_COMMAND_HISTORY_PICKER: &str = "\u{0}osp-repl-history-picker";

/// Runs the interactive REPL and delegates submitted lines to `execute`.
pub fn run_repl<F>(config: ReplRunConfig, mut execute: F) -> Result<ReplRunResult>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
{
    let ReplRunConfig {
        prompt,
        completion_words,
        completion_tree,
        appearance,
        history_config,
        input_mode,
        prompt_right,
        line_projector,
    } = config;
    let history_store = SharedHistory::new(history_config)?;
    let mut submission = SubmissionContext {
        history_store: &history_store,
        execute: &mut execute,
    };
    let prompt = OspPrompt::new(prompt.left, prompt.indicator, prompt_right);

    if let Some(reason) = basic_input_reason(input_mode) {
        match reason {
            BasicInputReason::NotATerminal => {
                eprintln!("Warning: Input is not a terminal (fd=0).");
            }
            BasicInputReason::CursorProbeUnsupported => {
                eprintln!(
                    "Warning: terminal does not support cursor position requests; using basic input mode."
                );
            }
            BasicInputReason::Explicit => {}
        }
        run_repl_basic(&prompt, &mut submission)?;
        return Ok(ReplRunResult::Exit(0));
    }

    run_repl_interactive(
        InteractiveLoopConfig {
            prompt: &prompt,
            completion_words,
            completion_tree,
            appearance,
            line_projector,
        },
        history_store.clone(),
        &mut submission,
    )
}

#[cfg(test)]
mod tests;
