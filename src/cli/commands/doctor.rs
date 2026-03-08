use miette::Result;

use crate::app::{AuthState, LastFailure, UiState};
use crate::app::{
    CMD_CONFIG, CMD_PLUGINS, CMD_THEME, CliCommandResult, document_from_json, document_from_text,
    ensure_builtin_visible_for,
};
use crate::cli::rows::output::rows_to_output_result;
use crate::cli::{DoctorArgs, DoctorCommands, PluginsArgs, PluginsCommands};
use crate::core::output::OutputFormat;
use crate::core::row::Row;
use crate::ui::document::{Block, Document, PanelBlock, PanelRules};
use crate::ui::format::build_document_from_output;
use crate::ui::theme_loader::ThemeCatalog;

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
        DoctorCommands::Last => Ok(CliCommandResult::document(render_last_failure_document(
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
        return Ok(CliCommandResult::document(document_from_json(payload)));
    }

    let blocks = sections
        .into_iter()
        .map(|(name, rows)| {
            let output = rows_to_output_result(rows);
            let body = build_document_from_output(&output, &context.ui.render_settings);
            Block::Panel(PanelBlock {
                title: Some(name.to_string()),
                body,
                rules: PanelRules::Top,
                kind: None,
                border_token: None,
                title_token: None,
            })
        })
        .collect();

    Ok(CliCommandResult::document(Document { blocks }))
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

fn render_last_failure_document(ui: &UiState, last_failure: Option<&LastFailure>) -> Document {
    let Some(last) = last_failure else {
        return document_from_text("No recorded REPL failure in this session.\n");
    };

    if matches!(ui.render_settings.format, OutputFormat::Json) {
        let payload = serde_json::json!({
            "status": "error",
            "command": last.command_line,
            "summary": last.summary,
            "detail": last.detail,
        });
        return document_from_json(payload);
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
    document_from_text(&out)
}

#[cfg(test)]
mod tests {
    use super::{
        DoctorCommandContext, render_last_failure_document, run_doctor_command, theme_doctor_rows,
    };
    use crate::app::ReplCommandOutput;
    use crate::app::{AuthState, LastFailure, RuntimeContext, TerminalKind, UiState};
    use crate::cli::commands::{config as config_cmd, plugins as plugins_cmd};
    use crate::cli::{DoctorArgs, DoctorCommands};
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions, RuntimeLoadOptions};
    use crate::core::output::OutputFormat;
    use crate::plugin::PluginManager;
    use crate::ui::RenderSettings;
    use crate::ui::document::{Block, LinePart};
    use crate::ui::messages::MessageLevel;
    use crate::ui::theme_loader::{ThemeCatalog, ThemeLoadIssue};
    use serde_json::Value;
    use std::path::PathBuf;

    fn ui_state(format: OutputFormat, debug_verbosity: u8) -> UiState {
        UiState {
            render_settings: RenderSettings::test_plain(format),
            message_verbosity: MessageLevel::Success,
            debug_verbosity,
        }
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
        let plugin_manager = Box::leak(Box::new(PluginManager::new(Vec::new())));
        let context = Box::leak(Box::new(RuntimeContext::new(None, TerminalKind::Cli, None)));

        DoctorCommandContext {
            config: config_cmd::ConfigReadContext {
                context,
                config: resolved,
                ui,
                themes,
                config_overrides,
                runtime_load: RuntimeLoadOptions::default(),
            },
            plugins: plugins_cmd::PluginsCommandContext {
                config: resolved,
                config_state: None,
                auth,
                clients: None,
                plugin_manager,
            },
            ui,
            auth,
            themes,
            last_failure: None,
        }
    }

    #[test]
    fn doctor_last_without_failure_returns_plain_notice_unit() {
        let document = render_last_failure_document(&ui_state(OutputFormat::Table, 0), None);

        let rendered = render_line_blocks(&document.blocks);
        assert!(rendered.contains("No recorded REPL failure"));
    }

    #[test]
    fn doctor_last_text_includes_detail_when_debug_is_enabled_unit() {
        let failure = LastFailure {
            command_line: "ldap user nope".to_string(),
            summary: "request failed".to_string(),
            detail: "request failed\nbackend said no".to_string(),
        };

        let document =
            render_last_failure_document(&ui_state(OutputFormat::Table, 1), Some(&failure));

        let rendered = render_line_blocks(&document.blocks);
        assert!(rendered.contains("Command: ldap user nope"));
        assert!(rendered.contains("Error:   request failed"));
        assert!(rendered.contains("Detail:"));
        assert!(rendered.contains("backend said no"));
    }

    #[test]
    fn doctor_last_json_shape_is_stable_unit() {
        let failure = LastFailure {
            command_line: "plugins refresh".to_string(),
            summary: "plugin failed".to_string(),
            detail: "plugin failed".to_string(),
        };

        let document =
            render_last_failure_document(&ui_state(OutputFormat::Json, 0), Some(&failure));

        let Some(Block::Json(json)) = document.blocks.first() else {
            panic!("expected json block");
        };
        assert_eq!(json.payload["status"], Value::String("error".to_string()));
        assert_eq!(
            json.payload["command"],
            Value::String("plugins refresh".to_string())
        );
    }

    #[test]
    fn doctor_last_text_omits_detail_when_debug_is_disabled_unit() {
        let failure = LastFailure {
            command_line: "ldap user nope".to_string(),
            summary: "request failed".to_string(),
            detail: "request failed\nbackend said no".to_string(),
        };

        let document =
            render_last_failure_document(&ui_state(OutputFormat::Table, 0), Some(&failure));

        let rendered = render_line_blocks(&document.blocks);
        assert!(rendered.contains("Error:   request failed"));
        assert!(!rendered.contains("Detail:"));
    }

    #[test]
    fn theme_doctor_rows_report_issues_and_empty_state_unit() {
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
    fn doctor_all_json_includes_visible_sections_unit() {
        let result = run_doctor_command(
            doctor_context(OutputFormat::Json, Some("config,plugins,theme")),
            DoctorArgs {
                command: Some(DoctorCommands::All),
            },
        )
        .expect("doctor all should succeed");

        let Some(ReplCommandOutput::Document(document)) = result.output else {
            panic!("expected document output");
        };
        let Some(Block::Json(json)) = document.blocks.first() else {
            panic!("expected json block");
        };
        assert!(json.payload.get("config").is_some());
        assert!(json.payload.get("plugins").is_some());
        assert!(json.payload.get("theme").is_some());
    }

    #[test]
    fn doctor_all_respects_builtin_visibility_unit() {
        let result = run_doctor_command(
            doctor_context(OutputFormat::Json, Some("theme")),
            DoctorArgs {
                command: Some(DoctorCommands::All),
            },
        )
        .expect("doctor all should succeed");

        let Some(ReplCommandOutput::Document(document)) = result.output else {
            panic!("expected document output");
        };
        let Some(Block::Json(json)) = document.blocks.first() else {
            panic!("expected json block");
        };
        assert!(json.payload.get("theme").is_some());
        assert!(json.payload.get("config").is_none());
        assert!(json.payload.get("plugins").is_none());
    }

    #[test]
    fn doctor_config_requires_builtin_visibility_unit() {
        let err = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("theme")),
            DoctorArgs {
                command: Some(DoctorCommands::Config),
            },
        )
        .expect_err("hidden config builtin should fail");

        assert!(!err.to_string().trim().is_empty());
    }

    #[test]
    fn doctor_theme_returns_output_rows_unit() {
        let result = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("theme")),
            DoctorArgs {
                command: Some(DoctorCommands::Theme),
            },
        )
        .expect("doctor theme should succeed");

        assert!(result.output.is_some());
        assert!(result.messages.is_empty());
    }

    #[test]
    fn doctor_all_table_groups_sections_into_panels_unit() {
        let result = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("config,theme")),
            DoctorArgs {
                command: Some(DoctorCommands::All),
            },
        )
        .expect("doctor all should succeed");

        let Some(ReplCommandOutput::Document(document)) = result.output else {
            panic!("expected document output");
        };
        assert!(
            document
                .blocks
                .iter()
                .all(|block| matches!(block, Block::Panel(_)))
        );
    }

    #[test]
    fn doctor_theme_requires_builtin_visibility_unit() {
        let err = run_doctor_command(
            doctor_context(OutputFormat::Table, Some("config")),
            DoctorArgs {
                command: Some(DoctorCommands::Theme),
            },
        )
        .expect_err("hidden theme builtin should fail");

        assert!(!err.to_string().trim().is_empty());
    }

    fn render_line_blocks(blocks: &[Block]) -> String {
        blocks
            .iter()
            .filter_map(|block| match block {
                Block::Line(line) => Some(
                    line.parts
                        .iter()
                        .map(|LinePart { text, .. }| text.as_str())
                        .collect::<String>(),
                ),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
