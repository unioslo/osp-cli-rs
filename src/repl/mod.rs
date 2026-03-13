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
//! A submitted line travels through the layers like this:
//!
//! ```text
//! user keystrokes
//!       │
//!       ▼ [ engine ]    reedline editor, prompt, completion/history menus
//!       │ line accepted
//!       ▼ [ dispatch ]  execute command, apply shell scope and aliases
//!       │ ReplLineResult
//!       ├── Continue     → render output, show next prompt
//!       ├── ReplaceInput → update input buffer without printing
//!       ├── Restart      → [ lifecycle ] rebuild REPL state, loop again
//!       └── Exit         → return ReplRunResult to the caller
//! ```
//!
//! Embedders drive the loop with [`crate::repl::run_repl`], configured through
//! [`crate::repl::ReplRunConfig::builder`]. The engine, dispatch, and
//! presentation layers are internal; only the config/result types cross the
//! boundary.
//!
//! Minimal embedder path:
//!
//! ```no_run
//! use anyhow::Result;
//! use osp_cli::repl::{
//!     HistoryConfig, ReplLineResult, ReplPrompt, ReplRunConfig, run_repl,
//! };
//!
//! let config = ReplRunConfig::builder(
//!     ReplPrompt::simple("osp> "),
//!     HistoryConfig::builder().build(),
//! )
//! .build();
//!
//! let _result = run_repl(config, |line, _history| -> Result<ReplLineResult> {
//!     match line.trim() {
//!         "exit" | "quit" => Ok(ReplLineResult::Exit(0)),
//!         _ => Ok(ReplLineResult::Continue(String::new())),
//!     }
//! })?;
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! Choose [`crate::app`] instead when you want the full `osp` host with config
//! loading, command dispatch, and rendering already wired together. Choose
//! this module directly when you already own the execution callback and only
//! want the interactive editor loop.
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
//! - primary host-facing entrypoints are [`crate::repl::run_repl`],
//!   [`crate::repl::ReplRunConfig`], [`crate::repl::HistoryConfig`], and
//!   [`crate::repl::ReplPrompt`]
//! - debug snapshots and inspection helpers such as
//!   [`crate::repl::CompletionDebug`] and
//!   [`crate::repl::debug_completion`] stay available without becoming the
//!   default path for ordinary embedders
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
// Primary host-facing entry points.
pub use engine::{
    HistoryConfig, HistoryConfigBuilder, HistoryEntry, HistoryShellContext, LineProjection,
    LineProjector, PromptRightRenderer, ReplAppearance, ReplAppearanceBuilder, ReplInputMode,
    ReplLineResult, ReplPrompt, ReplReloadKind, ReplRunConfig, ReplRunConfigBuilder, ReplRunResult,
    SharedHistory, color_from_style_spec, default_pipe_verbs, run_repl,
};
// Debug surfaces for REPL completion, highlight, and history inspection.
pub use engine::{
    CompletionDebug, CompletionDebugFrame, CompletionDebugMatch, CompletionDebugOptions, DebugStep,
    HighlightDebugSpan, debug_completion, debug_completion_steps, debug_highlight,
    debug_history_menu, debug_history_menu_steps,
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
pub(crate) use presentation::render_repl_prompt_right_for_test;
