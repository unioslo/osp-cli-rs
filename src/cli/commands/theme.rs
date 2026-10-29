//! `theme` builtin command implementation.
//!
//! Palette inspection and live theme selection share the normal renderer and
//! configuration writer. Temporary selection remains an in-process override.

use crate::app::{AppRuntime, AppSession, UiState};
use crate::app::{CommandOutput, resolve_known_theme_name};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::{ThemeArgs, ThemeCommands, ThemePreviewArgs, ThemeUseArgs};
use crate::completion::{CompletionNode, SuggestionEntry};
use crate::config::Scope;
use crate::core::output_model::{DisplayRule, OutputResult, RenderRecommendation};
use crate::core::row::Row;
use crate::ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};
use crate::ui::theme_catalog::{ThemeCatalog, ThemeSource};
use clap::Command;
use miette::Result;

pub(crate) fn run_theme_command(
    session: &mut AppSession,
    runtime: &AppRuntime,
    ui: &UiState,
    args: ThemeArgs,
) -> Result<CommandOutput> {
    match args.command {
        ThemeCommands::Reset => reset_theme(runtime, session),
        ThemeCommands::Preview(ThemePreviewArgs { name: None }) => {
            preview::run(runtime, session, ui)
        }
        ThemeCommands::List => {
            let mut effective = runtime.themes.clone();
            for entry in effective.entries.values_mut() {
                entry.theme = entry.theme.with_color_overrides(runtime.config.resolved());
            }
            let rows = theme_list_rows(&effective, &ui.render_settings.theme_name);
            let mut output = rows_to_output_result(rows);
            output.meta.display_columns = Some(
                [
                    "name", "source", "text", "accent", "info", "warning", "success", "error",
                    "border", "muted", "title", "status",
                ]
                .map(str::to_string)
                .to_vec(),
            );
            output.meta.display_column_labels = Some(
                [
                    "name", "type", "text", "accent", "info", "warning", "success", "error",
                    "border", "muted", "title", "status",
                ]
                .map(str::to_string)
                .to_vec(),
            );
            if ui.render_settings.requested_format() == crate::core::output::OutputFormat::Auto {
                output.meta.render_recommendation = Some(RenderRecommendation::Format(
                    crate::core::output::OutputFormat::Table,
                ));
            }
            add_palette_swatches(&mut output);
            let overridden = crate::core::style::ColorRole::ALL
                .into_iter()
                .filter(|role| runtime.config.resolved().get(role.key()).is_some())
                .collect::<Vec<_>>();
            if !overridden.is_empty() {
                for row in match &mut output.items {
                    crate::core::output_model::OutputItems::Rows(rows) => rows,
                    _ => unreachable!(),
                } {
                    row.insert(
                        "overridden".into(),
                        serde_json::json!(
                            overridden
                                .iter()
                                .map(|role| (role.name(), true))
                                .collect::<std::collections::BTreeMap<_, _>>()
                        ),
                    );
                }
                output
                    .meta
                    .display_rules
                    .extend(overridden.iter().map(|role| DisplayRule::SuffixWhenTrue {
                        field: role.name().into(),
                        when: format!("overridden.{}", role.name()),
                        suffix: "*".into(),
                    }));
                output
                    .meta
                    .display_notes
                    .push("* = global color.* override".into());
            }
            Ok(CommandOutput::output(output, None))
        }
        ThemeCommands::Preview(ThemePreviewArgs { name: Some(name) })
        | ThemeCommands::Use(ThemeUseArgs { name }) => {
            let selected = resolve_known_theme_name(&name, &runtime.themes)?;
            session.config_overrides.replace_scoped(
                "theme.name",
                selected.clone().into(),
                Scope::global(),
            );

            let mut result = CommandOutput::exit(0);
            result
                .messages
                .success(format!("active theme set to: {selected}"));
            result.messages.info(format!(
                "theme change is for the current process; persistent writes land with `{}`",
                ui.render_settings
                    .runtime
                    .command_scope
                    .command("config set")
            ));
            Ok(result)
        }
    }
}

