//! Suggestion ranking and shaping for completion results.
//!
//! This module exists to turn parsed cursor context plus a static completion
//! tree into ranked suggestion outputs. It is deliberately separate from
//! tokenization so ranking rules can evolve without changing the parser.
//!
//! Contract:
//!
//! - suggestion ranking lives here
//! - this layer may depend on fuzzy matching and provider context helpers
//! - it should not own shell parsing or terminal rendering

use crate::completion::context::{ProviderSelection, TreeResolver};
use crate::completion::model::{
    CommandLine, CompletionAnalysis, CompletionNode, CompletionRequest, CompletionTree, Suggestion,
    SuggestionEntry, SuggestionOutput, ValueType,
};
use crate::core::fuzzy::{completion_fuzzy_matcher, fold_case};
use skim::fuzzy_matcher::FuzzyMatcher;
use std::collections::BTreeSet;

const MATCH_SCORE_EXACT: u32 = 0;
const MATCH_SCORE_EMPTY_STUB: u32 = 1_000;
const MATCH_SCORE_PREFIX_BASE: u32 = 100;
const MATCH_SCORE_BOUNDARY_PREFIX_BASE: u32 = 200;
const MATCH_SCORE_FUZZY_BASE: u32 = 10_000;
const MATCH_SCORE_FUZZY_NORMALIZED_MAX: u32 = 100_000;
// Lower scores win:
// exact < prefix < boundary-prefix < fuzzy fallback.

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
pub struct SuggestionEngine {
    tree: CompletionTree,
}

impl SuggestionEngine {
    /// Creates a suggestion engine for the provided completion tree.
    pub fn new(tree: CompletionTree) -> Self {
        Self { tree }
    }

    /// Produces sorted completion outputs for the current cursor analysis.
    ///
    /// # Examples
    ///
    /// ```
    /// use osp_cli::completion::{
    ///     CommandLine, CompletionAnalysis, CompletionContext, CompletionNode,
    ///     CompletionRequest, CompletionTree, CursorState, ParsedLine,
    ///     SuggestionEngine, SuggestionOutput,
    /// };
    /// use std::collections::BTreeMap;
    ///
    /// let tree = CompletionTree {
    ///     root: CompletionNode::default()
    ///         .with_child("ldap", CompletionNode::default()),
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
    ///     cursor: CursorState::synthetic("ld"),
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

        let mut out = match request {
            CompletionRequest::Pipe => self.pipe_suggestions(stub),
            CompletionRequest::FlagNames { flag_scope_path } => {
                let flag_scope_node = resolver
                    .resolve_exact(flag_scope_path)
                    .unwrap_or(&self.tree.root);
                self.flag_name_suggestions(flag_scope_node, stub, cmd)
                    .into_iter()
                    .map(SuggestionOutput::Item)
                    .collect()
            }
            CompletionRequest::FlagValues {
                flag_scope_path,
                flag,
            } => {
                let flag_scope_node = resolver
                    .resolve_exact(flag_scope_path)
                    .unwrap_or(&self.tree.root);
                self.flag_value_suggestions(flag_scope_node, flag, stub, cmd)
            }
            CompletionRequest::Positionals {
                context_path,
                flag_scope_path,
                arg_index,
                show_subcommands,
                show_flag_names,
            } => {
                let context_node = resolver
                    .resolve_exact(context_path)
                    .unwrap_or(&self.tree.root);
                let flag_scope_node = resolver
                    .resolve_exact(flag_scope_path)
                    .unwrap_or(&self.tree.root);
                let request = PositionalRequest {
                    context_node,
                    flag_scope_node,
                    arg_index: *arg_index,
                    stub,
                    cmd,
                    show_subcommands: *show_subcommands,
                    show_flag_names: *show_flag_names,
                };
                let mut out = self.positional_suggestions(request);
                sort_suggestion_outputs(&mut out);
                return out;
            }
        };

