use miette::{Result, WrapErr, miette};

use crate::app::{AppClients, AppRuntime, AppSession};
use crate::app::{AuthState, RuntimeContext, UiState};
use crate::cli::invocation::extend_with_invocation_help;
use crate::cli::pipeline::parse_command_tokens_with_aliases;
use crate::cli::{Commands, parse_inline_command_tokens};
use crate::guide::GuideView;
use crate::native::{NativeCommandContext, NativeCommandOutcome};
use crate::plugin::PluginManager;
use crate::repl::ReplViewContext;
use crate::repl::completion;

use super::{
    CMD_HELP, CliCommandResult, ResolvedInvocation, cli_result_from_plugin_response,
    enrich_dispatch_error, ensure_plugin_visible_for, plugin_dispatch_context_for,
    run_inline_builtin_command, runtime_hints_for_runtime,
};

pub(super) struct ExternalCommandRuntime<'a> {
    pub(super) context: &'a RuntimeContext,
    pub(super) config_state: &'a crate::app::ConfigState,
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
            plugins: clients.plugins(),
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
    invocation: &ResolvedInvocation,
) -> Result<CliCommandResult> {
    run_external_command_with_help_renderer(
        runtime,
        session,
        clients,
        tokens,
        invocation,
        |stdout| {
            let mut guide = GuideView::from_text(stdout);
            extend_with_invocation_help(&mut guide, invocation.help_level);
            guide.filtered_for_help_level(invocation.help_level)
        },
    )
}

pub(crate) fn run_external_command_with_help_renderer(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    tokens: &[String],
    invocation: &ResolvedInvocation,
    guide_help: impl Fn(&str) -> GuideView,
) -> Result<CliCommandResult> {
    let parsed = match parse_external_invocation(runtime, session, tokens, invocation.help_level)
        .wrap_err_with(|| {
            format!(
                "failed to parse external command invocation for `{}`",
                tokens.first().map(String::as_str).unwrap_or("external")
            )
        })? {
        ExternalParse::Handled(result) => return Ok(result),
        ExternalParse::Invocation(parsed) => parsed,
    };

    if let Some(command) = parsed.inline_command
        && let Some(result) = run_inline_builtin_command(
            runtime,
            session,
            clients,
            Some(invocation),
            command,
            &parsed.stages,
        )?
    {
        return Ok(result);
    }
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)
            .wrap_err("failed to validate DSL pipeline stages")?;
    }

    let (command, args) = parsed
        .tokens
        .split_first()
        .ok_or_else(|| miette!("missing external command"))?;
    let external_runtime = ExternalCommandRuntime::from_parts(runtime, clients);

    if let Some(native_command) = clients.native_commands().command(command) {
        ensure_plugin_visible_for(&runtime.auth, command)?;
        return run_native_command(
            native_command.as_ref(),
            runtime,
            args,
            &parsed.stages,
            guide_help,
        );
    }

    run_external_plugin_command(
        &external_runtime,
        command,
        args,
        &parsed.stages,
        invocation,
        guide_help,
    )
}

fn run_native_command(
    command: &dyn crate::native::NativeCommand,
    runtime: &mut AppRuntime,
    args: &[String],
    stages: &[String],
    guide_help: impl Fn(&str) -> GuideView,
) -> Result<CliCommandResult> {
    let context = NativeCommandContext::new(
        runtime.config.resolved(),
        runtime_hints_for_runtime(runtime),
    );

    match command
        .execute(args, &context)
        .map_err(|err| miette!("{err:#}"))?
    {
        NativeCommandOutcome::Help(text) => Ok(CliCommandResult::guide(guide_help(&text))),
        NativeCommandOutcome::Exit(code) => Ok(CliCommandResult::exit(code)),
        NativeCommandOutcome::Response(response) => render_native_response(*response, stages),
    }
}

fn render_native_response(
    response: crate::core::plugin::ResponseV1,
    stages: &[String],
) -> Result<CliCommandResult> {
    cli_result_from_plugin_response(response, stages)
}

