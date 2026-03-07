use crate::app::{CliCommandResult, resolve_known_theme_name};
use crate::cli::{ThemeArgs, ThemeCommands, ThemeShowArgs, ThemeUseArgs};
use crate::rows::output::rows_to_output_result;
use crate::state::UiState;
use crate::theme_loader::{ThemeCatalog, ThemeSource};
use miette::Result;
use miette::miette;
use osp_config::ConfigLayer;
use osp_core::row::Row;
use osp_ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};

#[derive(Clone, Copy)]
pub(crate) struct ThemeCommandContext<'a> {
    pub(crate) ui: &'a UiState,
    pub(crate) themes: &'a ThemeCatalog,
}

pub(crate) fn run_theme_command(
    session_overrides: &mut ConfigLayer,
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
            session_overrides.set("theme.name", selected.clone());

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
    use super::{ThemeCommandContext, run_theme_command, theme_list_rows, theme_show_rows};
    use crate::app::CliCommandResult;
    use crate::app::ReplCommandOutput;
    use crate::cli::{ThemeArgs, ThemeCommands, ThemeShowArgs};
    use crate::state::UiState;
    use crate::theme_loader::{ThemeCatalog, ThemeEntry, ThemeSource};
    use osp_config::ConfigLayer;
    use osp_core::output::OutputFormat;
    use osp_core::row::Row;
    use osp_ui::RenderSettings;
    use osp_ui::messages::MessageLevel;
    use osp_ui::theme::find_builtin_theme;
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
        UiState {
            render_settings: RenderSettings::test_plain(OutputFormat::Table),
            message_verbosity: MessageLevel::Info,
            debug_verbosity: 0,
        }
    }

    fn extract_output_rows(result: CliCommandResult) -> Option<Vec<Row>> {
        let output = match result.output? {
            ReplCommandOutput::Output { output, .. } => output,
            ReplCommandOutput::Document(_) | ReplCommandOutput::Text(_) => return None,
        };
        output.into_rows()
    }

    #[test]
    fn theme_list_rows_marks_custom_theme_source_unit() {
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
    }

    #[test]
    fn run_theme_command_list_emits_builtin_theme_rows_unit() {
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
    }

    #[test]
    fn theme_show_rows_marks_custom_theme_source_unit() {
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
    }

    #[test]
    fn run_theme_command_show_uses_active_builtin_theme_when_name_is_omitted_unit() {
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
    fn extract_output_rows_returns_none_for_text_results_unit() {
        assert!(extract_output_rows(CliCommandResult::text("hello")).is_none());
    }
}
