//! Completion engine for OSP CLI/REPL.
//!
//! The crate is intentionally split into three pure phases:
//! - `tree`: build a plain completion tree
//! - `parse`: parse line input into a structured `CommandLine`
//! - `suggest`: generate candidates from `CommandLine + CompletionTree`
//!
//! Dynamic hints (network/provider/openapi derived) are injected by outer
//! layers (`osp-cli` / `osp-repl`) and not fetched here.

mod context;
pub(crate) mod engine;
pub(crate) mod model;
pub(crate) mod parse;
pub(crate) mod suggest;
pub(crate) mod tree;

pub use engine::CompletionEngine;
pub use model::{
    ArgNode, CommandLine, CompletionAnalysis, CompletionContext, CompletionNode, CompletionTree,
    ContextScope, CursorState, FlagNode, FlagOccurrence, MatchKind, ParsedLine, QuoteStyle,
    Suggestion, SuggestionEntry, SuggestionOutput, TailItem, ValueType,
};
pub use parse::{CommandLineParser, TokenSpan};
pub use suggest::SuggestionEngine;
pub use tree::{CommandSpec, CompletionTreeBuilder, ConfigKeySpec};
