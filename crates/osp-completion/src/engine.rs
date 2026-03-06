use crate::{
    model::{
        CommandLine, CompletionAnalysis, CompletionContext, CompletionNode, CompletionTree,
        ContextScope, MatchKind, SuggestionOutput,
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

    pub fn suggestions_with_stub(
        &self,
        line: &str,
        cursor: usize,
    ) -> (String, Vec<SuggestionOutput>) {
        let analysis = self.analyze(line, cursor);
        let suggestions = self.suggestions_for_analysis(&analysis);
        (analysis.stub, suggestions)
    }

    pub fn suggestions_for_analysis(&self, analysis: &CompletionAnalysis) -> Vec<SuggestionOutput> {
        self.suggester.generate(analysis)
    }

    pub fn analyze(&self, line: &str, cursor: usize) -> CompletionAnalysis {
        let safe_cursor = clamp_to_char_boundary(line, cursor.min(line.len()));
        let before_cursor = &line[..safe_cursor];

        let full_tokens = self.parser.tokenize(line);
        let full_cmd = self.parser.parse(&full_tokens);

        let cursor_tokens = self.parser.tokenize(before_cursor);
        let stub = self.parser.compute_stub(before_cursor, &cursor_tokens);
        let mut analysis =
            self.analyze_command_parts(full_cmd, self.parser.parse(&cursor_tokens), stub);
        analysis.safe_cursor = safe_cursor;
        analysis.full_tokens = full_tokens;
        analysis.cursor_tokens = cursor_tokens;
        analysis
    }

    pub fn analyze_command(
        &self,
        full_cmd: CommandLine,
        cursor_cmd: CommandLine,
        stub: impl Into<String>,
    ) -> CompletionAnalysis {
        self.analyze_command_parts(full_cmd, cursor_cmd, stub.into())
    }

    fn analyze_command_parts(
        &self,
        full_cmd: CommandLine,
        mut cursor_cmd: CommandLine,
        stub: String,
    ) -> CompletionAnalysis {
        if !cursor_cmd.has_pipe {
            self.merge_context_flags(&mut cursor_cmd, &full_cmd, &stub);
        }

        let context = self.resolve_completion_context(&cursor_cmd, &stub);

        CompletionAnalysis {
            safe_cursor: 0,
            full_tokens: Vec::new(),
            cursor_tokens: Vec::new(),
            full_cmd,
            cursor_cmd,
            stub,
            context,
        }
    }

    pub fn tokenize(&self, line: &str) -> Vec<String> {
        self.parser.tokenize(line)
    }

    pub fn matched_command_len_tokens(&self, tokens: &[String]) -> usize {
        let mut node = &self.tree.root;
        let mut matched = 0usize;

        for token in tokens {
            if token == "|" || token.starts_with('-') {
                break;
            }
            let Some(child) = node.children.get(token) else {
                break;
            };
            if child.value_key {
                break;
            }
            matched += 1;
            if child.value_leaf {
                break;
            }
            node = child;
        }

        matched
    }

    pub fn classify_match(&self, analysis: &CompletionAnalysis, value: &str) -> MatchKind {
        if analysis.cursor_cmd.has_pipe {
            return MatchKind::Pipe;
        }
        let context_node = self
            .resolve_context_exact(&analysis.context.matched_path)
            .unwrap_or(&self.tree.root);
        let flag_scope = self
            .resolve_context_exact(&analysis.context.flag_scope_path)
            .unwrap_or(&self.tree.root);

        if value.starts_with("--") || flag_scope.flags.contains_key(value) {
            return MatchKind::Flag;
        }
        if context_node.children.contains_key(value) {
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
        for i in (0..=context.matched_path.len()).rev() {
            let (node, matched) = self.resolve_context(&context.matched_path[..i]);
            if matched.len() == i {
                scoped_flags.extend(node.flags.keys().cloned());
            }
        }
        scoped_flags.extend(self.global_context_flags.iter().cloned());

        for (flag, values) in &full_cmd.flags {
            if cursor_cmd.flags.contains_key(flag) {
                continue;
            }
            if !scoped_flags.contains(flag) {
                continue;
            }
            cursor_cmd.flags.insert(flag.clone(), values.clone());
        }
    }

    fn resolve_completion_context(&self, cmd: &CommandLine, stub: &str) -> CompletionContext {
        let (pre_node, _) = self.resolve_context(&cmd.head);
        let has_subcommands = !pre_node.children.is_empty();
        let head_for_context = if !stub.is_empty() && !stub.starts_with('-') && has_subcommands {
            &cmd.head[..cmd.head.len().saturating_sub(1)]
        } else {
            cmd.head.as_slice()
        };
        let (_, matched) = self.resolve_context(head_for_context);
        let flag_scope_path = self.resolve_flag_scope_path(&matched);

        let arg_tokens: Vec<String> = cmd
            .head
            .iter()
            .skip(matched.len())
            .filter(|token| token.as_str() != stub)
            .cloned()
            .chain(
                cmd.args
                    .iter()
                    .filter(|token| token.as_str() != stub)
                    .cloned(),
            )
            .collect();

        let context_node = self
            .resolve_context_exact(&matched)
            .unwrap_or(&self.tree.root);
        let subcommand_context =
            context_node.value_key || (has_subcommands && arg_tokens.is_empty());

        CompletionContext {
            matched_path: matched,
            flag_scope_path,
            subcommand_context,
        }
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

    fn resolve_context_exact<'a>(&'a self, path: &[String]) -> Option<&'a CompletionNode> {
        let (node, matched) = self.resolve_context(path);
        (matched.len() == path.len()).then_some(node)
    }

    fn resolve_flag_scope_path(&self, matched_path: &[String]) -> Vec<String> {
        for i in (0..=matched_path.len()).rev() {
            let prefix = &matched_path[..i];
            let Some(node) = self.resolve_context_exact(prefix) else {
                continue;
            };
            if !node.flags.is_empty() {
                return prefix.to_vec();
            }
        }
        Vec::new()
    }
}

fn clamp_to_char_boundary(input: &str, cursor: usize) -> usize {
    if input.is_char_boundary(cursor) {
        return cursor;
    }
    let mut safe = cursor;
    while safe > 0 && !input.is_char_boundary(safe) {
        safe -= 1;
    }
    safe
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
            CompletionNode, CompletionTree, ContextScope, FlagNode, SuggestionEntry,
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

        let (_, suggestions) = engine.suggestions_with_stub(line, cursor);
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

        let (_, suggestions) = engine.suggestions_with_stub(line, cursor);
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
        let (_stub, _suggestions) = engine.suggestions_with_stub(line, cursor);
    }

    #[test]
    fn equals_flag_without_value_still_requests_suggestions() {
        let engine = CompletionEngine::new(tree());
        let line = "orch provision --os=";
        let (_, suggestions) = engine.suggestions_with_stub(line, line.len());
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
        let (_, suggestions) = engine.suggestions_with_stub(line, cursor);
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
        let (_, suggestions) = engine.suggestions_with_stub(line, cursor);
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

        let (_, suggestions) = engine.suggestions_with_stub("ld", 2);
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

        assert_eq!(analysis.stub, "");
        assert_eq!(analysis.context.matched_path, vec!["orch", "provision"]);
        assert_eq!(analysis.context.flag_scope_path, vec!["orch", "provision"]);
        assert!(!analysis.context.subcommand_context);
        assert_eq!(
            analysis
                .cursor_cmd
                .flags
                .get("--provider")
                .expect("provider should merge into cursor context"),
            &vec!["vmware".to_string()]
        );
    }
}
