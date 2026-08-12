//! Completion-tree path resolution and command-derived context hints.
//!
//! The parser tells completion "what tokens exist near the cursor". This
//! module answers the next question: which completion-tree node and flag scope
//! those tokens imply, and which provider hints should influence the later
//! suggestion pass.

use std::collections::BTreeSet;

use crate::completion::model::{CommandLine, CompletionContext, CompletionNode, CompletionTree};
use crate::core::fuzzy::fold_case;

pub(crate) struct ResolvedNodes<'a> {
    pub(crate) context_node: &'a CompletionNode,
    pub(crate) flag_scope_node: &'a CompletionNode,
}

pub(crate) struct TreeResolver<'a> {
    tree: &'a CompletionTree,
}

impl<'a> TreeResolver<'a> {
    pub(crate) fn new(tree: &'a CompletionTree) -> Self {
        Self { tree }
    }

    pub(crate) fn matched_command_len_tokens(&self, tokens: &[String]) -> usize {
        let mut node = &self.tree.root;
        let mut matched = 0usize;

        for token in tokens {
            if token == "|" || token.starts_with('-') {
                break;
            }
            let Some(child) = node.children.get(token) else {
                break;
            };
            matched += 1;
            if child.value_key || child.value_leaf {
                break;
            }
            node = child;
        }

        matched
    }

    pub(crate) fn resolved_nodes(&self, context: &CompletionContext) -> ResolvedNodes<'a> {
        ResolvedNodes {
            context_node: self.resolve_exact_or_root(&context.matched_path),
            flag_scope_node: self.resolve_exact_or_root(&context.flag_scope_path),
        }
    }

    pub(crate) fn resolve_exact_or_root(&self, path: &[String]) -> &'a CompletionNode {
        self.resolve_exact(path).unwrap_or(&self.tree.root)
    }

    pub(crate) fn resolve_flag_scope_path(&self, matched_path: &[String]) -> Vec<String> {
        let floor = if matched_path.is_empty() { 0 } else { 1 };
        for i in (floor..=matched_path.len()).rev() {
            let prefix = &matched_path[..i];
            let Some(node) = self.resolve_exact(prefix) else {
                continue;
            };
            if !node.flags.is_empty() {
                return prefix.to_vec();
            }
        }
        if matched_path.is_empty() {
            Vec::new()
        } else {
            matched_path.to_vec()
        }
    }

    pub(crate) fn resolve_context(&self, path: &[String]) -> (&'a CompletionNode, Vec<String>) {
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

    pub(crate) fn resolve_exact(&self, path: &[String]) -> Option<&'a CompletionNode> {
        let (node, matched) = self.resolve_context(path);
        (matched.len() == path.len()).then_some(node)
    }
}

pub(crate) struct ProviderSelection<'a> {
    explicit: Option<&'a str>,
    candidates: BTreeSet<&'a str>,
}

