use crate::completion::{
    context::TreeResolver,
    model::{
        CommandLine, CompletionAnalysis, CompletionContext, CompletionNode, CompletionRequest,
        CompletionTree, ContextScope, CursorState, MatchKind, ParsedLine, SuggestionOutput,
        TailItem,
    },
    parse::CommandLineParser,
    suggest::SuggestionEngine,
};
use crate::core::fuzzy::fold_case;
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
/// High-level entry point for parsing and completing command lines.
pub struct CompletionEngine {
    parser: CommandLineParser,
    suggester: SuggestionEngine,
    tree: CompletionTree,
    global_context_flags: BTreeSet<String>,
}

impl CompletionEngine {
    /// Creates an engine for a prebuilt completion tree.
    pub fn new(tree: CompletionTree) -> Self {
        let global_context_flags = collect_global_context_flags(&tree.root);
        Self {
            parser: CommandLineParser,
            suggester: SuggestionEngine::new(tree.clone()),
            tree,
            global_context_flags,
        }
    }

    /// Parses `line` at `cursor` and returns the cursor state with suggestions.
    pub fn complete(&self, line: &str, cursor: usize) -> (CursorState, Vec<SuggestionOutput>) {
        let analysis = self.analyze(line, cursor);
        let suggestions = self.suggestions_for_analysis(&analysis);
        (analysis.cursor, suggestions)
    }

    /// Generates suggestions for a previously computed completion analysis.
    pub fn suggestions_for_analysis(&self, analysis: &CompletionAnalysis) -> Vec<SuggestionOutput> {
        self.suggester.generate(analysis)
    }

    /// Parses `line` and resolves completion context at `cursor`.
    pub fn analyze(&self, line: &str, cursor: usize) -> CompletionAnalysis {
        let parsed = self.parser.analyze(line, cursor);

        self.analyze_command_parts(parsed.parsed, parsed.cursor)
    }

    /// Resolves completion state from pre-parsed command representations.
    ///
    /// This is mainly useful for tests or callers that already have split
    /// command state for the full line and the cursor-local prefix.
    pub fn analyze_command(
        &self,
        full_cmd: CommandLine,
        cursor_cmd: CommandLine,
        cursor: CursorState,
    ) -> CompletionAnalysis {
        self.analyze_command_parts(
            ParsedLine {
                safe_cursor: 0,
                full_tokens: Vec::new(),
                cursor_tokens: Vec::new(),
                full_cmd,
                cursor_cmd,
            },
            cursor,
        )
    }

    fn analyze_command_parts(
        &self,
        mut parsed: ParsedLine,
        cursor: CursorState,
    ) -> CompletionAnalysis {
        // Prefilled alias and shell defaults can change the effective scope, so
        // resolve once to inject them and then resolve again against the
        // completed command state.
        let mut context =
            self.resolve_completion_context(&parsed.cursor_cmd, cursor.token_stub.as_str());
        self.merge_prefilled_values(&mut parsed.cursor_cmd, &context.matched_path);
        context = self.resolve_completion_context(&parsed.cursor_cmd, cursor.token_stub.as_str());

        // Context-only flags can appear later in the line than the cursor.
        // Merge them before scope resolution so completion reflects the user's
        // effective command state, not just the prefix before the cursor.
        if !parsed.cursor_cmd.has_pipe() {
            self.merge_context_flags(
                &mut parsed.cursor_cmd,
                &parsed.full_cmd,
                cursor.token_stub.as_str(),
            );
        }

        let request = self.build_completion_request(&parsed.cursor_cmd, &cursor, &context);

        CompletionAnalysis {
            parsed,
            cursor,
            context,
            request,
        }
    }

    /// Tokenizes a shell-like command line using the parser's permissive rules.
    pub fn tokenize(&self, line: &str) -> Vec<String> {
        self.parser.tokenize(line)
    }

    /// Returns how many leading tokens resolve to a command path in the tree.
    pub fn matched_command_len_tokens(&self, tokens: &[String]) -> usize {
        TreeResolver::new(&self.tree).matched_command_len_tokens(tokens)
    }

