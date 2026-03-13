use crate::app::UiState;
use crate::app::{CliCommandResult, resolve_known_theme_name};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::{ThemeArgs, ThemeCommands, ThemeShowArgs, ThemeUseArgs};
use crate::config::ConfigLayer;
use crate::core::command_def::{ArgDef, CommandDef, ValueChoice};
use crate::core::row::Row;
use crate::ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};
use crate::ui::theme_loader::{ThemeCatalog, ThemeSource};
use miette::Result;
use miette::miette;

#[derive(Clone, Copy)]
pub(crate) struct ThemeCommandContext<'a> {
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
}

pub(crate) fn run_theme_command(
    config_overrides: &mut ConfigLayer,
    context: ThemeCommandContext<'_>,
    args: ThemeArgs,
) -> Result<CliCommandResult> {
    match args.command {
        ThemeCommands::List => Ok(CliCommandResult::output(
            rows_to_output_result(theme_list_rows(
                context.themes,
                &context.ui.render_settings.theme_name,
            )),
            None,
        )),
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| context.ui.render_settings.theme_name.clone());
            Ok(CliCommandResult::output(
                rows_to_output_result(theme_show_rows(context.themes, &selected)?),
                None,
            ))
        }
        ThemeCommands::Use(ThemeUseArgs { name }) => {
            let selected = resolve_known_theme_name(&name, context.themes)?;
            config_overrides.set("theme.name", selected.clone());

            let mut result = CliCommandResult::exit(0);
            result
                .messages
                .success(format!("active theme set to: {selected}"));
            result.messages.info(
                "theme change is for the current process; persistent writes land with `config set`",
            );
            Ok(result)
        }
    }
}

pub(crate) fn theme_command_def(themes: &ThemeCatalog, sort_key: impl Into<String>) -> CommandDef {
    let theme_choices = themes
        .ids()
        .into_iter()
        .map(ValueChoice::new)
        .collect::<Vec<_>>();

    CommandDef::new("theme")
        .about("Inspect and apply themes")
        .sort(sort_key)
        .subcommands([
            CommandDef::new("list")
                .about("List available themes")
                .sort("10"),
            CommandDef::new("show")
                .about("Show a theme definition")
                .sort("11")
                .arg(
                    ArgDef::new("name")
                        .value_name("name")
                        .help("Theme name")
                        .choices(theme_choices.clone()),
                ),
            CommandDef::new("use")
                .about("Set active theme")
                .sort("12")
                .arg(
                    ArgDef::new("name")
                        .value_name("name")
                        .help("Theme name")
                        .required()
                        .choices(theme_choices),
                ),
        ])
}

fn theme_list_rows(themes: &ThemeCatalog, active_theme: &str) -> Vec<Row> {
    let active = normalize_theme_name(active_theme);
    themes
        .entries
        .values()
        .map(|entry| {
            let origin = entry
                .origin
                .as_ref()
                .map(|path| serde_json::Value::from(path.to_string_lossy().to_string()))
                .unwrap_or(serde_json::Value::Null);
            crate::row! {
                "id" => entry.theme.id.to_string(),
                "name" => entry.theme.name.to_string(),
                "source" => match entry.source {
                    ThemeSource::Builtin => "builtin",
                    ThemeSource::Custom => "custom",
                },
                "origin" => origin,
                "active" => entry.theme.id == active.as_str(),
                "default" => entry.theme.id == DEFAULT_THEME_NAME,
            }
        })
        .collect()
}

fn theme_show_rows(themes: &ThemeCatalog, name: &str) -> Result<Vec<Row>> {
    let selected = resolve_known_theme_name(name, themes)?;
    let entry = themes
        .entries
        .get(&selected)
        .ok_or_else(|| miette!("theme missing: {selected}"))?;
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

    Ok(vec![crate::row! {
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
        "error" => palette.error.to_string(),
        "border" => palette.border.to_string(),
        "title" => palette.title.to_string(),
        "selection" => palette.selection.to_string(),
        "link" => palette.link.to_string(),
        "bg" => bg,
        "bg_alt" => bg_alt,
    }])
}

#[cfg(test)]
mod tests {
    use super::{
        ThemeCommandContext, run_theme_command, theme_command_def, theme_list_rows, theme_show_rows,
    };
    use crate::app::CliCommandResult;
    use crate::app::ReplCommandOutput;
    use crate::app::UiState;
    use crate::cli::{ThemeArgs, ThemeCommands, ThemeShowArgs};
    use crate::config::ConfigLayer;
    use crate::core::output::OutputFormat;
    use crate::core::row::Row;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme::find_builtin_theme;
    use crate::ui::theme_loader::{ThemeCatalog, ThemeEntry, ThemeSource};
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

    fn extract_output_rows(result: CliCommandResult) -> Option<Vec<Row>> {
        let output = match result.output? {
            ReplCommandOutput::Output { output, .. } => output,
            ReplCommandOutput::Guide(_)
            | ReplCommandOutput::Document { .. }
            | ReplCommandOutput::Text(_) => return None,
        };
        output.into_rows()
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
        let mut overrides = ConfigLayer::default();

        let result = run_theme_command(
            &mut overrides,
            ThemeCommandContext {
                ui: &ui,
                themes: &themes,
            },
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

        let rows = theme_show_rows(&custom_theme_catalog(), "nord").expect("theme should resolve");
        let row = &rows[0];

        assert_eq!(
            row.get("source").and_then(|value| value.as_str()),
            Some("custom")
        );
        assert_eq!(
            row.get("origin").and_then(|value| value.as_str()),
            Some("/tmp/themes/nord.toml")
        );

        let mut ui = test_ui_state();
        ui.render_settings.theme_name = "nord".to_string();
        let themes = builtin_theme_catalog();
        let mut overrides = ConfigLayer::default();

        let result = run_theme_command(
            &mut overrides,
            ThemeCommandContext {
                ui: &ui,
                themes: &themes,
            },
            ThemeArgs {
                command: ThemeCommands::Show(ThemeShowArgs { name: None }),
            },
        )
        .expect("theme show should succeed");

        let rows = extract_output_rows(result).expect("rows");
        let row = &rows[0];
        assert_eq!(row.get("id").and_then(|value| value.as_str()), Some("nord"));
        assert_eq!(
            row.get("source").and_then(|value| value.as_str()),
            Some("builtin")
        );
    }

    #[test]
    fn theme_output_helpers_and_command_def_expose_runtime_choices_unit() {
        assert!(extract_output_rows(CliCommandResult::text("hello")).is_none());

        let def = theme_command_def(&builtin_theme_catalog(), "20");
        assert_eq!(def.name, "theme");
        assert_eq!(def.sort_key.as_deref(), Some("20"));
        let show = def
            .subcommands
            .iter()
            .find(|subcommand| subcommand.name == "show")
            .expect("show subcommand");
        assert_eq!(show.args[0].choices[0].value, "nord");
    }
}
