use miette::Result;

use crate::app::{AppClients, AppRuntime, AppSession, AuthState, LastFailure, UiState};
use crate::app::{
    CMD_CONFIG, CMD_PLUGINS, CMD_THEME, CliCommandResult, ensure_builtin_visible_for,
};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::{DoctorArgs, DoctorCommands, PluginsArgs, PluginsCommands};
use crate::core::command_def::CommandDef;
use crate::core::output::OutputFormat;
use crate::core::output_model::OutputResult;
use crate::core::row::Row;
use crate::guide::{GuideSection, GuideSectionKind, GuideView};
use crate::ui::theme_catalog::ThemeCatalog;
use serde_json::{Map, Value};

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

impl<'a> DoctorCommandContext<'a> {
    pub(crate) fn from_parts(
        runtime: &'a AppRuntime,
        session: &'a AppSession,
        clients: &'a AppClients,
        ui: &'a UiState,
    ) -> Self {
        Self {
            config: config_cmd::ConfigReadContext::from_parts(runtime, session, ui),
            plugins: plugins_cmd::PluginsCommandContext::from_parts(runtime, clients),
            ui,
            auth: &runtime.auth,
            themes: &runtime.themes,
            last_failure: session.last_failure.as_ref(),
        }
    }
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
        DoctorCommands::Last => Ok(render_last_failure_document(
            context.ui,
            context.last_failure,
        )),
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

pub(crate) fn doctor_command_def(sort_key: impl Into<String>) -> CommandDef {
    CommandDef::new("doctor")
        .about("Run diagnostics checks")
        .sort(sort_key)
        .subcommands([
            CommandDef::new("all")
                .about("Run all visible diagnostics")
                .sort("10"),
            CommandDef::new(CMD_CONFIG)
                .about("Show config diagnostics")
                .sort("11"),
            CommandDef::new("last")
                .about("Show the last REPL failure")
                .sort("12"),
            CommandDef::new(CMD_PLUGINS)
                .about("Run plugin diagnostics")
                .sort("13"),
            CommandDef::new(CMD_THEME)
                .about("Show theme diagnostics")
                .sort("14"),
        ])
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
        let report = context.plugins.plugin_manager.doctor();
        sections.push(("plugins", plugins_cmd::doctor_rows(&report)));
    }
    if context.auth.is_builtin_visible(CMD_THEME) {
        sections.push(("theme", theme_doctor_rows(context.themes)));
    }

    if matches!(context.ui.render_settings.format, OutputFormat::Json) {
        return Ok(CliCommandResult::json(doctor_report_json_value(&sections)));
    }

    if sections.is_empty() {
        return Ok(CliCommandResult::text(String::new()));
    }

    Ok(CliCommandResult::guide_with_output(
        doctor_report_guide(&sections),
        doctor_report_output(&sections),
        None,
    ))
}

fn doctor_report_guide(sections: &[(&str, Vec<Row>)]) -> GuideView {
    GuideView {
        sections: sections
            .iter()
            .map(|(name, rows)| {
                GuideSection::new(*name, GuideSectionKind::Custom)
                    .data(doctor_section_view_value(rows))
            })
            .collect(),
        ..GuideView::default()
    }
}

fn doctor_report_output(sections: &[(&str, Vec<Row>)]) -> OutputResult {
    OutputResult::from_rows(vec![doctor_report_row(sections, doctor_section_view_value)])
}

fn doctor_report_json_value(sections: &[(&str, Vec<Row>)]) -> Value {
    doctor_report_value(sections, doctor_section_json_value)
}

fn doctor_report_value(
    sections: &[(&str, Vec<Row>)],
    value_for_rows: impl Fn(&[Row]) -> Value,
) -> Value {
    Value::Object(doctor_report_row(sections, value_for_rows))
}

fn doctor_report_row(
    sections: &[(&str, Vec<Row>)],
    value_for_rows: impl Fn(&[Row]) -> Value,
) -> Map<String, Value> {
    sections
        .iter()
        .map(|(name, rows)| ((*name).to_string(), value_for_rows(rows)))
        .collect::<Map<String, Value>>()
}

fn doctor_section_view_value(rows: &[Row]) -> Value {
    match rows {
        [] => Value::Array(Vec::new()),
        [row] => Value::Object(row.clone()),
        _ => Value::Array(rows.iter().cloned().map(Value::Object).collect()),
    }
}

