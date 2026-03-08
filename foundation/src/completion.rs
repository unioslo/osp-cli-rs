//! Completion tree, engine, and suggestion model types.

pub use crate::osp_completion::{
    ArgNode, CommandLine, CommandLineParser, CommandSpec, CompletionAnalysis,
    CompletionContext, CompletionEngine, CompletionNode, CompletionTree,
    CompletionTreeBuilder, ConfigKeySpec, ContextScope, CursorState, FlagNode,
    FlagOccurrence, MatchKind, ParsedLine, QuoteStyle, Suggestion, SuggestionEngine,
    SuggestionEntry, SuggestionOutput, TailItem, TokenSpan, ValueType,
};
