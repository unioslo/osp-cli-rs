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
        CommandSpec::new("help").tooltip("Show REPL help"),
        CommandSpec::new("exit").tooltip("Exit REPL"),
        CommandSpec::new("quit").tooltip("Exit REPL"),
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
            summary: "subcommands: all, config, last, plugins, theme".to_string(),
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
        words.extend(spec_completion_words(&entry.completion));
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

    Some(entry.completion.clone())
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

fn spec_completion_words(spec: &CommandSpec) -> Vec<String> {
    let mut words = vec![spec.name.clone()];
    for flag in spec.flags.keys() {
        words.push(flag.clone());
    }
    for subcommand in &spec.subcommands {
        words.extend(spec_completion_words(subcommand));
    }
    words
}

fn plugins_command_spec(catalog: &[CommandCatalogEntry]) -> CommandSpec {
    let plugin_ids = catalog
        .iter()
        .map(|entry| entry.provider.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();

    CommandSpec::new(CMD_PLUGINS)
        .tooltip("Inspect and manage plugin providers")
        .subcommands([
            CommandSpec::new(CMD_LIST).tooltip("List available plugins"),
            CommandSpec::new("commands").tooltip("Show plugin command catalog"),
            CommandSpec::new("doctor").tooltip("Run plugin diagnostics"),
            CommandSpec::new("enable")
                .tooltip("Enable plugin by id")
                .arg(ArgNode::named("plugin_id").suggestions(plugin_ids.clone())),
            CommandSpec::new("disable")
                .tooltip("Disable plugin by id")
                .arg(ArgNode::named("plugin_id").suggestions(plugin_ids)),
        ])
}

fn theme_command_spec(state: &AppState) -> CommandSpec {
    let theme_names = state
        .themes
        .ids()
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();

    CommandSpec::new(CMD_THEME)
        .tooltip("Inspect and apply themes")
        .subcommands([
            CommandSpec::new(CMD_LIST).tooltip("List available themes"),
            CommandSpec::new(CMD_SHOW)
                .tooltip("Show a theme definition")
                .arg(ArgNode::named("name").suggestions(theme_names.clone())),
            CommandSpec::new(CMD_USE)
                .tooltip("Set active theme")
                .arg(ArgNode::named("name").suggestions(theme_names)),
        ])
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

    let show_flags = BTreeMap::from([
        (
            "--sources".to_string(),
            FlagNode::new().flag_only().tooltip("Include source layers"),
        ),
        (
            "--raw".to_string(),
            FlagNode::new().flag_only().tooltip("Show raw values"),
        ),
    ]);

    let explain_flags = BTreeMap::from([(
        "--show-secrets".to_string(),
        FlagNode::new().flag_only().tooltip("Reveal secret values"),
    )]);

    let mut set_flags = BTreeMap::from([
        ("--global".to_string(), FlagNode::new().flag_only()),
        (
            "--profile".to_string(),
            FlagNode::new().suggestions(profile_suggestions),
        ),
        ("--profile-all".to_string(), FlagNode::new().flag_only()),
        (
            "--terminal".to_string(),
            FlagNode::new().suggestions([
                SuggestionEntry::value(CURRENT_TERMINAL_SENTINEL),
                SuggestionEntry::value("cli"),
                SuggestionEntry::value("repl"),
            ]),
        ),
    ]);
    for flag in [
        "--session",
        "--config",
        "--secrets",
        "--save",
        "--dry-run",
        "--yes",
        "--explain",
    ] {
        set_flags.insert(flag.to_string(), FlagNode::new().flag_only());
    }

    CommandSpec::new(CMD_CONFIG)
        .tooltip("Inspect and edit runtime config")
        .subcommands([
            CommandSpec::new(CMD_SHOW)
                .tooltip("Show current config")
                .flags(show_flags.clone()),
            CommandSpec::new("get")
                .tooltip("Get one config key")
                .arg(ArgNode::named("key").suggestions(key_suggestions.clone()))
                .flags(show_flags),
            CommandSpec::new("explain")
                .tooltip("Explain one config key")
                .arg(ArgNode::named("key").suggestions(key_suggestions))
                .flags(explain_flags),
            CommandSpec::new("set")
                .tooltip("Set config value")
                .flags(set_flags),
            CommandSpec::new("doctor").tooltip("Show config diagnostics"),
        ])
}

fn doctor_command_spec() -> CommandSpec {
    CommandSpec::new(CMD_DOCTOR)
        .tooltip("Run diagnostics checks")
        .subcommands([
            CommandSpec::new("all"),
            CommandSpec::new(CMD_CONFIG),
            CommandSpec::new("last").tooltip("Show the last REPL failure"),
            CommandSpec::new(CMD_PLUGINS),
            CommandSpec::new(CMD_THEME),
        ])
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

            osp_completion::ConfigKeySpec::new(key).value_suggestions(value_suggestions)
        })
        .collect()
}