fn default_theme_name(runtime: &AppRuntime) -> Result<String> {
    let config = crate::app::resolve_runtime_config(
        crate::app::RuntimeConfigRequest::new(None, None)
            .with_runtime_load(crate::config::RuntimeLoadOptions::defaults_only())
            .with_product_defaults(runtime.product_defaults().clone()),
    )?;
    resolve_known_theme_name(
        config
            .get_string("theme.name")
            .unwrap_or(DEFAULT_THEME_NAME),
        &runtime.themes,
    )
}

fn reset_theme(runtime: &AppRuntime, session: &mut AppSession) -> Result<CommandOutput> {
    let default = default_theme_name(runtime)?;
    let name = &runtime
        .themes
        .resolve(&default)
        .ok_or_else(|| miette::miette!("default theme is unavailable: {default}"))?
        .theme
        .name;
    let keys = std::iter::once("theme.name".to_string())
        .chain(crate::core::style::ColorRole::ALL.map(|role| role.key().to_string()))
        .collect::<Vec<_>>();
    let mut changed = super::config::clear_theme_values(runtime, session, &keys)?;
    changed.sort();
    Ok(CommandOutput::text(if changed.is_empty() {
        "Nothing needed resetting.".into()
    } else {
        format!("Reset theme to {name}; cleared {}.", changed.join(", "))
    }))
}

pub(crate) fn theme_command() -> Command {
    crate::cli::builtin_command("theme")
}

pub(crate) fn augment_theme_completion(spec: &mut CompletionNode, themes: &ThemeCatalog) {
    let suggestions = themes
        .ids()
        .into_iter()
        .map(SuggestionEntry::value)
        .collect::<Vec<_>>();
    for child in spec.children.values_mut() {
        for arg in &mut child.args {
            arg.name = Some("name".to_string());
            arg.tooltip = Some("Theme name".to_string());
            arg.suggestions = suggestions.clone();
        }
    }
}

fn add_palette_swatches(output: &mut OutputResult) {
    output
        .meta
        .display_rules
        .extend(
            crate::core::style::ColorRole::ALL
                .into_iter()
                .map(|role| DisplayRule::Swatch {
                    field: role.name().into(),
                }),
        );
    output
        .meta
        .display_rules
        .extend(
            ["selection", "link", "bg", "bg_alt"].map(|field| DisplayRule::ValueStyle {
                field: field.into(),
                bold: false,
            }),
        );
}

fn theme_list_rows(themes: &ThemeCatalog, active_theme: &str) -> Vec<Row> {
    let active = normalize_theme_name(active_theme);
    themes
        .entries
        .values()
        .map(|entry| {
            let is_active = entry.theme.id == active.as_str();
            let is_default = entry.theme.id == DEFAULT_THEME_NAME;
            let status = match (is_active, is_default) {
                (true, true) => "active, default",
                (true, false) => "active",
                (false, true) => "default",
                (false, false) => "",
            };
            let mut row = theme_palette_row(entry);
            row.extend(crate::row! {
                "active" => is_active,
                "default" => is_default,
                "status" => status,
            });
            row
        })
        .collect()
}

