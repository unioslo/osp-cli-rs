use super::command_output::{
    CommandRenderRuntime, apply_output_stages, parse_output_format_hint, run_cli_command,
};
use super::help::parse_help_render_overrides;
use super::{
    CliCommandResult, authorized_command_catalog_for, bootstrap_message_verbosity, command_output,
    plugin_dispatch_context_for_runtime, resolve_runtime_config, run_cli_command_with_ui, run_from,
    run_from_with_sink,
};
use super::{
    EXIT_CODE_CONFIG, EXIT_CODE_PLUGIN, EXIT_CODE_USAGE, PluginConfigEntry, PluginConfigScope,
    ReplCommandOutput, RunAction, RuntimeConfigRequest, build_cli_session_layer,
    build_dispatch_plan, classify_exit_code, collect_plugin_config_env, config_value_to_plugin_env,
    enrich_dispatch_error, is_sensitive_key, plugin_config_env_name, plugin_path_discovery_enabled,
    plugin_process_timeout, render_report_message, resolve_invocation_ui,
    resolve_render_settings_with_hint, run_inline_builtin_command,
};
use crate::app::sink::BufferedUiSink;
use crate::app::{AppState, AppStateInit, LaunchContext, RuntimeContext, TerminalKind};
use crate::cli::commands::doctor as doctor_cmd;
use crate::cli::invocation::{InvocationOptions, scan_cli_argv};
use crate::cli::{Cli, Commands, ConfigCommands, PluginsCommands, ThemeCommands};
use crate::config::{ConfigLayer, ConfigResolver, ConfigValue, ResolveOptions, RuntimeLoadOptions};
use crate::core::command_policy::{CommandPath, VisibilityMode};
use crate::core::output::{ColorMode, OutputFormat, RenderMode, UnicodeMode};
use crate::core::plugin::{
    DescribeCommandAuthV1, DescribeCommandV1, DescribeVisibilityModeV1, PLUGIN_PROTOCOL_V1,
    ResponseErrorV1, ResponseMessageLevelV1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
};
use crate::guide::GuideView;
use crate::plugin::{
    CommandCatalogEntry, DEFAULT_PLUGIN_PROCESS_TIMEOUT_MS, PluginDispatchError, PluginManager,
    PluginSource,
};
use crate::repl;
use crate::repl::{HistoryConfig, HistoryShellContext, SharedHistory};
use crate::repl::{completion, dispatch as repl_dispatch, help as repl_help, surface};
use crate::ui::build_presentation_defaults_layer;
use crate::ui::messages::{MessageBuffer, MessageLevel};
use crate::ui::{RenderSettings, render_output};
use crate::{NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry};
use clap::Command;
use clap::Parser;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;

mod app_runtime;
mod cli_dispatch;
mod command_surfaces;
mod plugin_config;
mod presentation;
mod repl_completion;
#[cfg(unix)]
mod repl_runtime;

fn profiles(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

fn repl_view<'a>(
    runtime: &'a crate::app::AppRuntime,
    session: &'a crate::app::AppSession,
) -> repl::ReplViewContext<'a> {
    repl::ReplViewContext::from_parts(runtime, session)
}

fn make_completion_state(auth_visible_builtins: Option<&str>) -> AppState {
    make_completion_state_with_entries_and_native(
        auth_visible_builtins,
        &[],
        NativeCommandRegistry::default(),
    )
}

fn make_completion_state_with_entries(
    auth_visible_builtins: Option<&str>,
    entries: &[(&str, &str)],
) -> AppState {
    make_completion_state_with_entries_and_native(
        auth_visible_builtins,
        entries,
        NativeCommandRegistry::default(),
    )
}