    /// Classifies a candidate value relative to the current completion analysis.
    pub fn classify_match(&self, analysis: &CompletionAnalysis, value: &str) -> MatchKind {
        if analysis.parsed.cursor_cmd.has_pipe() {
            return MatchKind::Pipe;
        }
        let nodes = TreeResolver::new(&self.tree).resolved_nodes(&analysis.context);

        if value.starts_with("--") || nodes.flag_scope_node.flags.contains_key(value) {
            return MatchKind::Flag;
        }
        if nodes.context_node.children.contains_key(value) {
            return if analysis.context.matched_path.is_empty() {
                MatchKind::Command
            } else {
                MatchKind::Subcommand
            };
        }
        MatchKind::Value
    }

    fn merge_context_flags(
        &self,
        cursor_cmd: &mut CommandLine,
        full_cmd: &CommandLine,
        stub: &str,
    ) {
        let context = self.resolve_completion_context(cursor_cmd, stub);
        let mut scoped_flags = BTreeSet::new();
        let resolver = TreeResolver::new(&self.tree);
        for i in (0..=context.matched_path.len()).rev() {
            let (node, matched) = resolver.resolve_context(&context.matched_path[..i]);
            if matched.len() == i {
                scoped_flags.extend(node.flags.keys().cloned());
            }
        }
        scoped_flags.extend(self.global_context_flags.iter().cloned());

        for item in full_cmd.tail().iter().skip(cursor_cmd.tail_len()) {
            let TailItem::Flag(flag) = item else {
                continue;
            };
            if cursor_cmd.has_flag(&flag.name) {
                continue;
            }
            if !scoped_flags.contains(&flag.name) {
                continue;
            }
            cursor_cmd.merge_flag_values(flag.name.clone(), flag.values.clone());
        }
    }

    fn merge_prefilled_values(&self, cursor_cmd: &mut CommandLine, matched_path: &[String]) {
        let resolver = TreeResolver::new(&self.tree);
        let mut prefilled_positionals = Vec::new();
        for i in 0..=matched_path.len() {
            let Some(node) = resolver.resolve_exact(&matched_path[..i]) else {
                continue;
            };
            // Prefilled values are inherited down the matched command path so
            // alias targets and shell roots behave like already-typed input.
            prefilled_positionals.extend(node.prefilled_positionals.iter().cloned());
            for (flag, values) in &node.prefilled_flags {
                if cursor_cmd.has_flag(flag) {
                    continue;
                }
                cursor_cmd.merge_flag_values(flag.clone(), values.clone());
            }
        }
        cursor_cmd.prepend_positional_values(prefilled_positionals);
    }

    fn resolve_completion_context(&self, cmd: &CommandLine, stub: &str) -> CompletionContext {
        let resolver = TreeResolver::new(&self.tree);
        let exact_token_commits = if !stub.is_empty() && !stub.starts_with('-') {
            let parent_path = &cmd.head()[..cmd.head().len().saturating_sub(1)];
            resolver
                .resolve_exact(parent_path)
                .and_then(|node| node.children.get(stub))
                .is_some_and(|child| child.exact_token_commits)
        } else {
            false
        };
        // A command token is not committed until the user types a delimiter.
        // Keep exact and partial head tokens in the parent scope so Tab keeps
        // cycling sibling commands until a trailing space commits the token,
        // unless the exact token explicitly commits scope on its own.
        let head_without_partial_subcommand = if !stub.is_empty()
            && !stub.starts_with('-')
            && cmd.head().last().is_some_and(|token| token == stub)
            && !exact_token_commits
        {
            &cmd.head()[..cmd.head().len().saturating_sub(1)]
        } else {
            cmd.head()
        };
        let (_, matched) = resolver.resolve_context(head_without_partial_subcommand);
        let flag_scope_path = resolver.resolve_flag_scope_path(&matched);

        // Keep the in-progress stub out of arg accounting so a partial
        // subcommand or value does not shift completion into the next slot.
        let arg_tokens: Vec<String> = cmd
            .head()
            .iter()
            .skip(matched.len())
            .filter(|token| token.as_str() != stub)
            .cloned()
            .chain(
                cmd.positional_args()
                    .filter(|token| token.as_str() != stub)
                    .cloned(),
            )
            .collect();

        let context_node = resolver.resolve_exact(&matched).unwrap_or(&self.tree.root);
        let has_subcommands = !context_node.children.is_empty();
        let subcommand_context =
            context_node.value_key || (has_subcommands && arg_tokens.is_empty());

        CompletionContext {
            matched_path: matched,
            flag_scope_path,
            subcommand_context,
        }
    }

