use crate::model::{
    CommandLine, CompletionNode, CompletionTree, Suggestion, SuggestionEntry, SuggestionOutput,
    ValueType,
};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::collections::BTreeSet;
use std::sync::OnceLock;

#[derive(Debug, Clone)]
pub struct SuggestionEngine {
    tree: CompletionTree,
}

impl SuggestionEngine {
    pub fn new(tree: CompletionTree) -> Self {
        Self { tree }
    }

    pub fn generate(&self, cmd: &CommandLine, stub: &str) -> Vec<SuggestionOutput> {
        if cmd.has_pipe {
            let mut out = self.pipe_suggestions(stub);
            sort_suggestion_outputs(&mut out);
            return out;
        }

        let (context_node, matched_path, has_subcommands) = self.resolve_context_state(cmd, stub);
        let flag_scope = self.nearest_flag_scope(&matched_path);

        if stub.starts_with('-') {
            let mut out = self
                .flag_name_suggestions(flag_scope, stub, cmd)
                .into_iter()
                .map(SuggestionOutput::Item)
                .collect::<Vec<_>>();
            sort_suggestion_outputs(&mut out);
            return out;
        }

        let (needs_value, last_flag) = self.last_flag_needs_value(flag_scope, cmd, stub);
        if needs_value && let Some(flag) = last_flag {
            let mut out = self.flag_value_suggestions(flag_scope, &flag, stub, cmd);
            sort_suggestion_outputs(&mut out);
            return out;
        }

        let mut out: Vec<SuggestionOutput> = Vec::new();

        let mut arg_tokens: Vec<String> = cmd
            .head
            .iter()
            .skip(matched_path.len())
            .filter(|token| token.as_str() != stub)
            .cloned()
            .collect();
        arg_tokens.extend(
            cmd.args
                .iter()
                .filter(|token| token.as_str() != stub)
                .cloned(),
        );

        let subcommand_context =
            context_node.value_key || (has_subcommands && arg_tokens.is_empty());

        if subcommand_context {
            out.extend(
                self.subcommand_suggestions(context_node, stub)
                    .into_iter()
                    .map(SuggestionOutput::Item),
            );
        } else {
            out.extend(self.arg_value_suggestions(context_node, arg_tokens.len(), stub));
        }

        if stub.is_empty() && !needs_value && !subcommand_context {
            out.extend(
                self.flag_name_suggestions(flag_scope, stub, cmd)
                    .into_iter()
                    .filter(|s| !cmd.flags.contains_key(&s.text))
                    .map(SuggestionOutput::Item),
            );
        }

        sort_suggestion_outputs(&mut out);
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

    fn resolve_context_state<'a>(
        &'a self,
        cmd: &CommandLine,
        stub: &str,
    ) -> (&'a CompletionNode, Vec<String>, bool) {
        let (pre_node, _) = self.resolve_context(&cmd.head);
        let has_subcommands = !pre_node.children.is_empty();

        let head_for_context = if !stub.is_empty() && !stub.starts_with('-') && has_subcommands {
            &cmd.head[..cmd.head.len().saturating_sub(1)]
        } else {
            cmd.head.as_slice()
        };

        let (node, matched) = self.resolve_context(head_for_context);
        (node, matched, has_subcommands)
    }