impl<'a> ProviderSelection<'a> {
    pub(crate) fn from_command(cmd: &'a CommandLine, node: &'a CompletionNode) -> Self {
        let explicit = cmd
            .flag_values("--provider")
            .and_then(|values| values.first())
            .map(String::as_str)
            .filter(|value| !value.trim().is_empty());

        let Some(hints) = node.flag_hints.as_ref() else {
            return Self {
                explicit,
                candidates: BTreeSet::new(),
            };
        };

        let all = hints
            .by_provider
            .keys()
            .chain(hints.required_by_provider.keys())
            .map(String::as_str)
            .collect::<BTreeSet<_>>();

        if let Some(provider) = explicit {
            return Self {
                explicit,
                candidates: if all.contains(provider) {
                    BTreeSet::from([provider])
                } else {
                    all
                },
            };
        }

        let common = hints
            .common
            .iter()
            .chain(&hints.required_common)
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut candidates = all.clone();
        let mut constrained = false;

        for (flag, values) in cmd.flag_values_map() {
            if flag == "--provider" {
                continue;
            }

            if !common.contains(flag.as_str()) {
                let compatible =
                    all.iter()
                        .copied()
                        .filter(|provider| {
                            hints.by_provider.get(*provider).is_some_and(|flags| {
                                flags.iter().any(|candidate| candidate == flag)
                            }) || hints
                                .required_by_provider
                                .get(*provider)
                                .is_some_and(|flags| {
                                    flags.iter().any(|candidate| candidate == flag)
                                })
                        })
                        .collect::<BTreeSet<_>>();
                if !compatible.is_empty() {
                    constrained = true;
                    candidates = candidates
                        .intersection(&compatible)
                        .copied()
                        .collect::<BTreeSet<_>>();
                }
            }

            let Some(flag_node) = node.flags.get(flag) else {
                continue;
            };
            for value in values.iter().filter(|value| !value.is_empty()) {
                let value = fold_case(value);
                let matching = all
                    .iter()
                    .copied()
                    .filter(|provider| {
                        flag_node
                            .exhaustive_suggestions_by_provider
                            .contains(*provider)
                            && flag_node
                                .suggestions_by_provider
                                .get(*provider)
                                .is_some_and(|entries| {
                                    entries
                                        .iter()
                                        .any(|entry| fold_case(&entry.value).starts_with(&value))
                                })
                    })
                    .collect::<BTreeSet<_>>();
                // An unknown value is still valid input for the backend; it
                // contributes no completion constraint.
                if matching.is_empty() {
                    continue;
                }
                let compatible = all
                    .iter()
                    .copied()
                    .filter(|provider| {
                        // Advisory or free-form choices are not a closed
                        // catalog, so completion cannot exclude that provider.
                        matching.contains(provider)
                            || !flag_node
                                .exhaustive_suggestions_by_provider
                                .contains(*provider)
                    })
                    .collect::<BTreeSet<_>>();
                constrained = true;
                candidates = candidates
                    .intersection(&compatible)
                    .copied()
                    .collect::<BTreeSet<_>>();
            }
        }

        if constrained && candidates.is_empty() {
            candidates = all;
        }

        Self {
            explicit,
            candidates,
        }
    }

