//! External command parsing and dispatch.
//!
//! This module owns the path from already-tokenized user input to either a
//! native command, plugin command, inline builtin, or help rewrite on the
//! external command surface. It sits after top-level host setup but before the
//! final native/plugin execution paths.

use std::cell::RefCell;

use miette::{Result, WrapErr, miette};

use crate::app::{AppClients, AppRuntime, AppSession};
use crate::app::{RuntimeContext, UiState};
use crate::cli::invocation::extend_with_invocation_help;
use crate::cli::pipeline::parse_command_tokens_with_aliases;
use crate::cli::{Commands, parse_inline_command_tokens};
use crate::guide::GuideView;
use crate::native::{
    NativeCommandContext, NativeCommandOutcome, NativeProgressEvent, NativeProgressSink,
};
use crate::plugin::PluginManager;
use crate::repl::ReplViewContext;
use crate::repl::completion;
use crate::repl::is_repl_shellable_command;

use super::dispatch::{
    ExternalCommandSource, ExternalPathAccessRequirement, canonical_external_command_name,
    ensure_external_path_access, resolve_external_command_source,
};
use super::sink::UiSink;
use super::{
    CMD_HELP, CliCommandResult, ResolvedInvocation, cli_result_from_plugin_response,
    enrich_dispatch_error, plugin_dispatch_context_for, run_cli_command_with_ui,
    run_inline_builtin_command, runtime_hints_for_invocation,
};

pub(super) struct ExternalCommandRuntime<'a> {
    pub(super) context: &'a RuntimeContext,
    pub(super) config_state: &'a crate::app::ConfigState,
    pub(super) ui: &'a UiState,
    pub(super) clients: &'a AppClients,
    pub(super) plugins: &'a PluginManager,
}

