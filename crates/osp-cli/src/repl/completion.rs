use crate::app::CMD_CONFIG;
use crate::pipeline::{is_cli_help_stage, validate_cli_dsl_stages};
use crate::state::ReplScopeStack;
use miette::Result;
use osp_completion::{
    ArgNode, CommandLineParser, CompletionNode, CompletionTree, CompletionTreeBuilder,
    ContextScope, FlagNode, SuggestionEntry,
};
use osp_dsl::parse::pipeline::parse_stage;
use osp_repl::default_pipe_verbs;
use osp_ui::messages::render_section_block_with_overrides;
use osp_ui::style::StyleToken;

use super::ReplViewContext;
use super::surface::{ReplAliasEntry, ReplSurface, config_set_key_specs};

const ALIAS_PLACEHOLDER_TOKEN: &str = "__osp_alias__";

pub(crate) fn build_repl_completion_tree(
    view: ReplViewContext<'_>,
    surface: &ReplSurface,
) -> CompletionTree {
    let mut tree = CompletionTreeBuilder.build_from_specs(&surface.specs, default_pipe_verbs());
    if view.auth.is_builtin_visible(CMD_CONFIG) {
        CompletionTreeBuilder.apply_config_set_keys(&mut tree, config_set_key_specs());
    }
    inject_invocation_flags(&mut tree.root);
    mark_context_only_flags(&mut tree.root);
    for alias in &surface.aliases {
        let alias_name = &alias.name;
        if tree.root.children.contains_key(alias_name.as_str()) {
            continue;
        }
        tree.root
            .children
            .insert(alias_name.clone(), alias_completion_node(&tree, alias));
    }

    let root_suggestions = surface
        .root_words
        .iter()
        .enumerate()
        .map(|(index, word)| SuggestionEntry::value(word.clone()).sort(index.to_string()))
        .collect::<Vec<_>>();
    tree.root.args = vec![ArgNode {
        name: Some("command".to_string()),
        suggestions: root_suggestions,
        ..ArgNode::default()
    }];

    scope_completion_tree(&tree, view.scope)
}

fn alias_completion_node(tree: &CompletionTree, alias: &ReplAliasEntry) -> CompletionNode {
    let Some(command) = alias_completion_command(&alias.template) else {
        return CompletionNode {
            tooltip: Some(alias.tooltip.clone()),
            ..CompletionNode::default()
        };
    };

    let Some((mut node, prefilled_positionals)) = resolved_alias_target_node(&tree.root, &command)
    else {
        return CompletionNode {
            tooltip: Some(alias.tooltip.clone()),
            ..CompletionNode::default()
        };
    };

    node.tooltip = Some(alias.tooltip.clone());
    node.prefilled_flags = command.flag_values_map().clone();
    node.prefilled_positionals = prefilled_positionals;
    node
}

fn alias_completion_command(template: &str) -> Option<osp_completion::CommandLine> {
    let sanitized = sanitize_alias_template_for_completion(template);
    let parsed = osp_dsl::parse_pipeline(&sanitized).ok()?;
    let tokens = shell_words::split(&parsed.command).ok()?;
    if tokens.is_empty() {
        return None;
    }

    let parser = CommandLineParser;
    Some(parser.parse(&tokens))
}

fn sanitize_alias_template_for_completion(template: &str) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;

    while let Some(rel_start) = template[cursor..].find("${") {
        let start = cursor + rel_start;
        out.push_str(&template[cursor..start]);
        let after_open = start + 2;
        let Some(rel_end) = template[after_open..].find('}') else {
            out.push_str(&template[start..]);
            return out;
        };
        out.push_str(ALIAS_PLACEHOLDER_TOKEN);
        cursor = after_open + rel_end + 1;
    }

    out.push_str(&template[cursor..]);
    out
}

fn resolved_alias_target_node(
    root: &CompletionNode,
    command: &osp_completion::CommandLine,
) -> Option<(CompletionNode, Vec<String>)> {
    let mut node = root;
    let mut matched = 0usize;

    for segment in command.head() {
        let Some(child) = node.children.get(segment) else {
            break;
        };
        node = child;
        matched += 1;
    }

    if matched == 0 {
        return None;
    }

    let mut prefilled_positionals = Vec::new();
    for segment in command.head().iter().skip(matched) {
        if segment == ALIAS_PLACEHOLDER_TOKEN {
            break;
        }
        prefilled_positionals.push(segment.clone());
    }

    Some((node.clone(), prefilled_positionals))
}

fn scope_completion_tree(tree: &CompletionTree, scope: &ReplScopeStack) -> CompletionTree {
    if scope.is_root() {
        return tree.clone();
    }

    let mut rooted = CompletionTree {
        root: scoped_completion_root(&tree.root, &scope.commands()),
        ..tree.clone()
    };
    apply_shell_root_controls(&mut rooted.root);
    rooted
}

