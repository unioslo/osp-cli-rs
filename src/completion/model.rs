pub use crate::core::shell_words::QuoteStyle;
use std::{collections::BTreeMap, ops::Range};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorState {
    pub token_stub: String,
    pub raw_stub: String,
    pub replace_range: Range<usize>,
    pub quote_style: Option<QuoteStyle>,
}

impl CursorState {
    pub fn new(
        token_stub: impl Into<String>,
        raw_stub: impl Into<String>,
        replace_range: Range<usize>,
        quote_style: Option<QuoteStyle>,
    ) -> Self {
        Self {
            token_stub: token_stub.into(),
            raw_stub: raw_stub.into(),
            replace_range,
            quote_style,
        }
    }

    pub fn synthetic(token_stub: impl Into<String>) -> Self {
        let token_stub = token_stub.into();
        let len = token_stub.len();
        Self {
            raw_stub: token_stub.clone(),
            token_stub,
            replace_range: 0..len,
            quote_style: None,
        }
    }
}

impl Default for CursorState {
    fn default() -> Self {
        Self::synthetic("")
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContextScope {
    Global,
    #[default]
    Subtree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionEntry {
    /// Text inserted into the buffer if this suggestion is accepted.
    pub value: String,
    /// Short right-column description in menu-style UIs.
    pub meta: Option<String>,
    /// Optional human-friendly label when the inserted value should stay terse.
    pub display: Option<String>,
    /// Hidden sort key for cases where display order should differ from labels.
    pub sort: Option<String>,
}

impl SuggestionEntry {
    pub fn value(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            meta: None,
            display: None,
            sort: None,
        }
    }

    pub fn meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    pub fn display(mut self, display: impl Into<String>) -> Self {
        self.display = Some(display.into());
        self
    }

    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.sort = Some(sort.into());
        self
    }
}

impl From<&str> for SuggestionEntry {
    fn from(value: &str) -> Self {
        Self::value(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsVersions {
    pub union: BTreeMap<String, Vec<SuggestionEntry>>,
    pub by_provider: BTreeMap<String, BTreeMap<String, Vec<SuggestionEntry>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestHints {
    pub keys: Vec<String>,
    pub required: Vec<String>,
    pub tiers: BTreeMap<String, Vec<String>>,
    pub defaults: BTreeMap<String, String>,
    pub choices: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RequestHintSet {
    pub common: RequestHints,
    pub by_provider: BTreeMap<String, RequestHints>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagHints {
    pub common: Vec<String>,
    pub by_provider: BTreeMap<String, Vec<String>>,
    pub required_common: Vec<String>,
    pub required_by_provider: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArgNode {
    /// Positional-argument metadata for one command slot.
    pub name: Option<String>,
    pub tooltip: Option<String>,
    pub multi: bool,
    pub value_type: Option<ValueType>,
    pub suggestions: Vec<SuggestionEntry>,
}

impl ArgNode {
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    pub fn value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = Some(value_type);
        self
    }

    pub fn suggestions(mut self, suggestions: impl IntoIterator<Item = SuggestionEntry>) -> Self {
        self.suggestions = suggestions.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagNode {
    pub tooltip: Option<String>,
    pub flag_only: bool,
    pub multi: bool,
    // Context-only flags are merged from the full line into the cursor context.
    // `context_scope` controls whether merge is global or path-scoped.
    pub context_only: bool,
    pub context_scope: ContextScope,
    pub value_type: Option<ValueType>,
    pub suggestions: Vec<SuggestionEntry>,
    pub suggestions_by_provider: BTreeMap<String, Vec<SuggestionEntry>>,
    pub os_provider_map: BTreeMap<String, Vec<String>>,
    pub os_versions: Option<OsVersions>,
    pub request_hints: Option<RequestHintSet>,
    pub flag_hints: Option<FlagHints>,
}

impl FlagNode {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn flag_only(mut self) -> Self {
        self.flag_only = true;
        self
    }

    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    pub fn context_only(mut self, scope: ContextScope) -> Self {
        self.context_only = true;
        self.context_scope = scope;
        self
    }

    pub fn value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = Some(value_type);
        self
    }

    pub fn suggestions(mut self, suggestions: impl IntoIterator<Item = SuggestionEntry>) -> Self {
        self.suggestions = suggestions.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionNode {
    /// One command/subcommand scope in the completion tree.
    ///
    /// A node can expose child commands, flags, positional arguments, or
    /// value-like leaves for config-style key completion.
    pub tooltip: Option<String>,
    /// Optional suggestion-order hint for command/subcommand completion.
    pub sort: Option<String>,
    /// This node expects the next token to be a key chosen from `children`.
    pub value_key: bool,
    /// This node is itself a terminal value that can be suggested/accepted.
    pub value_leaf: bool,
    /// Hidden context flags injected when this node is matched.
    pub prefilled_flags: BTreeMap<String, Vec<String>>,
    /// Fixed positional values contributed before user-provided args.
    pub prefilled_positionals: Vec<String>,
    pub children: BTreeMap<String, CompletionNode>,
    pub flags: BTreeMap<String, FlagNode>,
    pub args: Vec<ArgNode>,
    pub flag_hints: Option<FlagHints>,
}

impl CompletionNode {
    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.sort = Some(sort.into());
        self
    }

    pub fn with_child(mut self, name: impl Into<String>, node: CompletionNode) -> Self {
        self.children.insert(name.into(), node);
        self
    }

    pub fn with_flag(mut self, name: impl Into<String>, node: FlagNode) -> Self {
        self.flags.insert(name.into(), node);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionTree {
    pub root: CompletionNode,
    /// Pipe verbs are kept separate from the command tree because they only
    /// become visible after the parser has entered DSL mode.
    pub pipe_verbs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandLine {
    /// Parsed command-line shape before completion-specific analysis.
    ///
    /// `head` is the command path, `flags` and `args` are the option/positional
    /// tail, and `pipes` contains the first pipeline segment onward.
    pub(crate) head: Vec<String>,
    pub(crate) tail: Vec<TailItem>,
    pub(crate) flag_values: BTreeMap<String, Vec<String>>,
    pub(crate) pipes: Vec<String>,
    pub(crate) has_pipe: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagOccurrence {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TailItem {
    Flag(FlagOccurrence),
    Positional(String),
}

impl CommandLine {
    pub fn head(&self) -> &[String] {
        &self.head
    }

    pub fn tail(&self) -> &[TailItem] {
        &self.tail
    }

    pub fn pipes(&self) -> &[String] {
        &self.pipes
    }

    pub fn has_pipe(&self) -> bool {
        self.has_pipe
    }

    pub fn flag_values_map(&self) -> &BTreeMap<String, Vec<String>> {
        &self.flag_values
    }

    pub fn flag_values(&self, name: &str) -> Option<&[String]> {
        self.flag_values.get(name).map(Vec::as_slice)
    }

    pub fn has_flag(&self, name: &str) -> bool {
        self.flag_values.contains_key(name)
    }

    pub fn flag_occurrences(&self) -> impl Iterator<Item = &FlagOccurrence> {
        self.tail.iter().filter_map(|item| match item {
            TailItem::Flag(flag) => Some(flag),
            TailItem::Positional(_) => None,
        })
    }

    pub fn last_flag_occurrence(&self) -> Option<&FlagOccurrence> {
        self.flag_occurrences().last()
    }

    pub fn positional_args(&self) -> impl Iterator<Item = &String> {
        self.tail.iter().filter_map(|item| match item {
            TailItem::Positional(value) => Some(value),
            TailItem::Flag(_) => None,
        })
    }

    pub fn tail_len(&self) -> usize {
        self.tail.len()
    }

    pub fn push_flag_occurrence(&mut self, occurrence: FlagOccurrence) {
        self.flag_values
            .entry(occurrence.name.clone())
            .or_default()
            .extend(occurrence.values.iter().cloned());
        self.tail.push(TailItem::Flag(occurrence));
    }

    pub fn push_positional(&mut self, value: impl Into<String>) {
        self.tail.push(TailItem::Positional(value.into()));
    }

    pub fn merge_flag_values(&mut self, name: impl Into<String>, values: Vec<String>) {
        self.flag_values
            .entry(name.into())
            .or_default()
            .extend(values);
    }

    pub fn prepend_positional_values(&mut self, values: impl IntoIterator<Item = String>) {
        let mut values = values
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .map(TailItem::Positional)
            .collect::<Vec<_>>();
        if values.is_empty() {
            return;
        }
        values.extend(std::mem::take(&mut self.tail));
        self.tail = values;
    }

    pub fn set_pipe(&mut self, pipes: Vec<String>) {
        self.has_pipe = true;
        self.pipes = pipes;
    }

    pub fn push_head(&mut self, segment: impl Into<String>) {
        self.head.push(segment.into());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedLine {
    pub safe_cursor: usize,
    pub full_tokens: Vec<String>,
    pub cursor_tokens: Vec<String>,
    pub full_cmd: CommandLine,
    pub cursor_cmd: CommandLine,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionAnalysis {
    /// Full parser output plus the cursor-local context derived from it.
    pub parsed: ParsedLine,
    pub cursor: CursorState,
    pub context: CompletionContext,
}

/// Resolved completion state for the cursor position.
///
/// The parser only knows about tokens. This structure captures the derived
/// command context the suggester/debug layers actually care about:
/// which command path matched, which node contributes visible flags, and
/// whether the cursor is still in subcommand-selection mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionContext {
    pub matched_path: Vec<String>,
    pub flag_scope_path: Vec<String>,
    pub subcommand_context: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Pipe,
    Flag,
    Command,
    Subcommand,
    Value,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::Flag => "flag",
            Self::Command => "command",
            Self::Subcommand => "subcommand",
            Self::Value => "value",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub text: String,
    pub meta: Option<String>,
    pub display: Option<String>,
    pub is_exact: bool,
    pub sort: Option<String>,
    pub match_score: u32,
}

impl Suggestion {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            meta: None,
            display: None,
            is_exact: false,
            sort: None,
            match_score: u32::MAX,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SuggestionOutput {
    Item(Suggestion),
    PathSentinel,
}

#[cfg(test)]
mod tests;
