use crate::completion::{ArgNode, CommandSpec, FlagNode, SuggestionEntry};
use crate::config::{ConfigSchema, SchemaValueType};
use std::collections::{BTreeMap, BTreeSet};

use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HISTORY, CMD_LIST, CMD_PLUGINS, CMD_SHOW, CMD_THEME, CMD_USE,
    CURRENT_TERMINAL_SENTINEL,
};
use crate::plugin::CommandCatalogEntry;
use crate::ui::presentation::{HelpLayout, help_layout};

use super::ReplViewContext;
use super::history;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplOverviewEntry {
    pub(crate) name: String,
    pub(crate) summary: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReplAliasEntry {
    pub(crate) name: String,
    pub(crate) template: String,
    pub(crate) tooltip: String,
}

#[derive(Debug, Clone)]
pub(crate) struct ReplSurface {
    pub(crate) root_words: Vec<String>,
    pub(crate) intro_commands: Vec<String>,
    pub(crate) specs: Vec<CommandSpec>,
    pub(crate) aliases: Vec<ReplAliasEntry>,
    pub(crate) overview_entries: Vec<ReplOverviewEntry>,
}

pub(crate) fn build_repl_surface(
    view: ReplViewContext<'_>,
    catalog: &[CommandCatalogEntry],
) -> ReplSurface {
    let history_enabled = history::repl_history_enabled(view.config);
    let aliases = collect_alias_entries(view.config);
    let help_layout = help_layout(view.config);

    let mut root_words = catalog_completion_words(catalog);
    let mut specs = vec![
        CommandSpec::new("help")
            .tooltip("Show REPL help")
            .sort(command_sort_key("help", help_layout)),
        CommandSpec::new("exit")
            .tooltip("Exit REPL")
            .sort(command_sort_key("exit", help_layout)),
        CommandSpec::new("quit")
            .tooltip("Exit REPL")
            .sort(command_sort_key("quit", help_layout)),
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
    if shows_invocation_options_overview(help_layout) {
        overview_entries.push(ReplOverviewEntry {
            name: "options".to_string(),
            summary: "per invocation: --format/--json/--table/--value/--md, --mode, --color, --unicode/--ascii, -v/-q/-d, --cache, --plugin-provider".to_string(),
        });
    }

    specs.extend(
        catalog
            .iter()
            .filter_map(command_spec_from_catalog)
            .collect::<Vec<_>>(),
    );

    if view.auth.is_builtin_visible(CMD_PLUGINS) {
        root_words.extend([CMD_PLUGINS.to_string(), CMD_LIST.to_string()]);
        specs.push(plugins_command_spec(catalog, help_layout));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_PLUGINS.to_string(),
            summary: "subcommands: list, commands, enable, disable, doctor".to_string(),
        });
    }
    if view.auth.is_builtin_visible(CMD_DOCTOR) {
        root_words.push(CMD_DOCTOR.to_string());
        specs.push(doctor_command_spec(help_layout));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_DOCTOR.to_string(),
            summary: "subcommands: all, config, last, plugins, theme".to_string(),
        });
    }
    if view.auth.is_builtin_visible(CMD_THEME) {
        root_words.extend([
            CMD_THEME.to_string(),
            CMD_LIST.to_string(),
            CMD_SHOW.to_string(),
            CMD_USE.to_string(),
        ]);
        specs.push(theme_command_spec(view));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_THEME.to_string(),
            summary: "subcommands: list, show, use".to_string(),
        });
    }
    if view.auth.is_builtin_visible(CMD_CONFIG) {
        root_words.extend([
            CMD_CONFIG.to_string(),
            "get".to_string(),
            "show".to_string(),
            "explain".to_string(),
            "set".to_string(),
            "doctor".to_string(),
        ]);
        specs.push(config_command_spec(view));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_CONFIG.to_string(),
            summary: "subcommands: show, get, explain, set, doctor".to_string(),
        });
    }
    if history_enabled && view.auth.is_builtin_visible(CMD_HISTORY) {
        root_words.extend([
            CMD_HISTORY.to_string(),
            CMD_LIST.to_string(),
            "prune".to_string(),
            "clear".to_string(),
        ]);
        specs
            .push(history::history_command_spec().sort(command_sort_key(CMD_HISTORY, help_layout)));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_HISTORY.to_string(),
            summary: "subcommands: list, prune, clear".to_string(),
        });
    }

    overview_entries.extend(catalog.iter().map(plugin_overview_entry));

    root_words.extend(view.themes.ids());
    root_words.extend(aliases.iter().map(|entry| entry.name.clone()));
    normalize_root_words(&mut root_words);
    order_root_words(&mut root_words, help_layout);
    let intro_commands = root_words
        .iter()
        .filter(|word| root_word_can_appear_in_intro(word))
        .take(4)
        .cloned()
        .collect();

    ReplSurface {
        root_words,
        intro_commands,
        specs,
        aliases,
        overview_entries,
    }
}