        sort_suggestion_outputs(&mut out);
        out
    }

    fn positional_suggestions(&self, request: PositionalRequest<'_>) -> Vec<SuggestionOutput> {
        let mut out = Vec::new();

        if request.show_subcommands {
            let subcommand_stub = if request.context_node.children.contains_key(request.stub) {
                ""
            } else {
                request.stub
            };
            out.extend(
                self.subcommand_suggestions(request.context_node, subcommand_stub)
                    .into_iter()
                    .map(SuggestionOutput::Item),
            );
        } else {
            out.extend(self.arg_value_suggestions(
                request.context_node,
                request.arg_index,
                request.stub,
            ));
        }

        if request.show_flag_names {
            out.extend(
                self.flag_name_suggestions(request.flag_scope_node, request.stub, request.cmd)
                    .into_iter()
                    .map(SuggestionOutput::Item),
            );
        }

        out
    }

    fn pipe_suggestions(&self, stub: &str) -> Vec<SuggestionOutput> {
        self.tree
            .pipe_verbs
            .iter()
            .filter_map(|(verb, tooltip)| {
                let score = self.match_score(stub, verb)?;
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
        let allowlist = self.resolved_flag_allowlist(node, cmd);
        let required = self.required_flags(node, cmd);
        let flag_stub = if node.flags.contains_key(stub) {
            ""
        } else {
            stub
        };

        node.flags
            .iter()
            .filter_map(|(flag, meta)| {
                let score = self.match_score(flag_stub, flag)?;
                Some((flag, meta, score))
            })
            .filter(|(flag, _, _)| {
                allowlist
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(flag.as_str()))
            })
            .filter(|(flag, meta, _)| meta.multi || !cmd.has_flag(flag) || stub == *flag)
            .map(|(flag, meta, score)| Suggestion {
                text: flag.clone(),
                meta: meta.tooltip.clone(),
                display: required.contains(flag.as_str()).then(|| format!("{flag}*")),
                is_exact: score == 0,
                sort: None,
                match_score: score,
            })
            .collect()
    }

    fn flag_value_suggestions(
        &self,
        node: &CompletionNode,
        flag: &str,
        stub: &str,
        cmd: &CommandLine,
    ) -> Vec<SuggestionOutput> {
        let Some(flag_node) = node.flags.get(flag) else {
            return Vec::new();
        };

        if flag_node.flag_only {
            return Vec::new();
        }

        if flag_node.value_type == Some(ValueType::Path) {
            return vec![SuggestionOutput::PathSentinel];
        }

        if let Some(output) =
            self.provider_specific_flag_value_suggestions(flag_node, flag, stub, cmd)
        {
            return output;
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

        self.entry_suggestions(&arg.suggestions, stub)
    }

    fn subcommand_suggestions(&self, node: &CompletionNode, stub: &str) -> Vec<Suggestion> {
        node.children
            .iter()
            .filter_map(|(name, child)| {
                let score = self.match_score(stub, name)?;
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
    fn provider_specific_flag_value_suggestions(
        &self,
        flag_node: &crate::completion::model::FlagNode,
        flag: &str,
        stub: &str,
        cmd: &CommandLine,
    ) -> Option<Vec<SuggestionOutput>> {
        // Provider completion has two special cases:
        // - selecting `--provider` may be constrained by the current `--os`
        // - many flags expose provider-specific value sets once a provider is chosen
        //
        // `osp-cli` marks these selector flags as context-only in
        // `repl/completion.rs`; the suggestion engine still needs a small
        // amount of flag-name-specific logic until that relationship is fully
        // expressed in completion metadata.
        let provider = ProviderSelection::from_command(cmd);

        if flag == "--provider" {
            let os_token = provider.normalized_os();
            if let Some(os_token) = os_token {
                let filtered = flag_node
                    .suggestions
                    .iter()
                    .filter(|entry| {
                        flag_node
                            .os_provider_map
                            .get(os_token)
                            .is_none_or(|providers| providers.iter().any(|p| p == &entry.value))
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                if !filtered.is_empty() {
                    return Some(self.entry_suggestions(&filtered, stub));
                }
            }
        }

        let provider_values = flag_node.suggestions_by_provider.get(provider.name()?)?;
        Some(self.entry_suggestions(provider_values, stub))
    }

    fn entry_suggestions(&self, entries: &[SuggestionEntry], stub: &str) -> Vec<SuggestionOutput> {
        let stub = if entries
            .iter()
            .any(|entry| fold_case(&entry.value) == fold_case(stub))
        {
            ""
        } else {
            stub
        };
        entries
            .iter()
            .filter_map(|entry| {
                let score = self.match_score(stub, &entry.value)?;
                Some(SuggestionOutput::Item(entry_to_suggestion(entry, score)))
            })
            .collect()
    }

    fn match_score(&self, stub: &str, candidate: &str) -> Option<u32> {
        // Lower scores are better:
        // - exact match wins with 0
        // - 100-range keeps ordinary prefix matches together
        // - 200-range keeps word-boundary prefixes behind direct prefixes
        // - 10_000+ is fuzzy fallback, where higher fuzzy scores reduce the
        //   penalty and therefore sort earlier
        if stub.is_empty() {
            return Some(MATCH_SCORE_EMPTY_STUB);
        }

        let stub_lc = fold_case(stub);
        let candidate_lc = fold_case(candidate);

        if stub_lc == candidate_lc {
            return Some(MATCH_SCORE_EXACT);
        }
        // Prefer deterministic prefix classes before fuzzy fallback so short
        // tab-completion stubs stay predictable.
        if candidate_lc.starts_with(&stub_lc) {
            return Some(MATCH_SCORE_PREFIX_BASE + (candidate_lc.len() - stub_lc.len()) as u32);
        }

        if let Some(boundary) = boundary_prefix_index(&candidate_lc, &stub_lc) {
            return Some(MATCH_SCORE_BOUNDARY_PREFIX_BASE + boundary as u32);
        }

        // Fuzzy matching is the rescue path, not the primary ranking model.
        // Its normalized penalty keeps rescues behind explicit prefix matches.
        let fuzzy = completion_fuzzy_matcher().fuzzy_match(&candidate_lc, &stub_lc)?;
        let normalized = fuzzy.max(0) as u32;
        let penalty = MATCH_SCORE_FUZZY_NORMALIZED_MAX.saturating_sub(normalized);
        Some(MATCH_SCORE_FUZZY_BASE + penalty)
    }

    fn resolved_flag_allowlist(
        &self,
        node: &CompletionNode,
        cmd: &CommandLine,
    ) -> Option<BTreeSet<String>> {
        let hints = node.flag_hints.as_ref()?;
        let mut allowed = hints.common.iter().cloned().collect::<BTreeSet<_>>();

        if let Some(provider) = ProviderSelection::from_command(cmd).name() {
            if let Some(provider_specific) = hints.by_provider.get(provider) {
                allowed.extend(provider_specific.iter().cloned());
            }
            // Once provider is selected, hide selector flags.
            allowed.remove("--provider");
            allowed.remove("--nrec");
            allowed.remove("--vmware");
        }

        if cmd.has_flag("--linux") {
            allowed.remove("--windows");
        }
        if cmd.has_flag("--windows") {
            allowed.remove("--linux");
        }

        Some(allowed)
    }

    fn required_flags(&self, node: &CompletionNode, cmd: &CommandLine) -> BTreeSet<String> {
        let mut required = BTreeSet::new();
        let Some(hints) = node.flag_hints.as_ref() else {
            return required;
        };

        required.extend(hints.required_common.iter().cloned());
        if let Some(provider) = ProviderSelection::from_command(cmd).name()
            && let Some(provider_required) = hints.required_by_provider.get(provider)
        {
            required.extend(provider_required.iter().cloned());
        }
        required
    }
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

    let mut summary = format!("subcommands: {}", preview.join(", "));
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

fn boundary_prefix_index(candidate: &str, stub: &str) -> Option<usize> {
    candidate
        .match_indices(stub)
        .find(|(idx, _)| {
            *idx == 0
                || candidate
                    .as_bytes()
                    .get(idx.saturating_sub(1))
                    .is_some_and(|byte| matches!(byte, b'-' | b'_' | b'.' | b':' | b'/'))
        })
        .map(|(idx, _)| idx)
}

#[cfg(test)]
mod tests;
