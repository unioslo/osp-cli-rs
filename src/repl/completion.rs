//! REPL-specific completion tree shaping and metadata injection.
//!
//! The pure completion engine only knows about command trees and cursor state.
//! This module adapts live REPL behavior into that model by:
//! - projecting the visible command catalog into a completion tree
//! - injecting invocation flags that remain legal in REPL mode
//! - expanding aliases into prefilled command and flag state
//! - re-rooting the tree when the REPL is inside a shell scope

use crate::app::CMD_CONFIG;
use crate::cli::invocation::scan_command_tokens;
use crate::cli::pipeline::{is_cli_help_stage, validate_cli_dsl_stages};
use crate::completion::{
    ArgNode, CommandLineParser, CompletionNode, CompletionTree, CompletionTreeBuilder,
    ContextScope, FlagNode, SuggestionEntry,
};
use crate::dsl::parse::pipeline::parse_stage;
use crate::dsl::{VerbStreaming, registered_verbs, render_streaming_badge, verb_info};
use crate::repl::default_pipe_verbs;
use crate::ui::section_chrome::{
    SectionRenderContext, SectionStyleTokens, render_section_block_with_overrides,
};
use crate::ui::style::StyleToken;
use miette::Result;
use std::collections::BTreeMap;

use super::ReplViewContext;
use super::surface::{ReplAliasEntry, ReplSurface, config_set_key_specs};

const ALIAS_PLACEHOLDER_TOKEN: &str = "__osp_alias__";

pub(crate) fn build_repl_completion_tree(
    view: ReplViewContext<'_>,
    surface: &ReplSurface,
) -> Result<CompletionTree> {
    let mut tree = CompletionTreeBuilder
        .build_from_specs(&surface.specs, default_pipe_verbs())
        .map_err(|err| miette::miette!(err.to_string()))?;
    if view.auth.is_builtin_visible(CMD_CONFIG) {
        CompletionTreeBuilder
            .apply_config_set_keys(&mut tree, config_set_key_specs())
            .map_err(|err| miette::miette!(err.to_string()))?;
    }
    inject_invocation_flags(&mut tree.root);
    mark_context_only_flags(&mut tree.root);

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

    let root_base = tree.root.clone();
    inject_alias_nodes(&mut tree.root, &root_base, None, &surface.aliases);

    if view.scope.is_root() {
        return Ok(tree);
    }

    let mut rooted = CompletionTree {
        root: scoped_completion_root(&tree.root, &view.scope.commands()),
        ..tree.clone()
    };
    let scoped_base = rooted.root.clone();
    inject_alias_nodes(
        &mut rooted.root,
        &scoped_base,
        Some(&tree.root),
        &surface.aliases,
    );
    apply_shell_root_controls(&mut rooted.root);
    Ok(rooted)
}

#[derive(Debug, Clone)]
struct AliasCompletionCommand {
    command: crate::completion::CommandLine,
    prefilled_flags: BTreeMap<String, Vec<String>>,
}

fn inject_alias_nodes(
    target_root: &mut CompletionNode,
    preferred_root: &CompletionNode,
    fallback_root: Option<&CompletionNode>,
    aliases: &[ReplAliasEntry],
) {
    for alias in aliases {
        if target_root.children.contains_key(alias.name.as_str()) {
            continue;
        }
        target_root.children.insert(
            alias.name.clone(),
            alias_completion_node(preferred_root, fallback_root, alias),
        );
    }
}

fn alias_completion_node(
    preferred_root: &CompletionNode,
    fallback_root: Option<&CompletionNode>,
    alias: &ReplAliasEntry,
) -> CompletionNode {
    let Some(parsed) = alias_completion_command(&alias.template) else {
        return CompletionNode {
            tooltip: Some(alias.tooltip.clone()),
            ..CompletionNode::default()
        };
    };

    let Some((mut node, prefilled_positionals)) =
        resolved_alias_target_node(preferred_root, &parsed.command).or_else(|| {
            fallback_root.and_then(|root| resolved_alias_target_node(root, &parsed.command))
        })
    else {
        return CompletionNode {
            tooltip: Some(alias.tooltip.clone()),
            ..CompletionNode::default()
        };
    };

    node.tooltip = Some(alias.tooltip.clone());
    node.exact_token_commits = true;
    node.prefilled_flags = parsed.prefilled_flags;
    for (flag, values) in parsed.command.flag_values_map() {
        node.prefilled_flags
            .entry(flag.clone())
            .or_default()
            .extend(values.clone());
    }
    node.prefilled_positionals = prefilled_positionals;
    node
}