fn scoped_completion_root(root: &CompletionNode, path: &[String]) -> CompletionNode {
    let mut node = root;
    for segment in path {
        let Some(child) = node.children.get(segment) else {
            // Unknown shell scopes can happen for plugin-owned shells that do
            // not contribute a completion subtree. Fall back to the root tree
            // so completion remains usable instead of going empty.
            return root.clone();
        };
        node = child;
    }
    node.clone()
}

fn apply_shell_root_controls(root: &mut CompletionNode) {
    root.children
        .entry("help".to_string())
        .or_insert_with(|| CompletionNode {
            tooltip: Some("Show help for the current shell".to_string()),
            sort: Some("0".to_string()),
            ..CompletionNode::default()
        });
    root.children
        .entry("exit".to_string())
        .or_insert_with(|| CompletionNode {
            tooltip: Some("Leave the current shell".to_string()),
            sort: Some("1".to_string()),
            ..CompletionNode::default()
        });
    root.children
        .entry("quit".to_string())
        .or_insert_with(|| CompletionNode {
            tooltip: Some("Alias for exit".to_string()),
            sort: Some("2".to_string()),
            ..CompletionNode::default()
        });

    let mut suggestions = root
        .children
        .iter()
        .map(|(name, child)| {
            let mut entry = SuggestionEntry::value(name.clone());
            if let Some(sort) = &child.sort {
                entry = entry.sort(sort.clone());
            }
            entry
        })
        .collect::<Vec<_>>();
    suggestions.sort_by(|left, right| {
        left.sort
            .cmp(&right.sort)
            .then_with(|| left.value.cmp(&right.value))
    });
    suggestions.dedup_by(|left, right| left.value == right.value);

    // Keep root args in sync with current child commands so "no stub yet"
    // completion can suggest shell commands from this scoped root.
    root.args = vec![ArgNode {
        name: Some("command".to_string()),
        suggestions,
        ..ArgNode::default()
    }];
}

fn inject_invocation_flags(node: &mut CompletionNode) {
    for (name, flag) in invocation_flag_nodes() {
        node.flags.entry(name).or_insert(flag);
    }

    for (name, child) in &mut node.children {
        if repl_host_command_without_invocation_flags(name) {
            continue;
        }
        inject_invocation_flags(child);
    }
}

fn repl_host_command_without_invocation_flags(name: &str) -> bool {
    matches!(name, "help" | "exit" | "quit")
}

fn invocation_flag_nodes() -> Vec<(String, FlagNode)> {
    let format_values = ["auto", "json", "table", "mreg", "value", "md"]
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();
    let mode_values = ["auto", "plain", "rich"]
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();
    let color_values = ["auto", "always", "never"]
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();
    let unicode_values = ["auto", "always", "never"]
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();

    vec![
        (
            "--format".to_string(),
            FlagNode::new()
                .tooltip("Format this invocation only")
                .suggestions(format_values),
        ),
        (
            "--json".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Alias for --format json"),
        ),
        (
            "--table".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Alias for --format table"),
        ),
        (
            "--mreg".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Alias for --format mreg"),
        ),
        (
            "--value".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Alias for --format value"),
        ),
        (
            "--md".to_string(),
            FlagNode::new().flag_only().tooltip("Alias for --format md"),
        ),
        (
            "--mode".to_string(),
            FlagNode::new()
                .tooltip("Render mode for this invocation")
                .suggestions(mode_values),
        ),
        (
            "--plain".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Alias for --mode plain"),
        ),
        (
            "--rich".to_string(),
            FlagNode::new().flag_only().tooltip("Alias for --mode rich"),
        ),
        (
            "--color".to_string(),
            FlagNode::new()
                .tooltip("Color policy for this invocation")
                .suggestions(color_values),
        ),
        (
            "--unicode".to_string(),
            FlagNode::new()
                .tooltip("Unicode policy for this invocation")
                .suggestions(unicode_values),
        ),
        (
            "--ascii".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Alias for --unicode never"),
        ),
        (
            "--verbose".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Increase message verbosity"),
        ),
        (
            "--quiet".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Decrease message verbosity"),
        ),
        (
            "--debug".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Increase developer log verbosity"),
        ),
        (
            "--cache".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Reuse identical result in this REPL session"),
        ),
        (
            "--plugin-provider".to_string(),
            FlagNode::new().tooltip("Select plugin provider for this invocation"),
        ),
    ]
}

fn mark_context_only_flags(node: &mut CompletionNode) {
    // These flags influence suggestion context even when they appear later in
    // the line than the cursor. The suggestion engine still has a small amount
    // of matching logic keyed on these same names in `osp-completion`.
    const CONTEXT_ONLY_FLAGS: [&str; 6] = [
        "--provider",
        "--vmware",
        "--nrec",
        "--linux",
        "--windows",
        "--os",
    ];

    for (name, flag) in &mut node.flags {
        if CONTEXT_ONLY_FLAGS.contains(&name.as_str()) {
            flag.context_only = true;
            flag.context_scope = ContextScope::Subtree;
        }
    }

    for child in node.children.values_mut() {
        mark_context_only_flags(child);
    }
}

