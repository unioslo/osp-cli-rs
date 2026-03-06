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
    pub value: String,
    pub meta: Option<String>,
    pub display: Option<String>,
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
    pub name: Option<String>,
    pub tooltip: Option<String>,
    pub multi: bool,
    pub value_type: Option<ValueType>,
    pub suggestions: Vec<SuggestionEntry>,
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionNode {
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
    pub pipe_verbs: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CommandLine {
    pub head: Vec<String>,
    pub args: Vec<String>,
    pub flags: BTreeMap<String, Vec<String>>,
    pub flag_order: Vec<String>,
    pub pipes: Vec<String>,
    pub has_pipe: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CompletionAnalysis {
    pub safe_cursor: usize,
    pub full_tokens: Vec<String>,
    pub cursor_tokens: Vec<String>,
    pub full_cmd: CommandLine,
    pub cursor_cmd: CommandLine,
    pub stub: String,
    pub matched_path: Vec<String>,
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