fn parse_external_invocation(
    runtime: &AppRuntime,
    session: &AppSession,
    tokens: &[String],
    help_level: crate::guide::HelpLevel,
) -> Result<ExternalParse> {
    let parsed = parse_command_tokens_with_aliases(tokens, runtime.config.resolved())?;
    if parsed.tokens.is_empty() {
        return Err(miette!("missing external command"));
    }
    if let Some(help) = completion::maybe_render_dsl_help(
        ReplViewContext::from_parts(runtime, session),
        &parsed.stages,
    ) {
        return Ok(ExternalParse::Handled(CliCommandResult::guide(
            GuideView::from_text(&help),
        )));
    }

    let inline_command = match parse_inline_command_tokens(&parsed.tokens) {
        Ok(command) => command,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let mut view = GuideView::from_text(&err.to_string());
                extend_with_invocation_help(&mut view, help_level);
                return Ok(ExternalParse::Handled(CliCommandResult::guide(
                    view.filtered_for_help_level(help_level),
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

fn run_external_plugin_command(
    runtime: &ExternalCommandRuntime<'_>,
    command: &str,
    args: &[String],
    stages: &[String],
    invocation: &ResolvedInvocation,
    guide_help: impl Fn(&str) -> GuideView,
) -> Result<CliCommandResult> {
    ensure_plugin_visible_for(runtime.auth, command)?;

    tracing::debug!(
        command = %command,
        args = ?args,
        "dispatching external command"
    );

    if is_help_passthrough(args) {
        let dispatch_context = plugin_dispatch_context_for(runtime, Some(invocation));
        let raw = runtime
            .plugins
            .dispatch_passthrough(command, args, &dispatch_context)
            .map_err(enrich_dispatch_error)?;
        let mut result = if !raw.stdout.is_empty() {
            CliCommandResult::guide(guide_help(&raw.stdout))
        } else {
            CliCommandResult::exit(raw.status_code)
        };
        if !raw.stderr.is_empty() {
            result.stderr_text = Some(raw.stderr);
        }
        result.exit_code = raw.status_code;
        return Ok(result);
    }

    let dispatch_context = plugin_dispatch_context_for(runtime, Some(invocation));
    let response = runtime
        .plugins
        .dispatch(command, args, &dispatch_context)
        .map_err(enrich_dispatch_error)?;

    render_external_plugin_response(response, stages)
}

fn render_external_plugin_response(
    response: crate::core::plugin::ResponseV1,
    stages: &[String],
) -> Result<CliCommandResult> {
    cli_result_from_plugin_response(response, stages)
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
    use super::{
        ExternalParse, is_help_passthrough, parse_external_invocation,
        render_external_plugin_response, run_external_command_with_help_renderer,
    };
    use crate::app::{
        AppClients, AppRuntime, AppSession, AppStateBuilder, LaunchContext, RuntimeContext,
        TerminalKind, UiState, resolve_invocation_ui,
    };
    use crate::app::{CliCommandResult, ReplCommandOutput};
    use crate::cli::invocation::InvocationOptions;
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::output::OutputFormat;
    use crate::core::plugin::{
        DescribeCommandAuthV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
        ResponseMessageLevelV1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
    };
    use crate::guide::GuideView;
    use crate::native::{
        NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    };
    use crate::plugin::PluginManager;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use clap::Command;

    #[derive(Clone, Copy)]
    enum NativeOutcomeKind {
        Help,
        Exit,
        Response,
    }

    struct TestNativeCommand {
        kind: NativeOutcomeKind,
    }

    impl NativeCommand for TestNativeCommand {
        fn command(&self) -> Command {
            Command::new("ldap").about("Directory lookup")
        }

        fn auth(&self) -> Option<DescribeCommandAuthV1> {
            Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                required_capabilities: Vec::new(),
                feature_flags: Vec::new(),
            })
        }

        fn execute(
            &self,
            args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            Ok(match self.kind {
                NativeOutcomeKind::Help => NativeCommandOutcome::Help(format!(
                    "Usage: osp ldap\n\nArgs: {}\n",
                    args.join(" ")
                )),
                NativeOutcomeKind::Exit => NativeCommandOutcome::Exit(7),
                NativeOutcomeKind::Response => {
                    NativeCommandOutcome::Response(Box::new(ResponseV1 {
                        protocol_version: PLUGIN_PROTOCOL_V1,
                        ok: true,
                        data: serde_json::json!([{ "command": "ldap", "args": args }]),
                        error: None,
                        messages: vec![ResponseMessageV1 {
                            level: ResponseMessageLevelV1::Info,
                            text: "native ok".to_string(),
                        }],
                        meta: ResponseMetaV1 {
                            format_hint: Some("json".to_string()),
                            columns: None,
                            column_align: Vec::new(),
                        },
                    }))
                }
            })
        }
    }

    fn make_test_state_with_native(
        kind: Option<NativeOutcomeKind>,
    ) -> (AppRuntime, AppSession, AppClients) {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default())
            .expect("test config should resolve");

        let state = AppStateBuilder::new(
            RuntimeContext::new(None, TerminalKind::Cli, None),
            config,
            UiState::new(
                RenderSettings::test_plain(OutputFormat::Json),
                MessageLevel::Success,
                0,
            ),
        )
        .with_launch(LaunchContext::default())
        .with_plugins(PluginManager::new(Vec::new()))
        .with_native_commands(
            kind.map(|kind| NativeCommandRegistry::new().with_command(TestNativeCommand { kind }))
                .unwrap_or_default(),
        )
        .build();
        (state.runtime, state.session, state.clients)
    }

    #[test]
    fn external_builtin_help_passthrough_is_handled_unit() {
        let (runtime, session, _) = make_test_state_with_native(None);
        let tokens = ["config", "--help"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let parsed = parse_external_invocation(&runtime, &session, &tokens, Default::default())
            .expect("help should parse");
        assert!(matches!(
            parsed,
            ExternalParse::Handled(CliCommandResult {
                exit_code: 0,
                output: Some(ReplCommandOutput::Output(_)),
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
                column_align: Vec::new(),
            },
        };

        let result =
            render_external_plugin_response(response, &[]).expect("response should prepare");
        assert!(!result.messages.is_empty());
    }

    #[test]
    fn help_passthrough_detection_covers_flags_and_help_subcommand_unit() {
        assert!(!is_help_passthrough(&[]));
        assert!(is_help_passthrough(&["--help".to_string()]));
        assert!(is_help_passthrough(&[
            "topic".to_string(),
            "-h".to_string()
        ]));
        assert!(is_help_passthrough(&["help".to_string()]));
        assert!(!is_help_passthrough(&[
            "ldap".to_string(),
            "user".to_string()
        ]));
    }

    #[test]
    fn external_native_command_help_exit_and_response_paths_unit() {
        for kind in [
            NativeOutcomeKind::Help,
            NativeOutcomeKind::Exit,
            NativeOutcomeKind::Response,
        ] {
            let (mut runtime, mut session, clients) = make_test_state_with_native(Some(kind));
            let invocation = resolve_invocation_ui(
                runtime.config.resolved(),
                &runtime.ui,
                &InvocationOptions::default(),
            );
            let result = run_external_command_with_help_renderer(
                &mut runtime,
                &mut session,
                &clients,
                &["ldap".to_string(), "user".to_string()],
                &invocation,
                |text| GuideView::from_text(&format!("HELP::{text}")),
            )
            .expect("native command should dispatch");

            match kind {
                NativeOutcomeKind::Help => {
                    assert!(matches!(
                        result.output,
                        Some(ReplCommandOutput::Output(guide))
                            if guide
                                .source_guide
                                .as_ref()
                                .expect("expected semantic guide payload")
                                .preamble
                                .iter()
                                .any(|line| line.contains("HELP::Usage: osp ldap"))
                    ));
                }
                NativeOutcomeKind::Exit => {
                    assert_eq!(result.exit_code, 7);
                    assert!(result.output.is_none());
                }
                NativeOutcomeKind::Response => {
                    assert_eq!(result.exit_code, 0);
                    assert!(!result.messages.is_empty());
                    assert!(matches!(result.output, Some(ReplCommandOutput::Output(_))));
                }
            }
        }
    }
}
