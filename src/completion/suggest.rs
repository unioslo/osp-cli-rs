//! Suggestion ranking and shaping for completion results.
//!
//! This module exists to turn parsed cursor context plus a static completion
//! tree into ranked suggestion outputs. It is deliberately separate from
//! tokenization so ranking rules can evolve without changing the parser.
//!
//! Contract:
//!
//! - suggestion ranking lives here
//! - this layer may depend on fuzzy matching and resolved command context
//! - it should not own shell parsing or terminal rendering

use crate::completion::context::TreeResolver;
use crate::completion::model::{
    CommandLine, CompletionAnalysis, CompletionNode, CompletionRequest, CompletionTree, Suggestion,
    SuggestionEntry, SuggestionOutput, ValueType,
};
use crate::completion::model::{FlagNode, ValueCatalog};
use crate::core::fuzzy::{completion_fuzzy_matcher, fold_case};
use std::collections::{BTreeMap, BTreeSet};

const MATCH_SCORE_EXACT: u32 = 0;
const MATCH_SCORE_EMPTY_STUB: u32 = 1_000;
const MATCH_SCORE_PREFIX_BASE: u32 = 100;
const MATCH_SCORE_BOUNDARY_PREFIX_BASE: u32 = 200;
const MATCH_SCORE_INTERIOR_BASE: u32 = 500;
const MATCH_SCORE_FUZZY_BASE: u32 = 10_000;
const MATCH_SCORE_FUZZY_NORMALIZED_MAX: u32 = 100_000;
const MATCH_SCORE_HINT_LITERAL_PENALTY: u32 = 1_000;
const MATCH_SCORE_HINT_FUZZY_PENALTY: u32 = 200_000;
// Encode lexicographic classes in Suggestion's existing lower-is-better score.
const MATCH_CLASS_STRIDE: u32 = 1 << 28;

#[derive(Clone, Copy)]
enum ValueMatch {
    Ordinary,
    Separator,
    Typo,
}

/// Catalogue matches shown at once; the menu lays them out as a compact grid.
const CATALOG_MATCH_LIMIT: usize = 25;
/// Typo rescue needs enough typed text for similarity to mean something.
const TYPO_RESCUE_MIN_CHARS: usize = 4;
/// Jaro-Winkler floor; measured on the 53k-name MREG index, 0.85 keeps
/// `mial-mx01` -> `mail-mx01` while dropping unrelated short names.
const TYPO_RESCUE_MIN_SIMILARITY: f64 = 0.85;

struct PositionalRequest<'a> {
    context_node: &'a CompletionNode,
    flag_scope_node: &'a CompletionNode,
    arg_index: usize,
    stub: &'a str,
    cmd: &'a CommandLine,
    show_subcommands: bool,
    show_flag_names: bool,
}

/// Generates ranked completion suggestions from a completion tree and cursor analysis.
#[derive(Debug, Clone)]
pub(crate) struct SuggestionEngine {
    tree: std::sync::Arc<CompletionTree>,
}

impl SuggestionEngine {
    /// Creates a suggestion engine for the provided completion tree.
    pub fn new(tree: impl Into<std::sync::Arc<CompletionTree>>) -> Self {
        Self { tree: tree.into() }
    }

    pub(super) fn tree(&self) -> &CompletionTree {
        &self.tree
    }

