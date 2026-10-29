//! Derived REPL browse/completion surface.
//!
//! The REPL needs a compact view of what the operator can see and type at the
//! root: completion specs, overview entries, intro command suggestions, and
//! alias hints. This module builds that derived surface from auth, config,
//! command definitions, and plugin catalog state.

use crate::completion::{CompletionNode, SuggestionEntry, ValueType};
use crate::config::{ConfigSchema, SchemaValueType};
use crate::core::document::HelpLevel;
use clap::CommandFactory;

use crate::app::help::help_level;
use crate::app::{
    CMD_CONFIG, CMD_CONTEXT, CMD_DOCTOR, CMD_HISTORY, CMD_PLUGINS, CMD_THEME,
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
    pub(crate) specs: Vec<CompletionNode>,
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

    let repl_commands = crate::cli::repl_only_commands();
    let mut specs = repl_commands
        .iter()
        .flat_map(|command| {
            let spec = clap_command_spec(command, help_layout);
            std::iter::once(spec.clone()).chain(command.get_visible_aliases().map(move |alias| {
                CompletionNode {
                    name: alias.to_string(),
                    ..spec.clone()
                }
            }))
        })
        .collect::<Vec<_>>();
    specs.push(plugins_command_spec(catalog, help_layout));
    if let Some(command) = crate::cli::Cli::command().find_subcommand("notifications") {
        specs.push(clap_command_spec(command, help_layout));
    }

    let mut doctor = clap_command_spec(&doctor_cmd::doctor_command(), help_layout);
    if let Some(app) = catalog.iter().find(|entry| entry.name == CMD_DOCTOR) {
        for (name, child) in &app.completion.children {
            doctor
                .children
                .entry(name.clone())
                .or_insert_with(|| child.clone());
        }
    }
    specs.push(doctor);
    let def = theme_cmd::theme_command();
    let mut spec = clap_command_spec(&def, help_layout);
    theme_cmd::augment_theme_completion(&mut spec, view.themes);
    specs.push(spec);

    specs.push(config_command_spec(view));
    specs.push(alias_command_spec(view, &aliases));
    if history_enabled {
        let def = history::history_command();
        let mut spec = clap_command_spec(&def, help_layout);
        history::augment_history_completion(&mut spec);
        specs.push(spec);
    }
    specs.push(context_command_spec(help_layout));
    specs.extend(catalog.iter().filter_map(command_spec_from_catalog));

    let mut overview_entries = specs
        .iter()
        .filter(|spec| !spec.hidden)
        .filter(|spec| {
            !repl_commands
                .iter()
                .any(|command| command.get_all_aliases().any(|alias| alias == spec.name))
        })
        .map(|spec| {
            let summary = match spec.name.as_str() {
                "exit" => "Exit application.".to_string(),
                "help" => "Show this command overview.".to_string(),
                "last" => "Replay the last result (--raw before pipes).".to_string(),
                "source" => "Run commands from files.".to_string(),
                CMD_CONTEXT => "Inspect or clear remembered session values".to_string(),
                _ => {
                    if let Some(entry) = catalog.iter().find(|entry| entry.name == spec.name) {
                        let mut overview = plugin_overview_entry(entry);
                        if !view.auth.external_command_access(&entry.name).is_runnable() {
                            overview.summary.push_str(" (restricted)");
                        }
                        return overview;
                    }
                    spec.tooltip
                        .clone()
                        .unwrap_or_else(|| "Command".to_string())
                }
            };
            ReplOverviewEntry {
                name: spec.name.clone(),
                summary,
            }
        })
        .collect::<Vec<_>>();
    // The overview has a historical presentation order independent of the menu.
    overview_entries.sort_by_key(|entry| match entry.name.as_str() {
        "exit" => 0,
        "help" => 1,
        "last" => 2,
        "source" => 3,
        CMD_PLUGINS => 4,
        CMD_DOCTOR => 5,
        CMD_THEME => 6,
        CMD_CONFIG => 7,
        "alias" => 8,
        CMD_HISTORY => 9,
        CMD_CONTEXT => 10,
        _ => 11,
    });
    if shows_invocation_options_overview(help_level) {
        overview_entries.insert(
            4,
            ReplOverviewEntry {
                name: "options".to_string(),
                summary: crate::cli::invocation::invocation_overview_summary(),
            },
        );
    }

    let mut root_words = specs
        .iter()
        .flat_map(|spec| {
            if catalog.iter().any(|entry| entry.name == spec.name) {
                spec_completion_words(spec)
            } else {
                vec![spec.name.clone()]
            }
        })
        .collect::<Vec<_>>();
    // Shorthand and builtin child words historically offered at the root.
    root_words.extend(
        [
            "P", "F", "V", "|", "list", "show", "use", "add", "get", "explain", "set", "remove",
            "doctor",
        ]
        .map(str::to_string),
    );
    if history_enabled {
        root_words.extend(["prune", "clear"].map(str::to_string));
    }
    root_words.extend(view.themes.ids());
    root_words.extend(aliases.iter().map(|entry| entry.name.clone()));
    normalize_root_words(&mut root_words);
    order_root_words(&mut root_words, help_layout);
    let intro_commands = root_words
        .iter()
        .filter(|word| {
            root_word_can_appear_in_intro(word)
                && (specs.iter().any(|spec| &spec.name == *word)
                    || aliases.iter().any(|alias| &alias.name == *word))
        })
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
        CMD_CONTEXT => (1, 5),
        "alias" => (1, 6),
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
        CMD_CONTEXT => (1, 5),
        _ => (9, 0),
    }
}

