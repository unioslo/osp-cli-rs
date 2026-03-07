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
pub mod engine;
pub mod model;
pub mod parse;
pub mod suggest;
pub mod tree;

pub use engine::CompletionEngine;
pub use model::{
    ArgNode, CommandLine, CompletionAnalysis, CompletionContext, CompletionNode, CompletionTree,
    ContextScope, CursorState, FlagNode, FlagOccurrence, MatchKind, ParsedLine, QuoteStyle,
    Suggestion, SuggestionEntry, SuggestionOutput, TailItem, ValueType,
};
pub use parse::CommandLineParser;
pub use suggest::SuggestionEngine;
pub use tree::{CommandSpec, CompletionTreeBuilder, ConfigKeySpec};
