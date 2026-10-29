//! Data structures shared across completion parsing, analysis, and ranking.
//!
//! This module exists to give the completion engine a stable vocabulary for
//! cursor state, command-line structure, command-tree metadata, and ranked
//! suggestions. The parser and suggester can evolve independently as long as
//! they keep exchanging these values.
//!
//! Contract:
//!
//! - types here should stay pure data and small helpers
//! - this layer may depend on shell tokenization details, but not on terminal
//!   painting or REPL host state
//! - public builders should describe the stable completion contract, not
//!   internal parser quirks

pub use crate::core::shell_words::QuoteStyle;
use std::{collections::BTreeMap, ops::Range};

/// Semantic type for values completed by the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueType {
    /// Filesystem path value.
    Path,
}

/// Replacement details for the token currently being completed.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    ///
    /// `raw_stub` keeps the exact buffer slice that will be replaced, while
    /// `token_stub` keeps the normalized text used for matching.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{CursorState, QuoteStyle};
    ///
    /// let state = CursorState::new("tok", "\"tok", 3..7, Some(QuoteStyle::Double));
    ///
    /// assert_eq!(state.token_stub, "tok");
    /// assert_eq!(state.raw_stub, "\"tok");
    /// assert_eq!(state.replace_range, 3..7);
    /// assert_eq!(state.quote_style, Some(QuoteStyle::Double));
    /// ```
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
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
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

/// Scope used when merging context-only flags into the cursor view.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContextScope {
    /// Merge the flag regardless of the matched command path.
    Global,
    /// Merge the flag only within the matched subtree.
    #[default]
    Subtree,
}

/// Suggestion payload shown to the user and inserted on accept.
///
/// This separates the inserted value from the optional display label so menu
/// UIs can stay human-friendly without changing what lands in the buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct SuggestionEntry {
    /// Text inserted into the buffer if this suggestion is accepted.
    pub value: String,
    /// Short right-column description, searched separately from the value.
    pub meta: Option<String>,
    /// Optional human-friendly label when the inserted value should stay terse.
    pub display: Option<String>,
    /// Hidden sort key for cases where display order should differ from labels.
    pub sort: Option<String>,
    /// Other spellings that select this value.
    ///
    /// Aliases are matched against typed input but never listed on their
    /// own; accepting a match inserts `value`.
    pub aliases: Vec<String>,
}

impl SuggestionEntry {
    /// Creates a suggestion that inserts `value`.
    pub fn value(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            meta: None,
            display: None,
            sort: None,
            aliases: Vec::new(),
        }
    }

    /// Sets the right-column metadata text.
    ///
    /// If omitted, menu-style UIs show no metadata for this suggestion.
    pub fn meta(mut self, meta: impl Into<String>) -> Self {
        self.meta = Some(meta.into());
        self
    }

    /// Sets the hidden sort key for this suggestion.
    ///
    /// If omitted, the suggestion carries no explicit sort hint.
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

/// Flag-name visibility, ordering and requiredness hints.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagHints {
    /// Ordinary allowed flag spellings.
    pub common: Vec<String>,
    /// Allowed flag spellings ranked before ordinary flags.
    pub preferred: Vec<String>,
    /// Additional allowed spellings, ranked after ordinary flags.
    pub other: Vec<String>,
    /// Required flag spellings, ranked before optional flags.
    pub required_common: Vec<String>,
}

/// Context-dependent flag and positional metadata returned before suggestion ranking.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlagCompletions {
    /// Replaces flag-name hints when present; otherwise keeps static hints.
    pub hints: Option<FlagHints>,
    /// Replaces suggestions for these flags, including with an empty list.
    /// Path and large-catalog completion retain their usual precedence.
    pub values: BTreeMap<String, Vec<SuggestionEntry>>,
    /// Replaces suggestions at zero-based slots in [`CompletionNode::args`].
    /// Omitted slots retain static suggestions; an empty list suppresses them.
    /// A variadic slot uses the same index for every repeated value. Unknown
    /// slots are ignored. Path and large-catalog completion keep precedence.
    pub positionals: BTreeMap<usize, Vec<SuggestionEntry>>,
    /// Ordered catalogue tiers replacing the catalogues of these flags.
    pub flag_catalogs: BTreeMap<String, Vec<ValueCatalog>>,
    /// Ordered catalogue tiers replacing the catalogues of these argument slots.
    pub positional_catalogs: BTreeMap<usize, Vec<ValueCatalog>>,
}