    fn resolve_context<'a>(&'a self, path: &[String]) -> (&'a CompletionNode, Vec<String>) {
        let mut node = &self.tree.root;
        let mut matched = Vec::new();

        for segment in path {
            let Some(next) = node.children.get(segment) else {
                break;
            };
            node = next;
            matched.push(segment.clone());
            if node.value_leaf {
                break;
            }
        }

        (node, matched)
    }

    fn nearest_flag_scope<'a>(&'a self, path: &[String]) -> &'a CompletionNode {
        for i in (0..=path.len()).rev() {
            let prefix = &path[..i];
            let (node, matched) = self.resolve_context(prefix);
            if matched.len() == prefix.len() && !node.flags.is_empty() {
                return node;
            }
        }
        &self.tree.root
    }

    fn flag_name_suggestions(
        &self,
        node: &CompletionNode,
        stub: &str,
        cmd: &CommandLine,
    ) -> Vec<Suggestion> {
        let allowlist = self.resolved_flag_allowlist(node, cmd);
        let required = self.required_flags(node, cmd);

        node.flags
            .iter()
            .filter_map(|(flag, meta)| {
                let score = self.match_score(stub, flag)?;
                Some((flag, meta, score))
            })
            .filter(|(flag, _, _)| {
                allowlist
                    .as_ref()
                    .is_none_or(|allowed| allowed.contains(flag.as_str()))
            })
            .filter(|(flag, _, _)| !cmd.flags.contains_key(*flag) || stub == *flag)
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

        if flag == "--provider" {
            let os_token = cmd
                .flags
                .get("--os")
                .and_then(|vals| vals.first())
                .map(|value| normalize_token(value));
            if let Some(os_token) = os_token {
                let filtered: Vec<SuggestionOutput> = flag_node
                    .suggestions
                    .iter()
                    .filter(|entry| {
                        flag_node
                            .os_provider_map
                            .get(&os_token)
                            .is_none_or(|providers| providers.iter().any(|p| p == &entry.value))
                    })
                    .filter_map(|entry| {
                        let score = self.match_score(stub, &entry.value)?;
                        Some(SuggestionOutput::Item(entry_to_suggestion(entry, score)))
                    })
                    .collect();
                if !filtered.is_empty() {
                    return filtered;
                }
            }
        }

        if !flag_node.suggestions_by_provider.is_empty()
            && let Some(provider) = selected_provider(cmd)
            && let Some(provider_values) = flag_node.suggestions_by_provider.get(&provider)
        {
            return provider_values
                .iter()
                .filter_map(|entry| {
                    let score = self.match_score(stub, &entry.value)?;
                    Some(SuggestionOutput::Item(entry_to_suggestion(entry, score)))
                })
                .collect();
        }

        flag_node
            .suggestions
            .iter()
            .filter_map(|entry| {
                let score = self.match_score(stub, &entry.value)?;
                Some(SuggestionOutput::Item(entry_to_suggestion(entry, score)))
            })
            .collect()
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

        arg.suggestions
            .iter()
            .filter_map(|entry| {
                let score = self.match_score(stub, &entry.value)?;
                Some(SuggestionOutput::Item(entry_to_suggestion(entry, score)))
            })
            .collect()
    }

    fn subcommand_suggestions(&self, node: &CompletionNode, stub: &str) -> Vec<Suggestion> {
        node.children
            .iter()
            .filter_map(|(name, child)| {
                let score = self.match_score(stub, name)?;
                Some(Suggestion {
                    text: name.clone(),
                    meta: child.tooltip.clone(),
                    display: None,
                    is_exact: score == 0,
                    sort: None,
                    match_score: score,
                })
            })
            .collect()
    }

    fn last_flag_needs_value(
        &self,
        node: &CompletionNode,
        cmd: &CommandLine,
        stub: &str,
    ) -> (bool, Option<String>) {
        let Some(last_flag) = cmd.flag_order.last() else {
            return (false, None);
        };

        let Some(flag_node) = node.flags.get(last_flag) else {
            return (false, None);
        };

        if flag_node.flag_only {
            return (false, None);
        }

        let values = cmd.flags.get(last_flag).cloned().unwrap_or_default();
        if values.is_empty() {
            return (true, Some(last_flag.clone()));
        }

        if !stub.is_empty()
            && values.last().is_some_and(|value| {
                value
                    .to_ascii_lowercase()
                    .starts_with(&stub.to_ascii_lowercase())
            })
        {
            return (true, Some(last_flag.clone()));
        }

        (flag_node.multi, Some(last_flag.clone()))
    }

    fn match_score(&self, stub: &str, candidate: &str) -> Option<u32> {
        if stub.is_empty() {
            return Some(1_000);
        }

        let stub_lc = stub.to_ascii_lowercase();
        let candidate_lc = candidate.to_ascii_lowercase();

        if stub_lc == candidate_lc {
            return Some(0);
        }
        if candidate_lc.starts_with(&stub_lc) {
            return Some(100 + (candidate_lc.len() - stub_lc.len()) as u32);
        }

        if let Some(boundary) = boundary_prefix_index(&candidate_lc, &stub_lc) {
            return Some(200 + boundary as u32);
        }

        let fuzzy = fuzzy_matcher().fuzzy_match(&candidate_lc, &stub_lc)?;
        let normalized = fuzzy.max(0) as u32;
        let penalty = 100_000u32.saturating_sub(normalized);
        Some(10_000 + penalty)
    }

    fn resolved_flag_allowlist(
        &self,
        node: &CompletionNode,
        cmd: &CommandLine,
    ) -> Option<BTreeSet<String>> {
        let hints = node.flag_hints.as_ref()?;
        let mut allowed = hints.common.iter().cloned().collect::<BTreeSet<_>>();

        if let Some(provider) = selected_provider(cmd) {
            if let Some(provider_specific) = hints.by_provider.get(&provider) {
                allowed.extend(provider_specific.iter().cloned());
            }
            // Once provider is selected, hide selector flags.
            allowed.remove("--provider");
            allowed.remove("--nrec");
            allowed.remove("--vmware");
        }

        if cmd.flags.contains_key("--linux") {
            allowed.remove("--windows");
        }
        if cmd.flags.contains_key("--windows") {
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
        if let Some(provider) = selected_provider(cmd)
            && let Some(provider_required) = hints.required_by_provider.get(&provider)
        {
            required.extend(provider_required.iter().cloned());
        }
        required
    }
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
        .then_with(|| {
            left.text
                .to_ascii_lowercase()
                .cmp(&right.text.to_ascii_lowercase())
        })
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
                _ => left.to_ascii_lowercase().cmp(&right.to_ascii_lowercase()),
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

fn fuzzy_matcher() -> &'static SkimMatcherV2 {
    static MATCHER: OnceLock<SkimMatcherV2> = OnceLock::new();
    MATCHER.get_or_init(SkimMatcherV2::default)
}