fn make_completion_state_with_entries_and_native(
    auth_visible_builtins: Option<&str>,
    entries: &[(&str, &str)],
    native_commands: NativeCommandRegistry,
) -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    if let Some(allowlist) = auth_visible_builtins {
        defaults.set("auth.visible.builtins", allowlist);
    }
    for (key, value) in entries {
        defaults.set(*key, *value);
    }
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let options = ResolveOptions::default().with_terminal("repl");
    let base = resolver
        .resolve(options.clone())
        .expect("base test config should resolve");
    resolver.set_presentation(build_presentation_defaults_layer(&base));
    let config = resolver
        .resolve(options)
        .expect("test config should resolve");

    let settings = RenderSettings::test_plain(OutputFormat::Json);

    AppState::new(AppStateInit {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        render_settings: settings,
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
        plugins: PluginManager::new(Vec::new()),
        native_commands,
        themes: crate::ui::theme_catalog::ThemeCatalog::default(),
        launch: LaunchContext::default(),
    })
}

struct TestNativeCommand;

struct ProductDefaultsCommand;

impl NativeCommand for TestNativeCommand {
    fn command(&self) -> Command {
        Command::new("ldap")
            .about("Directory lookup")
            .subcommand(Command::new("user").about("Look up a user"))
    }

    fn auth(&self) -> Option<DescribeCommandAuthV1> {
        Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::Public),
            required_capabilities: Vec::new(),
            feature_flags: Vec::new(),
        })
    }

    fn describe(&self) -> DescribeCommandV1 {
        let mut describe = DescribeCommandV1::from_clap(self.command());
        describe.auth = self.auth();
        describe.subcommands[0].auth = Some(DescribeCommandAuthV1 {
            visibility: Some(DescribeVisibilityModeV1::CapabilityGated),
            required_capabilities: vec!["ldap.user.read".to_string()],
            feature_flags: Vec::new(),
        });
        describe
    }

    fn execute(
        &self,
        args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> anyhow::Result<NativeCommandOutcome> {
        if args.iter().any(|arg| arg == "--help") {
            return Ok(NativeCommandOutcome::Help(
                "Usage: osp ldap [COMMAND]\n\nCommands:\n  user  Look up a user\n".to_string(),
            ));
        }

        Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!([{ "command": "ldap", "args": args }]),
            error: None,
            messages: Vec::new(),
            meta: ResponseMetaV1 {
                format_hint: Some("table".to_string()),
                columns: Some(vec!["command".to_string(), "args".to_string()]),
                column_align: Vec::new(),
            },
        })))
    }
}

fn test_native_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(TestNativeCommand)
}

impl NativeCommand for ProductDefaultsCommand {
    fn command(&self) -> Command {
        Command::new("site-status").about("Show wrapper defaults from resolved config")
    }

    fn execute(
        &self,
        _args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> anyhow::Result<NativeCommandOutcome> {
        let enabled = context
            .config
            .get_bool("extensions.site.enabled")
            .unwrap_or(false);
        let profile_banner = context
            .config
            .get_string("extensions.site.banner")
            .unwrap_or("missing");

        Ok(NativeCommandOutcome::Help(format!(
            "site_enabled={enabled}\nsite_banner={profile_banner}\nactive_profile={}\n",
            context.config.active_profile()
        )))
    }
}

fn product_defaults_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(ProductDefaultsCommand)
}

fn test_config(entries: &[(&str, &str)]) -> crate::config::ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    for (key, value) in entries {
        defaults.set(*key, *value);
    }

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let options = ResolveOptions::default().with_terminal("cli");
    let base = resolver
        .resolve(options.clone())
        .expect("base test config should resolve");
    resolver.set_presentation(build_presentation_defaults_layer(&base));
    resolver
        .resolve(options)
        .expect("test config should resolve")
}

fn render_prompt_snapshot(entries: &[(&str, &str)]) -> String {
    let state = make_completion_state_with_entries(None, entries);
    crate::repl::presentation::build_repl_prompt(repl_view(&state.runtime, &state.session)).left
}

fn sample_catalog() -> Vec<CommandCatalogEntry> {
    vec![CommandCatalogEntry {
        name: "orch".to_string(),
        about: "Provision orchestrator resources".to_string(),
        auth: None,
        subcommands: vec!["provision".to_string(), "status".to_string()],
        completion: crate::completion::CommandSpec {
            name: "orch".to_string(),
            tooltip: Some("Provision orchestrator resources".to_string()),
            subcommands: vec![
                crate::completion::CommandSpec::new("provision"),
                crate::completion::CommandSpec::new("status"),
            ],
            ..crate::completion::CommandSpec::default()
        },
        provider: Some("mock-provider".to_string()),
        providers: vec!["mock-provider (explicit)".to_string()],
        conflicted: false,
        requires_selection: false,
        selected_explicitly: false,
        source: Some(PluginSource::Explicit),
    }]
}