/// A local completion callback shared by tree clones.
///
/// Called with the resolved cursor command, including merged context flags.
/// While completing a flag value, the active value is excluded from
/// [`CommandLine::flag_values`] and [`CommandLine::flag_values_map`] so it
/// cannot constrain its own suggestions. Callbacks must not perform network I/O.
/// Equality compares callback identity, like [`ValueCatalog`].
#[derive(Clone)]
pub struct DynamicCompletion(std::sync::Arc<dyn Fn(&CommandLine) -> FlagCompletions + Send + Sync>);

impl DynamicCompletion {
    /// Supplies context-dependent flag hints and flag or positional suggestions.
    ///
    /// The tree keeps its static grammar for parsing. The callback replaces
    /// only suggestion metadata, and may return empty value lists to suppress
    /// static suggestions. Positional overrides use zero-based argument slots,
    /// including repeated values of a variadic slot. Omitted flags and slots keep
    /// their static suggestions. Flags outside a returned hint list are hidden.
    /// Called during editing, highlighting and menu painting: read local data
    /// only and never wait on locks held across remote I/O.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{
    ///     CompletionNode, DynamicCompletion, FlagCompletions, FlagNode, SuggestionEntry,
    /// };
    /// use std::collections::BTreeMap;
    ///
    /// let mut command = CompletionNode::new("deploy")
    ///     .flag("--region", FlagNode::new())
    ///     .flag("--image", FlagNode::new());
    /// command.dynamic = Some(DynamicCompletion::new(|line| {
    ///     let image = if line.has_flag("--region") { "local" } else { "default" };
    ///     FlagCompletions {
    ///         values: BTreeMap::from([("--image".into(), vec![SuggestionEntry::value(image)])]),
    ///         ..FlagCompletions::default()
    ///     }
    /// }));
    /// ```
    pub fn new(complete: impl Fn(&CommandLine) -> FlagCompletions + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(complete))
    }

    pub(super) fn resolve(&self, cmd: &CommandLine, node: &CompletionNode) -> CompletionNode {
        let completion = (self.0)(cmd);
        let mut node = node.clone();
        if let Some(hints) = completion.hints {
            node.flag_hints = Some(hints);
        }
        for (flag, entries) in completion.values {
            if let Some(flag) = node.flags.get_mut(&flag) {
                flag.suggestions = entries;
            }
        }
        for (slot, entries) in completion.positionals {
            if let Some(arg) = node.args.get_mut(slot) {
                arg.suggestions = entries;
            }
        }
        for (flag, catalogs) in completion.flag_catalogs {
            if let Some(flag) = node.flags.get_mut(&flag) {
                flag.catalogs = catalogs;
            }
        }
        for (slot, catalogs) in completion.positional_catalogs {
            if let Some(arg) = node.args.get_mut(slot) {
                arg.catalogs = catalogs;
            }
        }
        node
    }
}

impl std::fmt::Debug for DynamicCompletion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DynamicCompletion(..)")
    }
}

impl PartialEq for DynamicCompletion {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for DynamicCompletion {}

/// Resolves an alternate command word to its canonical child name.
/// Callbacks must use local grammar only; they never add menu entries.
#[derive(Clone)]
pub struct CommandWordResolver(std::sync::Arc<ResolveCommandWord>);

type ResolveCommandWord = dyn Fn(&str) -> Option<String> + Send + Sync;

impl CommandWordResolver {
    pub(crate) fn new(resolve: impl Fn(&str) -> Option<String> + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(resolve))
    }
}

impl std::fmt::Debug for CommandWordResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("CommandWordResolver(..)")
    }
}

impl PartialEq for CommandWordResolver {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for CommandWordResolver {}

/// Positional argument definition for one command slot.
///
/// This is declarative completion metadata, not parser state. One `ArgNode`
/// says what a command slot expects once command-path resolution has reached the
/// owning node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub struct ArgNode {
    /// Argument name shown in completion UIs.
    pub name: Option<String>,
    /// Optional description shown alongside the argument.
    pub tooltip: Option<String>,
    /// Whether this position must be filled before a following subcommand.
    pub required: bool,
    /// Whether the argument may consume multiple values.
    pub multi: bool,
    /// Semantic type for the argument value.
    pub value_type: Option<ValueType>,
    /// Suggested values for the argument.
    pub suggestions: Vec<SuggestionEntry>,
    /// Large shared catalogues searched in order; a later one is used only
    /// when earlier ones have no match (e.g. your VMs, then every VM).
    pub catalogs: Vec<ValueCatalog>,
}

