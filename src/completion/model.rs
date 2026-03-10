pub use crate::core::shell_words::QuoteStyle;
use std::{collections::BTreeMap, ops::Range};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Semantic type for values completed by the engine.
pub enum ValueType {
    /// Filesystem path value.
    Path,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Replacement details for the token currently being completed.
pub struct CursorState {
    /// Normalized token text used for matching suggestions.
    pub token_stub: String,
    /// Raw slice from the input buffer that will be replaced.
    pub raw_stub: String,
    /// Byte range in the input buffer that should be replaced.
    pub replace_range: Range<usize>,
    /// Quote style active at the cursor, if the token is quoted.
    pub quote_style: Option<QuoteStyle>,
}

impl CursorState {
    /// Creates a cursor state from explicit replacement data.
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

    /// Creates a synthetic cursor state for a standalone token stub.
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
/// Scope used when merging context-only flags into the cursor view.
pub enum ContextScope {
    /// Merge the flag regardless of the matched command path.
    Global,
    /// Merge the flag only within the matched subtree.
    #[default]
    Subtree,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Suggestion payload shown to the user and inserted on accept.
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
    /// Creates a suggestion that inserts `value`.
    pub fn value(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            meta: None,
            display: None,
            sort: None,
        }
    }

    /// Sets the right-column metadata text.
    pub fn meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    /// Sets the human-friendly label shown in menus.
    pub fn display(mut self, display: impl Into<String>) -> Self {
        self.display = Some(display.into());
        self
    }

    /// Sets the hidden sort key for this suggestion.
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
/// OS version suggestions shared globally or scoped by provider.
pub struct OsVersions {
    /// Suggestions indexed by OS name across all providers.
    pub union: BTreeMap<String, Vec<SuggestionEntry>>,
    /// Suggestions indexed first by provider, then by OS name.
    pub by_provider: BTreeMap<String, BTreeMap<String, Vec<SuggestionEntry>>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Request-form hints used to derive flag and value suggestions.
pub struct RequestHints {
    /// Known request keys.
    pub keys: Vec<String>,
    /// Request keys that must be present.
    pub required: Vec<String>,
    /// Allowed values grouped by tier.
    pub tiers: BTreeMap<String, Vec<String>>,
    /// Default values by request key.
    pub defaults: BTreeMap<String, String>,
    /// Explicit value choices by request key.
    pub choices: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Request hints shared globally and overridden by provider.
pub struct RequestHintSet {
    /// Hints available regardless of provider.
    pub common: RequestHints,
    /// Provider-specific request hints.
    pub by_provider: BTreeMap<String, RequestHints>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Flag-name hints shared globally and overridden by provider.
pub struct FlagHints {
    /// Optional flags available regardless of provider.
    pub common: Vec<String>,
    /// Optional flags available for specific providers.
    pub by_provider: BTreeMap<String, Vec<String>>,
    /// Required flags available regardless of provider.
    pub required_common: Vec<String>,
    /// Required flags available for specific providers.
    pub required_by_provider: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Positional argument definition for one command slot.
pub struct ArgNode {
    /// Argument name shown in completion UIs.
    pub name: Option<String>,
    /// Optional description shown alongside the argument.
    pub tooltip: Option<String>,
    /// Whether the argument may consume multiple values.
    pub multi: bool,
    /// Semantic type for the argument value.
    pub value_type: Option<ValueType>,
    /// Suggested values for the argument.
    pub suggestions: Vec<SuggestionEntry>,
}

impl ArgNode {
    /// Creates an argument node with a visible argument name.
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    /// Sets the display tooltip for this argument.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Marks this argument as accepting multiple values.
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Sets the semantic value type for this argument.
    pub fn value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = Some(value_type);
        self
    }