    /// Produces sorted completion outputs for the current cursor analysis.
    ///
    /// # Examples
    ///
    /// ```text
    /// use osp_cli::completion::{
    ///     CommandLine, CompletionAnalysis, CompletionContext, CompletionNode,
    ///     CompletionRequest, CompletionTree, CursorState, ParsedLine,
    ///     SuggestionEngine, SuggestionOutput,
    /// };
    /// use std::collections::BTreeMap;
    ///
    /// let tree = CompletionTree {
    ///     root: CompletionNode {
    ///         children: BTreeMap::from([("ldap".into(), CompletionNode::default())]),
    ///         ..CompletionNode::default()
    ///     },
    ///     pipe_verbs: BTreeMap::new(),
    /// };
    /// let analysis = CompletionAnalysis {
    ///     parsed: ParsedLine {
    ///         safe_cursor: 2,
    ///         full_tokens: vec!["ld".to_string()],
    ///         cursor_tokens: vec!["ld".to_string()],
    ///         full_cmd: CommandLine::default(),
    ///         cursor_cmd: CommandLine::default(),
    ///     },
    ///     cursor: CursorState::new("ld", "ld", 0..2, None),
    ///     context: CompletionContext {
    ///         matched_path: Vec::new(),
    ///         flag_scope_path: Vec::new(),
    ///         subcommand_context: true,
    ///     },
    ///     request: CompletionRequest::Positionals {
    ///         context_path: Vec::new(),
    ///         flag_scope_path: Vec::new(),
    ///         arg_index: 0,
    ///         show_subcommands: true,
    ///         show_flag_names: false,
    ///     },
    /// };
    ///
    /// let suggestions = SuggestionEngine::new(tree).generate(&analysis);
    ///
    /// assert!(matches!(
    ///     suggestions.first(),
    ///     Some(SuggestionOutput::Item(item)) if item.text == "ldap"
    /// ));
    /// ```
    pub fn generate(&self, analysis: &CompletionAnalysis) -> Vec<SuggestionOutput> {
        self.emit_suggestions(&analysis.request, analysis)
    }

    fn emit_suggestions(
        &self,
        request: &CompletionRequest,
        analysis: &CompletionAnalysis,
    ) -> Vec<SuggestionOutput> {
        let cmd = &analysis.parsed.cursor_cmd;
        let stub = analysis.cursor.token_stub.as_str();
        let resolver = TreeResolver::new(&self.tree);
        let nodes = resolver.resolved_nodes(&analysis.context);
        let completion_cmd = match request {
            CompletionRequest::FlagValues { flag, .. } => command_without_current_value(cmd, flag),
            _ => cmd.clone(),
        };
        let flag_scope = resolve_dynamic(nodes.flag_scope_node, &completion_cmd);

        let mut out = match request {
            CompletionRequest::Pipe => self.pipe_suggestions(stub),
            CompletionRequest::FlagNames { .. } => self
                .flag_name_suggestions(&flag_scope, stub, cmd)
                .into_iter()
                .map(SuggestionOutput::Item)
                .collect(),
            CompletionRequest::FlagValues { flag, .. } => {
                self.flag_value_suggestions(&flag_scope, flag, stub)
            }
            CompletionRequest::Positionals {
                arg_index,
                show_subcommands,
                show_flag_names,
                ..
            } => {
                let context = if std::ptr::eq(nodes.context_node, nodes.flag_scope_node) {
                    std::borrow::Cow::Borrowed(flag_scope.as_ref())
                } else {
                    resolve_dynamic(nodes.context_node, &completion_cmd)
                };
                let request = PositionalRequest {
                    context_node: nodes.context_node,
                    flag_scope_node: &flag_scope,
                    arg_index: *arg_index,
                    stub,
                    cmd,
                    show_subcommands: *show_subcommands,
                    show_flag_names: *show_flag_names,
                };
                let mut out = self.positional_suggestions(request, &context);
                sort_suggestion_outputs(&mut out);
                return out;
            }
        };

        sort_suggestion_outputs(&mut out);
        out
    }