pub(crate) fn maybe_render_dsl_help(
    view: ReplViewContext<'_>,
    stages: &[String],
) -> Option<String> {
    for raw in stages {
        let Ok(parsed) = parse_stage(raw) else {
            continue;
        };
        if is_cli_help_stage(&parsed) {
            return Some(render_dsl_help(view, &parsed.spec));
        }
    }
    None
}

fn render_dsl_help(view: ReplViewContext<'_>, spec: &str) -> String {
    let resolved = view.ui.render_settings.resolve_render_settings();
    let theme = &resolved.theme;
    let mut lines = Vec::new();

    let verbs = default_pipe_verbs();
    let target = spec.split_whitespace().next().unwrap_or("").trim();
    if target.is_empty() {
        for (verb, desc) in verbs {
            lines.push(format!("  {verb:<5} {desc}"));
        }
        lines.push(String::new());
        lines.push("  Use | H <verb> for details.".to_string());
    } else {
        let lookup = target.to_ascii_uppercase();
        if let Some(desc) = verbs.get(&lookup) {
            lines.push(format!("  {lookup}  {desc}"));
        } else {
            lines.push(format!("  Unknown DSL verb: {target}"));
            lines.push("  Use | H to list available verbs.".to_string());
        }
    }

    render_section_block_with_overrides(
        "DSL Help",
        &lines.join("\n"),
        resolved.chrome_frame,
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::MessageInfo,
        StyleToken::PanelTitle,
        &resolved.style_overrides,
    )
}

pub(crate) fn validate_dsl_stages(stages: &[String]) -> Result<()> {
    validate_cli_dsl_stages(stages)
}

#[cfg(test)]
mod tests {
    use super::{inject_invocation_flags, mark_context_only_flags, scope_completion_tree};
    use crate::state::ReplScopeStack;
    use osp_completion::{CompletionNode, CompletionTree, ContextScope, FlagNode};

    #[test]
    fn marks_context_only_flags_recursively() {
        let mut root = CompletionNode::default();
        root.flags
            .insert("--provider".to_string(), FlagNode::default());
        root.flags
            .insert("--other".to_string(), FlagNode::default());

        let mut child = CompletionNode::default();
        child
            .flags
            .insert("--windows".to_string(), FlagNode::default());
        root.children.insert("orch".to_string(), child);

        mark_context_only_flags(&mut root);

        assert!(root.flags["--provider"].context_only);
        assert_eq!(
            root.flags["--provider"].context_scope,
            ContextScope::Subtree
        );
        assert!(!root.flags["--other"].context_only);
        assert!(root.children["orch"].flags["--windows"].context_only);
        assert_eq!(
            root.children["orch"].flags["--windows"].context_scope,
            ContextScope::Subtree
        );
    }

    #[test]
    fn scope_completion_tree_roots_to_current_shell() {
        let mut ldap = CompletionNode::default();
        ldap.children
            .insert("user".to_string(), CompletionNode::default());

        let tree = CompletionTree {
            root: CompletionNode::default().with_child("ldap", ldap),
            ..CompletionTree::default()
        };
        let mut scope = ReplScopeStack::default();
        scope.enter("ldap");

        let rooted = scope_completion_tree(&tree, &scope);

        assert!(rooted.root.children.contains_key("user"));
        assert!(rooted.root.children.contains_key("help"));
        assert!(rooted.root.children.contains_key("exit"));
        assert!(rooted.root.children.contains_key("quit"));
        assert!(!rooted.root.children.contains_key("ldap"));
        let suggestions = rooted.root.args[0]
            .suggestions
            .iter()
            .map(|entry| entry.value.as_str())
            .collect::<Vec<_>>();
        assert!(suggestions.contains(&"user"));
        assert!(suggestions.contains(&"help"));
        assert!(suggestions.contains(&"exit"));
    }

    #[test]
    fn scope_completion_tree_falls_back_to_root_for_unknown_scope() {
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("ldap", CompletionNode::default()),
            ..CompletionTree::default()
        };
        let mut scope = ReplScopeStack::default();
        scope.enter("missing");

        let rooted = scope_completion_tree(&tree, &scope);

        assert!(rooted.root.children.contains_key("ldap"));
        assert!(rooted.root.children.contains_key("exit"));
    }

    #[test]
    fn injects_invocation_flags_on_root_and_children() {
        let mut root = CompletionNode::default();
        root.children
            .insert("ldap".to_string(), CompletionNode::default());
        root.children
            .insert("exit".to_string(), CompletionNode::default());

        inject_invocation_flags(&mut root);

        assert!(root.flags.contains_key("--json"));
        assert!(root.flags.contains_key("--format"));
        assert!(root.children["ldap"].flags.contains_key("--json"));
        assert!(root.children["ldap"].flags.contains_key("--format"));
        assert!(!root.children["exit"].flags.contains_key("--json"));
        assert_eq!(
            root.flags["--format"]
                .suggestions
                .iter()
                .map(|entry| entry.value.as_str())
                .collect::<Vec<_>>(),
            vec!["auto", "json", "table", "mreg", "value", "md"]
        );
    }
}
