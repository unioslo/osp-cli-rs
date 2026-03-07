use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    Path,
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
    pub value_key: bool,
    pub value_leaf: bool,
    pub children: BTreeMap<String, CompletionNode>,
    pub flags: BTreeMap<String, FlagNode>,
    pub args: Vec<ArgNode>,
    pub flag_hints: Option<FlagHints>,
}

impl CompletionNode {
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
    pub head: Vec<String>,
    pub args: Vec<String>,
    pub flags: BTreeMap<String, Vec<String>>,
    pub flag_order: Vec<String>,
    pub pipes: Vec<String>,
    pub has_pipe: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionAnalysis {
    /// Full parser output plus the cursor-local context derived from it.
    pub safe_cursor: usize,
    pub full_tokens: Vec<String>,
    pub cursor_tokens: Vec<String>,
    pub full_cmd: CommandLine,
    pub cursor_cmd: CommandLine,
    pub stub: String,
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