fn sample_catalog_with_provision_context() -> Vec<CommandCatalogEntry> {
    vec![CommandCatalogEntry {
        name: "orch".to_string(),
        about: "Provision orchestrator resources".to_string(),
        auth: None,
        subcommands: vec!["provision".to_string(), "status".to_string()],
        completion: crate::completion::CommandSpec {
            name: "orch".to_string(),
            tooltip: Some("Provision orchestrator resources".to_string()),
            subcommands: vec![
                crate::completion::CommandSpec::new("provision")
                    .arg(
                        crate::completion::ArgNode::named("guest")
                            .tooltip("Guest name for the provision request"),
                    )
                    .arg(
                        crate::completion::ArgNode::named("image")
                            .tooltip("Base image to provision")
                            .suggestions([
                                crate::completion::SuggestionEntry::from("ubuntu"),
                                crate::completion::SuggestionEntry::from("alma"),
                            ]),
                    )
                    .flag(
                        "--provider",
                        crate::completion::FlagNode::new().suggestions([
                            crate::completion::SuggestionEntry::from("vmware"),
                            crate::completion::SuggestionEntry::from("nrec"),
                        ]),
                    )
                    .flag(
                        "--os",
                        crate::completion::FlagNode {
                            suggestions: vec![
                                crate::completion::SuggestionEntry::from("rhel"),
                                crate::completion::SuggestionEntry::from("alma"),
                            ],
                            suggestions_by_provider: BTreeMap::from([
                                (
                                    "vmware".to_string(),
                                    vec![crate::completion::SuggestionEntry::from("rhel")],
                                ),
                                (
                                    "nrec".to_string(),
                                    vec![crate::completion::SuggestionEntry::from("alma")],
                                ),
                            ]),
                            ..crate::completion::FlagNode::default()
                        },
                    ),
                crate::completion::CommandSpec::new("status"),
            ],
            ..crate::completion::CommandSpec::default()
        },
        provider: Some("mock-provider".to_string()),
        providers: vec!["mock-provider (explicit)".to_string()],
        conflicted: false,
        requires_selection: false,
        selected_explicitly: false,
        source: Some(PluginSource::Explicit),
    }]
}

fn sample_conflicted_catalog() -> Vec<CommandCatalogEntry> {
    vec![CommandCatalogEntry {
        name: "hello".to_string(),
        about: "hello plugin".to_string(),
        auth: None,
        subcommands: Vec::new(),
        completion: crate::completion::CommandSpec {
            name: "hello".to_string(),
            tooltip: Some("hello plugin".to_string()),
            ..crate::completion::CommandSpec::default()
        },
        provider: None,
        providers: vec![
            "alpha-provider (env)".to_string(),
            "beta-provider (user)".to_string(),
        ],
        conflicted: true,
        requires_selection: true,
        selected_explicitly: false,
        source: None,
    }]
}

fn layer_value<'a>(layer: &'a ConfigLayer, key: &str) -> Option<&'a ConfigValue> {
    layer
        .entries()
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| &entry.value)
}

#[cfg(unix)]
fn make_test_history(state: &mut AppState) -> SharedHistory {
    let history_dir = make_temp_dir("osp-cli-test-history");
    let history_path = history_dir.join("history.jsonl");
    let history_shell = state.session.history_shell.clone();
    state.sync_history_shell_context();

    let history_config = HistoryConfig {
        path: Some(history_path),
        max_entries: 128,
        enabled: true,
        dedupe: true,
        profile_scoped: true,
        exclude_patterns: Vec::new(),
        profile: Some(state.runtime.config.resolved().active_profile().to_string()),
        terminal: Some(
            state
                .runtime
                .context
                .terminal_kind()
                .as_config_terminal()
                .to_string(),
        ),
        shell_context: history_shell,
    }
    .normalized();

    SharedHistory::new(history_config)
}

