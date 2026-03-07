use crate::{
    context::TreeResolver,
    model::{
        CommandLine, CompletionAnalysis, CompletionContext, CompletionNode, CompletionTree,
        ContextScope, CursorState, MatchKind, ParsedLine, SuggestionOutput, TailItem,
    },
    parse::CommandLineParser,
    suggest::SuggestionEngine,
};
use std::collections::BTreeSet;

#[derive(Debug, Clone)]
pub struct CompletionEngine {
    parser: CommandLineParser,
    suggester: SuggestionEngine,
    tree: CompletionTree,
    global_context_flags: BTreeSet<String>,
}

impl CompletionEngine {
    pub fn new(tree: CompletionTree) -> Self {
        let global_context_flags = collect_global_context_flags(&tree.root);
        Self {
            parser: CommandLineParser,
            suggester: SuggestionEngine::new(tree.clone()),
            tree,
            global_context_flags,
        }
    }

    pub fn complete(&self, line: &str, cursor: usize) -> (CursorState, Vec<SuggestionOutput>) {
        let analysis = self.analyze(line, cursor);
        let suggestions = self.suggestions_for_analysis(&analysis);
        (analysis.cursor, suggestions)
    }

    pub fn suggestions_for_analysis(&self, analysis: &CompletionAnalysis) -> Vec<SuggestionOutput> {
        self.suggester.generate(analysis)
    }

    pub fn analyze(&self, line: &str, cursor: usize) -> CompletionAnalysis {
        let parsed = self.parser.analyze(line, cursor);

        self.analyze_command_parts(parsed.parsed, parsed.cursor)
    }

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

        let context =
            self.resolve_completion_context(&parsed.cursor_cmd, cursor.token_stub.as_str());

