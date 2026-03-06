use crate::app::CMD_CONFIG;
use crate::pipeline::{is_cli_help_stage, validate_cli_dsl_stages};
use crate::state::AppState;
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

    tree
}

fn mark_context_only_flags(node: &mut CompletionNode) {
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
    use super::mark_context_only_flags;
    use osp_completion::{CompletionNode, ContextScope, FlagNode};

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
}