    pub(crate) fn name(&self) -> Option<&'a str> {
        if let Some(provider) = self.explicit {
            return (self.candidates.is_empty() || self.candidates.contains(provider))
                .then_some(provider);
        }
        (self.candidates.len() == 1)
            .then(|| self.candidates.first().copied())
            .flatten()
    }

    pub(crate) fn candidates(&self) -> impl Iterator<Item = &'a str> + '_ {
        self.candidates.iter().copied()
    }

    pub(crate) fn hides_selector(&self) -> bool {
        self.explicit.is_some() || self.candidates.len() == 1
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::ProviderSelection;
    use crate::completion::model::{
        CommandLine, CompletionNode, FlagHints, FlagNode, FlagOccurrence, SuggestionEntry,
    };

    fn hints() -> FlagHints {
        FlagHints {
            common: vec!["--comment".to_string(), "--provider".to_string()],
            by_provider: BTreeMap::from([
                (
                    "alpha".to_string(),
                    vec!["--cpu".to_string(), "--shared".to_string()],
                ),
                (
                    "beta".to_string(),
                    vec!["--instance".to_string(), "--shared".to_string()],
                ),
            ]),
            ..FlagHints::default()
        }
    }

    fn command(flags: &[&str]) -> CommandLine {
        let mut command = CommandLine::default();
        for flag in flags {
            command.push_flag_occurrence(FlagOccurrence {
                name: (*flag).to_string(),
                values: Vec::new(),
            });
        }
        command
    }

    fn node(hints: FlagHints) -> CompletionNode {
        CompletionNode {
            flag_hints: Some(hints),
            ..CompletionNode::default()
        }
    }

    #[test]
    fn provider_selection_infers_only_compatible_providers_from_flags_unit() {
        let hints = hints();
        let node = node(hints);

        let unique_command = command(&["--cpu"]);
        let unique = ProviderSelection::from_command(&unique_command, &node);
        assert_eq!(unique.name(), Some("alpha"));
        assert_eq!(unique.candidates().collect::<Vec<_>>(), vec!["alpha"]);
        assert!(unique.hides_selector());

        let ambiguous_command = command(&["--shared"]);
        let ambiguous = ProviderSelection::from_command(&ambiguous_command, &node);
        assert_eq!(ambiguous.name(), None);
        assert_eq!(
            ambiguous.candidates().collect::<Vec<_>>(),
            vec!["alpha", "beta"]
        );
        assert!(!ambiguous.hides_selector());
    }

    #[test]
    fn provider_selection_keeps_conflicting_and_unknown_flags_permissive_unit() {
        let hints = hints();
        let node = node(hints);

        for flags in [
            &["--cpu", "--instance"][..],
            &["--unknown"][..],
            &["--comment"][..],
        ] {
            let command = command(flags);
            let selection = ProviderSelection::from_command(&command, &node);
            assert_eq!(selection.name(), None);
            assert_eq!(
                selection.candidates().collect::<Vec<_>>(),
                vec!["alpha", "beta"]
            );
            assert!(!selection.hides_selector());
        }
    }

    #[test]
    fn provider_selection_never_replaces_an_explicit_unknown_provider_unit() {
        let hints = FlagHints {
            by_provider: BTreeMap::from([("alpha".to_string(), vec!["--cpu".to_string()])]),
            ..FlagHints::default()
        };
        let node = node(hints);
        let mut command = command(&["--cpu"]);
        command.push_flag_occurrence(FlagOccurrence {
            name: "--provider".to_string(),
            values: vec!["unknown".to_string()],
        });

        let selection = ProviderSelection::from_command(&command, &node);

        assert_eq!(selection.name(), None);
        assert_eq!(selection.candidates().collect::<Vec<_>>(), vec!["alpha"]);
        assert!(selection.hides_selector());
    }

    #[test]
    fn provider_selection_uses_shared_flag_value_catalogs_without_rejecting_input_unit() {
        let mut node = node(FlagHints {
            common: vec!["--os".to_string(), "--provider".to_string()],
            by_provider: BTreeMap::from([
                ("alpha".to_string(), Vec::new()),
                ("beta".to_string(), Vec::new()),
            ]),
            ..FlagHints::default()
        });
        node.flags.insert(
            "--os".to_string(),
            FlagNode {
                suggestions_by_provider: BTreeMap::from([
                    (
                        "alpha".to_string(),
                        vec![
                            SuggestionEntry::from("ubuntu"),
                            SuggestionEntry::from("shared"),
                        ],
                    ),
                    (
                        "beta".to_string(),
                        vec![
                            SuggestionEntry::from("rhel"),
                            SuggestionEntry::from("shared"),
                        ],
                    ),
                ]),
                exhaustive_suggestions_by_provider: BTreeSet::from([
                    "alpha".to_string(),
                    "beta".to_string(),
                ]),
                ..FlagNode::default()
            },
        );

        for (value, expected) in [("ubu", Some("alpha")), ("shared", None), ("unknown", None)] {
            let mut command = CommandLine::default();
            command.push_flag_occurrence(FlagOccurrence {
                name: "--os".to_string(),
                values: vec![value.to_string()],
            });
            let selection = ProviderSelection::from_command(&command, &node);
            assert_eq!(selection.name(), expected, "value: {value}");
        }

        let mut explicit = CommandLine::default();
        explicit.push_flag_occurrence(FlagOccurrence {
            name: "--os".to_string(),
            values: vec!["ubuntu".to_string()],
        });
        explicit.push_flag_occurrence(FlagOccurrence {
            name: "--provider".to_string(),
            values: vec!["beta".to_string()],
        });
        assert_eq!(
            ProviderSelection::from_command(&explicit, &node).name(),
            Some("beta")
        );
    }

    #[test]
    fn provider_selection_only_excludes_providers_with_exhaustive_value_catalogs_unit() {
        let mut node = node(FlagHints {
            common: vec!["--os".to_string()],
            by_provider: BTreeMap::from([
                ("alpha".to_string(), Vec::new()),
                ("beta".to_string(), Vec::new()),
            ]),
            ..FlagHints::default()
        });
        node.flags.insert(
            "--os".to_string(),
            FlagNode {
                suggestions_by_provider: BTreeMap::from([
                    ("alpha".to_string(), vec![SuggestionEntry::from("ubuntu")]),
                    ("beta".to_string(), Vec::new()),
                ]),
                ..FlagNode::default()
            },
        );
        let mut command = CommandLine::default();
        command.push_flag_occurrence(FlagOccurrence {
            name: "--os".to_string(),
            values: vec!["ubu".to_string()],
        });

        assert_eq!(
            ProviderSelection::from_command(&command, &node).name(),
            None
        );

        node.flags
            .get_mut("--os")
            .unwrap()
            .exhaustive_suggestions_by_provider
            .insert("alpha".to_string());
        assert_eq!(
            ProviderSelection::from_command(&command, &node).name(),
            None
        );

        node.flags
            .get_mut("--os")
            .unwrap()
            .exhaustive_suggestions_by_provider
            .insert("beta".to_string());
        assert_eq!(
            ProviderSelection::from_command(&command, &node).name(),
            Some("alpha")
        );
    }
}
