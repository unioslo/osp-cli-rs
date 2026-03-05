use miette::{IntoDiagnostic, Result};

use crate::app::{
    CliCommandResult, ReplCommandOutput, ensure_builtin_visible, CMD_CONFIG, CMD_PLUGINS,
    CMD_THEME,
};
use crate::cli::{ConfigArgs, ConfigCommands, DoctorArgs, DoctorCommands, PluginsArgs, PluginsCommands};
use crate::rows::output::rows_to_output_result;
use crate::state::AppState;
use osp_core::output::OutputFormat;
use osp_core::row::Row;
use osp_ui::render_output;

use super::{config as config_cmd, plugins as plugins_cmd};

pub(crate) fn run_doctor_command(
    state: &mut AppState,
    args: DoctorArgs,
) -> Result<CliCommandResult> {
    let command = args.command.unwrap_or(DoctorCommands::All);
    match command {
        DoctorCommands::Config => {
            ensure_builtin_visible(state, CMD_CONFIG)?;
            config_cmd::run_config_command(state, ConfigArgs {
                command: ConfigCommands::Diagnostics,
            })
        }
        DoctorCommands::Plugins => {
            ensure_builtin_visible(state, CMD_PLUGINS)?;
            plugins_cmd::run_plugins_command(
                state,
                PluginsArgs {
                    command: PluginsCommands::Doctor,
                },
            )
        }
        DoctorCommands::Theme => {
            ensure_builtin_visible(state, CMD_THEME)?;
            Ok(CliCommandResult::output(
                rows_to_output_result(theme_doctor_rows(state)),
                None,
            ))
        }
        DoctorCommands::All => run_doctor_all(state),
    }
}

pub(crate) fn run_doctor_repl_command(
    state: &mut AppState,
    args: DoctorArgs,
    verbosity: osp_ui::messages::MessageLevel,
) -> Result<ReplCommandOutput> {
    let command = args.command.unwrap_or(DoctorCommands::All);
    match command {
        DoctorCommands::Config => {
            ensure_builtin_visible(state, CMD_CONFIG)?;
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(config_cmd::config_diagnostics_rows(state)),
                format_hint: None,
            })
        }
        DoctorCommands::Plugins => {
            ensure_builtin_visible(state, CMD_PLUGINS)?;
            plugins_cmd::run_plugins_repl_command(
                state,
                PluginsArgs {
                    command: PluginsCommands::Doctor,
                },
                verbosity,
            )
        }
        DoctorCommands::Theme => {
            ensure_builtin_visible(state, CMD_THEME)?;
            Ok(ReplCommandOutput::Output {
                output: rows_to_output_result(theme_doctor_rows(state)),
                format_hint: None,
            })
        }
        DoctorCommands::All => {
            let result = run_doctor_all(state)?;
            Ok(result
                .output
                .unwrap_or_else(|| ReplCommandOutput::Text(String::new())))
        }
    }
}

fn run_doctor_all(state: &mut AppState) -> Result<CliCommandResult> {
    let mut sections: Vec<(&str, Vec<Row>)> = Vec::new();

    if state.auth.is_builtin_visible(CMD_CONFIG) {
        sections.push(("config", config_cmd::config_diagnostics_rows(state)));
    }
    if state.auth.is_builtin_visible(CMD_PLUGINS) {
        let report = state
            .clients
            .plugins
            .doctor()
            .map_err(|err| miette::miette!("{err:#}"))?;
        sections.push(("plugins", plugins_cmd::doctor_rows(&report)));
    }
    if state.auth.is_builtin_visible(CMD_THEME) {
        sections.push(("theme", theme_doctor_rows(state)));
    }

    if matches!(state.ui.render_settings.format, OutputFormat::Json) {
        let mut root = serde_json::Map::new();
        for (name, rows) in sections {
            let values = rows
                .into_iter()
                .map(serde_json::Value::Object)
                .collect::<Vec<_>>();
            root.insert(name.to_string(), serde_json::Value::Array(values));
        }
        let payload = serde_json::Value::Object(root);
        return Ok(CliCommandResult::text(format!(
            "{}\n",
            serde_json::to_string_pretty(&payload).into_diagnostic()?
        )));
    }

    let mut rendered = String::new();
    for (name, rows) in sections {
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        rendered.push_str(&format!("{name}:\n"));
        let output = rows_to_output_result(rows);
        rendered.push_str(&render_output(&output, &state.ui.render_settings));
    }

    Ok(CliCommandResult::text(rendered))
}

fn theme_doctor_rows(state: &AppState) -> Vec<Row> {
    let issues = &state.themes.issues;
    if issues.is_empty() {
        return vec![crate::row! {
            "status" => "ok",
            "issue_count" => 0,
        }];
    }

    let count = issues.len() as i64;
    issues
        .iter()
        .map(|issue| {
            crate::row! {
                "status" => "issue",
                "issue_count" => count,
                "path" => issue.path.display().to_string(),
                "message" => issue.message.clone(),
            }
        })
        .collect()
}
