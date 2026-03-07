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
    CMD_HELP, CommandRenderRuntime, PreparedPluginResponse, emit_messages_with_runtime,
    enrich_dispatch_error, ensure_plugin_visible_for, maybe_copy_output_with_runtime,
    plugin_dispatch_context_for, prepare_plugin_response, resolve_effective_render_settings,
    run_cli_command, run_inline_builtin_command,
};
use super::{ResolvedConfig, render_output};

pub(super) struct ExternalCommandRuntime<'a> {
    pub(super) context: &'a RuntimeContext,
    pub(super) config_state: &'a crate::state::ConfigState,
    pub(super) config: &'a ResolvedConfig,
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
            config: runtime.config.resolved(),
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
    Handled(i32),
    Invocation(ParsedExternalInvocation),
}

pub(super) fn run_external_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    tokens: &[String],
) -> Result<i32> {
    let invocation = match parse_external_invocation(runtime, session, tokens)? {
        ExternalParse::Handled(code) => return Ok(code),
        ExternalParse::Invocation(invocation) => invocation,
    };

    if let Some(command) = invocation.inline_command
        && let Some(result) =
            run_inline_builtin_command(runtime, session, clients, command, &invocation.stages)?
    {
        return run_cli_command(
            &CommandRenderRuntime::new(runtime.config.resolved(), &runtime.ui),
            result,
        );
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
        print!("{help}");
        return Ok(ExternalParse::Handled(0));
    }

    let inline_command = match parse_inline_command_tokens(&parsed.tokens) {
        Ok(command) => command,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let resolved = runtime.ui.render_settings.resolve_render_settings();
                print!(
                    "{}",
                    repl_help::render_help_with_chrome(&err.to_string(), &resolved)
                );
                return Ok(ExternalParse::Handled(0));
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
    let providers = plugin_manager.command_providers(command);
    if providers.len() <= 1 {
        return;
    }
    let selected = plugin_manager
        .selected_provider_label(command)
        .unwrap_or_else(|| {
            providers
                .first()
                .cloned()
                .unwrap_or_else(|| "unknown".to_string())
        });
    let mut messages = osp_ui::messages::MessageBuffer::default();
    messages.warning(format!(
        "command `{command}` is provided by multiple plugins: {}. Using {selected}.",
        providers.join(", ")
    ));
    let render_runtime = CommandRenderRuntime::new(runtime.config, runtime.ui);
    emit_messages_with_runtime(&render_runtime, &messages, runtime.ui.message_verbosity);
}

fn run_external_plugin_command(
    runtime: &ExternalCommandRuntime<'_>,
    command: &str,
    args: &[String],
    stages: &[String],
) -> Result<i32> {
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
        if !raw.stdout.is_empty() {
            let resolved = runtime.ui.render_settings.resolve_render_settings();
            print!(
                "{}",
                repl_help::render_help_with_chrome(&raw.stdout, &resolved)
            );
        }
        if !raw.stderr.is_empty() {
            eprint!("{}", raw.stderr);
        }
        return Ok(raw.status_code);
    }

    let dispatch_context = plugin_dispatch_context_for(runtime, None);
    let response = runtime
        .plugins
        .dispatch(command, args, &dispatch_context)
        .map_err(enrich_dispatch_error)?;

    render_external_plugin_response(runtime, response, stages)
}

fn render_external_plugin_response(
    runtime: &ExternalCommandRuntime<'_>,
    response: osp_core::plugin::ResponseV1,
    stages: &[String],
) -> Result<i32> {
    let render_runtime = CommandRenderRuntime::new(runtime.config, runtime.ui);
    match prepare_plugin_response(response, stages).map_err(|err| miette!("{err:#}"))? {
        PreparedPluginResponse::Failure(failure) => {
            emit_messages_with_runtime(
                &render_runtime,
                &failure.messages,
                runtime.ui.message_verbosity,
            );
            Ok(1)
        }
        PreparedPluginResponse::Output(prepared) => {
            if !prepared.messages.is_empty() {
                emit_messages_with_runtime(
                    &render_runtime,
                    &prepared.messages,
                    runtime.ui.message_verbosity,
                );
            }
            let effective = resolve_effective_render_settings(
                &runtime.ui.render_settings,
                prepared.format_hint,
            );
            print!("{}", render_output(&prepared.output, &effective));
            maybe_copy_output_with_runtime(&render_runtime, &prepared.output);
            Ok(0)
        }
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
    use super::{ExternalParse, parse_external_invocation};
    use crate::plugin_manager::PluginManager;
    use crate::state::{
        AppRuntime, AppSession, AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind,
    };
    use osp_config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use osp_core::output::OutputFormat;
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
        assert!(matches!(parsed, ExternalParse::Handled(0)));
    }
}