fn context_command_spec(help_layout: HelpLayout) -> CompletionNode {
    builtin_command_spec(crate::cli::builtin_command(CMD_CONTEXT), help_layout)
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

fn command_spec_from_catalog(entry: &CommandCatalogEntry) -> Option<CompletionNode> {
    if matches!(
        entry.name.as_str(),
        "help"
            | "last"
            | "exit"
            | "quit"
            | CMD_DOCTOR
            | CMD_PLUGINS
            | CMD_THEME
            | CMD_CONFIG
            | CMD_HISTORY
    ) {
        return None;
    }

    let mut spec = entry.completion.clone();
    if let Some(auth_hint) = entry.auth_hint() {
        let base = spec.tooltip.as_deref().unwrap_or("Plugin command");
        spec.tooltip = Some(format!("{base} [{auth_hint}]"));
    }

    Some(spec)
}

fn plugin_overview_entry(entry: &CommandCatalogEntry) -> ReplOverviewEntry {
    let summary = if entry.about.trim().is_empty() {
        "Plugin command".to_string()
    } else {
        entry.about.clone()
    };
    ReplOverviewEntry {
        name: entry.name.clone(),
        summary,
    }
}
fn clap_command_spec(command: &clap::Command, help_layout: HelpLayout) -> CompletionNode {
    let mut spec =
        CompletionNode::from_clap(command).sort(command_sort_key(command.get_name(), help_layout));
    for child in spec.children.values_mut() {
        if let Some(grammar) = command.find_subcommand(&child.name) {
            child.sort = Some(grammar.get_display_order().to_string());
        }
    }
    spec
}

fn spec_completion_words(spec: &CompletionNode) -> Vec<String> {
    let mut words = vec![spec.name.clone()];
    for flag in spec.flags.keys() {
        words.push(flag.clone());
    }
    for subcommand in spec.children.values() {
        words.extend(spec_completion_words(subcommand));
    }
    words
}

fn plugins_command_spec(
    catalog: &[CommandCatalogEntry],
    help_layout: HelpLayout,
) -> CompletionNode {
    let command_names = catalog
        .iter()
        .filter(|entry| matches!(entry.source, crate::plugin::CommandSource::Plugin(_)))
        .map(|entry| SuggestionEntry::value(entry.name.clone()))
        .collect::<Vec<_>>();

    let mut spec = builtin_command_spec(crate::cli::builtin_command(CMD_PLUGINS), help_layout);
    for child in spec.children.values_mut() {
        // Plugin mutations intentionally expose only their target arguments.
        child.flags.clear();
        for arg in &mut child.args {
            arg.suggestions = command_names.clone();
        }
    }
    spec
}

fn config_command_spec(view: ReplViewContext<'_>) -> CompletionNode {
    let mut spec = builtin_command_spec(
        crate::cli::builtin_command(CMD_CONFIG),
        help_layout_from_config(view.config),
    );
    config_write_suggestions(&mut spec, view.config);
    for child in spec.children.values_mut() {
        if child.name == "set" {
            // Config keys and their values are injected as a schema-derived tree.
            child.args.clear();
            child.flags.remove("--user");
            child.flags.remove("-u");
        } else {
            for arg in &mut child.args {
                arg.suggestions = config_key_suggestions();
            }
        }
    }
    spec
}

fn alias_command_spec(view: ReplViewContext<'_>, aliases: &[ReplAliasEntry]) -> CompletionNode {
    let mut spec = builtin_command_spec(
        crate::cli::builtin_command("alias"),
        help_layout_from_config(view.config),
    );
    config_write_suggestions(&mut spec, view.config);
    for child in spec.children.values_mut() {
        if child.name == "remove" {
            for arg in &mut child.args {
                arg.suggestions = aliases
                    .iter()
                    .map(|alias| {
                        SuggestionEntry::value(alias.name.clone()).meta(alias.tooltip.clone())
                    })
                    .collect();
            }
        }
        if let Some(flag) = child.flags.get_mut("--sources") {
            flag.tooltip = Some("Include config source and scope".to_string());
        }
    }
    spec
}

fn config_write_suggestions(spec: &mut CompletionNode, config: &crate::config::ResolvedConfig) {
    for child in spec.children.values_mut() {
        if let Some(flag) = child.flags.get_mut("--profile") {
            flag.suggestions = config
                .known_profiles()
                .iter()
                .map(SuggestionEntry::value)
                .collect();
        }
        if let Some(flag) = child.flags.get_mut("--terminal") {
            flag.suggestions = [CURRENT_TERMINAL_SENTINEL, "cli", "repl"]
                .into_iter()
                .map(SuggestionEntry::value)
                .collect();
        }
    }
}

fn builtin_command_spec(command: clap::Command, help_layout: HelpLayout) -> CompletionNode {
    let mut spec = clap_command_spec(&command, help_layout);
    for child in spec.children.values_mut() {
        for arg in &mut child.args {
            arg.name = arg.name.take().map(|name| name.to_ascii_lowercase());
            arg.required = false;
            arg.tooltip = None;
        }
        for (name, flag) in &mut child.flags {
            flag.tooltip = match name.as_str() {
                "--sources" => Some("Include source layers"),
                "--raw" => Some("Show raw values"),
                "--show-secrets" => Some("Reveal secret values"),
                "--global" => Some("Write to the global config store"),
                "--profile" => Some("Write to one named profile"),
                "--profile-all" => Some("Write to every known profile"),
                "--terminal" => Some("Write to one terminal context"),
                "--session" => Some("Change only this in-memory session"),
                "--config" => Some("Use the regular config store"),
                "--secrets" => Some("Use the secrets store"),
                "--permanent" | "--save" => Some("Persist immediately after validation"),
                "--dry-run" => Some("Show the write plan without changing config"),
                "--yes" => Some("Skip interactive confirmation"),
                "--explain" => Some("Explain the resolved write targets"),
                "--from-file" => None,
                _ => flag.tooltip.as_deref(),
            }
            .map(str::to_string);
        }
    }
    for child in spec.children.values_mut() {
        if let Some(flag) = child.flags.get_mut("--from-file") {
            // PathBuf's inferred parser does not supply a clap value hint.
            flag.value_type = Some(ValueType::Path);
            flag.suggestions = vec![SuggestionEntry::value("-")];
        }
    }
    spec
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
            } else if key == "repl.complete_on" {
                [
                    "flag",
                    "space",
                    "after_1_letter",
                    "after_2_letters",
                    "after_3_letters",
                    "after_4_letters",
                    "after_5_letters",
                ]
                .into_iter()
                .map(SuggestionEntry::value)
                .collect()
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

            let mut spec = crate::completion::ConfigKeySpec::new(view.config.display_key(key))
                .value_suggestions(value_suggestions);
            if let Some(doc) = entry.doc() {
                spec = spec.tooltip(doc);
            }
            spec
        })
        // Open namespaces (`extensions.*`, per-user display names, aliases)
        // have no schema entries; offer the keys this session actually has.
        .chain(
            view.config
                .values()
                .keys()
                .filter(|key| schema.entries().all(|(known, _)| known != key.as_str()))
                .map(|key| crate::completion::ConfigKeySpec::new(view.config.display_key(key))),
        )
        .collect()
}
