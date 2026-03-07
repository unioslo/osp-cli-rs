use miette::{Result, miette};

use crate::cli::{Commands, parse_inline_command_tokens};
use crate::pipeline::parse_command_tokens_with_aliases;
use crate::plugin_manager::PluginManager;
use crate::repl::ReplViewContext;
use crate::repl::completion;
use crate::repl::help as repl_help;
use crate::state::{AppClients, AppRuntime, AppSession};
use crate::state::{AuthState, RuntimeContext, UiState};

use super::{
    CMD_HELP, CliCommandResult, CommandRenderRuntime, PreparedPluginResponse, ReplCommandOutput,
    command_output::emit_messages_with_runtime, enrich_dispatch_error, ensure_plugin_visible_for,
    plugin_dispatch_context_for, prepare_plugin_response, run_inline_builtin_command,
};

pub(super) struct ExternalCommandRuntime<'a> {
    pub(super) context: &'a RuntimeContext,
    pub(super) config_state: &'a crate::state::ConfigState,
    pub(super) ui: &'a UiState,
    pub(super) auth: &'a AuthState,
    pub(super) clients: &'a AppClients,
    pub(super) plugins: &'a PluginManager,
}

impl<'a> ExternalCommandRuntime<'a> {
    pub(super) fn from_parts(runtime: &'a AppRuntime, clients: &'a AppClients) -> Self {
        Self {
            context: &runtime.context,
            config_state: &runtime.config,
            ui: &runtime.ui,
            auth: &runtime.auth,
            clients,
            plugins: &clients.plugins,
        }
    }
}

struct ParsedExternalInvocation {
    tokens: Vec<String>,
    stages: Vec<String>,
    inline_command: Option<Commands>,
}

enum ExternalParse {
    Handled(CliCommandResult),
    Invocation(ParsedExternalInvocation),
}

pub(super) fn run_external_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    tokens: &[String],
) -> Result<CliCommandResult> {
    let invocation = match parse_external_invocation(runtime, session, tokens)? {
        ExternalParse::Handled(result) => return Ok(result),
        ExternalParse::Invocation(invocation) => invocation,
    };

    if let Some(command) = invocation.inline_command
        && let Some(result) =
            run_inline_builtin_command(runtime, session, clients, command, &invocation.stages)?
    {
        return Ok(result);
    }
    if !invocation.stages.is_empty() {
        completion::validate_dsl_stages(&invocation.stages)?;
    }

    let (command, args) = invocation
        .tokens
        .split_first()
        .ok_or_else(|| miette!("missing external command"))?;
    let external_runtime = ExternalCommandRuntime::from_parts(runtime, clients);
    run_external_plugin_command(&external_runtime, command, args, &invocation.stages)
}

fn parse_external_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    tokens: &[String],
) -> Result<ExternalParse> {
    let parsed = parse_command_tokens_with_aliases(tokens, runtime.config.resolved())?;
    if parsed.tokens.is_empty() {
        return Err(miette!("missing external command"));
    }
    if let Some(help) = completion::maybe_render_dsl_help(
        ReplViewContext::from_parts(runtime, session),
        &parsed.stages,
    ) {
        return Ok(ExternalParse::Handled(CliCommandResult::text(help)));
    }

    let inline_command = match parse_inline_command_tokens(&parsed.tokens) {
        Ok(command) => command,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let resolved = runtime.ui.render_settings.resolve_render_settings();
                return Ok(ExternalParse::Handled(CliCommandResult::text(
                    repl_help::render_help_with_chrome(&err.to_string(), &resolved),
                )));
            }
            return Err(miette!(err.to_string()));
        }
    };

    Ok(ExternalParse::Invocation(ParsedExternalInvocation {
        tokens: parsed.tokens,
        stages: parsed.stages,
        inline_command,
    }))
}

fn emit_command_conflict_warning_for(
    runtime: &ExternalCommandRuntime<'_>,
    command: &str,
    plugin_manager: &PluginManager,
) {
    let Some(message) = plugin_manager.conflict_warning(command) else {
        return;
    };
    let mut messages = osp_ui::messages::MessageBuffer::default();
    messages.warning(message);
    let render_runtime = CommandRenderRuntime::new(runtime.config_state.resolved(), runtime.ui);
    emit_messages_with_runtime(&render_runtime, &messages, runtime.ui.message_verbosity);
}