    fn build_completion_request(
        &self,
        cmd: &CommandLine,
        cursor: &CursorState,
        context: &CompletionContext,
    ) -> CompletionRequest {
        let stub = cursor.token_stub.as_str();
        if cmd.has_pipe() {
            return CompletionRequest::Pipe;
        }

        if stub.starts_with('-') {
            return CompletionRequest::FlagNames {
                flag_scope_path: context.flag_scope_path.clone(),
            };
        }

        let resolver = TreeResolver::new(&self.tree);
        let flag_scope_node = resolver
            .resolve_exact(&context.flag_scope_path)
            .unwrap_or(&self.tree.root);
        let (needs_flag_value, last_flag) = last_flag_needs_value(flag_scope_node, cmd, stub);
        if needs_flag_value && let Some(flag) = last_flag {
            return CompletionRequest::FlagValues {
                flag_scope_path: context.flag_scope_path.clone(),
                flag,
            };
        }

        CompletionRequest::Positionals {
            context_path: context.matched_path.clone(),
            flag_scope_path: context.flag_scope_path.clone(),
            arg_index: positional_arg_index(cmd, stub, context.matched_path.len()),
            show_subcommands: context.subcommand_context,
            show_flag_names: stub.is_empty() && !context.subcommand_context,
        }
    }
}

fn last_flag_needs_value(
    node: &CompletionNode,
    cmd: &CommandLine,
    stub: &str,
) -> (bool, Option<String>) {
    let Some(last_occurrence) = cmd.last_flag_occurrence() else {
        return (false, None);
    };
    let last_flag = &last_occurrence.name;

    let Some(flag_node) = node.flags.get(last_flag) else {
        return (false, None);
    };

    if flag_node.flag_only {
        return (false, None);
    }

    if last_occurrence.values.is_empty() {
        return (true, Some(last_flag.clone()));
    }

    if !stub.is_empty()
        && last_occurrence
            .values
            .last()
            .is_some_and(|value| fold_case(value).starts_with(&fold_case(stub)))
    {
        return (true, Some(last_flag.clone()));
    }

    (flag_node.multi, Some(last_flag.clone()))
}

fn positional_arg_index(cmd: &CommandLine, stub: &str, matched_head_len: usize) -> usize {
    cmd.head()
        .iter()
        .skip(matched_head_len)
        .chain(cmd.positional_args())
        .filter(|token| token.as_str() != stub)
        .count()
}

