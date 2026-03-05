use miette::{Result, miette};
use osp_completion::{
    ArgNode, CommandSpec, CompletionNode, CompletionTree, CompletionTreeBuilder, ConfigKeySpec,
    ContextScope, FlagNode, SuggestionEntry,
};
use osp_config::{ConfigSchema, SchemaValueType};
use osp_dsl::parse::pipeline::parse_stage;
use osp_ui::messages::render_section_divider_with_overrides;
use osp_ui::style::StyleToken;
use std::collections::{BTreeMap, BTreeSet};

use crate::app::{
    CMD_CONFIG, CMD_HISTORY, CMD_LIST, CMD_PLUGINS, CMD_SHOW, CMD_THEME, CMD_USE,
    CURRENT_TERMINAL_SENTINEL,
};
use crate::plugin_manager::CommandCatalogEntry;
use crate::state::AppState;

use super::history;

pub(crate) fn catalog_completion_words(catalog: &[CommandCatalogEntry]) -> Vec<String> {
    let mut words = vec![
        "help".to_string(),
        "exit".to_string(),
        "quit".to_string(),
        "P".to_string(),
        "F".to_string(),
        "V".to_string(),
        "|".to_string(),
    ];
    for entry in catalog {
        words.push(entry.name.clone());
        words.extend(entry.subcommands.clone());
    }
    words.sort();
    words.dedup();
    words
}

pub(crate) fn build_repl_completion_tree(
    state: &AppState,
    catalog: &[CommandCatalogEntry],
    words: &[String],
) -> CompletionTree {
    let mut specs = vec![
        CommandSpec {
            name: "help".to_string(),
            tooltip: Some("Show REPL help".to_string()),
            ..CommandSpec::default()
        },
        CommandSpec {
            name: "exit".to_string(),
            tooltip: Some("Exit REPL".to_string()),
            ..CommandSpec::default()
        },
        CommandSpec {
            name: "quit".to_string(),
            tooltip: Some("Exit REPL".to_string()),
            ..CommandSpec::default()
        },
    ];

    let command_catalog_specs = catalog
        .iter()
        .filter_map(command_spec_from_catalog)
        .collect::<Vec<_>>();
    specs.extend(command_catalog_specs);

    if state.auth.is_builtin_visible(CMD_PLUGINS) {
        specs.push(plugins_command_spec(catalog));
    }
    if state.auth.is_builtin_visible(CMD_THEME) {
        specs.push(theme_command_spec(state));
    }
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        specs.push(config_command_spec(state));
    }
    if history::repl_history_enabled(state.config.resolved())
        && state.auth.is_builtin_visible(CMD_HISTORY)
    {
        specs.push(history::history_command_spec());
    }

    let mut tree = CompletionTreeBuilder.build_from_specs(&specs, default_pipe_verbs());
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        CompletionTreeBuilder.apply_config_set_keys(&mut tree, config_set_key_specs());
    }
    mark_context_only_flags(&mut tree.root);
    for (alias_name, tooltip) in collect_alias_entries(state.config.resolved()) {
        if tree.root.children.contains_key(&alias_name) {
            continue;
        }
        tree.root.children.insert(
            alias_name,
            CompletionNode {
                tooltip: Some(tooltip),
                ..CompletionNode::default()
            },
        );
    }

    let root_suggestions = words
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

pub(crate) fn collect_alias_entries(config: &osp_config::ResolvedConfig) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, entry) in config.values() {
        let Some(name) = key.strip_prefix("alias.") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let template = entry.raw_value.to_string();
        out.push((name.to_string(), format!("alias: {template}")));
    }
    out
}

fn command_spec_from_catalog(entry: &CommandCatalogEntry) -> Option<CommandSpec> {
    if matches!(
        entry.name.as_str(),
        "help" | "exit" | "quit" | CMD_PLUGINS | CMD_THEME | CMD_CONFIG | CMD_HISTORY
    ) {
        return None;
    }

    let mut spec = CommandSpec::new(entry.name.clone());
    if !entry.about.trim().is_empty() {
        spec.tooltip = Some(entry.about.clone());
    }

    spec.subcommands = entry
        .subcommands
        .iter()
        .map(|subcommand| CommandSpec::new(subcommand.clone()))
        .collect();

    Some(spec)
}

