use miette::{IntoDiagnostic, Result};

use crate::app::{
    CMD_CONFIG, CMD_PLUGINS, CMD_THEME, CliCommandResult, ensure_builtin_visible_for,
};
use crate::cli::{DoctorArgs, DoctorCommands, PluginsArgs, PluginsCommands};
use crate::rows::output::rows_to_output_result;
use crate::state::{AuthState, LastFailure, UiState};
use crate::theme_loader::ThemeCatalog;
use osp_core::output::OutputFormat;
use osp_core::row::Row;
use osp_ui::render_output;

use super::{config as config_cmd, plugins as plugins_cmd};

#[derive(Clone, Copy)]
pub(crate) struct DoctorCommandContext<'a> {
    pub(crate) config: config_cmd::ConfigReadContext<'a>,
    pub(crate) plugins: plugins_cmd::PluginsCommandContext<'a>,
    pub(crate) ui: &'a UiState,
    pub(crate) auth: &'a AuthState,
    pub(crate) themes: &'a ThemeCatalog,
    pub(crate) last_failure: Option<&'a LastFailure>,
}

pub(crate) fn run_doctor_command(
    context: DoctorCommandContext<'_>,
    args: DoctorArgs,
) -> Result<CliCommandResult> {
    let command = args.command.unwrap_or(DoctorCommands::All);
    match command {
        DoctorCommands::Config => {
            ensure_builtin_visible_for(context.auth, CMD_CONFIG)?;
            Ok(CliCommandResult::output(
                rows_to_output_result(config_cmd::config_diagnostics_rows(context.config)),
                None,
            ))
        }
        DoctorCommands::Plugins => {
            ensure_builtin_visible_for(context.auth, CMD_PLUGINS)?;
            plugins_cmd::run_plugins_command(
                context.plugins,
                PluginsArgs {
                    command: PluginsCommands::Doctor,
                },
            )
        }
        DoctorCommands::Last => Ok(CliCommandResult::text(render_last_failure(
            context.ui,
            context.last_failure,
        ))),
        DoctorCommands::Theme => {
            ensure_builtin_visible_for(context.auth, CMD_THEME)?;
            Ok(CliCommandResult::output(
                rows_to_output_result(theme_doctor_rows(context.themes)),
                None,
            ))
        }
        DoctorCommands::All => run_doctor_all(context),
    }
}

fn run_doctor_all(context: DoctorCommandContext<'_>) -> Result<CliCommandResult> {
    let mut sections: Vec<(&str, Vec<Row>)> = Vec::new();

    if context.auth.is_builtin_visible(CMD_CONFIG) {
        sections.push((
            "config",
            config_cmd::config_diagnostics_rows(context.config),
        ));
    }
    if context.auth.is_builtin_visible(CMD_PLUGINS) {
        let report = context
            .plugins
            .plugin_manager
            .doctor()
            .map_err(|err| miette::miette!("{err:#}"))?;
        sections.push(("plugins", plugins_cmd::doctor_rows(&report)));
    }
    if context.auth.is_builtin_visible(CMD_THEME) {
        sections.push(("theme", theme_doctor_rows(context.themes)));
    }

    if matches!(context.ui.render_settings.format, OutputFormat::Json) {
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
        rendered.push_str(&render_output(&output, &context.ui.render_settings));
    }

    Ok(CliCommandResult::text(rendered))
}

fn theme_doctor_rows(themes: &ThemeCatalog) -> Vec<Row> {
    let issues = &themes.issues;
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

fn render_last_failure(ui: &UiState, last_failure: Option<&LastFailure>) -> String {
    let Some(last) = last_failure else {
        return "No recorded REPL failure in this session.\n".to_string();
    };

    if matches!(ui.render_settings.format, OutputFormat::Json) {
        let payload = serde_json::json!({
            "status": "error",
            "command": last.command_line,
            "summary": last.summary,
            "detail": last.detail,
        });
        return format!(
            "{}\n",
            serde_json::to_string_pretty(&payload).expect("last failure json should serialize")
        );
    }

    let mut out = String::new();
    out.push_str("Last REPL failure:\n");
    out.push_str(&format!("  Command: {}\n", last.command_line));
    out.push_str(&format!("  Error:   {}\n", last.summary));
    if ui.debug_verbosity > 0 && last.detail != last.summary {
        out.push('\n');
        out.push_str("Detail:\n");
        for line in last.detail.lines() {
            out.push_str("  ");
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}
