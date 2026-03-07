use crate::app::CMD_CONFIG;
use crate::pipeline::{is_cli_help_stage, validate_cli_dsl_stages};
use crate::state::AppState;
use crate::state::ReplScopeStack;
use miette::Result;
use osp_completion::{
    ArgNode, CompletionNode, CompletionTree, CompletionTreeBuilder, ContextScope, SuggestionEntry,
};
use osp_dsl::parse::pipeline::parse_stage;
use osp_repl::default_pipe_verbs;
use osp_ui::messages::render_section_divider_with_overrides;
use osp_ui::style::StyleToken;

use super::surface::{ReplSurface, config_set_key_specs};

pub(crate) fn build_repl_completion_tree(
    state: &AppState,
    surface: &ReplSurface,
) -> CompletionTree {
    let mut tree = CompletionTreeBuilder.build_from_specs(&surface.specs, default_pipe_verbs());
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        CompletionTreeBuilder.apply_config_set_keys(&mut tree, config_set_key_specs());
    }
    mark_context_only_flags(&mut tree.root);
    for (alias_name, tooltip) in &surface.aliases {
        if tree.root.children.contains_key(alias_name.as_str()) {
            continue;
        }
        tree.root.children.insert(
            alias_name.clone(),
            CompletionNode {
                tooltip: Some(tooltip.clone()),
                ..CompletionNode::default()
            },
        );
    }

    let root_suggestions = surface
        .root_words
        .iter()
        .map(|word| SuggestionEntry::value(word.clone()))
        .collect::<Vec<_>>();
    tree.root.args = vec![ArgNode {
        name: Some("command".to_string()),
        suggestions: root_suggestions,
        ..ArgNode::default()
    }];

    scope_completion_tree(&tree, &state.session.scope)
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
            ..CompletionNode::default()
        });
    root.children
        .entry("exit".to_string())
        .or_insert_with(|| CompletionNode {
            tooltip: Some("Leave the current shell".to_string()),
            ..CompletionNode::default()
        });
    root.children
        .entry("quit".to_string())
        .or_insert_with(|| CompletionNode {
            tooltip: Some("Alias for exit".to_string()),
            ..CompletionNode::default()
        });

    let mut suggestions = root
        .children
        .keys()
        .cloned()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();
    suggestions.sort_by(|left, right| left.value.cmp(&right.value));
    suggestions.dedup_by(|left, right| left.value == right.value);

    // Keep root args in sync with current child commands so "no stub yet"
    // completion can suggest shell commands from this scoped root.
    root.args = vec![ArgNode {
        name: Some("command".to_string()),
        suggestions,
        ..ArgNode::default()
    }];
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

pub(crate) fn maybe_render_dsl_help(state: &AppState, stages: &[String]) -> Option<String> {
    for raw in stages {
        let Ok(parsed) = parse_stage(raw) else {
            continue;
        };
        if is_cli_help_stage(&parsed) {
            return Some(render_dsl_help(state, &parsed.spec));
        }
    }
    None
}

fn render_dsl_help(state: &AppState, spec: &str) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let theme = &resolved.theme;
    let mut out = String::new();
    out.push_str(&render_section_divider_with_overrides(
        "DSL Help",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme,
        StyleToken::MessageInfo,
        &resolved.style_overrides,
    ));
    out.push('\n');

    let verbs = default_pipe_verbs();
    let target = spec.split_whitespace().next().unwrap_or("").trim();
    if target.is_empty() {
        for (verb, desc) in verbs {
            out.push_str(&format!("  {verb:<5} {desc}\n"));
        }
        out.push_str("\n  Use | H <verb> for details.\n");
        return out;
    }

    let lookup = target.to_ascii_uppercase();
    if let Some(desc) = verbs.get(&lookup) {
        out.push_str(&format!("  {lookup}  {desc}\n"));
    } else {
        out.push_str(&format!("  Unknown DSL verb: {target}\n"));
        out.push_str("  Use | H to list available verbs.\n");
    }
    out
}

pub(crate) fn validate_dsl_stages(stages: &[String]) -> Result<()> {
    validate_cli_dsl_stages(stages)
}

#[cfg(test)]
mod tests {
    use super::{mark_context_only_flags, scope_completion_tree};
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
}