impl<'a> ExternalCommandRuntime<'a> {
    pub(super) fn from_parts(runtime: &'a AppRuntime, clients: &'a AppClients) -> Self {
        Self {
            context: &runtime.context,
            config_state: &runtime.config,
            ui: &runtime.ui,
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
    Handled(Box<CliCommandResult>),
    Invocation(ParsedExternalInvocation),
}

pub(super) fn run_external_command(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    tokens: &[String],
    invocation: &ResolvedInvocation,
    sink: &mut dyn UiSink,
) -> Result<CliCommandResult> {
    run_external_command_with_help_renderer_and_progress(
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
        Some(sink),
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
    run_external_command_with_help_renderer_and_progress(
        runtime, session, clients, tokens, invocation, guide_help, None,
    )
}

pub(crate) fn run_external_command_with_help_renderer_and_progress(
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    tokens: &[String],
    invocation: &ResolvedInvocation,
    guide_help: impl Fn(&str) -> GuideView,
    progress_sink: Option<&mut dyn UiSink>,
) -> Result<CliCommandResult> {
    let mut parsed = match parse_external_invocation(runtime, session, tokens, invocation)
        .wrap_err_with(|| {
            format!(
                "failed to parse external command invocation for `{}`",
                tokens.first().map(String::as_str).unwrap_or("external")
            )
        })? {
        ExternalParse::Handled(result) => return Ok(*result),
        ExternalParse::Invocation(parsed) => parsed,
    };

    if let Some(command) = parsed.inline_command.take()
        && let Some(result) = run_inline_builtin_command(
            runtime,
            session,
            clients,
            Some(invocation),
            command,
            &parsed.stages,
        )?
    {
        return crate::app::command_output::apply_stages_to_cli_result(result, &parsed.stages);
    }
    if !parsed.stages.is_empty() {
        completion::validate_dsl_stages(&parsed.stages)
            .wrap_err("failed to validate DSL pipeline stages")?;
    }

    let (command, args) = parsed
        .tokens
        .split_first()
        .ok_or_else(|| miette!("missing external command"))?;
    let command = canonical_external_command_name(&runtime.auth, clients, command)?;
    match resolve_external_command_source(
        &runtime.auth,
        clients,
        &command,
        invocation.plugin_provider.as_deref(),
    )? {
        ExternalCommandSource::Native => {
            let native_command = clients
                .native_commands()
                .command(&command)
                .ok_or_else(|| miette!("no native command provides `{command}`"))?;
            let path = native_command.describe().resolved_subcommand_path(args);
            ensure_external_path_access(
                runtime,
                session,
                &path,
                external_path_access_requirement(args),
            )?;
            run_native_command(
                native_command.as_ref(),
                runtime,
                session,
                NativeRunInput {
                    args,
                    stages: &parsed.stages,
                    invocation,
                    progress_sink,
                },
                guide_help,
            )
        }
        ExternalCommandSource::Plugin => run_external_plugin_command(
            runtime, session, clients, &command, &parsed, invocation, guide_help,
        ),
    }
}

struct NativeRunInput<'args, 'stages, 'invocation, 'sink> {
    args: &'args [String],
    stages: &'stages [String],
    invocation: &'invocation ResolvedInvocation,
    progress_sink: Option<&'sink mut dyn UiSink>,
}

fn run_native_command(
    command: &dyn crate::native::NativeCommand,
    runtime: &mut AppRuntime,
    session: &mut AppSession,
    input: NativeRunInput<'_, '_, '_, '_>,
    guide_help: impl Fn(&str) -> GuideView,
) -> Result<CliCommandResult> {
    let progress_renderer = input.progress_sink.map(|sink| NativeProgressRenderer {
        config: runtime.config.resolved(),
        ui: &input.invocation.ui,
        sink: RefCell::new(sink),
    });
    let mut context = NativeCommandContext::new(
        runtime.config.resolved(),
        runtime_hints_for_invocation(runtime, input.invocation),
    )
    .with_session_context(session.native_context.clone());
    if let Some(renderer) = progress_renderer.as_ref() {
        context = context.with_progress_sink(renderer);
    }

    match command.execute(input.args, &context).map_err(|err| {
        crate::app::report_anyhow_with_context(err, "native command execution failed")
    })? {
        NativeCommandOutcome::Help(text) => Ok(CliCommandResult::guide(guide_help(&text))),
        NativeCommandOutcome::Exit(code) => Ok(CliCommandResult::exit(code)),
        NativeCommandOutcome::Response(response) => render_native_response(*response, input.stages),
        NativeCommandOutcome::ResponseWithExit {
            response,
            exit_code,
        } => {
            let mut result = render_native_response(*response, input.stages)?;
            result.exit_code = exit_code;
            Ok(result)
        }
    }
}

struct NativeProgressRenderer<'a> {
    config: &'a crate::config::ResolvedConfig,
    ui: &'a UiState,
    sink: RefCell<&'a mut dyn UiSink>,
}

impl NativeProgressSink for NativeProgressRenderer<'_> {
    fn emit(&self, event: NativeProgressEvent) -> anyhow::Result<()> {
        let result = cli_result_from_plugin_response(
            crate::core::plugin::ResponseV1 {
                protocol_version: crate::core::plugin::PLUGIN_PROTOCOL_V1,
                ok: true,
                data: event.data,
                error: None,
                messages: event.messages,
                meta: event.meta,
            },
            &[],
        )
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        let mut sink = self.sink.borrow_mut();
        let mut progress_sink = ProgressStderrSink { inner: &mut **sink };
        run_cli_command_with_ui(self.config, self.ui, result, &mut progress_sink)
            .map_err(|err| anyhow::anyhow!(err.to_string()))?;
        Ok(())
    }
}

/// Transient progress must not corrupt the stable final stdout document.
struct ProgressStderrSink<'a> {
    inner: &'a mut dyn UiSink,
}

impl UiSink for ProgressStderrSink<'_> {
    fn write_stdout(&mut self, text: &str) {
        self.inner.write_stderr(text);
    }

    fn write_stderr(&mut self, text: &str) {
        self.inner.write_stderr(text);
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
    invocation: &ResolvedInvocation,
) -> Result<ExternalParse> {
    let parsed = parse_command_tokens_with_aliases(tokens, runtime.config.resolved())?;
    if parsed.tokens.is_empty() {
        return Err(miette!("missing external command"));
    }
    if let Some(help) = completion::maybe_render_dsl_help(
        ReplViewContext::from_parts(runtime, session),
        &parsed.stages,
    ) {
        return Ok(ExternalParse::Handled(Box::new(CliCommandResult::guide(
            GuideView::from_text(&help),
        ))));
    }

    let inline_command = match parse_inline_command_tokens(&parsed.tokens) {
        Ok(command) => command,
        Err(err) => {
            if err.kind() == clap::error::ErrorKind::DisplayHelp
                || err.kind() == clap::error::ErrorKind::DisplayVersion
            {
                let mut view = GuideView::from_text(&err.to_string());
                extend_with_invocation_help(&mut view, invocation.help_level);
                return Ok(ExternalParse::Handled(Box::new(CliCommandResult::guide(
                    view.filtered_for_help_level(invocation.help_level),
                ))));
            }
            return Err(crate::app::report_std_error_with_context(
                err,
                "failed to parse external inline command",
            ));
        }
    };

    Ok(ExternalParse::Invocation(ParsedExternalInvocation {
        tokens: rewrite_shellable_root_help_tokens(
            parsed.tokens,
            invocation.plugin_provider.as_deref(),
        ),
        stages: parsed.stages,
        inline_command,
    }))
}