fn doctor_section_json_value(rows: &[Row]) -> Value {
    Value::Array(rows.iter().cloned().map(Value::Object).collect())
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

fn render_last_failure_document(
    ui: &UiState,
    last_failure: Option<&LastFailure>,
) -> crate::app::CliCommandResult {
    let Some(last) = last_failure else {
        return CliCommandResult::text("No recorded REPL failure in this session.\n");
    };

    if matches!(ui.render_settings.format, OutputFormat::Json) {
        let payload = serde_json::json!({
            "status": "error",
            "command": last.command_line,
            "summary": last.summary,
            "detail": last.detail,
        });
        return CliCommandResult::json(payload);
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
    CliCommandResult::text(out)
}

#[cfg(test)]
mod tests {
    use super::{
        DoctorCommandContext, doctor_command_def, render_last_failure_document, run_doctor_command,
        theme_doctor_rows,
    };
    use crate::app::ReplCommandOutput;
    use crate::app::{AuthState, LastFailure, RuntimeContext, TerminalKind, UiState};
    use crate::cli::commands::{config as config_cmd, plugins as plugins_cmd};
    use crate::cli::{DoctorArgs, DoctorCommands};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions, RuntimeLoadOptions};
    use crate::core::output::OutputFormat;
    use crate::plugin::PluginManager;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme_catalog::{ThemeCatalog, ThemeLoadIssue};
    use serde_json::Value;
    use std::path::PathBuf;

    fn ui_state(format: OutputFormat, debug_verbosity: u8) -> UiState {
        UiState::new(
            RenderSettings::test_plain(format),
            MessageLevel::Success,
            debug_verbosity,
        )
    }

    fn doctor_context(
        format: OutputFormat,
        builtins: Option<&str>,
    ) -> DoctorCommandContext<'static> {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        if let Some(builtins) = builtins {
            defaults.set("auth.visible.builtins", builtins);
        }
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let resolved = Box::leak(Box::new(
            resolver
                .resolve(ResolveOptions::default().with_terminal("cli"))
                .expect("test config should resolve"),
        ));
        let ui = Box::leak(Box::new(ui_state(format, 0)));
        let themes = Box::leak(Box::new(ThemeCatalog::default()));
        let auth = Box::leak(Box::new(AuthState::from_resolved(resolved)));
        let config_overrides = Box::leak(Box::new(ConfigLayer::default()));
        let product_defaults = Box::leak(Box::new(ConfigLayer::default()));
        let plugin_manager = Box::leak(Box::new(PluginManager::new(Vec::new())));
        let context = Box::leak(Box::new(RuntimeContext::new(None, TerminalKind::Cli, None)));

        DoctorCommandContext {
            config: config_cmd::ConfigReadContext {
                context,
                config: resolved,
                ui,
                themes,
                config_overrides,
                product_defaults,
                runtime_load: RuntimeLoadOptions::default(),
            },
            plugins: plugins_cmd::PluginsCommandContext {
                context,
                config: resolved,
                config_state: None,
                auth,
                clients: None,
                plugin_manager,
                product_defaults,
                runtime_load: RuntimeLoadOptions::default(),
            },
            ui,
            auth,
            themes,
            last_failure: None,
        }
    }

    #[test]
    fn doctor_last_rendering_covers_empty_text_debug_and_json_modes_unit() {
        let empty = render_last_failure_document(&ui_state(OutputFormat::Table, 0), None);
        let Some(ReplCommandOutput::Text(empty_text)) = empty.output else {
            panic!("expected text output");
        };
        assert!(empty_text.contains("No recorded REPL failure"));

        let failure = LastFailure {
            command_line: "ldap user nope".to_string(),
            summary: "request failed".to_string(),
            detail: "request failed\nbackend said no".to_string(),
        };

        let verbose =
            render_last_failure_document(&ui_state(OutputFormat::Table, 1), Some(&failure));
        let Some(ReplCommandOutput::Text(verbose_text)) = verbose.output else {
            panic!("expected text output");
        };
        assert!(verbose_text.contains("Command: ldap user nope"));
        assert!(verbose_text.contains("Error:   request failed"));
        assert!(verbose_text.contains("Detail:"));
        assert!(verbose_text.contains("backend said no"));

        let compact =
            render_last_failure_document(&ui_state(OutputFormat::Table, 0), Some(&failure));
        let Some(ReplCommandOutput::Text(compact_text)) = compact.output else {
            panic!("expected text output");
        };
        assert!(compact_text.contains("Error:   request failed"));
        assert!(!compact_text.contains("Detail:"));

        let json_failure = LastFailure {
            command_line: "plugins refresh".to_string(),
            summary: "plugin failed".to_string(),
            detail: "plugin failed".to_string(),
        };
        let json_result =
            render_last_failure_document(&ui_state(OutputFormat::Json, 0), Some(&json_failure));
        let Some(ReplCommandOutput::Json(json)) = json_result.output else {
            panic!("expected json output");
        };
        assert_eq!(json["status"], Value::String("error".to_string()));
        assert_eq!(
            json["command"],
            Value::String("plugins refresh".to_string())
        );

        let empty = theme_doctor_rows(&ThemeCatalog::default());
        assert_eq!(
            empty,
            vec![crate::row! { "status" => "ok", "issue_count" => 0 }]
        );

        let catalog = ThemeCatalog {
            entries: Default::default(),
            issues: vec![ThemeLoadIssue {
                path: PathBuf::from("/tmp/theme.toml"),
                message: "broken palette".to_string(),
            }],
        };

        let rows = theme_doctor_rows(&catalog);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["status"], Value::String("issue".to_string()));
        assert_eq!(rows[0]["issue_count"], Value::from(1));
        assert_eq!(
            rows[0]["message"],
            Value::String("broken palette".to_string())
        );
    }

    #[test]
    fn doctor_commands_respect_visibility_output_shapes_and_subcommand_defs_unit() {
        let visible = run_doctor_command(
            doctor_context(OutputFormat::Json, Some("config,plugins,theme")),
            DoctorArgs {
                command: Some(DoctorCommands::All),
            },
        )
        .expect("doctor all should succeed");
        let Some(ReplCommandOutput::Json(json)) = visible.output else {
            panic!("expected json output");
        };
        assert!(json.get("config").is_some());
        assert!(json.get("plugins").is_some());
        assert!(json.get("theme").is_some());

        let filtered = run_doctor_command(
            doctor_context(OutputFormat::Json, Some("theme")),
            DoctorArgs {
                command: Some(DoctorCommands::All),
            },
        )
        .expect("doctor all should succeed");
        let Some(ReplCommandOutput::Json(json)) = filtered.output else {
            panic!("expected json output");
        };
        assert!(json.get("theme").is_some());
        assert!(json.get("config").is_none());
        assert!(json.get("plugins").is_none());

        let theme = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("theme")),
            DoctorArgs {
                command: Some(DoctorCommands::Theme),
            },
        )
        .expect("doctor theme should succeed");
        assert!(theme.output.is_some());
        assert!(theme.messages.is_empty());

        let table = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("config,theme")),
            DoctorArgs {
                command: Some(DoctorCommands::All),
            },
        )
        .expect("doctor all should succeed");
        let Some(ReplCommandOutput::Output(guide)) = table.output else {
            panic!("expected guide output");
        };
        assert!(
            guide
                .source_guide
                .expect("expected semantic guide payload")
                .sections
                .iter()
                .all(|section| section.data.is_some())
        );
        let rows = guide
            .output
            .as_rows()
            .expect("doctor guide output should keep a structured fallback");
        assert!(rows[0].get("config").is_some());
        assert!(rows[0].get("theme").is_some());

        let config_err = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("theme")),
            DoctorArgs {
                command: Some(DoctorCommands::Config),
            },
        )
        .expect_err("hidden config builtin should fail");
        assert!(!config_err.to_string().trim().is_empty());

        let theme_err = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("config")),
            DoctorArgs {
                command: Some(DoctorCommands::Theme),
            },
        )
        .expect_err("hidden theme builtin should fail");
        assert!(!theme_err.to_string().trim().is_empty());

        let def = doctor_command_def("30");
        let names = def
            .subcommands
            .iter()
            .map(|child| child.name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(def.name, "doctor");
        assert_eq!(def.sort_key.as_deref(), Some("30"));
        assert_eq!(names, vec!["all", "config", "last", "plugins", "theme"]);
    }
}