fn shows_invocation_options_overview(help_layout: HelpLayout) -> bool {
    matches!(help_layout, HelpLayout::Full)
}

fn normalize_root_words(root_words: &mut Vec<String>) {
    root_words.sort();
    root_words.dedup();
}

fn order_root_words(root_words: &mut [String], help_layout: HelpLayout) {
    if matches!(help_layout, HelpLayout::Full) {
        return;
    }

    root_words.sort_by(|left, right| {
        root_word_priority(left)
            .cmp(&root_word_priority(right))
            .then_with(|| left.cmp(right))
    });
}

fn root_word_priority(word: &str) -> (u8, u8) {
    match word {
        "help" => (0, 0),
        "exit" => (0, 1),
        "quit" => (0, 2),
        CMD_CONFIG => (1, 0),
        CMD_THEME => (1, 1),
        CMD_PLUGINS => (1, 2),
        CMD_DOCTOR => (1, 3),
        CMD_HISTORY => (1, 4),
        "|" | "F" | "P" | "V" => (4, 0),
        _ => {
            if word.starts_with('-') {
                (5, 0)
            } else {
                (2, 0)
            }
        }
    }
}

fn root_word_can_appear_in_intro(word: &str) -> bool {
    !matches!(word, "exit" | "quit")
        && !word.starts_with('-')
        && !matches!(word, "|" | "F" | "P" | "V")
}

fn command_sort_key(name: &str, help_layout: HelpLayout) -> String {
    let (tier, order) = if matches!(help_layout, HelpLayout::Full) {
        expressive_command_priority(name)
    } else {
        compact_command_priority(name)
    };
    format!("{}{:02}", tier, order)
}

fn expressive_command_priority(name: &str) -> (u8, u8) {
    match name {
        "help" => (0, 0),
        "exit" => (0, 1),
        "quit" => (0, 2),
        _ => (9, 0),
    }
}