fn alias_completion_command(template: &str) -> Option<AliasCompletionCommand> {
    // Alias templates may contain `${...}` placeholders that are meaningful at
    // execution time but would break completion parsing. Replace them with a
    // stable sentinel so the completer can still resolve the alias shape.
    let sanitized = sanitize_alias_template_for_completion(template);
    let parsed = crate::dsl::parse_pipeline(&sanitized).ok()?;
    let tokens = shell_words::split(&parsed.command).ok()?;
    let scanned = scan_command_tokens(&tokens).ok()?;
    if scanned.tokens.is_empty() {
        return None;
    }

    let parser = CommandLineParser;
    Some(AliasCompletionCommand {
        command: parser.parse(&scanned.tokens),
        prefilled_flags: invocation_prefilled_flags(&scanned.invocation),
    })
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
    command: &crate::completion::CommandLine,
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
        // Keep concrete alias arguments as already-filled positionals so
        // completion lands on the next user-editable slot.
        prefilled_positionals.push(segment.clone());
    }

    Some((node.clone(), prefilled_positionals))
}

#[cfg(test)]
fn scope_completion_tree(
    tree: &CompletionTree,
    scope: &crate::app::ReplScopeStack,
) -> CompletionTree {
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

fn invocation_prefilled_flags(
    invocation: &crate::cli::invocation::InvocationOptions,
) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();

    // Invocation flags typed before the command proper should still shape
    // completion inside aliases and shell scopes, so the tree carries them as
    // prefilled values rather than treating them as out-of-band state.
    if let Some(format) = invocation.format {
        out.insert("--format".to_string(), vec![format.as_str().to_string()]);
    }
    if let Some(mode) = invocation.mode {
        out.insert("--mode".to_string(), vec![mode.as_str().to_string()]);
    }
    if let Some(color) = invocation.color {
        out.insert("--color".to_string(), vec![color.as_str().to_string()]);
    }
    if let Some(unicode) = invocation.unicode {
        out.insert("--unicode".to_string(), vec![unicode.as_str().to_string()]);
    }
    if invocation.verbose > 0 {
        out.insert("--verbose".to_string(), Vec::new());
    }
    if invocation.quiet > 0 {
        out.insert("--quiet".to_string(), Vec::new());
    }
    if invocation.debug > 0 {
        out.insert("--debug".to_string(), Vec::new());
    }
    if invocation.cache {
        out.insert("--cache".to_string(), Vec::new());
    }
    if let Some(provider) = invocation.plugin_provider.clone() {
        out.insert("--plugin-provider".to_string(), vec![provider]);
    }

    out
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

    // Keep root args in sync with current child commands so an empty stub
    // still suggests shell-local commands from the scoped root.
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
    let format_values = ["auto", "guide", "json", "table", "mreg", "value", "md"]
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
            "--guide".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Alias for --format guide"),
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

    let target = spec.split_whitespace().next().unwrap_or("").trim();
    if target.is_empty() {
        for info in registered_verbs() {
            let mut line = format!("  {:<5} {}", info.verb, info.summary);
            if let Some(badge) = render_streaming_badge(info.streaming) {
                line.push(' ');
                line.push_str(badge);
            }
            lines.push(line);
        }
        lines.push(String::new());
        lines.push("  Use | H <verb> for details.".to_string());
    } else {
        let lookup = target.to_ascii_uppercase();
        if let Some(info) = verb_info(&lookup) {
            lines.push(format!("  {}  {}", info.verb, info.summary));
            let streaming = match info.streaming {
                VerbStreaming::Streamable => "yes".to_string(),
                VerbStreaming::Conditional => "conditional".to_string(),
                VerbStreaming::Materializes => "no".to_string(),
                VerbStreaming::Meta => "n/a".to_string(),
            };
            lines.push(format!("  Streaming: {streaming}"));
            lines.push(format!("  Note: {}", info.streaming_note));
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
        SectionRenderContext {
            color: resolved.color,
            theme,
            style_overrides: &resolved.style_overrides,
        },
        SectionStyleTokens {
            border: StyleToken::MessageInfo,
            title: StyleToken::PanelTitle,
        },
    )
}

pub(crate) fn validate_dsl_stages(stages: &[String]) -> Result<()> {
    validate_cli_dsl_stages(stages)
}

#[cfg(test)]
mod tests {
    use super::{
        ALIAS_PLACEHOLDER_TOKEN, alias_completion_command, alias_completion_node,
        inject_alias_nodes, inject_invocation_flags, mark_context_only_flags, render_dsl_help,
        sanitize_alias_template_for_completion, scope_completion_tree,
    };
    use crate::app::{
        AppState, AppStateBuilder, LaunchContext, ReplScopeStack, RuntimeContext, TerminalKind,
        UiState,
    };
    use crate::completion::{CompletionNode, CompletionTree, ContextScope, FlagNode};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::plugin::PluginManager;
    use crate::repl::ReplViewContext;
    use crate::repl::surface::ReplAliasEntry;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;

    fn test_app_state() -> AppState {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default().with_terminal("repl"))
            .expect("test config should resolve");

        AppStateBuilder::new(
            RuntimeContext::new(None, TerminalKind::Repl, None),
            config,
            UiState::new(
                RenderSettings::test_plain(OutputFormat::Json),
                MessageLevel::Success,
                0,
            ),
        )
        .with_launch(LaunchContext::default())
        .with_plugins(PluginManager::new(Vec::new()))
        .build()
    }

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
    fn scope_completion_tree_handles_current_and_unknown_scopes_unit() {
        let mut ldap = CompletionNode::default();
        ldap.children
            .insert("user".to_string(), CompletionNode::default());
        let tree = CompletionTree {
            root: CompletionNode::default().with_child("ldap", ldap),
            ..CompletionTree::default()
        };

        let mut scoped = ReplScopeStack::default();
        scoped.enter("ldap");
        let rooted = scope_completion_tree(&tree, &scoped);
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

        let mut unknown = ReplScopeStack::default();
        unknown.enter("missing");
        let fallback = scope_completion_tree(&tree, &unknown);
        assert!(fallback.root.children.contains_key("ldap"));
        assert!(fallback.root.children.contains_key("exit"));
    }

    #[test]
    fn injects_invocation_flags_on_root_and_children() {
        let mut root = CompletionNode::default();
        root.children
            .insert("ldap".to_string(), CompletionNode::default());
        root.children
            .insert("exit".to_string(), CompletionNode::default());

        inject_invocation_flags(&mut root);

        assert!(root.flags.contains_key("--guide"));
        assert!(root.flags.contains_key("--json"));
        assert!(root.flags.contains_key("--format"));
        assert!(root.children["ldap"].flags.contains_key("--guide"));
        assert!(root.children["ldap"].flags.contains_key("--json"));
        assert!(root.children["ldap"].flags.contains_key("--format"));
        assert!(!root.children["exit"].flags.contains_key("--json"));
        assert_eq!(
            root.flags["--format"]
                .suggestions
                .iter()
                .map(|entry| entry.value.as_str())
                .collect::<Vec<_>>(),
            vec!["auto", "guide", "json", "table", "mreg", "value", "md"]
        );
    }

    #[test]
    fn alias_template_sanitizer_replaces_placeholders_and_preserves_suffixes() {
        let sanitized =
            sanitize_alias_template_for_completion("ldap user ${uid} | project ${field}.name");

        assert_eq!(
            sanitized,
            format!("ldap user {ALIAS_PLACEHOLDER_TOKEN} | project {ALIAS_PLACEHOLDER_TOKEN}.name")
        );
    }

    #[test]
    fn alias_completion_helpers_cover_prefilled_flags_missing_targets_and_collisions_unit() {
        for template in ["--json --no-color ldap user ${uid}", "--json --no-color"] {
            let parsed = alias_completion_command(template).expect("alias parses");
            assert_eq!(
                parsed.prefilled_flags.get("--format"),
                Some(&vec!["json".to_string()])
            );
            if template.contains("ldap user") {
                assert!(
                    !parsed.command.head().is_empty()
                        || parsed.command.tail_len() > 0
                        || !parsed.command.pipes().is_empty()
                );
            }
        }

        let alias = ReplAliasEntry {
            name: "lookup".to_string(),
            template: "missing user ${uid}".to_string(),
            tooltip: "Lookup a user".to_string(),
        };
        let node = alias_completion_node(&CompletionNode::default(), None, &alias);
        assert_eq!(node.tooltip.as_deref(), Some("Lookup a user"));
        assert!(node.children.is_empty());

        let mut root = CompletionNode::default();
        root.children
            .insert("lookup".to_string(), CompletionNode::default());
        let aliases = vec![ReplAliasEntry {
            name: "lookup".to_string(),
            template: "ldap user ${uid}".to_string(),
            tooltip: "Lookup user".to_string(),
        }];
        inject_alias_nodes(&mut root, &CompletionNode::default(), None, &aliases);
        assert!(root.children.contains_key("lookup"));
        assert_eq!(root.children.len(), 1);
    }

    #[test]
    fn dsl_help_describes_materialization_and_streaming_behaviors_unit() {
        let state = test_app_state();
        let listing = render_dsl_help(
            ReplViewContext::from_parts(&state.runtime, &state.session),
            "",
        );
        assert!(listing.contains("A") && listing.contains("[materializes]"));
        assert!(listing.contains("JQ") && listing.contains("[materializes]"));

        let aggregate = render_dsl_help(
            ReplViewContext::from_parts(&state.runtime, &state.session),
            "A",
        );
        assert!(aggregate.contains("Streaming: no"));
        assert!(aggregate.contains("aggregation needs the full input"));

        let filter = render_dsl_help(
            ReplViewContext::from_parts(&state.runtime, &state.session),
            "F",
        );
        assert!(filter.contains("Streaming: yes"));
    }
}
