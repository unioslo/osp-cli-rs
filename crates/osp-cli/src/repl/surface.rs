use osp_completion::{ArgNode, CommandSpec, FlagNode, SuggestionEntry};
use osp_config::{ConfigSchema, SchemaValueType};
use std::collections::{BTreeMap, BTreeSet};

use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HISTORY, CMD_LIST, CMD_PLUGINS, CMD_SHOW, CMD_THEME, CMD_USE,
    CURRENT_TERMINAL_SENTINEL,
};
use crate::plugin_manager::CommandCatalogEntry;
use crate::state::AppState;

use super::history;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplOverviewEntry {
    pub(crate) name: String,
    pub(crate) summary: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReplSurface {
    pub(crate) root_words: Vec<String>,
    pub(crate) specs: Vec<CommandSpec>,
    pub(crate) aliases: Vec<(String, String)>,
    pub(crate) overview_entries: Vec<ReplOverviewEntry>,
}

pub(crate) fn build_repl_surface(state: &AppState, catalog: &[CommandCatalogEntry]) -> ReplSurface {
    let history_enabled = history::repl_history_enabled(state.config.resolved());
    let aliases = collect_alias_entries(state.config.resolved());

    let mut root_words = catalog_completion_words(catalog);
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
    let mut overview_entries = vec![
        ReplOverviewEntry {
            name: "exit".to_string(),
            summary: "Exit application.".to_string(),
        },
        ReplOverviewEntry {
            name: "help".to_string(),
            summary: "Show this command overview.".to_string(),
        },
    ];

    specs.extend(
        catalog
            .iter()
            .filter_map(command_spec_from_catalog)
            .collect::<Vec<_>>(),
    );
    overview_entries.extend(catalog.iter().map(plugin_overview_entry));

    if state.auth.is_builtin_visible(CMD_PLUGINS) {
        root_words.extend([CMD_PLUGINS.to_string(), CMD_LIST.to_string()]);
        specs.push(plugins_command_spec(catalog));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_PLUGINS.to_string(),
            summary: "subcommands: list, commands, enable, disable, doctor".to_string(),
        });
    }
    if state.auth.is_builtin_visible(CMD_DOCTOR) {
        root_words.push(CMD_DOCTOR.to_string());
        specs.push(doctor_command_spec());
        overview_entries.push(ReplOverviewEntry {
            name: CMD_DOCTOR.to_string(),
            summary: "subcommands: all, config, plugins, theme".to_string(),
        });
    }
    if state.auth.is_builtin_visible(CMD_THEME) {
        root_words.extend([
            CMD_THEME.to_string(),
            CMD_LIST.to_string(),
            CMD_SHOW.to_string(),
            CMD_USE.to_string(),
        ]);
        specs.push(theme_command_spec(state));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_THEME.to_string(),
            summary: "subcommands: list, show, use".to_string(),
        });
    }
    if state.auth.is_builtin_visible(CMD_CONFIG) {
        root_words.extend([
            CMD_CONFIG.to_string(),
            "get".to_string(),
            "show".to_string(),
            "explain".to_string(),
            "set".to_string(),
            "doctor".to_string(),
        ]);
        specs.push(config_command_spec(state));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_CONFIG.to_string(),
            summary: "subcommands: show, get, explain, set, doctor".to_string(),
        });
    }
    if history_enabled && state.auth.is_builtin_visible(CMD_HISTORY) {
        root_words.extend([
            CMD_HISTORY.to_string(),
            CMD_LIST.to_string(),
            "prune".to_string(),
            "clear".to_string(),
        ]);
        specs.push(history::history_command_spec());
        overview_entries.push(ReplOverviewEntry {
            name: CMD_HISTORY.to_string(),
            summary: "subcommands: list, prune, clear".to_string(),
        });
    }

    root_words.extend(state.themes.ids());
    root_words.extend(aliases.iter().map(|(name, _)| name.clone()));
    root_words.sort();
    root_words.dedup();

    ReplSurface {
        root_words,
        specs,
        aliases,
        overview_entries,
    }
}

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

fn plugin_overview_entry(entry: &CommandCatalogEntry) -> ReplOverviewEntry {
    let summary = if entry.about.trim().is_empty() {
        "Plugin command".to_string()
    } else if entry.subcommands.is_empty() {
        entry.about.clone()
    } else {
        format!(
            "{} (subcommands: {})",
            entry.about,
            entry.subcommands.join(", ")
        )
    };

    ReplOverviewEntry {
        name: entry.name.clone(),
        summary,
    }
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
                name: "doctor".to_string(),
                tooltip: Some("Show config diagnostics".to_string()),
                ..CommandSpec::default()
            },
        ],
        ..CommandSpec::default()
    }
}

fn doctor_command_spec() -> CommandSpec {
    CommandSpec {
        name: CMD_DOCTOR.to_string(),
        tooltip: Some("Run diagnostics checks".to_string()),
        subcommands: vec![
            CommandSpec {
                name: "all".to_string(),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: CMD_CONFIG.to_string(),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: CMD_PLUGINS.to_string(),
                ..CommandSpec::default()
            },
            CommandSpec {
                name: CMD_THEME.to_string(),
                ..CommandSpec::default()
            },
        ],
        ..CommandSpec::default()
    }
}

fn config_key_suggestions() -> Vec<SuggestionEntry> {
    let schema = ConfigSchema::default();
    schema
        .entries()
        .map(|(key, _)| SuggestionEntry::value(key.to_string()))
        .collect()
}

pub(crate) fn config_set_key_specs() -> Vec<osp_completion::ConfigKeySpec> {
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

            osp_completion::ConfigKeySpec {
                key: key.to_string(),
                tooltip: None,
                value_suggestions,
            }
        })
        .collect()
}