        CompletionAnalysis {
            parsed,
            cursor,
            context,
        }
    }

    pub fn tokenize(&self, line: &str) -> Vec<String> {
        self.parser.tokenize(line)
    }

    pub fn matched_command_len_tokens(&self, tokens: &[String]) -> usize {
        TreeResolver::new(&self.tree).matched_command_len_tokens(tokens)
    }

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

    fn resolve_completion_context(&self, cmd: &CommandLine, stub: &str) -> CompletionContext {
        let resolver = TreeResolver::new(&self.tree);
        let (pre_node, _) = resolver.resolve_context(cmd.head());
        let has_subcommands = !pre_node.children.is_empty();
        // When the user is still typing a subcommand, the partial token is the
        // last `head` element but not yet a resolvable child node. Drop that
        // partial token so context resolution stays on the parent command.
        let head_without_partial_subcommand =
            if !stub.is_empty() && !stub.starts_with('-') && has_subcommands {
                &cmd.head()[..cmd.head().len().saturating_sub(1)]
            } else {
                cmd.head()
            };
        let (_, matched) = resolver.resolve_context(head_without_partial_subcommand);
        let flag_scope_path = resolver.resolve_flag_scope_path(&matched);

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
        let subcommand_context =
            context_node.value_key || (has_subcommands && arg_tokens.is_empty());

        CompletionContext {
            matched_path: matched,
            flag_scope_path,
            subcommand_context,
        }
    }
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

    use crate::{
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
            ..CompletionTree::default()
        }
    }

    #[test]
    fn merges_late_provider_flag_into_cursor_context() {
        let engine = CompletionEngine::new(tree());
        let line = "orch provision --os  --provider vmware";
        let cursor = line.find("--provider").expect("provider in test line") - 1;

        let (_, suggestions) = engine.complete(line, cursor);
        let values: Vec<String> = suggestions
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item.text),
                SuggestionOutput::PathSentinel => None,
            })
            .collect();

        assert!(values.contains(&"rhel".to_string()));
    }

    #[test]
    fn hides_flags_already_present_later_in_line() {
        let engine = CompletionEngine::new(tree());
        let line = "orch provision  --provider vmware";
        let cursor = line.find("--provider").expect("provider in test line") - 2;

        let (_, suggestions) = engine.complete(line, cursor);
        let values: Vec<String> = suggestions
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item.text),
                SuggestionOutput::PathSentinel => None,
            })
            .collect();

        assert!(!values.contains(&"--provider".to_string()));
    }

    #[test]
    fn supports_non_char_boundary_cursor_without_panicking() {
        let engine = CompletionEngine::new(tree());
        let line = "orch å";
        let cursor = line.find('å').expect("multibyte char should exist") + 1;
        let (_cursor, _suggestions) = engine.complete(line, cursor);
    }

    #[test]
    fn equals_flag_without_value_still_requests_suggestions() {
        let engine = CompletionEngine::new(tree());
        let line = "orch provision --os=";
        let (_, suggestions) = engine.complete(line, line.len());
        let values: Vec<String> = suggestions
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item.text),
                SuggestionOutput::PathSentinel => None,
            })
            .collect();
        assert!(values.contains(&"rhel".to_string()));
        assert!(values.contains(&"alma".to_string()));
    }

    #[test]
    fn merges_context_flags_from_metadata_even_if_not_in_scope() {
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
        orch.children.insert("provision".to_string(), provision);

        let mut hidden = CompletionNode::default();
        hidden.flags.insert(
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

        let tree = CompletionTree {
            root: CompletionNode::default()
                .with_child("orch", orch)
                .with_child("hidden", hidden),
            ..CompletionTree::default()
        };
        let engine = CompletionEngine::new(tree);

        let line = "orch provision --os  --provider vmware";
        let cursor = line.find("--provider").expect("provider in test line") - 1;
        let (_, suggestions) = engine.complete(line, cursor);
        let values: Vec<String> = suggestions
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item.text),
                SuggestionOutput::PathSentinel => None,
            })
            .collect();
        assert!(values.contains(&"rhel".to_string()));
        assert!(!values.contains(&"alma".to_string()));
    }

    #[test]
    fn subtree_context_flags_do_not_leak_across_branches() {
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
        orch.children.insert("provision".to_string(), provision);

        let mut hidden = CompletionNode::default();
        hidden.flags.insert(
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

        let tree = CompletionTree {
            root: CompletionNode::default()
                .with_child("orch", orch)
                .with_child("hidden", hidden),
            ..CompletionTree::default()
        };
        let engine = CompletionEngine::new(tree);

        let line = "orch provision --os  --provider vmware";
        let cursor = line.find("--provider").expect("provider in test line") - 1;
        let (_, suggestions) = engine.complete(line, cursor);
        let values: Vec<String> = suggestions
            .into_iter()
            .filter_map(|entry| match entry {
                SuggestionOutput::Item(item) => Some(item.text),
                SuggestionOutput::PathSentinel => None,
            })
            .collect();
        assert!(values.contains(&"rhel".to_string()));
        assert!(values.contains(&"alma".to_string()));
    }

    #[test]
    fn subcommand_meta_includes_tooltip_and_preview() {
        let mut ldap = CompletionNode {
            tooltip: Some("Directory lookup".to_string()),
            ..CompletionNode::default()
        };
        ldap.children
            .insert("user".to_string(), CompletionNode::default());
        ldap.children
            .insert("host".to_string(), CompletionNode::default());

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("ldap", ldap),
            ..CompletionTree::default()
        };
        let engine = CompletionEngine::new(tree);

        let (_, suggestions) = engine.complete("ld", 2);
        let meta = suggestions
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

    #[test]
    fn analyze_exposes_merged_cursor_context() {
        let engine = CompletionEngine::new(tree());
        let line = "orch provision --os  --provider vmware";
        let cursor = line.find("--provider").expect("provider in test line") - 1;

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
    fn analyze_preserves_open_quote_context() {
        let engine = CompletionEngine::new(tree());
        let line = "orch provision --os \"rh";

        let analysis = engine.analyze(line, line.len());

        assert_eq!(analysis.cursor.token_stub, "rh");
        assert_eq!(analysis.cursor.quote_style, Some(QuoteStyle::Double));
    }

    #[test]
    fn matched_command_len_counts_value_keys_consistently() {
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
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("config", config),
            ..CompletionTree::default()
        };
        let engine = CompletionEngine::new(tree);

        let tokens = vec![
            "config".to_string(),
            "set".to_string(),
            "ui.mode".to_string(),
        ];
        assert_eq!(engine.matched_command_len_tokens(&tokens), 3);
    }
}
