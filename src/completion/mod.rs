//! Completion exists to turn a partially typed line plus cursor position into a
//! ranked suggestion set.
//!
//! This module stays deliberately free of terminal state, network access, and
//! REPL/editor concerns. The core flow is:
//!
//! - `tree`: build a static command/config completion tree
//! - `parse`: tokenize and analyze a partially typed command line
//! - `suggest`: rank and shape suggestions from the parsed cursor context
//!
//! Outer layers such as [`crate::cli`] and [`crate::repl`] inject dynamic
//! command catalogs, shell scope, alias expansion, and live prompt behavior on
//! top of this pure engine.
//!
//! Contract:
//!
//! - completion logic may depend on structured command metadata and cursor
//!   state
//! - it should not depend on terminal painting, network I/O, plugin process
//!   execution, or interactive host state
//!
//! Public API shape:
//!
//! - `tree` exposes builder/factory-style entrypoints such as
//!   [`crate::completion::CompletionTreeBuilder`] and
//!   [`crate::completion::CommandSpec`]
//! - `model` stays mostly plain semantic data so parsers, suggesters, and
//!   embedders can exchange completion state without hauling builder objects
//!   around
//! - terminal/editor integration belongs in outer layers like [`crate::repl`]

mod context;
/// High-level orchestration that combines parsing, context resolution, and suggestion ranking.
pub mod engine;
/// Shared completion data structures passed between the parser and suggester.
pub mod model;
/// Tokenization and cursor-aware command-line parsing.
pub mod parse;
/// Suggestion ranking and output shaping from parsed cursor state.
pub mod suggest;
/// Declarative completion-tree builders derived from command and config metadata.
pub mod tree;

pub use engine::CompletionEngine;
pub use model::{
    ArgNode, CommandLine, CompletionAnalysis, CompletionContext, CompletionNode, CompletionRequest,
    CompletionTree, ContextScope, CursorState, FlagHints, FlagNode, FlagOccurrence, MatchKind,
    OsVersions, ParsedLine, QuoteStyle, RequestHintSet, RequestHints, Suggestion, SuggestionEntry,
    SuggestionOutput, TailItem, ValueType,
};
pub use parse::{CommandLineParser, ParsedCursorLine, TokenSpan};
pub use suggest::SuggestionEngine;
pub use tree::{CommandSpec, CompletionTreeBuildError, CompletionTreeBuilder, ConfigKeySpec};