fn normalize_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '-', '_'], "")
}

fn selected_provider(cmd: &CommandLine) -> Option<String> {
    if let Some(provider_values) = cmd.flags.get("--provider")
        && let Some(provider) = provider_values.first()
    {
        let trimmed = provider.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    if cmd.flags.contains_key("--nrec") {
        return Some("nrec".to_string());
    }
    if cmd.flags.contains_key("--vmware") {
        return Some("vmware".to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::model::{
        ArgNode, CommandLine, CompletionNode, CompletionTree, FlagHints, FlagNode, SuggestionEntry,
        SuggestionOutput, ValueType,
    };

    use super::SuggestionEngine;

    fn tree() -> CompletionTree {
        let mut provision = CompletionNode::default();
        provision.flags.insert(
            "--provider".to_string(),
            FlagNode {
                suggestions: vec![
                    SuggestionEntry::from("nrec"),
                    SuggestionEntry::from("vmware"),
                ],
                os_provider_map: BTreeMap::from([
                    ("alma".to_string(), vec!["nrec".to_string()]),
                    ("rhel".to_string(), vec!["vmware".to_string()]),
                ]),
                ..FlagNode::default()
            },
        );
        provision.flags.insert(
            "--os".to_string(),
            FlagNode {
                suggestions: vec![SuggestionEntry::from("alma"), SuggestionEntry::from("rhel")],
                ..FlagNode::default()
            },
        );

        let mut orch = CompletionNode::default();
        orch.children.insert("provision".to_string(), provision);

        CompletionTree {
            root: CompletionNode::default().with_child("orch", orch),
            pipe_verbs: BTreeMap::from([("F".to_string(), "Filter".to_string())]),
        }
    }

    fn values(output: Vec<SuggestionOutput>) -> Vec<String> {
        output
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item.text),
                SuggestionOutput::PathSentinel => None,
            })
            .collect()
    }

    #[test]
    fn suggests_flags_in_scope() {
        let engine = SuggestionEngine::new(tree());
        let cmd = CommandLine {
            head: vec!["orch".to_string(), "provision".to_string()],
            ..CommandLine::default()
        };

        let values = values(engine.generate(&cmd, "--"));
        assert!(values.contains(&"--provider".to_string()));
        assert!(values.contains(&"--os".to_string()));
    }

    #[test]
    fn fuzzy_matches_flag_names() {
        let engine = SuggestionEngine::new(tree());
        let cmd = CommandLine {
            head: vec!["orch".to_string(), "provision".to_string()],
            ..CommandLine::default()
        };

        let values = values(engine.generate(&cmd, "--prv"));
        assert!(values.contains(&"--provider".to_string()));
    }

    #[test]
    fn suggests_flag_values() {
        let engine = SuggestionEngine::new(tree());
        let cmd = CommandLine {
            head: vec!["orch".to_string(), "provision".to_string()],
            flags: BTreeMap::from([("--provider".to_string(), Vec::new())]),
            flag_order: vec!["--provider".to_string()],
            ..CommandLine::default()
        };

        let values = values(engine.generate(&cmd, ""));

        assert!(values.contains(&"nrec".to_string()));
        assert!(values.contains(&"vmware".to_string()));
    }

    #[test]
    fn filters_provider_values_by_os() {
        let engine = SuggestionEngine::new(tree());
        let cmd = CommandLine {
            head: vec!["orch".to_string(), "provision".to_string()],
            flags: BTreeMap::from([
                ("--os".to_string(), vec!["alma".to_string()]),
                ("--provider".to_string(), Vec::new()),
            ]),
            flag_order: vec!["--os".to_string(), "--provider".to_string()],
            ..CommandLine::default()
        };

        let values = values(engine.generate(&cmd, ""));

        assert!(values.contains(&"nrec".to_string()));
        assert!(!values.contains(&"vmware".to_string()));
    }

    #[test]
    fn suggests_pipe_verbs_after_pipe() {
        let engine = SuggestionEngine::new(tree());
        let cmd = CommandLine {
            has_pipe: true,
            ..CommandLine::default()
        };

        let output = engine.generate(&cmd, "F");
        assert!(
            output
                .iter()
                .any(|entry| matches!(entry, SuggestionOutput::Item(item) if item.text == "F"))
        );
    }

    #[test]
    fn fuzzy_matches_long_pipe_verbs() {
        let mut tree = tree();
        tree.pipe_verbs
            .insert("VALUE".to_string(), "Extract values".to_string());
        tree.pipe_verbs
            .insert("VAL".to_string(), "Extract".to_string());
        let engine = SuggestionEngine::new(tree);
        let cmd = CommandLine {
            has_pipe: true,
            ..CommandLine::default()
        };

        let output = engine.generate(&cmd, "vlu");
        assert!(
            output
                .iter()
                .any(|entry| matches!(entry, SuggestionOutput::Item(item) if item.text == "VALUE"))
        );
        let values = values(output);
        assert_eq!(values.first().map(String::as_str), Some("VALUE"));
    }

    #[test]
    fn single_value_flag_switches_to_other_flags_after_value() {
        let mut cmd_node = CompletionNode::default();
        cmd_node.flags.insert(
            "--context".to_string(),
            FlagNode {
                suggestions: vec![
                    SuggestionEntry::from("uio"),
                    SuggestionEntry::from("tsd"),
                    SuggestionEntry::from("edu"),
                ],
                ..FlagNode::default()
            },
        );
        cmd_node.flags.insert(
            "--terminal".to_string(),
            FlagNode {
                suggestions: vec![SuggestionEntry::from("cli"), SuggestionEntry::from("repl")],
                ..FlagNode::default()
            },
        );

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("alias", cmd_node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);

        let cmd = CommandLine {
            head: vec!["alias".to_string()],
            flags: BTreeMap::from([("--context".to_string(), vec!["uio".to_string()])]),
            flag_order: vec!["--context".to_string()],
            ..CommandLine::default()
        };
        let values = values(engine.generate(&cmd, ""));
        assert!(!values.contains(&"uio".to_string()));
        assert!(values.contains(&"--terminal".to_string()));
    }

    #[test]
    fn multi_value_flag_stays_in_value_mode_until_dash() {
        let mut cmd_node = CompletionNode::default();
        cmd_node.flags.insert(
            "--tags".to_string(),
            FlagNode {
                multi: true,
                suggestions: vec![
                    SuggestionEntry::from("red"),
                    SuggestionEntry::from("green"),
                    SuggestionEntry::from("blue"),
                ],
                ..FlagNode::default()
            },
        );
        cmd_node.flags.insert(
            "--mode".to_string(),
            FlagNode {
                suggestions: vec![SuggestionEntry::from("fast"), SuggestionEntry::from("full")],
                ..FlagNode::default()
            },
        );

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("tag", cmd_node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);

        let cmd = CommandLine {
            head: vec!["tag".to_string()],
            flags: BTreeMap::from([("--tags".to_string(), vec!["red".to_string()])]),
            flag_order: vec!["--tags".to_string()],
            ..CommandLine::default()
        };
        let values_for_space = values(engine.generate(&cmd, ""));
        assert!(values_for_space.contains(&"red".to_string()));
        assert!(!values_for_space.contains(&"--mode".to_string()));

        let values_for_dash = values(engine.generate(&cmd, "-"));
        assert!(values_for_dash.contains(&"--mode".to_string()));
    }

    #[test]
    fn args_after_double_dash_advance_index() {
        let cmd_node = CompletionNode {
            args: vec![
                ArgNode {
                    suggestions: vec![SuggestionEntry::from("one")],
                    ..ArgNode::default()
                },
                ArgNode {
                    suggestions: vec![SuggestionEntry::from("two"), SuggestionEntry::from("three")],
                    ..ArgNode::default()
                },
            ],
            ..CompletionNode::default()
        };
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("cmd", cmd_node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);
        let cmd = CommandLine {
            head: vec!["cmd".to_string()],
            args: vec!["one".to_string()],
            ..CommandLine::default()
        };

        let values = values(engine.generate(&cmd, ""));
        assert!(values.contains(&"two".to_string()));
        assert!(values.contains(&"three".to_string()));
        assert!(!values.contains(&"one".to_string()));
    }

    #[test]
    fn path_arg_emits_path_sentinel() {
        let cmd_node = CompletionNode {
            args: vec![ArgNode {
                value_type: Some(ValueType::Path),
                ..ArgNode::default()
            }],
            ..CompletionNode::default()
        };
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("cmd", cmd_node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);
        let cmd = CommandLine {
            head: vec!["cmd".to_string()],
            ..CommandLine::default()
        };

        let output = engine.generate(&cmd, "");
        assert!(
            output
                .iter()
                .any(|entry| matches!(entry, SuggestionOutput::PathSentinel))
        );
    }

    #[test]
    fn flag_hints_filter_provider_specific_flags_and_hide_selectors() {
        let mut node = CompletionNode::default();
        node.flags
            .insert("--provider".to_string(), FlagNode::default());
        node.flags.insert(
            "--nrec".to_string(),
            FlagNode {
                flag_only: true,
                ..FlagNode::default()
            },
        );
        node.flags.insert(
            "--vmware".to_string(),
            FlagNode {
                flag_only: true,
                ..FlagNode::default()
            },
        );
        node.flags
            .insert("--comment".to_string(), FlagNode::default());
        node.flags
            .insert("--flavor".to_string(), FlagNode::default());
        node.flags
            .insert("--vcenter".to_string(), FlagNode::default());
        node.flag_hints = Some(FlagHints {
            common: vec![
                "--provider".to_string(),
                "--nrec".to_string(),
                "--vmware".to_string(),
                "--comment".to_string(),
            ],
            by_provider: BTreeMap::from([
                ("nrec".to_string(), vec!["--flavor".to_string()]),
                ("vmware".to_string(), vec!["--vcenter".to_string()]),
            ]),
            required_common: vec!["--comment".to_string()],
            required_by_provider: BTreeMap::from([(
                "nrec".to_string(),
                vec!["--flavor".to_string()],
            )]),
        });

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("provision", node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);

        let cmd = CommandLine {
            head: vec!["provision".to_string()],
            flags: BTreeMap::from([("--provider".to_string(), vec!["nrec".to_string()])]),
            flag_order: vec!["--provider".to_string()],
            ..CommandLine::default()
        };
        let output = engine.generate(&cmd, "--");
        let values = values(output.clone());
        assert!(values.contains(&"--comment".to_string()));
        assert!(values.contains(&"--flavor".to_string()));
        assert!(!values.contains(&"--provider".to_string()));
        assert!(!values.contains(&"--nrec".to_string()));
        assert!(!values.contains(&"--vmware".to_string()));
        assert!(!values.contains(&"--vcenter".to_string()));

        let items = output
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item),
                SuggestionOutput::PathSentinel => None,
            })
            .collect::<Vec<_>>();
        let by_text = items
            .into_iter()
            .map(|item| (item.text.clone(), item))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            by_text
                .get("--comment")
                .and_then(|item| item.display.as_deref()),
            Some("--comment*")
        );
        assert_eq!(
            by_text
                .get("--flavor")
                .and_then(|item| item.display.as_deref()),
            Some("--flavor*")
        );
    }

    #[test]
    fn provider_alias_flag_enables_provider_specific_allowlist() {
        let mut node = CompletionNode::default();
        node.flags
            .insert("--provider".to_string(), FlagNode::default());
        node.flags.insert(
            "--nrec".to_string(),
            FlagNode {
                flag_only: true,
                ..FlagNode::default()
            },
        );
        node.flags
            .insert("--flavor".to_string(), FlagNode::default());
        node.flag_hints = Some(FlagHints {
            common: vec!["--provider".to_string(), "--nrec".to_string()],
            by_provider: BTreeMap::from([("nrec".to_string(), vec!["--flavor".to_string()])]),
            ..FlagHints::default()
        });

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("provision", node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);
        let cmd = CommandLine {
            head: vec!["provision".to_string()],
            flags: BTreeMap::from([("--nrec".to_string(), Vec::new())]),
            flag_order: vec!["--nrec".to_string()],
            ..CommandLine::default()
        };

        let values = values(engine.generate(&cmd, "--"));
        assert!(values.contains(&"--flavor".to_string()));
        assert!(!values.contains(&"--provider".to_string()));
    }

    #[test]
    fn path_flag_emits_path_sentinel() {
        let mut node = CompletionNode::default();
        node.flags.insert(
            "--file".to_string(),
            FlagNode {
                value_type: Some(ValueType::Path),
                ..FlagNode::default()
            },
        );

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("cmd", node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);
        let cmd = CommandLine {
            head: vec!["cmd".to_string()],
            flags: BTreeMap::from([("--file".to_string(), Vec::new())]),
            flag_order: vec!["--file".to_string()],
            ..CommandLine::default()
        };

        let output = engine.generate(&cmd, "");
        assert!(
            output
                .iter()
                .any(|entry| matches!(entry, SuggestionOutput::PathSentinel))
        );
    }

    #[test]
    fn flag_suggestions_preserve_meta_and_display_fields() {
        let mut node = CompletionNode::default();
        node.flags.insert(
            "--flavor".to_string(),
            FlagNode {
                suggestions: vec![
                    SuggestionEntry {
                        value: "m1.small".to_string(),
                        meta: Some("1 vCPU".to_string()),
                        display: Some("small".to_string()),
                        sort: Some("10".to_string()),
                    },
                    SuggestionEntry::from("m1.medium"),
                ],
                ..FlagNode::default()
            },
        );
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("orch", node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);
        let cmd = CommandLine {
            head: vec!["orch".to_string()],
            flags: BTreeMap::from([("--flavor".to_string(), Vec::new())]),
            flag_order: vec!["--flavor".to_string()],
            ..CommandLine::default()
        };

        let output = engine.generate(&cmd, "");
        let items = output
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item),
                SuggestionOutput::PathSentinel => None,
            })
            .collect::<Vec<_>>();

        let rich = items
            .iter()
            .find(|item| item.text == "m1.small")
            .expect("m1.small suggestion should exist");
        assert_eq!(rich.meta.as_deref(), Some("1 vCPU"));
        assert_eq!(rich.display.as_deref(), Some("small"));
        assert_eq!(rich.sort.as_deref(), Some("10"));
    }

    #[test]
    fn arg_suggestions_honor_numeric_sort_after_match_score() {
        let cmd_node = CompletionNode {
            args: vec![ArgNode {
                suggestions: vec![
                    SuggestionEntry {
                        value: "v10".to_string(),
                        meta: None,
                        display: None,
                        sort: Some("10".to_string()),
                    },
                    SuggestionEntry {
                        value: "v2".to_string(),
                        meta: None,
                        display: None,
                        sort: Some("2".to_string()),
                    },
                ],
                ..ArgNode::default()
            }],
            ..CompletionNode::default()
        };
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("cmd", cmd_node),
            ..CompletionTree::default()
        };
        let engine = SuggestionEngine::new(tree);
        let cmd = CommandLine {
            head: vec!["cmd".to_string()],
            ..CommandLine::default()
        };

        let values = values(engine.generate(&cmd, ""));
        assert_eq!(values, vec!["v2".to_string(), "v10".to_string()]);
    }
}