impl ArgNode {
    pub(crate) fn accepts_value(&self, value: &str) -> bool {
        self.suggestions.is_empty()
            || self.suggestions.iter().any(|entry| {
                entry.value == value || entry.aliases.iter().any(|alias| alias == value)
            })
    }

    /// Creates an argument node with a visible argument name.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{ArgNode, SuggestionEntry, ValueType};
    ///
    /// let entry = SuggestionEntry {
    ///     display: Some("AlmaLinux".into()),
    ///     ..SuggestionEntry::value("alma").meta("linux").sort("02")
    /// };
    /// let arg = ArgNode::named("image")
    ///     .tooltip("Image name")
    ///     .multi()
    ///     .value_type(ValueType::Path)
    ///     .suggestions([entry.clone()]);
    ///
    /// assert_eq!(entry.meta.as_deref(), Some("linux"));
    /// assert_eq!(entry.display.as_deref(), Some("AlmaLinux"));
    /// assert_eq!(entry.sort.as_deref(), Some("02"));
    /// assert_eq!(arg.name.as_deref(), Some("image"));
    /// assert_eq!(arg.tooltip.as_deref(), Some("Image name"));
    /// assert!(arg.multi);
    /// assert_eq!(arg.value_type, Some(ValueType::Path));
    /// assert_eq!(arg.suggestions, vec![entry]);
    /// ```
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: Some(name.into()),
            ..Self::default()
        }
    }

    /// Sets the display tooltip for this argument.
    ///
    /// If omitted, completion UIs show no description for this argument.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Marks this argument as accepting multiple values.
    ///
    /// If omitted, the argument accepts a single value.
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Sets the semantic value type for this argument.
    ///
    /// If omitted, the argument carries no special value-type hint.
    pub fn value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = Some(value_type);
        self
    }

    /// Replaces the suggestion list for this argument.
    ///
    /// If omitted, the argument contributes no direct value suggestions.
    pub fn suggestions(mut self, suggestions: impl IntoIterator<Item = SuggestionEntry>) -> Self {
        self.suggestions = suggestions.into_iter().collect();
        self
    }
}

/// A large, locally refreshed value catalogue shared by completion tree clones.
/// Values are sorted once on replacement; refresh outside the editing path,
/// without network I/O here. Clones observe the same replacements. Equality
/// compares shared catalogue identity, not the current entries. Ranking lives
/// in completion suggestion code, which reads the values through [`Self::scan`].
#[derive(Debug, Clone, Default)]
pub struct ValueCatalog {
    entries: std::sync::Arc<std::sync::RwLock<Vec<SuggestionEntry>>>,
    pub(super) matcher: Option<fn(&SuggestionEntry, &str) -> bool>,
    before_read: std::sync::Arc<CatalogReadHook>,
}

type CatalogLoader = std::sync::Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct CatalogReadHook(std::sync::RwLock<Option<CatalogLoader>>);

impl std::fmt::Debug for CatalogReadHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CatalogReadHook")
    }
}

impl PartialEq for ValueCatalog {
    fn eq(&self, other: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.entries, &other.entries)
    }
}
impl Eq for ValueCatalog {}

impl ValueCatalog {
    /// Installs a shared local snapshot loader, invoked before reading entries.
    /// The hook must not perform network I/O or recursively read this catalogue.
    /// Use weak references when the loader's owner also holds the catalogue.
    pub fn set_before_read(&self, load: impl Fn() + Send + Sync + 'static) {
        *self
            .before_read
            .0
            .write()
            .unwrap_or_else(|err| err.into_inner()) = Some(std::sync::Arc::new(load));
    }

    /// Uses a local semantic matcher instead of fuzzy matching and typo rescue.
    /// The matcher receives the entry and case-folded stub; it must not do I/O.
    pub fn with_matcher(matcher: fn(&SuggestionEntry, &str) -> bool) -> Self {
        Self {
            matcher: Some(matcher),
            ..Self::default()
        }
    }

    /// Atomically replaces the catalogue, sorting and removing duplicate values.
    pub fn replace(&self, values: Vec<String>) {
        self.replace_entries(values.into_iter().map(SuggestionEntry::value).collect());
    }

    /// Replaces values together with their menu labels and matching aliases.
    pub fn replace_entries(&self, mut entries: Vec<SuggestionEntry>) {
        entries.sort_unstable_by(|a, b| a.value.cmp(&b.value));
        entries.dedup_by(|a, b| a.value == b.value);
        *self.entries.write().unwrap_or_else(|err| err.into_inner()) = entries;
    }

