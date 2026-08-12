//! Derived REPL browse/completion surface.
//!
//! The REPL needs a compact view of what the operator can see and type at the
//! root: completion specs, overview entries, intro command suggestions, and
//! alias hints. This module builds that derived surface from auth, config,
//! command definitions, and plugin catalog state.

use crate::completion::tree::command_spec_from_command_def;
use crate::completion::{ArgNode, CommandSpec, FlagNode, SuggestionEntry, ValueType};
use crate::config::{ConfigSchema, SchemaValueType};
use crate::core::command_def::CommandDef;
use crate::guide::HelpLevel;
use std::collections::{BTreeMap, BTreeSet};

use crate::app::help::help_level;
use crate::app::{
    CMD_CONFIG, CMD_DOCTOR, CMD_HISTORY, CMD_LIST, CMD_PLUGINS, CMD_SHOW, CMD_THEME, CMD_USE,
    CURRENT_TERMINAL_SENTINEL,
};
use crate::plugin::CommandCatalogEntry;
use crate::ui::{HelpLayout, help_layout_from_config};

use super::ReplViewContext;
use super::history;
use crate::cli::commands::{doctor as doctor_cmd, theme as theme_cmd};

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
    let history_enabled = history::repl_history_enabled(view.config) && view.history_enabled;
    let aliases = collect_alias_entries(view.config);
    let help_layout = help_layout_from_config(view.config);
    let help_level = help_level(view.config, 0, 0);

    let mut root_words = catalog_completion_words(catalog);
    root_words.push("source".to_string());
    let mut specs = vec![
        CommandSpec::new("help")
            .tooltip("Show REPL help")
            .sort(command_sort_key("help", help_layout)),
        CommandSpec::new("last")
            .tooltip("Replay the last successful result")
            .sort(command_sort_key("last", help_layout))
            .flag(
                "--raw",
                FlagNode::new()
                    .flag_only()
                    .tooltip("Show the pre-pipeline result"),
            ),
        CommandSpec::new("source")
            .tooltip("Run commands from files")
            .sort(command_sort_key("source", help_layout))
            .arg(
                ArgNode::named("FILE")
                    .tooltip("Command file")
                    .multi()
                    .value_type(ValueType::Path),
            )
            .flag(
                "--ignore-errors",
                FlagNode::new()
                    .flag_only()
                    .tooltip("Continue after a command fails"),
            ),
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
        ReplOverviewEntry {
            name: "last".to_string(),
            summary: "Replay the last successful result; use --raw for the pre-pipeline payload."
                .to_string(),
        },
        ReplOverviewEntry {
            name: "source".to_string(),
            summary: "Run commands from files.".to_string(),
        },
    ];
    if shows_invocation_options_overview(help_level) {
        overview_entries.push(ReplOverviewEntry {
            name: "options".to_string(),
            summary: "per invocation: --format/--guide/--json/--table/--value/--md, --mode, --color, --unicode/--ascii, -v/-q/-d, --cache, --plugin-provider".to_string(),
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
            summary: "Inspect and manage plugin providers".to_string(),
        });
    }
    if view.auth.is_builtin_visible(CMD_DOCTOR)
        && let Some(def) = doctor_cmd::doctor_command_def(command_sort_key(CMD_DOCTOR, help_layout))
    {
        root_words.push(CMD_DOCTOR.to_string());
        specs.push(command_spec_from_command_def(&def));
        overview_entries.push(overview_entry_from_command_def(&def));
    }
    if view.auth.is_builtin_visible(CMD_THEME) {
        root_words.extend([
            CMD_THEME.to_string(),
            CMD_LIST.to_string(),
            CMD_SHOW.to_string(),
            CMD_USE.to_string(),
        ]);
        let def =
            theme_cmd::theme_command_def(view.themes, command_sort_key(CMD_THEME, help_layout));
        specs.push(command_spec_from_command_def(&def));
        overview_entries.push(overview_entry_from_command_def(&def));
    }
    if view.auth.is_builtin_visible(CMD_CONFIG) {
        root_words.extend([
            CMD_CONFIG.to_string(),
            "alias".to_string(),
            "add".to_string(),
            "get".to_string(),
            "show".to_string(),
            "explain".to_string(),
            "set".to_string(),
            "remove".to_string(),
            "doctor".to_string(),
        ]);
        specs.push(config_command_spec(view));
        specs.push(alias_command_spec(view, &aliases));
        overview_entries.push(ReplOverviewEntry {
            name: CMD_CONFIG.to_string(),
            summary: "Inspect and edit runtime config".to_string(),
        });
        overview_entries.push(ReplOverviewEntry {
            name: "alias".to_string(),
            summary: "Create and manage command aliases".to_string(),
        });
    }
    if history_enabled && view.auth.is_builtin_visible(CMD_HISTORY) {
        root_words.extend([
            CMD_HISTORY.to_string(),
            CMD_LIST.to_string(),
            "prune".to_string(),
            "clear".to_string(),
        ]);
        let def = history::history_command_def(command_sort_key(CMD_HISTORY, help_layout));
        specs.push(command_spec_from_command_def(&def));
        overview_entries.push(overview_entry_from_command_def(&def));
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

fn shows_invocation_options_overview(help_level: HelpLevel) -> bool {
    help_level >= HelpLevel::Verbose
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
        "last" => (0, 1),
        "exit" => (0, 2),
        "quit" => (0, 3),
        CMD_CONFIG => (1, 0),
        CMD_THEME => (1, 1),
        CMD_PLUGINS => (1, 2),
        CMD_DOCTOR => (1, 3),
        CMD_HISTORY => (1, 4),
        "alias" => (1, 5),
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
        "last" => (0, 1),
        "exit" => (0, 2),
        "quit" => (0, 3),
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
        "last".to_string(),
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
        "help" | "last" | "exit" | "quit" | CMD_PLUGINS | CMD_THEME | CMD_CONFIG | CMD_HISTORY
    ) {
        return None;
    }

    let mut spec = entry.completion.clone();
    if let Some(auth_hint) = entry.auth_hint() {
        let base = spec.tooltip.as_deref().unwrap_or("Plugin command");
        spec.tooltip = Some(format!("{base} [{auth_hint}]"));
    }
    if entry.conflicted || entry.requires_selection {
        spec.tooltip = Some(provider_selection_summary(entry, spec.tooltip.as_deref()));
    }

    Some(spec)
}