    fn positional_suggestions(
        &self,
        request: PositionalRequest<'_>,
        resolved_context: &CompletionNode,
    ) -> Vec<SuggestionOutput> {
        let mut out = Vec::new();

        if request.show_subcommands {
            out.extend(
                self.subcommand_suggestions(request.context_node, request.stub)
                    .into_iter()
                    .map(SuggestionOutput::Item),
            );
        }
        // A command may take a subcommand or a value in the same slot, as
        // `nh rhel9` beside `nh host`. Values join once a word is typed, so an
        // empty slot still lists the subcommands first; a subcommand wins ties.
        let values = if request.show_subcommands && request.stub.is_empty() {
            Vec::new()
        } else {
            self.arg_value_suggestions(resolved_context, request.arg_index, request.stub)
        };
        for value in values {
            let duplicate = matches!(&value, SuggestionOutput::Item(new)
                if out.iter().any(|seen| matches!(seen, SuggestionOutput::Item(old) if old.text == new.text)));
            if !duplicate {
                out.push(value);
            }
        }

        if request.show_flag_names {
            let prefer_positionals = request.stub.is_empty()
                && out
                    .iter()
                    .any(|item| matches!(item, SuggestionOutput::Item(_)));
            out.extend(
                self.flag_name_suggestions(request.flag_scope_node, request.stub, request.cmd)
                    .into_iter()
                    .map(|mut suggestion| {
                        if prefer_positionals {
                            suggestion.match_score = suggestion.match_score.saturating_add(1);
                        }
                        suggestion
                    })
                    .map(SuggestionOutput::Item),
            );
        }

        out
    }

    fn pipe_suggestions(&self, stub: &str) -> Vec<SuggestionOutput> {
        let stub_lc = fold_case(stub);
        self.tree
            .pipe_verbs
            .iter()
            .filter_map(|(verb, tooltip)| {
                let score = self.match_score(&stub_lc, verb)?;
                Some(SuggestionOutput::Item(Suggestion {
                    text: verb.clone(),
                    meta: Some(tooltip.clone()),
                    display: None,
                    is_exact: score == 0,
                    sort: None,
                    match_score: score,
                }))
            })
            .collect()
    }

