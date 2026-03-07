use crate::app::{
    CliCommandResult, ReplCommandOutput, emit_messages_for_ui, resolve_known_theme_name,
};
use crate::cli::{ThemeArgs, ThemeCommands, ThemeShowArgs, ThemeUseArgs};
use crate::rows::output::rows_to_output_result;
use crate::state::{SessionState, UiState};
use crate::theme_loader::{ThemeCatalog, ThemeSource};
use miette::Result;
use miette::miette;
use osp_config::{ConfigLayer, ResolvedConfig};
use osp_core::row::Row;
use osp_ui::theme::{DEFAULT_THEME_NAME, normalize_theme_name};

#[derive(Clone, Copy)]
pub(crate) struct ThemeCommandContext<'a> {
    pub(crate) config: &'a ResolvedConfig,
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

            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!("active theme set to: {selected}"));
            messages.info(
                "theme change is for the current process; persistent writes land with `config set`",
            );
            emit_messages_for_ui(
                context.config,
                context.ui,
                &messages,
                context.ui.message_verbosity,
            );
            Ok(CliCommandResult::exit(0))
        }
    }
}

pub(crate) fn run_theme_repl_command(
    session: &mut SessionState,
    themes: &ThemeCatalog,
    ui: &UiState,
    args: ThemeArgs,
) -> Result<ReplCommandOutput> {
    match args.command {
        ThemeCommands::List => Ok(ReplCommandOutput::Output {
            output: rows_to_output_result(theme_list_rows(themes, &ui.render_settings.theme_name)),
            format_hint: None,
        }),
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| ui.render_settings.theme_name.clone());
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(theme_show_rows(themes, &selected)?),
                format_hint: None,
            })
        }
        ThemeCommands::Use(ThemeUseArgs { name }) => {
            let selected = resolve_known_theme_name(&name, themes)?;
            session.config_overrides.set("theme.name", selected.clone());
            Ok(ReplCommandOutput::Text(format!(
                "active theme set to: {selected}\n"
            )))
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