fn plugin_overview_entry(entry: &CommandCatalogEntry) -> ReplOverviewEntry {
    let summary = if entry.about.trim().is_empty() {
        "Plugin command".to_string()
    } else if entry.conflicted || entry.requires_selection {
        provider_selection_summary(entry, Some(&entry.about))
    } else {
        entry.about.clone()
    };
    let summary = entry
        .auth_hint()
        .map(|hint| format!("{summary} [{hint}]"))
        .unwrap_or(summary);

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

fn overview_entry_from_command_def(def: &CommandDef) -> ReplOverviewEntry {
    let summary = def
        .about
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Command")
        .to_string();

    ReplOverviewEntry {
        name: def.name.clone(),
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
        .filter(|entry| entry.source.is_some() || !entry.providers.is_empty())
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
            CommandSpec::new("clear-state")
                .tooltip("Clear persisted state for one command")
                .sort("17")
                .arg(ArgNode::named("command").suggestions(command_names.clone())),
            CommandSpec::new("select-provider")
                .tooltip("Select provider for one command")
                .sort("18")
                .arg(ArgNode::named("command").suggestions(command_names.clone()))
                .arg(ArgNode::named("plugin_id").suggestions(plugin_ids)),
            CommandSpec::new("clear-provider")
                .tooltip("Clear selected provider for one command")
                .sort("19")
                .arg(ArgNode::named("command").suggestions(command_names)),
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

    let write_flags = config_write_flags(profile_suggestions);
    let mut set_flags = write_flags.clone();
    set_flags.insert(
        "--yes".to_string(),
        FlagNode::new()
            .flag_only()
            .tooltip("Skip interactive confirmation"),
    );
    set_flags.insert(
        "--explain".to_string(),
        FlagNode::new()
            .flag_only()
            .tooltip("Explain the resolved write targets"),
    );

    CommandSpec::new(CMD_CONFIG)
        .tooltip("Inspect and edit runtime config")
        .sort(command_sort_key(
            CMD_CONFIG,
            help_layout_from_config(view.config),
        ))
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
            CommandSpec::new("unset")
                .tooltip("Remove one config key")
                .sort("14")
                .arg(ArgNode::named("key").suggestions(config_key_suggestions()))
                .flags(write_flags),
            CommandSpec::new("doctor")
                .tooltip("Show config diagnostics")
                .sort("15"),
        ])
}

