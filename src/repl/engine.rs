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

use std::sync::atomic::{AtomicBool, Ordering};

pub use super::highlight::debug_highlight;
pub use super::history_store::{HistoryConfig, HistoryEntry, HistoryShellContext, SharedHistory};
use crate::completion::CompletionTree;
use anyhow::Result;

mod adapter;
mod config;
mod debug;
mod editor;
mod hint;
mod overlay;
mod session;

#[cfg(test)]
use crate::ui::prompt::parse_cursor_position_report;
pub(crate) use adapter::{
    CompletionTraceEvent, CompletionTraceMenuState, trace_completion, trace_completion_enabled,
};
#[cfg(test)]
pub(crate) use adapter::{
    ReplCompleter, build_repl_highlighter, path_suggestions, split_path_stub,
};
pub use adapter::{color_from_style_spec, default_pipe_verbs};
pub use config::{
    LineProjection, LineProjector, PromptRightRenderer, ReplAppearance, ReplCompleteOn,
    ReplInputMode, ReplPrompt, ReplReloadKind, ReplResult, ReplRunConfig,
};
pub use debug::{CompletionDebugOptions, DebugStep, debug_completion, debug_completion_steps};
#[cfg(test)]
use editor::AutoCompleteEmacs;
pub(crate) use editor::{BasicInputReason, OspPrompt, basic_input_reason};
#[cfg(test)]
use overlay::{build_history_picker_options, history_picker_items};
#[cfg(test)]
use session::evaluate_repl_submission;
pub(crate) use session::read_path;
use session::{InteractiveLoopConfig, SubmissionContext, run_repl_basic, run_repl_interactive};

const HOST_COMMAND_CLEAR_SCREEN: &str = "\0osp-repl-clear-screen";
const COMPLETION_MENU_NAME: &str = "completion_menu";
const HOST_COMMAND_LAST_ARGUMENT: &str = "\0osp-repl-last-argument";
const HOST_COMMAND_PREVIOUS_ARGUMENT: &str = "\0osp-repl-previous-argument";
const HOST_COMMAND_HISTORY_PICKER: &str = "\u{0}osp-repl-history-picker";
/// Page keys leave reedline through private host commands so they can step
/// paginated results with an empty line and edit normally otherwise.
const HOST_COMMAND_PAGE_PREVIOUS: &str = "\u{0}osp-repl-page-previous";
const HOST_COMMAND_PAGE_NEXT: &str = "\u{0}osp-repl-page-next";

struct ReplRunContext {
    prompt: OspPrompt,
    completion_tree: CompletionTree,
    appearance: ReplAppearance,
    complete_on: ReplCompleteOn,
    autosuggest: bool,
    line_projector: Option<LineProjector>,
    history_store: SharedHistory,
    actions: Vec<crate::repl::ReplAction>,
    on_start: Option<std::sync::Arc<dyn Fn(Option<crate::repl::ReplSession>) + Send + Sync>>,
}

/// Runs the interactive REPL and delegates submitted lines to `execute`.
///
/// # Fallback behavior
///
/// This prefers the interactive editor loop, but it falls back to basic
/// line-by-line stdin mode when stdin or stdout is not a terminal, with a
/// warning on stderr, or when the config explicitly requests basic input. A
/// terminal always gets the full editor, however slowly it answers.
///
/// # Errors
///
/// Returns an error when the editor or history backend fails. Errors from
/// `execute` are printed to stderr and the session continues.
///
/// Only [`ReplResult::Exit`] and [`ReplResult::Restart`] leave the editor loop;
/// the editor applies the other actions before reading another line.
pub fn run_repl<F>(config: ReplRunConfig, mut execute: F) -> Result<ReplResult>
where
    F: FnMut(crate::repl::ReplEvent<'_>, &SharedHistory) -> Result<ReplResult>,
{
    crate::startup_trace::initialize();
    let ReplRunConfig {
        prompt,
        completion_tree,
        appearance,
        history_config,
        input_mode,
        complete_on,
        autosuggest,
        prompt_right,
        line_projector,
        actions,
        on_start,
        exit_on_escape,
    } = config;
    let history_store = SharedHistory::new(history_config);
    let mut submission = SubmissionContext {
        root_input: false,
        exit_on_escape,
        history_store: &history_store,
        execute: &mut execute,
    };
    let prompt = OspPrompt::new(prompt.left, prompt.indicator, prompt_right);
    let basic_reason = basic_input_reason(input_mode);

    run_repl_with_reason(
        ReplRunContext {
            prompt,
            completion_tree,
            appearance,
            complete_on,
            autosuggest,
            line_projector,
            history_store: history_store.clone(),
            actions,
            on_start,
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
) -> Result<ReplResult>
where
    F: FnMut(crate::repl::ReplEvent<'_>, &SharedHistory) -> Result<ReplResult>,
    B: FnMut(
        &OspPrompt,
        &mut SubmissionContext<'_, F>,
        Box<dyn FnMut() + Send>,
    ) -> Result<ReplResult>,
    I: FnMut(
        InteractiveLoopConfig<'_>,
        SharedHistory,
        &mut SubmissionContext<'_, F>,
    ) -> Result<ReplResult>,
{
    let ReplRunContext {
        prompt,
        completion_tree,
        appearance,
        complete_on,
        autosuggest,
        line_projector,
        history_store,
        actions,
        on_start,
    } = context;

    let owner = basic_reason
        .is_none()
        .then(crate::repl::notices::SessionOwner::new);
    let session = owner.as_ref().map(|owner| owner.0.clone());
    let mut on_start = on_start;
    let after_paint = Box::new(move || {
        crate::startup_trace::first_prompt_rendered();
        if let Some(start) = on_start.take() {
            start(session.clone());
        }
    });
    if let Some(reason) = basic_reason {
        warn_basic_input_once(reason);
        return run_basic_fn(&prompt, submission, after_paint);
    }

    run_interactive_fn(
        InteractiveLoopConfig {
            prompt: &prompt,
            completion_tree,
            appearance,
            complete_on,
            autosuggest,
            line_projector,
            actions,
            after_paint,
        },
        history_store,
        submission,
    )
}

static BASIC_INPUT_WARNING_SHOWN: AtomicBool = AtomicBool::new(false);

fn warn_basic_input_once(reason: BasicInputReason) {
    if reason == BasicInputReason::Explicit
        || BASIC_INPUT_WARNING_SHOWN.swap(true, Ordering::Relaxed)
    {
        return;
    }
    match reason {
        BasicInputReason::NotATerminal => {
            eprintln!("warning: Input is not a terminal; using basic input mode.");
        }
        BasicInputReason::Explicit => {}
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;