fn plugins_command_spec(catalog: &[CommandCatalogEntry]) -> CommandSpec {
    let plugin_ids = catalog
        .iter()
        .map(|entry| entry.provider.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();

    CommandSpec {
        name: CMD_PLUGINS.to_string(),
        tooltip: Some("Inspect and manage plugin providers".to_string()),
        subcommands: vec![
            CommandSpec {
                name: CMD_LIST.to_string(),
                tooltip: Some("List available plugins".to_string()),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "commands".to_string(),
                tooltip: Some("Show plugin command catalog".to_string()),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "doctor".to_string(),
                tooltip: Some("Run plugin diagnostics".to_string()),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "enable".to_string(),
                tooltip: Some("Enable plugin by id".to_string()),
                args: vec![ArgNode {
                    name: Some("plugin_id".to_string()),
                    suggestions: plugin_ids.clone(),
                    ..ArgNode::default()
                }],
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "disable".to_string(),
                tooltip: Some("Disable plugin by id".to_string()),
                args: vec![ArgNode {
                    name: Some("plugin_id".to_string()),
                    suggestions: plugin_ids,
                    ..ArgNode::default()
                }],
                ..CommandSpec::default()
            },
        ],
        ..CommandSpec::default()
    }
}

fn theme_command_spec(state: &AppState) -> CommandSpec {
    let theme_names = state
        .themes
        .ids()
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();

    CommandSpec {
        name: CMD_THEME.to_string(),
        tooltip: Some("Inspect and apply themes".to_string()),
        subcommands: vec![
            CommandSpec {
                name: CMD_LIST.to_string(),
                tooltip: Some("List available themes".to_string()),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: CMD_SHOW.to_string(),
                tooltip: Some("Show a theme definition".to_string()),
                args: vec![ArgNode {
                    name: Some("name".to_string()),
                    suggestions: theme_names.clone(),
                    ..ArgNode::default()
                }],
                ..CommandSpec::default()
            },
            CommandSpec {
                name: CMD_USE.to_string(),
                tooltip: Some("Set active theme".to_string()),
                args: vec![ArgNode {
                    name: Some("name".to_string()),
                    suggestions: theme_names,
                    ..ArgNode::default()
                }],
                ..CommandSpec::default()
            },
        ],
        ..CommandSpec::default()
    }
}

fn config_command_spec(state: &AppState) -> CommandSpec {
    let key_suggestions = config_key_suggestions();
    let profile_suggestions = state
        .config
        .resolved()
        .known_profiles()
        .iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();

    let mut show_flags = BTreeMap::new();
    show_flags.insert(
        "--sources".to_string(),
        FlagNode {
            flag_only: true,
            tooltip: Some("Include source layers".to_string()),
            ..FlagNode::default()
        },
    );
    show_flags.insert(
        "--raw".to_string(),
        FlagNode {
            flag_only: true,
            tooltip: Some("Show raw values".to_string()),
            ..FlagNode::default()
        },
    );

    let mut explain_flags = BTreeMap::new();
    explain_flags.insert(
        "--show-secrets".to_string(),
        FlagNode {
            flag_only: true,
            tooltip: Some("Reveal secret values".to_string()),
            ..FlagNode::default()
        },
    );

    let mut set_flags = BTreeMap::new();
    set_flags.insert(
        "--global".to_string(),
        FlagNode {
            flag_only: true,
            ..FlagNode::default()
        },
    );
    set_flags.insert(
        "--profile".to_string(),
        FlagNode {
            suggestions: profile_suggestions,
            ..FlagNode::default()
        },
    );
    set_flags.insert(
        "--profile-all".to_string(),
        FlagNode {
            flag_only: true,
            ..FlagNode::default()
        },
    );
    set_flags.insert(
        "--terminal".to_string(),
        FlagNode {
            suggestions: vec![
                SuggestionEntry::value(CURRENT_TERMINAL_SENTINEL),
                SuggestionEntry::value("cli"),
                SuggestionEntry::value("repl"),
            ],
            ..FlagNode::default()
        },
    );
    for flag in [
        "--session",
        "--config",
        "--secrets",
        "--save",
        "--dry-run",
        "--yes",
        "--explain",
    ] {
        set_flags.insert(
            flag.to_string(),
            FlagNode {
                flag_only: true,
                ..FlagNode::default()
            },
        );
    }

    CommandSpec {
        name: CMD_CONFIG.to_string(),
        tooltip: Some("Inspect and edit runtime config".to_string()),
        subcommands: vec![
            CommandSpec {
                name: CMD_SHOW.to_string(),
                tooltip: Some("Show current config".to_string()),
                flags: show_flags.clone(),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "get".to_string(),
                tooltip: Some("Get one config key".to_string()),
                args: vec![ArgNode {
                    name: Some("key".to_string()),
                    suggestions: key_suggestions.clone(),
                    ..ArgNode::default()
                }],
                flags: show_flags,
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "explain".to_string(),
                tooltip: Some("Explain one config key".to_string()),
                args: vec![ArgNode {
                    name: Some("key".to_string()),
                    suggestions: key_suggestions,
                    ..ArgNode::default()
                }],
                flags: explain_flags,
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "set".to_string(),
                tooltip: Some("Set config value".to_string()),
                flags: set_flags,
                ..CommandSpec::default()
            },
            CommandSpec {
                name: "diagnostics".to_string(),
                tooltip: Some("Show config diagnostics".to_string()),
                ..CommandSpec::default()
            },
        ],
        ..CommandSpec::default()
    }
}

fn config_set_key_specs() -> Vec<ConfigKeySpec> {
    let schema = ConfigSchema::default();
    schema
        .entries()
        .map(|(key, entry)| {
            let value_suggestions = if let Some(allowed) = entry.allowed_values() {
                allowed
                    .iter()
                    .map(|value| SuggestionEntry::value(value.clone()))
                    .collect::<Vec<_>>()
            } else if matches!(entry.value_type(), SchemaValueType::Bool) {
                vec![
                    SuggestionEntry::value("true"),
                    SuggestionEntry::value("false"),
                ]
            } else {
                Vec::new()
            };

            ConfigKeySpec {
                key: key.to_string(),
                tooltip: None,
                value_suggestions,
            }
        })
        .collect()
}

fn config_key_suggestions() -> Vec<SuggestionEntry> {
    let schema = ConfigSchema::default();
    schema
        .entries()
        .map(|(key, _)| SuggestionEntry::value(key.to_string()))
        .collect()
}

fn default_pipe_verbs() -> BTreeMap<String, String> {
    BTreeMap::from([
        ("F".to_string(), "Filter rows".to_string()),
        ("P".to_string(), "Project columns".to_string()),
        ("S".to_string(), "Sort rows".to_string()),
        ("G".to_string(), "Group rows".to_string()),
        ("A".to_string(), "Aggregate rows/groups".to_string()),
        ("L".to_string(), "Limit rows".to_string()),
        ("Z".to_string(), "Collapse grouped output".to_string()),
        ("C".to_string(), "Count rows".to_string()),
        ("Y".to_string(), "Mark output for copy".to_string()),
        ("H".to_string(), "Show DSL help".to_string()),
        ("V".to_string(), "Value-only quick search".to_string()),
        ("K".to_string(), "Key-only quick search".to_string()),
        ("?".to_string(), "Clean rows / exists filter".to_string()),
        ("U".to_string(), "Unroll list field".to_string()),
        ("JQ".to_string(), "Run jq expression".to_string()),
        ("VAL".to_string(), "Extract values".to_string()),
        ("VALUE".to_string(), "Extract values".to_string()),
    ])
}

pub(crate) fn maybe_render_dsl_help(state: &AppState, stages: &[String]) -> Option<String> {
    for raw in stages {
        let parsed = parse_stage(raw);
        if parsed.verb.eq_ignore_ascii_case("H") {
            return Some(render_dsl_help(state, &parsed.spec));
        }
    }
    None
}

fn render_dsl_help(state: &AppState, spec: &str) -> String {
    let resolved = state.ui.render_settings.resolve_render_settings();
    let theme_name = resolved.theme_name.as_str();
    let mut out = String::new();
    out.push_str(&render_section_divider_with_overrides(
        "DSL Help",
        resolved.unicode,
        resolved.width,
        resolved.color,
        theme_name,
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
    let verbs = default_pipe_verbs();
    for raw in stages {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        let verb_token = trimmed.split_whitespace().next().unwrap_or_default();
        if verb_token.len() == 1 && verb_token.chars().all(|ch| ch.is_ascii_alphabetic()) {
            let verb = verb_token.to_ascii_uppercase();
            if !verbs.contains_key(&verb) {
                return Err(miette!(
                    "Unknown DSL verb '{}' in pipe '{}'. Use `| H <verb>` for help.",
                    verb,
                    trimmed
                ));
            }
        }
    }
    Ok(())
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