#[cfg(unix)]
fn make_test_state(plugin_dirs: Vec<std::path::PathBuf>) -> AppState {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    let config = resolver
        .resolve(ResolveOptions::default().with_terminal("repl"))
        .expect("test config should resolve");

    let settings = RenderSettings::test_plain(OutputFormat::Json);

    let config_root = make_temp_dir("osp-cli-test-config");
    let cache_root = make_temp_dir("osp-cli-test-cache");
    let launch = LaunchContext::default()
        .with_plugin_dirs(plugin_dirs.clone())
        .with_config_root(Some(config_root.to_path_buf()))
        .with_cache_root(Some(cache_root.to_path_buf()))
        .with_runtime_load(RuntimeLoadOptions::default());

    AppState::new(AppStateInit {
        context: RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
        render_settings: settings,
        message_verbosity: MessageLevel::Success,
        debug_verbosity: 0,
        plugins: PluginManager::new(plugin_dirs).with_roots(
            Some(config_root.to_path_buf()),
            Some(cache_root.to_path_buf()),
        ),
        native_commands: crate::native::NativeCommandRegistry::default(),
        themes: crate::ui::theme_catalog::ThemeCatalog::default(),
        launch,
    })
}

#[cfg(unix)]
fn with_temp_config_paths<T>(callback: impl FnOnce() -> T) -> T {
    let _guard = crate::tests::env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let root = make_temp_dir("osp-cli-test-config-paths");
    let config_path = root.join("config.toml");
    let secrets_path = root.join("secrets.toml");
    let previous_config = std::env::var_os("OSP_CONFIG_FILE");
    let previous_secrets = std::env::var_os("OSP_SECRETS_FILE");

    unsafe {
        std::env::set_var("OSP_CONFIG_FILE", &config_path);
        std::env::set_var("OSP_SECRETS_FILE", &secrets_path);
    }

    let result = callback();

    match previous_config {
        Some(value) => unsafe { std::env::set_var("OSP_CONFIG_FILE", value) },
        None => unsafe { std::env::remove_var("OSP_CONFIG_FILE") },
    }
    match previous_secrets {
        Some(value) => unsafe { std::env::set_var("OSP_SECRETS_FILE", value) },
        None => unsafe { std::env::remove_var("OSP_SECRETS_FILE") },
    }
    result
}

#[cfg(unix)]
fn write_pipeline_test_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-hello");
    std::fs::write(
            &plugin_path,
            r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"hello","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"hello","about":"hello plugin","args":[],"flags":{},"subcommands":[]}]}
JSON
  exit 0
fi

cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"hello-from-plugin"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON
"#,
        )
        .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_auth_pipeline_test_plugin(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join("osp-orch");
    std::fs::write(
        &plugin_path,
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{"protocol_version":1,"plugin_id":"orch","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{"name":"orch","about":"orch plugin","auth":{"visibility":"authenticated"},"args":[],"flags":{},"subcommands":[{"name":"approval","about":"approval","args":[],"flags":{},"subcommands":[{"name":"decide","about":"decide","auth":{"visibility":"capability_gated","required_capabilities":["orch.approval.decide"]},"args":[],"flags":{},"subcommands":[]}]}]}]}
JSON
  exit 0
fi

cat <<'JSON'
{"protocol_version":1,"ok":true,"data":{"message":"ok"},"error":null,"meta":{"format_hint":"table","columns":["message"]}}
JSON
"#,
    )
    .expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn write_provider_test_plugin(
    dir: &std::path::Path,
    plugin_id: &str,
    command_name: &str,
    message: &str,
) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{plugin_id} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"{message}-from-plugin"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
        message = message,
    );
    std::fs::write(&plugin_path, script).expect("plugin script should be written");
    let mut perms = std::fs::metadata(&plugin_path)
        .expect("metadata should be readable")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&plugin_path, perms).expect("script should be executable");
    plugin_path
}

#[cfg(unix)]
fn make_temp_dir(prefix: &str) -> crate::tests::TestTempDir {
    crate::tests::make_temp_dir(prefix)
}