    /// Returns lexicographically ordered prefix matches, bounded before cloning.
    pub fn matching(&self, prefix: &str, limit: usize) -> Vec<String> {
        self.scan(|entries| {
            let start = entries.partition_point(|entry| entry.value.as_str() < prefix);
            entries[start..]
                .iter()
                .take_while(|entry| entry.value.starts_with(prefix))
                .take(limit)
                .map(|entry| entry.value.clone())
                .collect()
        })
    }

    /// Reports exact membership without copying the catalogue.
    pub fn contains(&self, value: &str) -> bool {
        self.scan(|entries| {
            entries
                .binary_search_by(|entry| entry.value.as_str().cmp(value))
                .is_ok()
        })
    }

    /// Reads the sorted entries in place; callers copy only what they keep.
    pub fn scan<R>(&self, read: impl FnOnce(&[SuggestionEntry]) -> R) -> R {
        let load = self
            .before_read
            .0
            .read()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if let Some(load) = load {
            load();
        }
        read(&self.entries.read().unwrap_or_else(|err| err.into_inner()))
    }
}

/// Completion metadata for a flag spelling.
///
/// Flags can contribute both direct value suggestions and context that affects
/// later completion. `context_only` flags are the bridge for options that shape
/// suggestion scope even when the cursor is not currently editing that flag.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub struct FlagNode {
    /// Recognize this flag without suggesting its name in menus.
    pub hidden: bool,
    /// Large shared catalogues searched in order; a later one is used only
    /// when earlier ones have no match (e.g. your VMs, then every VM).
    pub catalogs: Vec<ValueCatalog>,
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
    /// Preferred spelling when this entry is another spelling of one flag.
    ///
    /// Menus list the preferred spelling once, labelled with its other
    /// spellings; an alias is offered on its own only when typed exactly.
    pub alias_of: Option<String>,
}

impl FlagNode {
    /// Creates an empty flag node.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the display tooltip for this flag.
    ///
    /// If omitted, completion UIs show no description for this flag.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    /// Marks this flag as taking no value.
    ///
    /// If omitted, the flag is treated as value-taking when the surrounding
    /// completion metadata asks for values.
    pub fn flag_only(mut self) -> Self {
        self.flag_only = true;
        self
    }

    /// Marks this flag as repeatable.
    ///
    /// If omitted, the flag is treated as non-repeatable.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self
    }

    /// Sets the semantic value type for this flag.
    ///
    /// If omitted, the flag carries no special value-type hint.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn value_type(mut self, value_type: ValueType) -> Self {
        self.value_type = Some(value_type);
        self
    }

    /// Replaces the suggestion list for this flag value.
    ///
    /// If omitted, the flag contributes no direct generic value suggestions.
    pub fn suggestions(mut self, suggestions: impl IntoIterator<Item = SuggestionEntry>) -> Self {
        self.suggestions = suggestions.into_iter().collect();
        self
    }
}

/// One node in the immutable completion tree.
///
/// A node owns the completion contract for one resolved command scope:
/// subcommands, flags, positional arguments, and any hidden defaults inherited
/// through aliases or shell scope.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[must_use]
pub struct CompletionNode {
    /// Command name used during construction; empty for synthetic value nodes.
    pub name: String,
    /// Optional description shown alongside the node.
    pub tooltip: Option<String>,
    /// Optional suggestion-order hint for command/subcommand completion.
    pub sort: Option<String>,
    /// Whether an exact token should commit scope even without a trailing delimiter.
    pub exact_token_commits: bool,
    /// Callable and recognized, but never suggested.
    pub hidden: bool,
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
    /// Optional local callback for context-dependent flag and positional completion.
    pub dynamic: Option<DynamicCompletion>,
    /// Optional command-word normalization shared with native execution.
    pub word_resolver: Option<CommandWordResolver>,
}

impl CompletionNode {
    pub(crate) fn resolve_child(&self, word: &str) -> Option<(&String, &Self)> {
        self.children.get_key_value(word).or_else(|| {
            let canonical = (self.word_resolver.as_ref()?.0)(word)?;
            self.children.get_key_value(&canonical)
        })
    }

    /// Sets the hidden sort key for this node.
    pub fn sort(mut self, sort: impl Into<String>) -> Self {
        self.sort = Some(sort.into());
        self
    }