fn run_external_plugin_command(
    runtime: &ExternalCommandRuntime<'_>,
    command: &str,
    args: &[String],
    stages: &[String],
) -> Result<CliCommandResult> {
    ensure_plugin_visible_for(runtime.auth, command)?;
    emit_command_conflict_warning_for(runtime, command, runtime.plugins);

    tracing::debug!(
        command = %command,
        args = ?args,
        "dispatching external command"
    );

    if is_help_passthrough(args) {
        let dispatch_context = plugin_dispatch_context_for(runtime, None);
        let raw = runtime
            .plugins
            .dispatch_passthrough(command, args, &dispatch_context)
            .map_err(enrich_dispatch_error)?;
        let mut result = if !raw.stdout.is_empty() {
            let resolved = runtime.ui.render_settings.resolve_render_settings();
            CliCommandResult::text(repl_help::render_help_with_chrome(&raw.stdout, &resolved))
        } else {
            CliCommandResult::exit(raw.status_code)
        };
        if !raw.stderr.is_empty() {
            result.stderr_text = Some(raw.stderr);
        }
        result.exit_code = raw.status_code;
        return Ok(result);
    }

    let dispatch_context = plugin_dispatch_context_for(runtime, None);
    let response = runtime
        .plugins
        .dispatch(command, args, &dispatch_context)
        .map_err(enrich_dispatch_error)?;

    render_external_plugin_response(response, stages)
}

fn render_external_plugin_response(
    response: osp_core::plugin::ResponseV1,
    stages: &[String],
) -> Result<CliCommandResult> {
    match prepare_plugin_response(response, stages).map_err(|err| miette!("{err:#}"))? {
        PreparedPluginResponse::Failure(failure) => Ok(CliCommandResult {
            exit_code: 1,
            messages: failure.messages,
            output: None,
            stderr_text: None,
        }),
        PreparedPluginResponse::Output(prepared) => Ok(CliCommandResult {
            exit_code: 0,
            messages: prepared.messages,
            output: Some(ReplCommandOutput::Output {
                output: prepared.output,
                format_hint: prepared.format_hint,
            }),
            stderr_text: None,
        }),
    }
}

pub(crate) fn is_help_passthrough(args: &[String]) -> bool {
    if args.is_empty() {
        return false;
    }

    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return true;
    }

    matches!(args.first(), Some(first) if first == CMD_HELP)
}

#[cfg(test)]
mod tests {
    use super::{ExternalParse, parse_external_invocation, render_external_plugin_response};
    use crate::app::{CliCommandResult, ReplCommandOutput};
    use crate::plugin_manager::PluginManager;
    use crate::state::{
        AppRuntime, AppSession, AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind,
    };
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use osp_core::output::OutputFormat;
    use osp_core::plugin::{ResponseMessageLevelV1, ResponseMessageV1, ResponseMetaV1, ResponseV1};
    use osp_ui::RenderSettings;
    use osp_ui::messages::MessageLevel;

    fn make_test_state() -> (AppRuntime, AppSession) {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default())
            .expect("test config should resolve");

        let state = AppState::new(AppStateInit {
            context: RuntimeContext::new(None, TerminalKind::Cli, None),
            config,
            render_settings: RenderSettings::test_plain(OutputFormat::Json),
            message_verbosity: MessageLevel::Success,
            debug_verbosity: 0,
            plugins: PluginManager::new(Vec::new()),
            themes: crate::theme_loader::ThemeCatalog::default(),
            launch: LaunchContext::default(),
        });
        (state.runtime, state.session)
    }

    #[test]
    fn external_builtin_help_passthrough_is_handled_unit() {
        let (runtime, session) = make_test_state();
        let tokens = ["config", "--help"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let parsed =
            parse_external_invocation(&runtime, &session, &tokens).expect("help should parse");
        assert!(matches!(
            parsed,
            ExternalParse::Handled(CliCommandResult {
                exit_code: 0,
                output: Some(ReplCommandOutput::Text(_)),
                ..
            })
        ));
    }

    #[test]
    fn external_plugin_response_preserves_messages_unit() {
        let response = ResponseV1 {
            protocol_version: 1,
            ok: true,
            data: serde_json::json!({ "message": "hello" }),
            error: None,
            messages: vec![ResponseMessageV1 {
                level: ResponseMessageLevelV1::Warning,
                text: "warning from plugin".to_string(),
            }],
            meta: ResponseMetaV1 {
                format_hint: Some("json".to_string()),
                columns: None,
            },
        };

        let result =
            render_external_plugin_response(response, &[]).expect("response should prepare");
        assert!(!result.messages.is_empty());
    }
}