fn collect_global_context_flags(root: &CompletionNode) -> BTreeSet<String> {
    fn walk(node: &CompletionNode, out: &mut BTreeSet<String>) {
        for (name, flag) in &node.flags {
            if flag.context_only && flag.context_scope == ContextScope::Global {
                out.insert(name.clone());
            }
        }
        for child in node.children.values() {
            walk(child, out);
        }
    }

    let mut out = BTreeSet::new();
    walk(root, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::completion::{
        CompletionEngine,
        model::{
            CompletionNode, CompletionTree, ContextScope, FlagNode, QuoteStyle, SuggestionEntry,
            SuggestionOutput,
        },
    };

    fn tree() -> CompletionTree {
        let mut provision = CompletionNode::default();
        provision.flags.insert(
            "--provider".to_string(),
            FlagNode {
                suggestions: vec![
                    SuggestionEntry::from("vmware"),
                    SuggestionEntry::from("nrec"),
                ],
                context_only: true,
                ..FlagNode::default()
            },
        );
        provision.flags.insert(
            "--os".to_string(),
            FlagNode {
                suggestions_by_provider: BTreeMap::from([
                    ("vmware".to_string(), vec![SuggestionEntry::from("rhel")]),
                    ("nrec".to_string(), vec![SuggestionEntry::from("alma")]),
                ]),
                suggestions: vec![SuggestionEntry::from("rhel"), SuggestionEntry::from("alma")],
                context_only: true,
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

    fn suggestion_texts(suggestions: impl IntoIterator<Item = SuggestionOutput>) -> Vec<String> {
        suggestions
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item.text),
                SuggestionOutput::PathSentinel => None,
            })
            .collect()
    }

    fn provider_cursor(line: &str) -> usize {
        line.find("--provider").expect("provider in test line") - 1
    }

    mod request_contracts {
        use super::*;

        #[test]
        fn completion_characterization_covers_representative_request_categories() {
            let engine = CompletionEngine::new(tree());
            let cases = [
                ("or", 2usize, "orch"),
                ("orch pr", "orch pr".len(), "provision"),
                ("orch provision --", "orch provision --".len(), "--provider"),
                (
                    "orch provision --provider ",
                    "orch provision --provider ".len(),
                    "vmware",
                ),
                ("orch provision | F", "orch provision | F".len(), "F"),
            ];

            for (line, cursor, expected) in cases {
                let values = suggestion_texts(engine.complete(line, cursor).1);
                assert!(
                    values.iter().any(|value| value == expected),
                    "expected `{expected}` in suggestions for `{line}`, got {values:?}"
                );
            }
        }

        #[test]
        fn completion_request_classifier_covers_representative_categories() {
            let engine = CompletionEngine::new(tree());
            let cases = [
                ("or", 2usize, "subcommands"),
                ("orch pr", "orch pr".len(), "subcommands"),
                ("orch provision --", "orch provision --".len(), "flag-names"),
                (
                    "orch provision --provider ",
                    "orch provision --provider ".len(),
                    "flag-values",
                ),
                ("orch provision | F", "orch provision | F".len(), "pipe"),
            ];

            for (line, cursor, expected) in cases {
                let analysis = engine.analyze(line, cursor);
                assert_eq!(
                    analysis.request.kind(),
                    expected,
                    "unexpected request kind for `{line}`"
                );
            }
        }
    }

    mod context_merge_contracts {
        use super::*;

        #[test]
        fn provider_context_merges_across_completion_and_analysis() {
            let engine = CompletionEngine::new(tree());
            let line = "orch provision --os  --provider vmware";
            let cursor = provider_cursor(line);

            let (_, suggestions) = engine.complete(line, cursor);
            let values = suggestion_texts(suggestions);
            assert!(values.contains(&"rhel".to_string()));

            let analysis = engine.analyze(line, cursor);
            assert_eq!(analysis.cursor.token_stub, "");
            assert_eq!(analysis.context.matched_path, vec!["orch", "provision"]);
            assert_eq!(analysis.context.flag_scope_path, vec!["orch", "provision"]);
            assert!(!analysis.context.subcommand_context);
            assert_eq!(
                analysis
                    .parsed
                    .cursor_cmd
                    .flag_values("--provider")
                    .expect("provider should merge into cursor context"),
                &vec!["vmware".to_string()][..]
            );
        }

        #[test]
        fn metadata_context_flags_respect_global_and_subtree_scope_boundaries() {
            let mut provision = CompletionNode::default();
            provision.flags.insert(
                "--os".to_string(),
                FlagNode {
                    suggestions_by_provider: BTreeMap::from([
                        ("vmware".to_string(), vec![SuggestionEntry::from("rhel")]),
                        ("nrec".to_string(), vec![SuggestionEntry::from("alma")]),
                    ]),
                    suggestions: vec![SuggestionEntry::from("rhel"), SuggestionEntry::from("alma")],
                    ..FlagNode::default()
                },
            );
            let mut orch = CompletionNode::default();
            orch.children
                .insert("provision".to_string(), provision.clone());

            let mut global_hidden = CompletionNode::default();
            global_hidden.flags.insert(
                "--provider".to_string(),
                FlagNode {
                    suggestions: vec![
                        SuggestionEntry::from("vmware"),
                        SuggestionEntry::from("nrec"),
                    ],
                    context_only: true,
                    context_scope: ContextScope::Global,
                    ..FlagNode::default()
                },
            );
            let global_engine = CompletionEngine::new(CompletionTree {
                root: CompletionNode::default()
                    .with_child("orch", orch.clone())
                    .with_child("hidden", global_hidden),
                ..CompletionTree::default()
            });

            let line = "orch provision --os  --provider vmware";
            let values = suggestion_texts(global_engine.complete(line, provider_cursor(line)).1);
            assert!(values.contains(&"rhel".to_string()));
            assert!(!values.contains(&"alma".to_string()));

            let mut subtree_hidden = CompletionNode::default();
            subtree_hidden.flags.insert(
                "--provider".to_string(),
                FlagNode {
                    suggestions: vec![
                        SuggestionEntry::from("vmware"),
                        SuggestionEntry::from("nrec"),
                    ],
                    context_only: true,
                    context_scope: ContextScope::Subtree,
                    ..FlagNode::default()
                },
            );
            let subtree_engine = CompletionEngine::new(CompletionTree {
                root: CompletionNode::default()
                    .with_child("orch", orch)
                    .with_child("hidden", subtree_hidden),
                ..CompletionTree::default()
            });
            let values = suggestion_texts(subtree_engine.complete(line, provider_cursor(line)).1);
            assert!(values.contains(&"rhel".to_string()));
            assert!(values.contains(&"alma".to_string()));
        }

        #[test]
        fn value_completion_handles_equals_flags_and_open_quotes() {
            let engine = CompletionEngine::new(tree());

            let equals_line = "orch provision --os=";
            let values = suggestion_texts(engine.complete(equals_line, equals_line.len()).1);
            assert!(values.contains(&"rhel".to_string()));
            assert!(values.contains(&"alma".to_string()));

            let open_quote_line = "orch provision --os \"rh";
            let analysis = engine.analyze(open_quote_line, open_quote_line.len());
            assert_eq!(analysis.cursor.token_stub, "rh");
            assert_eq!(analysis.cursor.quote_style, Some(QuoteStyle::Double));
        }
    }

    mod scope_resolution_contracts {
        use super::*;

        #[test]
        fn completion_hides_later_flags_and_does_not_inherit_root_flags() {
            let engine = CompletionEngine::new(tree());
            let line = "orch provision  --provider vmware";
            let cursor = line.find("--provider").expect("provider in test line") - 2;

            let values = suggestion_texts(engine.complete(line, cursor).1);
            assert!(!values.contains(&"--provider".to_string()));

            let mut root = CompletionNode::default();
            root.flags
                .insert("--json".to_string(), FlagNode::default().flag_only());
            root.children
                .insert("exit".to_string(), CompletionNode::default());
            let engine = CompletionEngine::new(CompletionTree {
                root,
                ..CompletionTree::default()
            });

            let analysis = engine.analyze("exit ", 5);
            assert_eq!(analysis.parsed.cursor_tokens, vec!["exit".to_string()]);
            assert_eq!(analysis.parsed.cursor_cmd.head(), &["exit".to_string()]);
            assert_eq!(analysis.context.matched_path, vec!["exit".to_string()]);
            assert_eq!(analysis.context.flag_scope_path, vec!["exit".to_string()]);

            let suggestions = engine.suggestions_for_analysis(&analysis);
            assert!(
                suggestions.is_empty(),
                "expected no inherited flags, got {suggestions:?}"
            );
        }

        #[test]
        fn analysis_tolerates_non_char_boundary_cursors_and_counts_value_keys() {
            let engine = CompletionEngine::new(tree());
            let line = "orch å";
            let cursor = line.find('å').expect("multibyte char should exist") + 1;
            let (_cursor, _suggestions) = engine.complete(line, cursor);

            let mut set = CompletionNode::default();
            set.children.insert(
                "ui.mode".to_string(),
                CompletionNode {
                    value_key: true,
                    ..CompletionNode::default()
                },
            );
            let mut config = CompletionNode::default();
            config.children.insert("set".to_string(), set);
            let engine = CompletionEngine::new(CompletionTree {
                root: CompletionNode::default().with_child("config", config),
                ..CompletionTree::default()
            });

            let tokens = vec![
                "config".to_string(),
                "set".to_string(),
                "ui.mode".to_string(),
            ];
            assert_eq!(engine.matched_command_len_tokens(&tokens), 3);
        }
    }

    mod metadata_contracts {
        use super::*;

        #[test]
        fn subcommand_metadata_includes_tooltip_and_preview() {
            let mut ldap = CompletionNode {
                tooltip: Some("Directory lookup".to_string()),
                ..CompletionNode::default()
            };
            ldap.children
                .insert("user".to_string(), CompletionNode::default());
            ldap.children
                .insert("host".to_string(), CompletionNode::default());

            let engine = CompletionEngine::new(CompletionTree {
                root: CompletionNode::default().with_child("ldap", ldap),
                ..CompletionTree::default()
            });

            let meta = engine
                .complete("ld", 2)
                .1
                .into_iter()
                .find_map(|entry| match entry {
                    SuggestionOutput::Item(item) if item.text == "ldap" => item.meta,
                    SuggestionOutput::PathSentinel => None,
                    _ => None,
                })
                .expect("ldap suggestion should have metadata");

            assert!(meta.contains("Directory lookup"));
            assert!(meta.contains("subcommands:"));
            assert!(meta.contains("host"));
            assert!(meta.contains("user"));
        }
    }
}
