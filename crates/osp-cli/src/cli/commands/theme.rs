use crate::app::{
    CliCommandResult, ReplCommandOutput, emit_messages_with_verbosity, resolve_known_theme_name,
};
use crate::cli::{ThemeArgs, ThemeCommands, ThemeShowArgs, ThemeUseArgs};
use crate::rows::output::rows_to_output_result;
use crate::state::AppState;
use miette::Result;
use osp_core::row::Row;
use osp_ui::theme::{DEFAULT_THEME_NAME, available_theme_names, find_theme, normalize_theme_name};

pub(crate) fn run_theme_command(state: &mut AppState, args: ThemeArgs) -> Result<CliCommandResult> {
    match args.command {
        ThemeCommands::List => Ok(CliCommandResult::output(
            rows_to_output_result(theme_list_rows(&state.ui.render_settings.theme_name)),
            None,
        )),
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| state.ui.render_settings.theme_name.clone());
            Ok(CliCommandResult::output(
                rows_to_output_result(theme_show_rows(&selected)?),
                None,
            ))
        }
        ThemeCommands::Use(ThemeUseArgs { name }) => {
            let selected = resolve_known_theme_name(&name)?;
            state.ui.render_settings.theme_name = selected.clone();

            let mut messages = osp_ui::messages::MessageBuffer::default();
            messages.success(format!("active theme set to: {selected}"));
            messages.info(
                "theme change is for the current process; persistent writes land with `config set`",
            );
            emit_messages_with_verbosity(state, &messages, state.ui.message_verbosity);
            Ok(CliCommandResult::exit(0))
        }
    }
}

pub(crate) fn run_theme_repl_command(
    state: &mut AppState,
    args: ThemeArgs,
) -> Result<ReplCommandOutput> {
    match args.command {
        ThemeCommands::List => Ok(ReplCommandOutput::Output {
            output: rows_to_output_result(theme_list_rows(&state.ui.render_settings.theme_name)),
            format_hint: None,
        }),
        ThemeCommands::Show(ThemeShowArgs { name }) => {
            let selected = name.unwrap_or_else(|| state.ui.render_settings.theme_name.clone());
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(theme_show_rows(&selected)?),
                format_hint: None,
            })
        }
        ThemeCommands::Use(ThemeUseArgs { name }) => {
            let selected = resolve_known_theme_name(&name)?;
            state.ui.render_settings.theme_name = selected.clone();
            Ok(ReplCommandOutput::Text(format!(
                "active theme set to: {selected}\n"
            )))
        }
    }
}

fn theme_list_rows(active_theme: &str) -> Vec<Row> {
    let active = normalize_theme_name(active_theme);
    available_theme_names()
        .into_iter()
        .map(|name| {
            crate::row! {
                "name" => name.to_string(),
                "active" => name == active.as_str(),
                "default" => name == DEFAULT_THEME_NAME,
            }
        })
        .collect()
}

fn theme_show_rows(name: &str) -> Result<Vec<Row>> {
    let selected = resolve_known_theme_name(name)?;
    let theme =
        find_theme(&selected).ok_or_else(|| miette::miette!("theme missing: {selected}"))?;
    let palette = theme.palette;

    Ok(vec![crate::row! {
        "name" => theme.name.to_string(),
        "text" => palette.text.to_string(),
        "muted" => palette.muted.to_string(),
        "accent" => palette.accent.to_string(),
        "info" => palette.info.to_string(),
        "warning" => palette.warning.to_string(),
        "success" => palette.success.to_string(),
        "error" => palette.error.to_string(),
        "border" => palette.border.to_string(),
        "title" => palette.title.to_string(),
    }])
}
