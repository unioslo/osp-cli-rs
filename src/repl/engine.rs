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
use crate::completion::CompletionTree;
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
pub(crate) use adapter::{
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

struct ReplRunContext {
    prompt: OspPrompt,
    completion_words: Vec<String>,
    completion_tree: Option<CompletionTree>,
    appearance: ReplAppearance,
    line_projector: Option<LineProjector>,
    history_store: SharedHistory,
}

/// Runs the interactive REPL and delegates submitted lines to `execute`.
///
/// # Fallback behavior
///
/// This prefers the interactive editor loop, but it falls back to basic
/// line-by-line stdin mode when interactive assumptions do not hold. That
/// includes non-terminal stdin and terminals that do not support the cursor
/// position probe used by the editor layer.
///
/// When that fallback happens, the function emits a warning to stderr unless
/// the caller explicitly requested basic input mode through the config.
///
/// # Errors
///
/// Returns an error when the interactive editor layer fails or when the
/// supplied `execute` callback returns an error for a submitted line.
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
    let history_store = SharedHistory::new(history_config);
    let mut submission = SubmissionContext {
        history_store: &history_store,
        execute: &mut execute,
    };
    let prompt = OspPrompt::new(prompt.left, prompt.indicator, prompt_right);
    let basic_reason = basic_input_reason(input_mode);

    run_repl_with_reason(
        ReplRunContext {
            prompt,
            completion_words,
            completion_tree,
            appearance,
            line_projector,
            history_store: history_store.clone(),
        },
        basic_reason,
        &mut submission,
        run_repl_basic,
        run_repl_interactive,
    )
}

fn run_repl_with_reason<F, B, I>(
    context: ReplRunContext,
    basic_reason: Option<BasicInputReason>,
    submission: &mut SubmissionContext<'_, F>,
    mut run_basic_fn: B,
    mut run_interactive_fn: I,
) -> Result<ReplRunResult>
where
    F: FnMut(&str, &SharedHistory) -> Result<ReplLineResult>,
    B: FnMut(&OspPrompt, &mut SubmissionContext<'_, F>) -> Result<ReplRunResult>,
    I: FnMut(
        InteractiveLoopConfig<'_>,
        SharedHistory,
        &mut SubmissionContext<'_, F>,
    ) -> Result<ReplRunResult>,
{
    let ReplRunContext {
        prompt,
        completion_words,
        completion_tree,
        appearance,
        line_projector,
        history_store,
    } = context;

    if let Some(reason) = basic_reason {
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
        return run_basic_fn(&prompt, submission);
    }

    run_interactive_fn(
        InteractiveLoopConfig {
            prompt: &prompt,
            completion_words,
            completion_tree,
            appearance,
            line_projector,
        },
        history_store,
        submission,
    )
}

#[cfg(test)]
mod tests;