fn rewrite_shellable_root_help_tokens(
    tokens: Vec<String>,
    provider_override: Option<&str>,
) -> Vec<String> {
    if provider_override.is_some() {
        return tokens;
    }

    if tokens.len() == 1
        && let Some(command) = tokens.first()
        && is_repl_shellable_command(command)
    {
        return vec![command.clone(), "--help".to_string()];
    }
    tokens
}

fn run_external_plugin_command(
    runtime_state: &mut AppRuntime,
    session: &mut AppSession,
    clients: &AppClients,
    command: &str,
    parsed: &ParsedExternalInvocation,
    invocation: &ResolvedInvocation,
    guide_help: impl Fn(&str) -> GuideView,
) -> Result<CliCommandResult> {
    let (_, args) = parsed
        .tokens
        .split_first()
        .ok_or_else(|| miette!("missing external command"))?;
    let (path, dispatch_policy) = clients.plugins().resolved_command_path_and_policy(
        command,
        args,
        invocation.plugin_provider.as_deref(),
    );
    runtime_state
        .auth_mut()
        .overlay_external_policy(dispatch_policy);
    let access_requirement = external_path_access_requirement(args);
    ensure_external_path_access(runtime_state, session, &path, access_requirement)?;
    let runtime = ExternalCommandRuntime::from_parts(runtime_state, clients);

    tracing::debug!(
        command = %path.as_slice().join(" "),
        args = ?args,
        "dispatching external command"
    );

    if access_requirement == ExternalPathAccessRequirement::Visible {
        let dispatch_context = plugin_dispatch_context_for(&runtime, Some(invocation));
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

    let dispatch_context = plugin_dispatch_context_for(&runtime, Some(invocation));
    let response = runtime
        .plugins
        .dispatch(command, args, &dispatch_context)
        .map_err(enrich_dispatch_error)?;

    render_external_plugin_response(response, &parsed.stages)
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

fn external_path_access_requirement(args: &[String]) -> ExternalPathAccessRequirement {
    if is_help_passthrough(args) {
        ExternalPathAccessRequirement::Visible
    } else {
        ExternalPathAccessRequirement::Runnable
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExternalParse, is_help_passthrough, parse_external_invocation,
        render_external_plugin_response, run_external_command_with_help_renderer,
        run_external_command_with_help_renderer_and_progress,
    };
    use crate::app::{
        AppClients, AppRuntime, AppSession, AuthState, ConfigState, LaunchContext, RuntimeContext,
        TerminalKind, UiState, resolve_invocation_ui,
    };
    use crate::app::{BufferedUiSink, CliCommandResult, ReplCommandOutput};
    use crate::cli::invocation::InvocationOptions;
    use crate::config::{ConfigLayer, ConfigResolver, ResolveOptions};
    use crate::core::command_policy::CommandPolicyContext;
    use crate::core::output::OutputFormat;
    use crate::core::plugin::{
        DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
        ResponseMessageLevelV1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
    };
    use crate::guide::GuideView;
    use crate::native::{
        NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
        NativeProgressEvent,
    };
    use crate::plugin::PluginManager;
    use crate::ui::RenderSettings;
    use crate::ui::messages::MessageLevel;
    use clap::Command;
    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    #[cfg(unix)]
    use std::path::Path;
    #[cfg(all(unix, miri))]
    use std::sync::atomic::{AtomicU64, Ordering};
    #[cfg(all(unix, not(miri)))]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[derive(Clone, Copy)]
    enum NativeOutcomeKind {
        Help,
        Exit,
        Response,
        ResponseWithExit,
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
                ..DescribeCommandAuthV1::default()
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
                            column_labels: Vec::new(),
                            row_path: None,
                            preserve_json_document: false,
                        },
                    }))
                }
                NativeOutcomeKind::ResponseWithExit => NativeCommandOutcome::ResponseWithExit {
                    response: Box::new(ResponseV1 {
                        protocol_version: PLUGIN_PROTOCOL_V1,
                        ok: true,
                        data: serde_json::json!({"status": "waiting_approval"}),
                        error: None,
                        messages: Vec::new(),
                        meta: ResponseMetaV1 {
                            format_hint: Some("json".to_string()),
                            preserve_json_document: true,
                            ..ResponseMetaV1::default()
                        },
                    }),
                    exit_code: 3,
                },
            })
        }
    }

    struct ProgressNativeCommand;

    impl NativeCommand for ProgressNativeCommand {
        fn command(&self) -> Command {
            Command::new("progress").about("Emit progress before returning")
        }

        fn auth(&self) -> Option<DescribeCommandAuthV1> {
            Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                ..DescribeCommandAuthV1::default()
            })
        }

        fn execute(
            &self,
            _args: &[String],
            context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            context.emit_progress(
                NativeProgressEvent::new(serde_json::json!({
                    "status": "running",
                    "message": "creating virtual machine",
                }))
                .with_meta(ResponseMetaV1 {
                    format_hint: Some("json".to_string()),
                    preserve_json_document: true,
                    ..ResponseMetaV1::default()
                }),
            )?;
            Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
                protocol_version: PLUGIN_PROTOCOL_V1,
                ok: true,
                data: serde_json::json!({"status": "completed"}),
                error: None,
                messages: Vec::new(),
                meta: ResponseMetaV1 {
                    format_hint: Some("json".to_string()),
                    preserve_json_document: true,
                    ..ResponseMetaV1::default()
                },
            })))
        }
    }

    struct VerbosityNativeCommand;

    impl NativeCommand for VerbosityNativeCommand {
        fn command(&self) -> Command {
            Command::new("verbosity").about("Echo native runtime hints")
        }

        fn auth(&self) -> Option<DescribeCommandAuthV1> {
            Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                ..DescribeCommandAuthV1::default()
            })
        }

        fn execute(
            &self,
            _args: &[String],
            context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
                protocol_version: PLUGIN_PROTOCOL_V1,
                ok: true,
                data: serde_json::json!({
                    "ui_verbosity": context.runtime_hints.ui_verbosity.as_str(),
                }),
                error: None,
                messages: Vec::new(),
                meta: ResponseMetaV1 {
                    format_hint: Some("json".to_string()),
                    columns: None,
                    column_align: Vec::new(),
                    column_labels: Vec::new(),
                    row_path: None,
                    preserve_json_document: false,
                },
            })))
        }
    }

    fn make_test_state_with_registry(
        native_commands: NativeCommandRegistry,
    ) -> (AppRuntime, AppSession, AppClients) {
        make_test_state_with_parts(
            PluginManager::new(Vec::new())
                .with_bundled_roots(false)
                .with_default_roots(false),
            native_commands,
        )
    }

    fn make_test_state_with_parts(
        plugins: PluginManager,
        native_commands: NativeCommandRegistry,
    ) -> (AppRuntime, AppSession, AppClients) {
        let mut defaults = ConfigLayer::default();
        defaults.set("profile.default", "default");
        defaults.set("theme.path", Vec::<String>::new());
        let mut resolver = ConfigResolver::default();
        resolver.set_defaults(defaults);
        let config = resolver
            .resolve(ResolveOptions::default())
            .expect("test config should resolve");
        let themes = crate::ui::theme_catalog::load_theme_catalog(&config);
        let mut auth = AuthState::from_resolved(&config);
        auth.replace_external_policy(native_commands.command_policy_registry());
        let runtime = AppRuntime::new(
            RuntimeContext::new(None, TerminalKind::Cli, None),
            ConfigState::new(config.clone()),
            UiState::new(
                RenderSettings::test_plain(OutputFormat::Json),
                MessageLevel::Success,
                0,
            ),
            auth,
            themes,
            LaunchContext::default(),
        );
        let session = AppSession::builder().build();
        let clients = AppClients::new(plugins, native_commands);
        (runtime, session, clients)
    }

    fn make_test_state_with_native(
        kind: Option<NativeOutcomeKind>,
    ) -> (AppRuntime, AppSession, AppClients) {
        make_test_state_with_registry(
            kind.map(|kind| NativeCommandRegistry::new().with_command(TestNativeCommand { kind }))
                .unwrap_or_default(),
        )
    }

    struct NestedAuthNativeCommand;

    impl NativeCommand for NestedAuthNativeCommand {
        fn command(&self) -> Command {
            Command::new("ldap")
                .about("Directory lookup")
                .arg(
                    clap::Arg::new("format")
                        .long("format")
                        .value_name("FORMAT")
                        .global(true),
                )
                .subcommand(Command::new("user").about("Look up one user"))
        }

        fn describe(&self) -> DescribeCommandV1 {
            let mut root = DescribeCommandV1::from_clap(self.command());
            root.auth = Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::Public),
                required_capabilities: Vec::new(),
                feature_flags: Vec::new(),
                ..DescribeCommandAuthV1::default()
            });
            root.subcommands[0].auth = Some(DescribeCommandAuthV1 {
                visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
                required_capabilities: vec!["ldap.user.read".to_string()],
                feature_flags: Vec::new(),
                ..DescribeCommandAuthV1::default()
            });
            root
        }

        fn execute(
            &self,
            args: &[String],
            _context: &NativeCommandContext<'_>,
        ) -> anyhow::Result<NativeCommandOutcome> {
            if super::is_help_passthrough(args) {
                return Ok(NativeCommandOutcome::Help(
                    "Usage: osp ldap user <UID>".to_string(),
                ));
            }
            panic!("nested-auth native command should not execute when auth denies it")
        }
    }

    #[cfg(unix)]
    fn make_temp_dir(prefix: &str) -> std::path::PathBuf {
        #[cfg(not(miri))]
        let unique = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be after epoch")
                .as_nanos()
        );
        #[cfg(miri)]
        let unique = {
            static NEXT_ID: AtomicU64 = AtomicU64::new(1);
            format!(
                "{prefix}-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            )
        };
        let dir = std::env::temp_dir().join(unique);
        fs::create_dir_all(&dir).expect("temp dir should be created");
        dir
    }

    #[cfg(unix)]
    fn write_provider_test_plugin(dir: &Path, plugin_id: &str, command: &str) {
        let plugin_path = dir.join(format!("osp-{plugin_id}"));
        let script = format!(
            r#"#!/bin/sh
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":null,"commands":[{{"name":"{command}","about":"{plugin_id} provider","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"provider":"{plugin_id}"}},"error":null,"messages":[],"meta":{{"format_hint":"json","columns":["provider"]}}}}
JSON
"#
        );
        fs::write(&plugin_path, script).expect("plugin script should be written");
        let mut perms = fs::metadata(&plugin_path)
            .expect("plugin metadata should be readable")
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&plugin_path, perms).expect("plugin should be executable");
    }

    #[test]
    fn external_builtin_help_passthrough_is_handled_unit() {
        let (runtime, session, _) = make_test_state_with_native(None);
        let tokens = ["config", "--help"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();

        let invocation = crate::app::resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &Default::default(),
        );
        let parsed = parse_external_invocation(&runtime, &session, &tokens, &invocation)
            .expect("help should parse");
        assert!(matches!(
            parsed,
            ExternalParse::Handled(result) if matches!(*result, CliCommandResult {
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
                column_labels: Vec::new(),
                row_path: None,
                preserve_json_document: false,
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
            NativeOutcomeKind::ResponseWithExit,
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
                NativeOutcomeKind::ResponseWithExit => {
                    assert_eq!(result.exit_code, 3);
                    assert!(matches!(result.output, Some(ReplCommandOutput::Output(_))));
                }
            }
        }
    }

    #[test]
    fn external_native_progress_renders_to_stderr_without_corrupting_final_output_unit() {
        let (mut runtime, mut session, clients) = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(ProgressNativeCommand),
        );
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );
        let mut sink = BufferedUiSink::default();

        let result = run_external_command_with_help_renderer_and_progress(
            &mut runtime,
            &mut session,
            &clients,
            &["progress".to_string()],
            &invocation,
            GuideView::from_text,
            Some(&mut sink),
        )
        .expect("native command should emit progress and return");

        assert!(sink.stdout.is_empty());
        assert!(sink.stderr.contains("creating virtual machine"));
        assert_eq!(result.exit_code, 0);
        assert!(matches!(result.output, Some(ReplCommandOutput::Output(_))));
    }

    #[test]
    fn external_native_command_receives_invocation_verbosity_unit() {
        let (mut runtime, mut session, clients) = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(VerbosityNativeCommand),
        );
        let options = InvocationOptions {
            verbose: 1,
            ..InvocationOptions::default()
        };
        let invocation = resolve_invocation_ui(runtime.config.resolved(), &runtime.ui, &options);

        let result = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["verbosity".to_string()],
            &invocation,
            GuideView::from_text,
        )
        .expect("native command should dispatch");

        let Some(ReplCommandOutput::Output(output)) = result.output else {
            panic!("expected structured output");
        };
        let rows = output.output.as_rows().expect("rows");
        assert_eq!(
            rows[0].get("ui_verbosity"),
            Some(&serde_json::json!("info"))
        );
    }

    #[test]
    fn external_bare_shellable_root_rewrites_to_help_unit() {
        let (mut runtime, mut session, clients) =
            make_test_state_with_native(Some(NativeOutcomeKind::Help));
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );
        let result = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["ldap".to_string()],
            &invocation,
            |text| GuideView::from_text(&format!("HELP::{text}")),
        )
        .expect("bare shellable root should rewrite to help");

        assert!(matches!(
            result.output,
            Some(ReplCommandOutput::Output(guide))
                if guide
                    .source_guide
                    .as_ref()
                    .expect("expected semantic guide payload")
                    .preamble
                    .iter()
                    .any(|line| line.contains("Args: --help"))
        ));
    }

    #[test]
    fn external_dispatch_matches_native_commands_case_insensitively_unit() {
        let (mut runtime, mut session, clients) =
            make_test_state_with_native(Some(NativeOutcomeKind::Help));
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );
        let result = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["LDAP".to_string()],
            &invocation,
            |text| GuideView::from_text(&format!("HELP::{text}")),
        )
        .expect("mixed-case native command should dispatch");

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

    #[test]
    fn external_dispatch_enforces_nested_native_command_auth_unit() {
        let (mut runtime, mut session, clients) = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(NestedAuthNativeCommand),
        );
        runtime
            .auth
            .set_policy_context(CommandPolicyContext::default().authenticated(true));

        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );
        let err = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &[
                "ldap".to_string(),
                "--format".to_string(),
                "json".to_string(),
                "user".to_string(),
                "alice".to_string(),
            ],
            &invocation,
            GuideView::from_text,
        )
        .expect_err("nested auth should block native external dispatch");

        assert!(err.to_string().contains("plugin command `ldap user`"));
        assert!(err.to_string().contains("requires additional capabilities"));
    }

    #[test]
    fn external_native_help_requires_visibility_but_not_runnability_unit() {
        let (mut runtime, mut session, clients) = make_test_state_with_registry(
            NativeCommandRegistry::new().with_command(NestedAuthNativeCommand),
        );
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );

        let result = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["ldap".to_string(), "help".to_string(), "user".to_string()],
            &invocation,
            GuideView::from_text,
        )
        .expect("visible nested help should not require its run capability");

        assert!(matches!(result.output, Some(ReplCommandOutput::Output(_))));

        let denied = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["ldap".to_string(), "user".to_string(), "alice".to_string()],
            &invocation,
            GuideView::from_text,
        )
        .expect_err("real execution should still require runnability");
        assert!(denied.to_string().contains("requires authentication"));
    }

    #[test]
    fn external_native_help_keeps_hidden_profile_and_feature_gates_unit() {
        use crate::core::command_policy::{CommandPath, CommandPolicy, VisibilityMode};

        let policies = [
            CommandPolicy::new(CommandPath::new(["ldap", "user"]))
                .visibility(VisibilityMode::Hidden),
            CommandPolicy::new(CommandPath::new(["ldap", "user"])).allow_profiles(["other"]),
            CommandPolicy::new(CommandPath::new(["ldap", "user"])).feature_flag("ldap-user"),
        ];

        for policy in policies {
            let (mut runtime, mut session, clients) = make_test_state_with_registry(
                NativeCommandRegistry::new().with_command(NestedAuthNativeCommand),
            );
            runtime.auth.external_policy_mut().register(policy);
            let invocation = resolve_invocation_ui(
                runtime.config.resolved(),
                &runtime.ui,
                &InvocationOptions::default(),
            );

            run_external_command_with_help_renderer(
                &mut runtime,
                &mut session,
                &clients,
                &["ldap".to_string(), "user".to_string(), "--help".to_string()],
                &invocation,
                GuideView::from_text,
            )
            .expect_err("hidden command help must remain inaccessible");
        }
    }

    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "external plugin filesystem/process test")]
    #[test]
    fn external_dispatch_rejects_native_plugin_source_collisions_unit() {
        let root = make_temp_dir("osp-cli-external-native-collision");
        let plugins_dir = root.join("plugins");
        fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
        write_provider_test_plugin(&plugins_dir, "alpha", "ldap");

        let (mut runtime, mut session, clients) = make_test_state_with_parts(
            PluginManager::new(vec![plugins_dir]),
            NativeCommandRegistry::new().with_command(TestNativeCommand {
                kind: NativeOutcomeKind::Response,
            }),
        );
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );

        let err = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["ldap".to_string()],
            &invocation,
            GuideView::from_text,
        )
        .expect_err("native/plugin collision should fail");

        assert!(err.to_string().contains("ambiguous across command sources"));
        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "external plugin filesystem/process test")]
    #[test]
    fn external_dispatch_requires_provider_selection_before_plugin_run_unit() {
        let root = make_temp_dir("osp-cli-external-provider-selection");
        let plugins_dir = root.join("plugins");
        fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
        write_provider_test_plugin(&plugins_dir, "alpha", "shared");
        write_provider_test_plugin(&plugins_dir, "beta", "shared");

        let (mut runtime, mut session, clients) = make_test_state_with_parts(
            PluginManager::new(vec![plugins_dir]),
            NativeCommandRegistry::default(),
        );
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );

        let err = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["shared".to_string()],
            &invocation,
            GuideView::from_text,
        )
        .expect_err("ambiguous plugin command should fail before dispatch");

        assert!(err.to_string().contains("requires provider selection"));
        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "external plugin filesystem/process test")]
    #[test]
    fn external_dispatch_provider_override_routes_plugin_over_native_unit() {
        let root = make_temp_dir("osp-cli-external-provider-override");
        let plugins_dir = root.join("plugins");
        fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
        write_provider_test_plugin(&plugins_dir, "alpha", "ldap");

        let (mut runtime, mut session, clients) = make_test_state_with_parts(
            PluginManager::new(vec![plugins_dir]),
            NativeCommandRegistry::new().with_command(TestNativeCommand {
                kind: NativeOutcomeKind::Response,
            }),
        );
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions {
                plugin_provider: Some("alpha".to_string()),
                ..InvocationOptions::default()
            },
        );

        let result = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["ldap".to_string()],
            &invocation,
            GuideView::from_text,
        )
        .expect("provider override should route to plugin");

        let rows = match result.output {
            Some(ReplCommandOutput::Output(output)) => output
                .output
                .as_rows()
                .expect("expected row output")
                .to_vec(),
            other => panic!("expected structured output, got {other:?}"),
        };
        assert_eq!(rows, vec![crate::row! { "provider" => "alpha" }]);
        fs::remove_dir_all(root).ok();
    }

    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "external plugin filesystem/process test")]
    #[test]
    fn external_dispatch_matches_plugin_commands_case_insensitively_unit() {
        let root = make_temp_dir("osp-cli-external-plugin-case-insensitive");
        let plugins_dir = root.join("plugins");
        fs::create_dir_all(&plugins_dir).expect("plugin dir should be created");
        write_provider_test_plugin(&plugins_dir, "alpha", "shared");

        let (mut runtime, mut session, clients) = make_test_state_with_parts(
            PluginManager::new(vec![plugins_dir]),
            NativeCommandRegistry::default(),
        );
        let invocation = resolve_invocation_ui(
            runtime.config.resolved(),
            &runtime.ui,
            &InvocationOptions::default(),
        );

        let result = run_external_command_with_help_renderer(
            &mut runtime,
            &mut session,
            &clients,
            &["SHARED".to_string()],
            &invocation,
            GuideView::from_text,
        )
        .expect("mixed-case plugin command should dispatch");

        let rows = match result.output {
            Some(ReplCommandOutput::Output(output)) => output
                .output
                .as_rows()
                .expect("expected row output")
                .to_vec(),
            other => panic!("expected structured output, got {other:?}"),
        };
        assert_eq!(rows, vec![crate::row! { "provider" => "alpha" }]);
        fs::remove_dir_all(root).ok();
    }
}