fn alias_command_spec(view: ReplViewContext<'_>, aliases: &[ReplAliasEntry]) -> CommandSpec {
    let profile_suggestions = view
        .config
        .known_profiles()
        .iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();
    let alias_suggestions = aliases
        .iter()
        .map(|alias| SuggestionEntry::value(alias.name.clone()).meta(alias.tooltip.clone()))
        .collect::<Vec<_>>();
    let write_flags = config_write_flags(profile_suggestions);

    CommandSpec::new("alias")
        .tooltip("Create and manage command aliases")
        .sort(command_sort_key(
            "alias",
            help_layout_from_config(view.config),
        ))
        .subcommands([
            CommandSpec::new(CMD_LIST)
                .tooltip("List aliases in the active scope")
                .sort("10")
                .flag(
                    "--sources",
                    FlagNode::new()
                        .flag_only()
                        .tooltip("Include config source and scope"),
                ),
            CommandSpec::new("add")
                .tooltip("Add or replace an alias")
                .sort("11")
                .arg(ArgNode::named("name"))
                .arg(ArgNode::named("template"))
                .flags(write_flags.clone()),
            CommandSpec::new("remove")
                .tooltip("Remove an alias")
                .sort("12")
                .arg(ArgNode::named("name").suggestions(alias_suggestions))
                .flags(write_flags),
        ])
}

fn config_write_flags(profile_suggestions: Vec<SuggestionEntry>) -> BTreeMap<String, FlagNode> {
    BTreeMap::from([
        (
            "--global".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Write to the global config store"),
        ),
        (
            "--profile".to_string(),
            FlagNode::new()
                .suggestions(profile_suggestions)
                .tooltip("Write to one named profile"),
        ),
        (
            "--profile-all".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Write to every known profile"),
        ),
        (
            "--terminal".to_string(),
            FlagNode::new()
                .suggestions([
                    SuggestionEntry::value(CURRENT_TERMINAL_SENTINEL),
                    SuggestionEntry::value("cli"),
                    SuggestionEntry::value("repl"),
                ])
                .tooltip("Write to one terminal context"),
        ),
        (
            "--session".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Change only this in-memory session"),
        ),
        (
            "--config".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Use the regular config store"),
        ),
        (
            "--secrets".to_string(),
            FlagNode::new().flag_only().tooltip("Use the secrets store"),
        ),
        (
            "--save".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Persist immediately after validation"),
        ),
        (
            "--dry-run".to_string(),
            FlagNode::new()
                .flag_only()
                .tooltip("Show the write plan without changing config"),
        ),
    ])
}

fn config_key_suggestions() -> Vec<SuggestionEntry> {
    let schema = ConfigSchema::default();
    schema
        .entries()
        .map(|(key, _)| SuggestionEntry::value(key.to_string()))
        .collect()
}

pub(crate) fn config_set_key_specs(
    view: ReplViewContext<'_>,
) -> Vec<crate::completion::ConfigKeySpec> {
    let schema = ConfigSchema::default();
    schema
        .entries()
        .map(|(key, entry)| {
            let value_suggestions = if key == "theme.name" {
                view.themes
                    .ids()
                    .into_iter()
                    .map(SuggestionEntry::value)
                    .collect::<Vec<_>>()
            } else if let Some(allowed) = entry.allowed_values() {
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

            let mut spec =
                crate::completion::ConfigKeySpec::new(key).value_suggestions(value_suggestions);
            if let Some(doc) = entry.doc() {
                spec = spec.tooltip(doc);
            }
            spec
        })
        .collect()
}