fn theme_palette_row(entry: &crate::ui::theme_catalog::ThemeEntry) -> Row {
    let theme = &entry.theme;
    let palette = &theme.palette;
    let origin = entry
        .origin
        .as_ref()
        .map(|path| serde_json::Value::from(path.to_string_lossy().to_string()))
        .unwrap_or(serde_json::Value::Null);
    let bg = palette
        .bg
        .clone()
        .map(serde_json::Value::from)
        .unwrap_or(serde_json::Value::Null);
    let bg_alt = palette
        .bg_alt
        .clone()
        .map(serde_json::Value::from)
        .unwrap_or(serde_json::Value::Null);

    crate::row! {
        "id" => theme.id.to_string(),
        "name" => theme.name.to_string(),
        "base" => theme
            .base
            .as_deref()
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null),
        "source" => match entry.source {
            ThemeSource::Builtin => "builtin",
            ThemeSource::Custom => "custom",
        },
        "origin" => origin,
        "text" => palette.text.to_string(),
        "muted" => palette.muted.to_string(),
        "accent" => palette.accent.to_string(),
        "info" => palette.info.to_string(),
        "warning" => palette.warning.to_string(),
        "success" => palette.success.to_string(),
        "error" => palette.error.strip_prefix("bold ").unwrap_or(&palette.error),
        "border" => palette.border.to_string(),
        "title" => palette.title.to_string(),
        "selection" => palette.selection.to_string(),
        "link" => palette.link.to_string(),
        "bg" => bg,
        "bg_alt" => bg_alt,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::{
        augment_theme_completion, run_theme_command, theme_command, theme_list_rows,
        theme_palette_row,
    };
    use crate::app::CommandOutput;
    use crate::app::UiState;
    use crate::cli::{ThemeArgs, ThemeCommands};
    use crate::core::output::OutputFormat;
    use crate::core::row::Row;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme::find_builtin_theme;
    use crate::ui::theme_catalog::{ThemeCatalog, ThemeEntry, ThemeSource};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn builtin_theme_catalog() -> ThemeCatalog {
        let builtin = find_builtin_theme("nord").expect("builtin theme").clone();
        let entry = ThemeEntry {
            theme: builtin,
            source: ThemeSource::Builtin,
            origin: None,
        };

        ThemeCatalog {
            entries: BTreeMap::from([("nord".to_string(), entry)]),
            issues: Vec::new(),
        }
    }

    fn custom_theme_catalog() -> ThemeCatalog {
        let builtin = find_builtin_theme("nord").expect("builtin theme").clone();
        let custom = ThemeEntry {
            theme: builtin,
            source: ThemeSource::Custom,
            origin: Some(PathBuf::from("/tmp/themes/nord.toml")),
        };

        ThemeCatalog {
            entries: BTreeMap::from([("nord".to_string(), custom)]),
            issues: Vec::new(),
        }
    }

    fn test_ui_state() -> UiState {
        UiState::new(
            RenderSettings::test_plain(OutputFormat::Table),
            MessageLevel::Info,
            0,
        )
    }

    fn extract_output_rows(result: CommandOutput) -> Option<Vec<Row>> {
        match result.body {
            crate::app::CommandBody::Output(output) => output.into_rows(),
            _ => None,
        }
    }

    #[test]
    fn theme_rows_and_commands_resolve_builtin_and_custom_sources_unit() {
        let rows = theme_list_rows(&custom_theme_catalog(), "nord");
        let row = &rows[0];

        assert_eq!(
            row.get("source").and_then(|value| value.as_str()),
            Some("custom")
        );
        assert_eq!(
            row.get("origin").and_then(|value| value.as_str()),
            Some("/tmp/themes/nord.toml")
        );

        let ui = test_ui_state();
        let themes = builtin_theme_catalog();
        let mut defaults = crate::config::ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = crate::config::ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(crate::config::ResolveOptions::default())
            .unwrap();
        let mut builder = crate::app::AppStateBuilder::new(
            crate::app::RuntimeContext::new(None, crate::app::TerminalKind::Cli, None),
            config,
            ui.clone(),
        );
        builder.themes = Some(themes);
        let state = builder.build();
        let mut session = state.session;

        let result = run_theme_command(
            &mut session,
            &state.runtime,
            &ui,
            ThemeArgs {
                command: ThemeCommands::List,
            },
        )
        .expect("theme list should succeed");

        let rows = extract_output_rows(result).expect("rows");
        let row = &rows[0];
        assert_eq!(
            row.get("source").and_then(|value| value.as_str()),
            Some("builtin")
        );

        let row = theme_palette_row(&custom_theme_catalog().entries["nord"]);

        assert_eq!(
            row.get("source").and_then(|value| value.as_str()),
            Some("custom")
        );
        assert_eq!(
            row.get("origin").and_then(|value| value.as_str()),
            Some("/tmp/themes/nord.toml")
        );
    }

    #[test]
    fn theme_output_helpers_and_command_def_expose_runtime_choices_unit() {
        assert!(extract_output_rows(CommandOutput::text("hello")).is_none());

        let mut def = crate::completion::CompletionNode::from_clap(&theme_command());
        augment_theme_completion(&mut def, &builtin_theme_catalog());
        assert_eq!(def.name, "theme");
        let use_theme = def
            .children
            .values()
            .find(|subcommand| subcommand.name == "use")
            .expect("use subcommand");
        assert_eq!(use_theme.args[0].suggestions[0].value, "nord");
    }
}

mod preview;