    /// Adds a child node keyed by command or value name.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub fn with_child(mut self, name: impl Into<String>, node: CompletionNode) -> Self {
        self.children.insert(name.into(), node);
        self
    }

    /// Adds a flag node keyed by its spelling.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
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
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
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
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn push_flag_occurrence(&mut self, occurrence: FlagOccurrence) {
        self.flag_values
            .entry(occurrence.name.clone())
            .or_default()
            .extend(occurrence.values.iter().cloned());
        self.tail.push(TailItem::Flag(occurrence));
    }

    /// Appends a positional argument to the tail.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn push_positional(&mut self, value: impl Into<String>) {
        self.tail.push(TailItem::Positional(value.into()));
    }

    /// Merges additional values for a flag spelling.
    pub(crate) fn merge_flag_values(&mut self, name: impl Into<String>, values: Vec<String>) {
        self.flag_values
            .entry(name.into())
            .or_default()
            .extend(values);
    }

    /// Inserts positional values ahead of the existing tail.
    pub(crate) fn prepend_positional_values(&mut self, values: impl IntoIterator<Item = String>) {
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
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn set_pipe(&mut self, pipes: Vec<String>) {
        self.has_pipe = true;
        self.pipes = pipes;
    }

    /// Appends one segment to the command path.
    #[cfg(test)]
    #[cfg_attr(coverage_nightly, coverage(off))]
    pub(crate) fn push_head(&mut self, segment: impl Into<String>) {
        self.head.push(segment.into());
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Parser output for the full line and the cursor-local prefix.
pub(crate) struct ParsedLine {
    /// Parsed command-line structure for the full line.
    pub full_cmd: CommandLine,
    /// Parsed command-line structure for the prefix before the cursor.
    pub cursor_cmd: CommandLine,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Explicit request kind for the current cursor position.
pub(crate) enum CompletionRequest {
    /// Completing a DSL pipe verb.
    Pipe,
    /// Completing a flag spelling in the current flag scope.
    FlagNames {
        /// Command path that contributes visible flags.
        flag_scope_path: Vec<String>,
    },
    /// Completing values for a specific flag.
    FlagValues {
        /// Command path that contributes the flag definition.
        flag_scope_path: Vec<String>,
        /// Flag currently requesting values.
        flag: String,
    },
    /// Completing subcommands, positional values, or empty-stub flags.
    Positionals {
        /// Command path contributing subcommands or positional args.
        context_path: Vec<String>,
        /// Command path that contributes visible flags.
        flag_scope_path: Vec<String>,
        /// Positional argument index relative to the resolved command path.
        arg_index: usize,
        /// Whether subcommand names should be suggested.
        show_subcommands: bool,
        /// Whether empty-stub flag spellings should also be suggested.
        show_flag_names: bool,
    },
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
impl CompletionRequest {
    /// Returns the stable request-kind label used by tests.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Pipe => "pipe",
            Self::FlagNames { .. } => "flag-names",
            Self::FlagValues { .. } => "flag-values",
            Self::Positionals {
                show_subcommands: true,
                ..
            } => "subcommands",
            Self::Positionals { .. } => "positionals",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Full completion analysis derived from parsing and context resolution.
pub(crate) struct CompletionAnalysis {
    /// Full parser output plus the cursor-local context derived from it.
    pub parsed: ParsedLine,
    /// Replacement details for the active token.
    pub cursor: CursorState,
    /// Resolved command context used for suggestion generation.
    pub context: CompletionContext,
    /// Explicit request kind for suggestion generation.
    pub request: CompletionRequest,
}

/// Resolved completion state for the cursor position.
///
/// The parser only knows about tokens. This structure captures the derived
/// command context the suggester/debug layers actually care about:
/// which command path matched, which node contributes visible flags, and
/// whether the cursor is still in subcommand-selection mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CompletionContext {
    /// Command path matched before the cursor.
    pub matched_path: Vec<String>,
    /// Command path that contributes visible flags.
    pub flag_scope_path: Vec<String>,
    /// Whether the cursor is completing a subcommand name.
    pub subcommand_context: bool,
}

/// High-level classification for a completion candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MatchKind {
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
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::completion::MatchKind;
    ///
    /// let labels = [
    ///     MatchKind::Pipe,
    ///     MatchKind::Flag,
    ///     MatchKind::Command,
    ///     MatchKind::Subcommand,
    ///     MatchKind::Value,
    /// ]
    /// .into_iter()
    /// .map(MatchKind::as_str)
    /// .collect::<Vec<_>>();
    ///
    /// assert_eq!(labels, vec!["pipe", "flag", "command", "subcommand", "value"]);
    /// ```
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

#[derive(Debug, Clone, PartialEq, Eq)]
/// Output emitted by the suggestion engine.
pub enum SuggestionOutput {
    /// A normal suggestion item.
    Item(Suggestion),
    /// Sentinel indicating that filesystem path completion should run next.
    PathSentinel,
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;