    /// Replaces the suggestion list for this argument.
    pub fn suggestions(mut self, suggestions: impl IntoIterator<Item = SuggestionEntry>) -> Self {
        self.suggestions = suggestions.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Completion metadata for a flag spelling.
pub struct FlagNode {
    /// Optional description shown alongside the flag.
    pub tooltip: Option<String>,
    /// Whether the flag does not accept a value.
    pub flag_only: bool,
    /// Whether the flag may be repeated.
    pub multi: bool,
    // Context-only flags are merged from the full line into the cursor context.
    // `context_scope` controls whether merge is global or path-scoped.
    /// Whether the flag should be merged from the full line into cursor context.
    pub context_only: bool,
    /// Scope used when merging a context-only flag.
    pub context_scope: ContextScope,
    /// Semantic type for the flag value, if any.
    pub value_type: Option<ValueType>,
    /// Generic suggestions for the flag value.
    pub suggestions: Vec<SuggestionEntry>,
    /// Provider-specific value suggestions.
    pub suggestions_by_provider: BTreeMap<String, Vec<SuggestionEntry>>,
    /// Allowed providers by OS name.
    pub os_provider_map: BTreeMap<String, Vec<String>>,
    /// OS version suggestions attached to this flag.
    pub os_versions: Option<OsVersions>,
    /// Request-form hints attached to this flag.
    pub request_hints: Option<RequestHintSet>,
    /// Extra flag-name hints attached to this flag.
    pub flag_hints: Option<FlagHints>,
}

impl FlagNode {
    /// Creates an empty flag node.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the display tooltip for this flag.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Marks this flag as taking no value.
    pub fn flag_only(mut self) -> Self {
        self.flag_only = true;
        self
    }

    /// Marks this flag as repeatable.
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Marks this flag as context-only within the given scope.
    pub fn context_only(mut self, scope: ContextScope) -> Self {
        self.context_only = true;
        self.context_scope = scope;
        self
    }

    /// Sets the semantic value type for this flag.
    pub fn value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = Some(value_type);
        self
    }

    /// Replaces the suggestion list for this flag value.
    pub fn suggestions(mut self, suggestions: impl IntoIterator<Item = SuggestionEntry>) -> Self {
        self.suggestions = suggestions.into_iter().collect();
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// One node in the immutable completion tree.
pub struct CompletionNode {
    /// Optional description shown alongside the node.
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
    /// Nested subcommands or value-like children.
    pub children: BTreeMap<String, CompletionNode>,
    /// Flags visible in this command scope.
    pub flags: BTreeMap<String, FlagNode>,
    /// Positional arguments accepted in this command scope.
    pub args: Vec<ArgNode>,
    /// Extra flag-name hints contributed by this node.
    pub flag_hints: Option<FlagHints>,
}

impl CompletionNode {
    /// Sets the hidden sort key for this node.
    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.sort = Some(sort.into());
        self
    }

    /// Adds a child node keyed by command or value name.
    pub fn with_child(mut self, name: impl Into<String>, node: CompletionNode) -> Self {
        self.children.insert(name.into(), node);
        self
    }

    /// Adds a flag node keyed by its spelling.
    pub fn with_flag(mut self, name: impl Into<String>, node: FlagNode) -> Self {
        self.flags.insert(name.into(), node);
        self
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Immutable completion data consumed by the engine.
pub struct CompletionTree {
    /// Root completion node for the command hierarchy.
    pub root: CompletionNode,
    /// Pipe verbs are kept separate from the command tree because they only
    /// become visible after the parser has entered DSL mode.
    pub pipe_verbs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Parsed command-line structure before higher-level completion analysis.
pub struct CommandLine {
    /// Command path tokens matched before tail parsing starts.
    pub(crate) head: Vec<String>,
    /// Parsed flags and positional arguments after the command path.
    pub(crate) tail: Vec<TailItem>,
    /// Merged flag values keyed by spelling.
    pub(crate) flag_values: BTreeMap<String, Vec<String>>,
    /// Tokens that appear after the first pipe.
    pub(crate) pipes: Vec<String>,
    /// Whether the parser entered pipe mode.
    pub(crate) has_pipe: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// One occurrence of a flag and the values consumed with it.
pub struct FlagOccurrence {
    /// Flag spelling as it appeared in the input.
    pub name: String,
    /// Values consumed by this flag occurrence.
    pub values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Item in the parsed tail after the command path.
pub enum TailItem {
    /// A flag occurrence with any values it consumed.
    Flag(FlagOccurrence),
    /// A positional argument.
    Positional(String),
}

impl CommandLine {
    /// Returns the matched command path tokens.
    pub fn head(&self) -> &[String] {
        &self.head
    }

    /// Returns the parsed tail items after the command path.
    pub fn tail(&self) -> &[TailItem] {
        &self.tail
    }

    /// Returns tokens in the pipe segment, if present.
    pub fn pipes(&self) -> &[String] {
        &self.pipes
    }

    /// Returns whether the line entered pipe mode.
    pub fn has_pipe(&self) -> bool {
        self.has_pipe
    }

    /// Returns all merged flag values keyed by flag spelling.
    pub fn flag_values_map(&self) -> &BTreeMap<String, Vec<String>> {
        &self.flag_values
    }

    /// Returns values collected for one flag spelling.
    pub fn flag_values(&self, name: &str) -> Option<&[String]> {
        self.flag_values.get(name).map(Vec::as_slice)
    }

    /// Returns whether the command line contains the flag spelling.
    pub fn has_flag(&self, name: &str) -> bool {
        self.flag_values.contains_key(name)
    }

    /// Iterates over flag occurrences in input order.
    pub fn flag_occurrences(&self) -> impl Iterator<Item = &FlagOccurrence> {
        self.tail.iter().filter_map(|item| match item {
            TailItem::Flag(flag) => Some(flag),
            TailItem::Positional(_) => None,
        })
    }

    /// Returns the last flag occurrence, if any.
    pub fn last_flag_occurrence(&self) -> Option<&FlagOccurrence> {
        self.flag_occurrences().last()
    }

    /// Iterates over positional arguments in the tail.
    pub fn positional_args(&self) -> impl Iterator<Item = &String> {
        self.tail.iter().filter_map(|item| match item {
            TailItem::Positional(value) => Some(value),
            TailItem::Flag(_) => None,
        })
    }

    /// Returns the number of tail items.
    pub fn tail_len(&self) -> usize {
        self.tail.len()
    }

    /// Appends a flag occurrence and merges its values into the lookup map.
    pub fn push_flag_occurrence(&mut self, occurrence: FlagOccurrence) {
        self.flag_values
            .entry(occurrence.name.clone())
            .or_default()
            .extend(occurrence.values.iter().cloned());
        self.tail.push(TailItem::Flag(occurrence));
    }

    /// Appends a positional argument to the tail.
    pub fn push_positional(&mut self, value: impl Into<String>) {
        self.tail.push(TailItem::Positional(value.into()));
    }

    /// Merges additional values for a flag spelling.
    pub fn merge_flag_values(&mut self, name: impl Into<String>, values: Vec<String>) {
        self.flag_values
            .entry(name.into())
            .or_default()
            .extend(values);
    }

    /// Inserts positional values ahead of the existing tail.
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

    /// Marks the command line as piped and stores the pipe tokens.
    pub fn set_pipe(&mut self, pipes: Vec<String>) {
        self.has_pipe = true;
        self.pipes = pipes;
    }

    /// Appends one segment to the command path.
    pub fn push_head(&mut self, segment: impl Into<String>) {
        self.head.push(segment.into());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Parser output for the full line and the cursor-local prefix.
pub struct ParsedLine {
    /// Cursor offset clamped to a valid UTF-8 boundary.
    pub safe_cursor: usize,
    /// Tokens parsed from the full line.
    pub full_tokens: Vec<String>,
    /// Tokens parsed from the line prefix before the cursor.
    pub cursor_tokens: Vec<String>,
    /// Parsed command-line structure for the full line.
    pub full_cmd: CommandLine,
    /// Parsed command-line structure for the prefix before the cursor.
    pub cursor_cmd: CommandLine,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Full completion analysis derived from parsing and context resolution.
pub struct CompletionAnalysis {
    /// Full parser output plus the cursor-local context derived from it.
    pub parsed: ParsedLine,
    /// Replacement details for the active token.
    pub cursor: CursorState,
    /// Resolved command context used for suggestion generation.
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
    /// Command path matched before the cursor.
    pub matched_path: Vec<String>,
    /// Command path that contributes visible flags.
    pub flag_scope_path: Vec<String>,
    /// Whether the cursor is completing a subcommand name.
    pub subcommand_context: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// High-level classification for a completion candidate.
pub enum MatchKind {
    /// Candidate belongs to pipe-mode completion.
    Pipe,
    /// Candidate is a flag spelling.
    Flag,
    /// Candidate is a top-level command.
    Command,
    /// Candidate is a nested subcommand.
    Subcommand,
    /// Candidate is a value or positional suggestion.
    Value,
}

impl MatchKind {
    /// Returns the stable string form used by presentation layers.
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
/// Ranked suggestion ready for formatting or rendering.
pub struct Suggestion {
    /// Text inserted into the buffer if accepted.
    pub text: String,
    /// Short metadata shown alongside the suggestion.
    pub meta: Option<String>,
    /// Optional human-friendly label.
    pub display: Option<String>,
    /// Whether the suggestion exactly matches the current stub.
    pub is_exact: bool,
    /// Hidden sort key for ordering.
    pub sort: Option<String>,
    /// Numeric score used for ranking.
    pub match_score: u32,
}

impl Suggestion {
    /// Creates a suggestion with default ranking metadata.
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
/// Output emitted by the suggestion engine.
pub enum SuggestionOutput {
    /// A normal suggestion item.
    Item(Suggestion),
    /// Sentinel indicating that filesystem path completion should run next.
    PathSentinel,
}

#[cfg(test)]
mod tests;