fn compact_command_priority(name: &str) -> (u8, u8) {
    match name {
        "help" => (0, 0),
        "exit" => (0, 1),
        "quit" => (0, 2),
        CMD_CONFIG => (1, 0),
        CMD_THEME => (1, 1),
        CMD_PLUGINS => (1, 2),
        CMD_DOCTOR => (1, 3),
        CMD_HISTORY => (1, 4),
        _ => (9, 0),
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

pub(crate) fn collect_alias_entries(config: &crate::config::ResolvedConfig) -> Vec<ReplAliasEntry> {
    let mut out = Vec::new();
    for (key, entry) in config.aliases() {
        let Some(name) = key.strip_prefix("alias.") else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let template = entry.raw_value.to_string();
        out.push(ReplAliasEntry {
            name: name.to_string(),
            tooltip: format!("alias: {template}"),
            template,
        });
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

    let mut spec = entry.completion.clone();
    if entry.conflicted || entry.requires_selection {
        spec.tooltip = Some(provider_selection_summary(entry, spec.tooltip.as_deref()));
    }

    Some(spec)
}

fn plugin_overview_entry(entry: &CommandCatalogEntry) -> ReplOverviewEntry {
    let summary = if entry.about.trim().is_empty() {
        "Plugin command".to_string()
    } else if entry.subcommands.is_empty() {
        if entry.conflicted || entry.requires_selection {
            provider_selection_summary(entry, Some(&entry.about))
        } else {
            entry.about.clone()
        }
    } else {
        let base = format!(
            "{} (subcommands: {})",
            entry.about,
            entry.subcommands.join(", ")
        );
        if entry.conflicted || entry.requires_selection {
            provider_selection_summary(entry, Some(&base))
        } else {
            base
        }
    };

    ReplOverviewEntry {
        name: entry.name.clone(),
        summary,
    }
}

fn provider_selection_summary(entry: &CommandCatalogEntry, base: Option<&str>) -> String {
    let base = base
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Plugin command");
    if entry.requires_selection {
        return format!(
            "{base} (provider selection required; available: {}; use --plugin-provider <plugin-id> or `plugins select-provider {} <plugin-id>`)",
            entry.providers.join(", "),
            entry.name,
        );
    }

    let selected_label = match (&entry.provider, entry.source) {
        (Some(provider), Some(source)) => format!("{provider} ({source})"),
        _ => return base.to_string(),
    };
    let alternatives = entry
        .providers
        .iter()
        .filter(|label| label.as_str() != selected_label.as_str())
        .cloned()
        .collect::<Vec<_>>();
    let selection_reason = if entry.selected_explicitly {
        "selected explicitly"
    } else {
        "resolved uniquely"
    };

    if alternatives.is_empty() {
        format!("{base} (using {selected_label}; {selection_reason})")
    } else {
        format!(
            "{base} (using {selected_label}; {selection_reason}; alternatives: {})",
            alternatives.join(", ")
        )
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

fn plugins_command_spec(catalog: &[CommandCatalogEntry], help_layout: HelpLayout) -> CommandSpec {
    let plugin_ids = catalog
        .iter()
        .flat_map(|entry| {
            entry
                .provider
                .iter()
                .cloned()
                .chain(entry.providers.iter().filter_map(|label| {
                    label
                        .split_once(" (")
                        .map(|(plugin_id, _)| plugin_id.to_string())
                }))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();
    let command_names = catalog
        .iter()
        .map(|entry| SuggestionEntry::value(entry.name.clone()))
        .collect::<Vec<_>>();

    CommandSpec::new(CMD_PLUGINS)
        .tooltip("Inspect and manage plugin providers")
        .sort(command_sort_key(CMD_PLUGINS, help_layout))
        .subcommands([
            CommandSpec::new(CMD_LIST)
                .tooltip("List available plugins")
                .sort("10"),
            CommandSpec::new("commands")
                .tooltip("Show plugin command catalog")
                .sort("11"),
            CommandSpec::new("config")
                .tooltip("Show projected plugin config")
                .sort("12")
                .arg(ArgNode::named("plugin_id").suggestions(plugin_ids.clone())),
            CommandSpec::new("refresh")
                .tooltip("Refresh plugin discovery cache")
                .sort("13"),
            CommandSpec::new("doctor")
                .tooltip("Run plugin diagnostics")
                .sort("14"),
            CommandSpec::new("enable")
                .tooltip("Enable plugin by id")
                .sort("15")
                .arg(ArgNode::named("plugin_id").suggestions(plugin_ids.clone())),
            CommandSpec::new("disable")
                .tooltip("Disable plugin by id")
                .sort("16")
                .arg(ArgNode::named("plugin_id").suggestions(plugin_ids.clone())),
            CommandSpec::new("select-provider")
                .tooltip("Select provider for one command")
                .sort("17")
                .arg(ArgNode::named("command").suggestions(command_names.clone()))
                .arg(ArgNode::named("plugin_id").suggestions(plugin_ids)),
            CommandSpec::new("clear-provider")
                .tooltip("Clear selected provider for one command")
                .sort("18")
                .arg(ArgNode::named("command").suggestions(command_names)),
        ])
}

fn theme_command_spec(view: ReplViewContext<'_>) -> CommandSpec {
    let theme_names = view
        .themes
        .ids()
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();

    CommandSpec::new(CMD_THEME)
        .tooltip("Inspect and apply themes")
        .sort(command_sort_key(CMD_THEME, help_layout(view.config)))
        .subcommands([
            CommandSpec::new(CMD_LIST)
                .tooltip("List available themes")
                .sort("10"),
            CommandSpec::new(CMD_SHOW)
                .tooltip("Show a theme definition")
                .sort("11")
                .arg(ArgNode::named("name").suggestions(theme_names.clone())),
            CommandSpec::new(CMD_USE)
                .tooltip("Set active theme")
                .sort("12")
                .arg(ArgNode::named("name").suggestions(theme_names)),
        ])
}

fn config_command_spec(view: ReplViewContext<'_>) -> CommandSpec {
    let key_suggestions = config_key_suggestions();
    let profile_suggestions = view
        .config
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
        .sort(command_sort_key(CMD_CONFIG, help_layout(view.config)))
        .subcommands([
            CommandSpec::new(CMD_SHOW)
                .tooltip("Show current config")
                .sort("10")
                .flags(show_flags.clone()),
            CommandSpec::new("get")
                .tooltip("Get one config key")
                .sort("11")
                .arg(ArgNode::named("key").suggestions(key_suggestions.clone()))
                .flags(show_flags),
            CommandSpec::new("explain")
                .tooltip("Explain one config key")
                .sort("12")
                .arg(ArgNode::named("key").suggestions(key_suggestions))
                .flags(explain_flags),
            CommandSpec::new("set")
                .tooltip("Set config value")
                .sort("13")
                .flags(set_flags),
            CommandSpec::new("doctor")
                .tooltip("Show config diagnostics")
                .sort("14"),
        ])
}

fn doctor_command_spec(help_layout: HelpLayout) -> CommandSpec {
    CommandSpec::new(CMD_DOCTOR)
        .tooltip("Run diagnostics checks")
        .sort(command_sort_key(CMD_DOCTOR, help_layout))
        .subcommands([
            CommandSpec::new("all").sort("10"),
            CommandSpec::new(CMD_CONFIG).sort("11"),
            CommandSpec::new("last")
                .tooltip("Show the last REPL failure")
                .sort("12"),
            CommandSpec::new(CMD_PLUGINS).sort("13"),
            CommandSpec::new(CMD_THEME).sort("14"),
        ])
}

fn config_key_suggestions() -> Vec<SuggestionEntry> {
    let schema = ConfigSchema::default();
    schema
        .entries()
        .map(|(key, _)| SuggestionEntry::value(key.to_string()))
        .collect()
}

pub(crate) fn config_set_key_specs() -> Vec<crate::completion::ConfigKeySpec> {
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

            crate::completion::ConfigKeySpec::new(key).value_suggestions(value_suggestions)
        })
        .collect()
}