    fn flag_name_suggestions(
        &self,
        node: &CompletionNode,
        stub: &str,
        cmd: &CommandLine,
    ) -> Vec<Suggestion> {
        let stub_lc = fold_case(stub);
        let allowlist = self.resolved_flag_allowlist(node);
        let required = self.required_flags(node);
        // Other spellings of one flag collapse into its preferred entry, e.g.
        // `--interactive, -i`, unless typed exactly.
        let mut other_spellings = BTreeMap::<&str, Vec<&str>>::new();
        for (flag, meta) in &node.flags {
            if let Some(preferred) = meta.alias_of.as_deref()
                && node.flags.contains_key(preferred)
            {
                other_spellings
                    .entry(preferred)
                    .or_default()
                    .push(flag.as_str());
            }
        }
        let is_hidden_alias = |flag: &str, meta: &FlagNode| {
            meta.alias_of
                .as_deref()
                .is_some_and(|preferred| node.flags.contains_key(preferred) && stub != flag)
        };
        node.flags
            .iter()
            .filter(|(flag, meta)| !meta.hidden && !is_hidden_alias(flag, meta))
            .filter_map(|(flag, meta)| {
                let score = self.match_score(&stub_lc, flag)?;
                let score = if matches!(stub, "-" | "--") {
                    MATCH_SCORE_EMPTY_STUB
                } else {
                    score
                };
                Some((flag, meta, score))
            })
            .filter(|(flag, _, _)| {
                allowlist
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(flag.as_str()))
            })
            .filter(|(flag, meta, _)| {
                meta.multi
                    || stub == *flag
                    || !(cmd.has_flag(flag)
                        || other_spellings
                            .get(flag.as_str())
                            .is_some_and(|spellings| spellings.iter().any(|s| cmd.has_flag(s))))
            })
            .map(|(flag, meta, score)| Suggestion {
                text: flag.clone(),
                meta: if required.contains(flag.as_str()) {
                    Some(match meta.tooltip.as_deref() {
                        Some(tooltip) => format!("required; {tooltip}"),
                        None => "required".to_string(),
                    })
                } else {
                    meta.tooltip.clone()
                },
                display: flag_display(
                    flag,
                    other_spellings.get(flag.as_str()),
                    required.contains(flag.as_str()),
                ),
                is_exact: score == 0,
                sort: Some(
                    if required.contains(flag.as_str()) {
                        0
                    } else if node
                        .flag_hints
                        .as_ref()
                        .is_some_and(|hints| hints.preferred.contains(flag))
                    {
                        1
                    } else if node
                        .flag_hints
                        .as_ref()
                        .is_some_and(|hints| hints.common.contains(flag))
                    {
                        2
                    } else {
                        3
                    }
                    .to_string(),
                ),
                match_score: score,
            })
            .collect()
    }

    fn flag_value_suggestions(
        &self,
        node: &CompletionNode,
        flag: &str,
        stub: &str,
    ) -> Vec<SuggestionOutput> {
        let Some(flag_node) = node.flags.get(flag) else {
            return Vec::new();
        };

        if flag_node.flag_only {
            return Vec::new();
        }

        if flag_node.value_type == Some(ValueType::Path) {
            let mut suggestions = self.entry_suggestions(&flag_node.suggestions, stub);
            suggestions.push(SuggestionOutput::PathSentinel);
            return suggestions;
        }

        if !flag_node.catalogs.is_empty() {
            return self.catalog_suggestions(&flag_node.catalogs, stub);
        }

        self.entry_suggestions(&flag_node.suggestions, stub)
    }

    fn arg_value_suggestions(
        &self,
        node: &CompletionNode,
        index: usize,
        stub: &str,
    ) -> Vec<SuggestionOutput> {
        let Some(arg) = node.args.get(index) else {
            return Vec::new();
        };

        if arg.value_type == Some(ValueType::Path) {
            return vec![SuggestionOutput::PathSentinel];
        }
        if !arg.catalogs.is_empty() {
            return self.catalog_suggestions(&arg.catalogs, stub);
        }

        self.entry_suggestions(&arg.suggestions, stub)
    }

    /// Searches catalogue tiers in order, bounding results before cloning.
    ///
    /// Value scans choose ordinary, separator fallback, or typo rescue before
    /// hints compete, so metadata cannot suppress an existing value rescue.
    /// Empty input retains the first nonempty catalogue's alphabetical menu.
    fn catalog_suggestions(&self, catalogs: &[ValueCatalog], stub: &str) -> Vec<SuggestionOutput> {
        let ranked = |hits: Vec<(u32, SuggestionEntry)>| {
            hits.into_iter()
                .map(|(score, entry)| SuggestionOutput::Item(entry_to_suggestion(&entry, score)))
                .collect::<Vec<_>>()
        };
        if stub.is_empty() {
            let first = catalogs
                .iter()
                .map(|catalog| {
                    catalog.scan(|entries| {
                        entries
                            .iter()
                            .take(CATALOG_MATCH_LIMIT)
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                })
                .find(|entries| !entries.is_empty())
                .unwrap_or_default();
            return ranked(
                first
                    .into_iter()
                    .map(|entry| (MATCH_SCORE_EMPTY_STUB, entry))
                    .collect(),
            );
        }
        let stub_lc = fold_case(stub);
        let compact_stub = compact(&stub_lc);
        let mut value_hits = Vec::new();
        for mode in [
            ValueMatch::Ordinary,
            ValueMatch::Separator,
            ValueMatch::Typo,
        ] {
            value_hits = catalogs
                .iter()
                .map(|catalog| {
                    catalog.scan(|entries| {
                        best(entries.iter().filter_map(|entry| {
                            let score = if let Some(matches) = catalog.matcher {
                                if !matches!(mode, ValueMatch::Ordinary) {
                                    return None;
                                }
                                matches(entry, &stub_lc).then_some(
                                    if fold_case(&entry.value) == stub_lc {
                                        MATCH_SCORE_EXACT
                                    } else {
                                        MATCH_SCORE_PREFIX_BASE
                                    },
                                )
                            } else {
                                self.value_match_score(&stub_lc, &compact_stub, entry, mode)
                            }?;
                            Some((score, entry))
                        }))
                    })
                })
                .collect::<Vec<_>>();
            if value_hits.iter().any(|hits| !hits.is_empty()) {
                break;
            }
        }
        for (catalog, hits) in catalogs.iter().zip(value_hits) {
            if catalog.matcher.is_some() {
                if !hits.is_empty() {
                    return ranked(hits);
                }
                continue;
            }
            let scores = hits
                .iter()
                .map(|(score, entry)| (entry.value.as_str(), *score))
                .collect::<BTreeMap<_, _>>();
            let hits = catalog.scan(|entries| {
                best(entries.iter().filter_map(|entry| {
                    let score = scores
                        .get(entry.value.as_str())
                        .copied()
                        .into_iter()
                        .chain(self.hint_match_score(&stub_lc, &compact_stub, entry))
                        .min()?;
                    Some((score, entry))
                }))
            });
            if !hits.is_empty() {
                return ranked(hits);
            }
        }
        Vec::new()
    }

    fn value_match_score(
        &self,
        stub: &str,
        compact_stub: &str,
        entry: &SuggestionEntry,
        mode: ValueMatch,
    ) -> Option<u32> {
        match mode {
            ValueMatch::Ordinary => {
                let score = std::iter::once(&entry.value)
                    .chain(&entry.aliases)
                    .filter_map(|value| self.field_match_score(stub, value))
                    .min()?;
                Some(value_score(stub, &entry.value, score))
            }
            ValueMatch::Separator => {
                if compact_stub.chars().count() < TYPO_RESCUE_MIN_CHARS {
                    return None;
                }
                let value = compact(&folded_candidate(&entry.value));
                let offset = value.find(compact_stub)?;
                let cost = if value == compact_stub {
                    0
                } else if offset == 0 {
                    100
                } else {
                    500 + offset as u32
                };
                Some(class_score(2, cost))
            }
            ValueMatch::Typo => {
                if stub.chars().count() < TYPO_RESCUE_MIN_CHARS {
                    return None;
                }
                // Compare the leading label, not the shared domain suffix.
                let label = entry.value.split('.').next().unwrap_or(&entry.value);
                let similarity = strsim::jaro_winkler(stub, &folded_candidate(label));
                (similarity >= TYPO_RESCUE_MIN_SIMILARITY)
                    .then(|| class_score(3, ((1.0 - similarity) * 10_000.0) as u32))
            }
        }
    }

    fn hint_match_score(
        &self,
        stub: &str,
        compact_stub: &str,
        entry: &SuggestionEntry,
    ) -> Option<u32> {
        let hint = folded_candidate(entry.meta.as_deref()?);
        let ordinary = self.field_match_score(stub, &hint);
        // Hint normalization is a literal alternative; values retain their gaps.
        let normalized = (compact_stub.chars().count() >= TYPO_RESCUE_MIN_CHARS)
            .then(|| compact(&hint).find(compact_stub))
            .flatten()
            .map(|offset| (if offset == 0 { 150 } else { 550 }) + offset as u32);
        let score = ordinary.into_iter().chain(normalized).min()?;
        Some(if score < MATCH_SCORE_FUZZY_BASE {
            class_score(3, score + MATCH_SCORE_HINT_LITERAL_PENALTY)
        } else {
            class_score(4, score + MATCH_SCORE_HINT_FUZZY_PENALTY)
        })
    }

    fn subcommand_suggestions(&self, node: &CompletionNode, stub: &str) -> Vec<Suggestion> {
        let stub_lc = fold_case(stub);
        node.children
            .iter()
            .filter(|(_, child)| !child.hidden)
            .filter_map(|(name, child)| {
                let score = self.match_score(&stub_lc, name)?;
                Some(Suggestion {
                    text: name.clone(),
                    meta: child_completion_meta(child),
                    display: None,
                    is_exact: score == 0,
                    sort: child.sort.clone(),
                    match_score: score,
                })
            })
            .collect()
    }
    fn entry_suggestions(&self, entries: &[SuggestionEntry], stub: &str) -> Vec<SuggestionOutput> {
        let stub_lc = fold_case(stub);
        entries
            .iter()
            .filter_map(|entry| {
                let score = self.entry_match_score(&stub_lc, entry)?;
                Some(SuggestionOutput::Item(entry_to_suggestion(entry, score)))
            })
            .collect()
    }

    /// Scores values and hints independently, protecting canonical exact/prefix matches.
    fn entry_match_score(&self, stub_lc: &str, entry: &SuggestionEntry) -> Option<u32> {
        if stub_lc.is_empty() {
            return Some(MATCH_SCORE_EMPTY_STUB);
        }
        let compact_stub = compact(stub_lc);
        self.value_match_score(stub_lc, &compact_stub, entry, ValueMatch::Ordinary)
            .into_iter()
            .chain(self.hint_match_score(stub_lc, &compact_stub, entry))
            .min()
    }

    fn match_score(&self, stub_lc: &str, candidate: &str) -> Option<u32> {
        let score = self.field_match_score(stub_lc, candidate)?;
        Some(if stub_lc.is_empty() {
            score
        } else {
            value_score(stub_lc, candidate, score)
        })
    }

    fn field_match_score(&self, stub_lc: &str, candidate: &str) -> Option<u32> {
        if stub_lc.is_empty() {
            return Some(MATCH_SCORE_EMPTY_STUB);
        }

        let candidate_lc = folded_candidate(candidate);

        if stub_lc == candidate_lc {
            return Some(MATCH_SCORE_EXACT);
        }
        let prefix = candidate_lc
            .starts_with(stub_lc)
            .then(|| MATCH_SCORE_PREFIX_BASE + (candidate_lc.len() - stub_lc.len()) as u32);
        let literal = candidate_lc
            .char_indices()
            .filter(|(offset, _)| candidate_lc[*offset..].starts_with(stub_lc))
            .map(|(offset, _)| {
                let boundary = candidate_lc[..offset]
                    .chars()
                    .next_back()
                    .is_none_or(|ch| !ch.is_alphanumeric());
                (if boundary {
                    MATCH_SCORE_BOUNDARY_PREFIX_BASE
                } else {
                    MATCH_SCORE_INTERIOR_BASE
                }) + offset as u32
            })
            .min();
        if let Some(score) = prefix.into_iter().chain(literal).min() {
            return Some(score);
        }

        // Fuzzy matching is the rescue path, not the primary ranking model.
        // Its normalized penalty keeps rescues behind explicit prefix matches.
        let fuzzy = completion_fuzzy_matcher().fuzzy_match(&candidate_lc, stub_lc)?;
        let normalized = fuzzy.max(0) as u32;
        let penalty = MATCH_SCORE_FUZZY_NORMALIZED_MAX.saturating_sub(normalized);
        Some(MATCH_SCORE_FUZZY_BASE + penalty)
    }

    fn resolved_flag_allowlist(&self, node: &CompletionNode) -> Option<BTreeSet<String>> {
        let hints = node.flag_hints.as_ref()?;
        let mut allowed = hints
            .common
            .iter()
            .chain(&hints.required_common)
            .cloned()
            .collect::<BTreeSet<_>>();

        allowed.extend(hints.preferred.iter().cloned());
        allowed.extend(hints.other.iter().cloned());

        Some(allowed)
    }

    pub(crate) fn required_flags(&self, node: &CompletionNode) -> BTreeSet<String> {
        let mut required = BTreeSet::new();
        let Some(hints) = node.flag_hints.as_ref() else {
            return required;
        };

        required.extend(hints.required_common.iter().cloned());
        required
    }
}

fn command_without_current_value(cmd: &CommandLine, flag: &str) -> CommandLine {
    let mut command = cmd.clone();
    match command.flag_values.get_mut(flag) {
        Some(values) if values.len() > 1 => {
            values.pop();
        }
        Some(_) => {
            command.flag_values.remove(flag);
        }
        None => {}
    }
    command
}

/// Menu label for a flag: its other spellings and a `*` when required.
///
/// Returns `None` when the label would equal the flag itself.
fn flag_display(flag: &str, other_spellings: Option<&Vec<&str>>, required: bool) -> Option<String> {
    let mut label = flag.to_string();
    for spelling in other_spellings.into_iter().flatten() {
        label.push_str(", ");
        label.push_str(spelling);
    }
    if required {
        label.push('*');
    }
    (label != flag).then_some(label)
}

fn child_completion_meta(child: &CompletionNode) -> Option<String> {
    let summary = child_subcommand_summary(child);
    match (child.tooltip.as_deref(), summary) {
        (Some(tooltip), Some(summary)) => Some(format!("{tooltip} ({summary})")),
        (Some(tooltip), None) => Some(tooltip.to_string()),
        (None, Some(summary)) => Some(summary),
        (None, None) => None,
    }
}

fn child_subcommand_summary(child: &CompletionNode) -> Option<String> {
    if child.children.is_empty() {
        return None;
    }

    let preview = child.children.keys().take(3).cloned().collect::<Vec<_>>();
    if preview.is_empty() {
        return None;
    }

    let label = if child.value_key {
        "values"
    } else {
        "subcommands"
    };
    let mut summary = format!("{label}: {}", preview.join(", "));
    if child.children.len() > preview.len() {
        summary.push_str(", ...");
    }
    Some(summary)
}

fn sort_suggestion_outputs(outputs: &mut Vec<SuggestionOutput>) {
    let mut items: Vec<Suggestion> = outputs
        .iter()
        .filter_map(|entry| match entry {
            SuggestionOutput::Item(item) => Some(item.clone()),
            SuggestionOutput::PathSentinel => None,
        })
        .collect();
    let path_sentinel_count = outputs
        .iter()
        .filter(|entry| matches!(entry, SuggestionOutput::PathSentinel))
        .count();

    items.sort_by(compare_suggestions);

    // Path sentinels are not ranked suggestions; they are control markers that
    // tell the caller to ask the shell for filesystem completion after all
    // normal ranked suggestions have been shown. Keep item ordering text-stable
    // before reattaching the sentinels so redraws do not jump between ties.
    outputs.clear();
    outputs.extend(items.into_iter().map(SuggestionOutput::Item));
    outputs.extend(std::iter::repeat_n(
        SuggestionOutput::PathSentinel,
        path_sentinel_count,
    ));
}

fn compare_suggestions(left: &Suggestion, right: &Suggestion) -> std::cmp::Ordering {
    (not_exact(left), left.match_score)
        .cmp(&(not_exact(right), right.match_score))
        .then_with(|| compare_sort_value(left.sort.as_deref(), right.sort.as_deref()))
        .then_with(|| fold_case(&left.text).cmp(&fold_case(&right.text)))
}

fn compare_sort_value(left: Option<&str>, right: Option<&str>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => {
            match (
                left.trim().parse::<f64>().ok(),
                right.trim().parse::<f64>().ok(),
            ) {
                (Some(left_num), Some(right_num)) => left_num
                    .partial_cmp(&right_num)
                    .unwrap_or(std::cmp::Ordering::Equal),
                _ => fold_case(left).cmp(&fold_case(right)),
            }
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn not_exact(suggestion: &Suggestion) -> bool {
    !suggestion.is_exact
}

fn entry_to_suggestion(entry: &SuggestionEntry, match_score: u32) -> Suggestion {
    Suggestion {
        text: entry.value.clone(),
        meta: entry.meta.clone(),
        display: entry.display.clone(),
        is_exact: match_score == 0,
        sort: entry.sort.clone(),
        match_score,
    }
}

/// Close values for a "did you mean" hint, ranked exactly like completion:
/// the same tiers, catalogue order and typo rescue, at most `limit`. These are
/// suggestions only, never a resolved identity.
///
/// # Examples
///
/// ```
/// use osp_cli::completion::closest_values;
/// use osp_cli::completion::ValueCatalog;
///
/// let yours = ValueCatalog::default();
/// yours.replace(vec!["oistes-test07.uio.no".into(), "oistes-test01.uio.no".into()]);
/// assert_eq!(closest_values(&[yours], "oistse-tst07", 1), ["oistes-test07.uio.no"]);
/// ```
pub fn closest_values(catalogs: &[ValueCatalog], text: &str, limit: usize) -> Vec<String> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    SuggestionEngine::new(CompletionTree::default())
        .catalog_suggestions(catalogs, text)
        .into_iter()
        .filter_map(|output| match output {
            SuggestionOutput::Item(item) => Some(item.text),
            SuggestionOutput::PathSentinel => None,
        })
        .take(limit)
        .collect()
}

/// The best-scoring catalogue hits, lowest score first, then alphabetical.
fn best<'a>(hits: impl Iterator<Item = (u32, &'a SuggestionEntry)>) -> Vec<(u32, SuggestionEntry)> {
    let mut hits = hits.collect::<Vec<_>>();
    let compare = |left: &(u32, &SuggestionEntry), right: &(u32, &SuggestionEntry)| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.value.cmp(&right.1.value))
    };
    if hits.len() > CATALOG_MATCH_LIMIT {
        hits.select_nth_unstable_by(CATALOG_MATCH_LIMIT, compare);
        hits.truncate(CATALOG_MATCH_LIMIT);
    }
    hits.sort_unstable_by(compare);
    hits.into_iter()
        .map(|(score, entry)| (score, entry.clone()))
        .collect()
}

// Lowercase ASCII is already folded; preserve Unicode folding for other text.
fn folded_candidate(candidate: &str) -> std::borrow::Cow<'_, str> {
    if candidate
        .bytes()
        .all(|byte| byte.is_ascii() && !byte.is_ascii_uppercase())
    {
        std::borrow::Cow::Borrowed(candidate)
    } else {
        std::borrow::Cow::Owned(fold_case(candidate))
    }
}

// The frozen lab extension protects literal values, then lets literal hints
// compete with value typos, and penalizes fuzzy hints against fuzzy values.
fn value_score(stub: &str, candidate: &str, field_score: u32) -> u32 {
    let candidate = folded_candidate(candidate);
    if candidate == stub {
        MATCH_SCORE_EXACT
    } else if candidate.starts_with(stub) {
        class_score(1, (candidate.len() - stub.len()) as u32)
    } else if field_score < MATCH_SCORE_FUZZY_BASE {
        class_score(2, field_score)
    } else {
        class_score(4, field_score)
    }
}

fn class_score(class: u32, cost: u32) -> u32 {
    class * MATCH_CLASS_STRIDE + cost.min(MATCH_CLASS_STRIDE - 1)
}

fn compact(text: &str) -> String {
    text.chars().filter(|ch| ch.is_alphanumeric()).collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests;

/// Resolve a local overlay without changing the parser's static grammar.
pub(super) fn resolve_dynamic<'a>(
    node: &'a CompletionNode,
    cmd: &CommandLine,
) -> std::borrow::Cow<'a, CompletionNode> {
    match &node.dynamic {
        Some(dynamic) => std::borrow::Cow::Owned(dynamic.resolve(cmd, node)),
        None => std::borrow::Cow::Borrowed(node),
    }
}
