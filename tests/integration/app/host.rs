use crate::temp_support::make_temp_dir;
use anyhow::Result;
use clap::Command;
use osp_cli::App;
use osp_cli::app::{BufferedUiSink, StdIoUiSink, UiSink};
use osp_cli::config::ConfigLayer;
use osp_cli::core::command_policy::{
    CommandPath, CommandPolicy, CommandPolicyContext, CommandPolicyRegistry, VisibilityMode,
};
use osp_cli::core::plugin::{
    PLUGIN_PROTOCOL_V1, ResponseMessageLevelV1, ResponseMessageV1, ResponseMetaV1, ResponseV1,
};
use osp_cli::{
    NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry,
    NativeProgressEvent,
};
use serde_json::json;

use super::support::{env_lock, parse_json_output, write_executable_script};

fn with_config_path<T>(config_toml: &str, callback: impl FnOnce() -> T) -> T {
    let _guard = env_lock()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let temp = make_temp_dir("osp-cli-app-host-config");
    let config_path = temp.path().join("config.toml");
    std::fs::write(&config_path, config_toml).expect("config should be written");

    let previous = std::env::var_os("OSP_CONFIG_FILE");
    unsafe {
        std::env::set_var("OSP_CONFIG_FILE", &config_path);
    }

    let result = callback();

    match previous {
        Some(value) => unsafe { std::env::set_var("OSP_CONFIG_FILE", value) },
        None => unsafe { std::env::remove_var("OSP_CONFIG_FILE") },
    }

    result
}

struct NativeProbeCommand;

struct GroupShapedServiceCommand;

impl NativeCommand for GroupShapedServiceCommand {
    fn command(&self) -> Command {
        Command::new("group-shaped")
    }

    fn execute(
        &self,
        _args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!([{"groups": {}, "aggregates": {}, "rows": []}]),
            error: None,
            messages: Vec::new(),
            meta: ResponseMetaV1::default(),
        })))
    }
}

#[test]
fn service_group_field_names_are_not_interpreted_as_dsl_groups() {
    let app = App::builder()
        .with_native_commands(NativeCommandRegistry::new().with_command(GroupShapedServiceCommand))
        .build();
    let mut sink = BufferedUiSink::default();
    assert_eq!(
        app.run_with_sink(
            [
                "osp",
                "--defaults-only",
                "--json",
                "group-shaped",
                "|",
                "A",
                "count()",
                "AS",
                "total"
            ],
            &mut sink
        )
        .unwrap(),
        0
    );
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&sink.stdout).unwrap(),
        json!([{"total": 1}])
    );
}

impl NativeCommand for NativeProbeCommand {
    fn command(&self) -> Command {
        Command::new("native-probe").about("Inspect resolved host config")
    }

    fn execute(
        &self,
        _args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!([{
                "active_profile": context.config.active_profile(),
                "theme": context.config.get_string("theme.name"),
            }]),
            error: None,
            messages: Vec::new(),
            meta: ResponseMetaV1 {
                format_hint: Some("json".to_string()),
                columns: Some(vec!["active_profile".to_string(), "theme".to_string()]),
                column_labels: Vec::new(),
                column_align: Vec::new(),
                row_path: None,
                unix_timestamp_columns: Vec::new(),
                display_rules: Vec::new(),
                preserve_json_document: false,
                presentation_lines: Vec::new(),
                progress_append: Vec::new(),
                progress_replace: false,
            },
        })))
    }
}

fn native_probe_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(NativeProbeCommand)
}

#[test]
fn native_session_context_shares_selection_and_redacts_private_state() {
    let session = osp_cli::NativeSessionContext::default();
    let next_command = session.clone();
    session.set_value("selected_target", "private-resource-ref");
    session.set_prompt_value("selection", "Target", "public-label");
    assert_eq!(
        next_command.value("selected_target").as_deref(),
        Some("private-resource-ref")
    );
    let entries = next_command.prompt_entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].label, "Target");
    assert_eq!(entries[0].value, "public-label");
    let diagnostic = format!("{next_command:?}");
    assert!(diagnostic.contains("[redacted]"));
    assert!(!diagnostic.contains("private-resource-ref"));
    next_command.remove_prompt_value("selection");
    assert!(session.prompt_entries().is_empty());
    assert_eq!(
        session.value("selected_target").as_deref(),
        Some("private-resource-ref")
    );
}

struct SiteStatusCommand;

struct CuratedOrchRowsCommand;

impl NativeCommand for CuratedOrchRowsCommand {
    fn command(&self) -> Command {
        Command::new("orch-view").about("Render a canonical orchestrator page")
    }

    fn execute(
        &self,
        _args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        for state in ["running", "completed"] {
            context.emit_progress(
                NativeProgressEvent::new(json!({"state": state, "name": "db01.uio.no"}))
                    .with_meta(ResponseMetaV1 {
                        presentation_lines: vec![
                            format!("{state}: db01.uio.no"),
                            "└─ Resource preparation with a description long enough to wrap on a narrow terminal".into(),
                        ],
                        progress_append: if state == "running" { vec![format!("{state}: db01.uio.no")] } else { Vec::new() },
                        progress_replace: true,
                        preserve_json_document: true,
                        ..ResponseMetaV1::default()
                    }),
            )?;
        }
        context.flush_progress()?;
        Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!({
                "items": [{
                    "name": "db01.uio.no",
                    "provider": {"name": "vmware"},
                    "state": {"name": "powered_on"},
                    "compute": {"display": "4 CPU / 8 GiB"},
                    "location": {"display": "vcsa-prod"}
                }],
                "page": {"next_cursor": "cursor-2"},
                "targets": [{"target_id": "vmware:prod", "status": "ok"}]
            }),
            error: None,
            messages: vec![
                ResponseMessageV1 {
                    level: ResponseMessageLevelV1::Warning,
                    text: "Results are incomplete".to_string(),
                },
                ResponseMessageV1 {
                    level: ResponseMessageLevelV1::Info,
                    text: "provider evidence".to_string(),
                },
                ResponseMessageV1 {
                    level: ResponseMessageLevelV1::Trace,
                    text: "runtime target vmware:prod".to_string(),
                },
            ],
            meta: ResponseMetaV1 {
                format_hint: Some("table".to_string()),
                columns: Some(vec![
                    "name".to_string(),
                    "provider.name".to_string(),
                    "state.name".to_string(),
                    "compute.display".to_string(),
                    "location.display".to_string(),
                ]),
                column_labels: vec![
                    "NAME".to_string(),
                    "PROVIDER".to_string(),
                    "STATE".to_string(),
                    "COMPUTE".to_string(),
                    "LOCATION".to_string(),
                ],
                column_align: Vec::new(),
                row_path: Some("items".to_string()),
                unix_timestamp_columns: Vec::new(),
                display_rules: Vec::new(),
                preserve_json_document: true,
                presentation_lines: Vec::new(),
                progress_append: Vec::new(),
                progress_replace: false,
            },
        })))
    }
}

impl NativeCommand for SiteStatusCommand {
    fn command(&self) -> Command {
        Command::new("site-status").about("Show wrapper defaults from resolved config")
    }

    fn execute(
        &self,
        _args: &[String],
        context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Response(Box::new(ResponseV1 {
            protocol_version: PLUGIN_PROTOCOL_V1,
            ok: true,
            data: json!([{
                "enabled": context.config.get_bool("extensions.site.enabled").unwrap_or(false),
                "banner": context.config.get_string("extensions.site.banner"),
            }]),
            error: None,
            messages: Vec::new(),
            meta: ResponseMetaV1 {
                format_hint: Some("json".to_string()),
                columns: Some(vec!["enabled".to_string(), "banner".to_string()]),
                column_labels: Vec::new(),
                column_align: Vec::new(),
                row_path: None,
                unix_timestamp_columns: Vec::new(),
                display_rules: Vec::new(),
                preserve_json_document: false,
                presentation_lines: Vec::new(),
                progress_append: Vec::new(),
                progress_replace: false,
            },
        })))
    }
}

fn site_status_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(SiteStatusCommand)
}

fn curated_orch_rows_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(CuratedOrchRowsCommand)
}

struct TerminalCapture {
    output: BufferedUiSink,
    width: usize,
    height: usize,
    resize_height: Option<usize>,
}

impl UiSink for TerminalCapture {
    fn write_stdout(&mut self, text: &str) {
        self.output.stdout.push_str(text);
    }
    fn write_stderr(&mut self, text: &str) {
        self.output.stderr.push_str(text);
        if let Some(height) = self.resize_height.take() {
            self.height = height;
        }
    }
    fn stderr_is_terminal(&self) -> bool {
        true
    }
    fn stderr_width(&self) -> Option<usize> {
        Some(self.width)
    }
    fn stderr_height(&self) -> Option<usize> {
        Some(self.height)
    }
}

#[test]
fn app_host_keeps_orch_documents_pipeable_while_rendering_curated_rows() {
    let app = App::builder()
        .with_native_commands(curated_orch_rows_registry())
        .build();

    if std::env::var_os("OSP_LIVE_OUTPUT_CONTRACT_CHILD").is_some() {
        if !StdIoUiSink.stderr_is_terminal() {
            assert_eq!(StdIoUiSink.stderr_width(), Some(80));
            assert_eq!(StdIoUiSink.stderr_height(), Some(24));
        }
        app.run_with_sink(
            ["osp", "--defaults-only", "--plain", "orch-view"],
            &mut StdIoUiSink,
        )
        .expect("the real terminal should receive progress and the final result");
        return;
    }

    let mut human = BufferedUiSink::default();
    let exit = app
        .run_with_sink(
            ["osp", "--defaults-only", "--plain", "orch-view"],
            &mut human,
        )
        .expect("curated human output should render");
    assert_eq!(exit, 0);
    assert!(human.stdout.contains("name"));
    assert!(human.stdout.contains("provider.name"));
    assert!(human.stdout.contains("4 CPU / 8 GiB"));
    assert!(!human.stdout.contains("next_cursor"));
    assert!(human.stderr.contains("Results are incomplete"));
    assert!(!human.stderr.contains("provider evidence"));
    assert!(!human.stderr.contains("runtime target vmware:prod"));

    let mut json_sink = BufferedUiSink::default();
    let json_args = ["osp", "--defaults-only", "--json", "orch-view"];
    let exit = app
        .run_with_sink(json_args, &mut json_sink)
        .expect("canonical JSON should render");
    assert_eq!(exit, 0);
    let document = parse_json_output(
        "app_host_keeps_orch_documents_pipeable_while_rendering_curated_rows/json",
        &json_args,
        &json_sink.stdout,
        &json_sink.stderr,
    );
    assert_eq!(document["page"]["next_cursor"], "cursor-2");
    assert_eq!(document["items"][0]["provider"]["name"], "vmware");

    let mut piped = BufferedUiSink::default();
    let pipe_args = [
        "osp",
        "--defaults-only",
        "--json",
        "orch-view",
        "|",
        "P",
        "provider.name",
    ];
    let exit = app
        .run_with_sink(pipe_args, &mut piped)
        .expect("DSL should receive canonical collection rows");
    assert_eq!(exit, 0);
    let projected = parse_json_output(
        "app_host_keeps_orch_documents_pipeable_while_rendering_curated_rows/pipeline",
        &pipe_args,
        &piped.stdout,
        &piped.stderr,
    );
    assert_eq!(projected, json!([{"provider":{"name":"vmware"}}]));

    let mut verbose = BufferedUiSink::default();
    app.run_with_sink(["osp", "--defaults-only", "-v", "orch-view"], &mut verbose)
        .expect("normal operator evidence should render at -v");
    assert!(verbose.stderr.contains("provider evidence"));
    assert!(!verbose.stderr.contains("runtime target vmware:prod"));

    let mut trace = BufferedUiSink::default();
    app.run_with_sink(["osp", "--defaults-only", "-vv", "orch-view"], &mut trace)
        .expect("diagnostic evidence should render at -vv");
    assert!(trace.stderr.contains("runtime target vmware:prod"));

    // Live output and the final document share filtering, but occupy separate
    // channels so consuming the final JSON never consumes transient progress.
    assert!(json_sink.stderr.contains("running: db01.uio.no"));
    assert!(json_sink.stderr.contains("completed"));
    assert!(!json_sink.stderr.contains('\x1b'));
    for (width, height, resize_height) in [
        (80, 24, None),
        (120, 24, None),
        (80, 2, None),
        (80, 24, Some(2)),
    ] {
        let mut terminal = TerminalCapture {
            output: BufferedUiSink::default(),
            width,
            height,
            resize_height,
        };
        app.run_with_sink(
            ["osp", "--defaults-only", "--plain", "orch-view"],
            &mut terminal,
        )
        .expect("live presentations should fit the terminal or append safely");
        assert!(terminal.output.stderr.contains("running: db01.uio.no"));
        assert!(terminal.output.stderr.contains("completed: db01.uio.no"));
        assert_eq!(
            terminal.output.stderr.contains("\x1b[2K"),
            height > 2 && resize_height.is_none()
        );
        assert!(terminal.output.stdout.contains("db01.uio.no"));
    }
    let mut filtered = BufferedUiSink::default();
    app.run_with_sink(
        [
            "osp",
            "--defaults-only",
            "--json",
            "orch-view",
            "|",
            "completed",
        ],
        &mut filtered,
    )
    .expect("live events should obey the invocation filter");
    assert!(filtered.stderr.contains("completed"));
    assert!(!filtered.stderr.contains("running"));
    assert!(!filtered.stderr.contains('\x1b'));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&filtered.stdout).unwrap(),
        json!([])
    );

    // Exercise actual terminal dimensions and paging, not just a sink that
    // claims to be a terminal. Even a failed pager must deliver the result.
    #[cfg(unix)]
    {
        use portable_pty::{CommandBuilder, PtySize, native_pty_system};
        use std::io::Read;
        let redirected = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "app::host::app_host_keeps_orch_documents_pipeable_while_rendering_curated_rows",
                "--nocapture",
            ])
            .env("OSP_LIVE_OUTPUT_CONTRACT_CHILD", "1")
            .env("COLUMNS", "80")
            .env("LINES", "24")
            .output()
            .expect("capture redirected host output");
        assert!(redirected.status.success());
        let stdout = String::from_utf8(redirected.stdout).unwrap();
        let stderr = String::from_utf8(redirected.stderr).unwrap();
        assert!(stdout.contains("4 CPU / 8 GiB"));
        assert!(stderr.contains("running: db01.uio.no"));
        assert!(stderr.contains("completed"));
        assert!(!stderr.contains('\x1b'));

        for (height, pager) in [(24, "cat"), (2, "cat"), (2, "exit 1")] {
            let pair = native_pty_system()
                .openpty(PtySize {
                    rows: height,
                    cols: 80,
                    pixel_width: 0,
                    pixel_height: 0,
                })
                .expect("open terminal for the host output contract");
            let mut command = CommandBuilder::new(std::env::current_exe().unwrap());
            command.args([
                "--exact",
                "app::host::app_host_keeps_orch_documents_pipeable_while_rendering_curated_rows",
                "--nocapture",
            ]);
            command.env("OSP_LIVE_OUTPUT_CONTRACT_CHILD", "1");
            command.env("PAGER", pager);
            let mut child = pair
                .slave
                .spawn_command(command)
                .expect("start terminal host");
            drop(pair.slave);
            let mut reader = pair.master.try_clone_reader().unwrap();
            let mut captured = String::new();
            reader
                .read_to_string(&mut captured)
                .expect("read terminal output");
            assert!(child.wait().unwrap().success(), "{captured}");
            assert!(captured.contains("running: db01.uio.no"));
            assert!(captured.contains("completed: db01.uio.no"));
            assert!(captured.contains("4 CPU / 8 GiB"), "{captured}");
        }
    }
}

#[cfg(unix)]
fn write_route_probe_plugin(dir: &std::path::Path, plugin_id: &str, command_name: &str) {
    let plugin_path = dir.join(format!("osp-{plugin_id}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{plugin_id}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{command_name}","about":"{command_name} route probe","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<JSON
{{"protocol_version":1,"ok":true,"data":[{{"profile":"${{OSP_PROFILE:-}}","selected_command":"${{OSP_COMMAND:-}}","arg0":"${{1:-}}","arg1":"${{2:-}}"}}],"error":null,"meta":{{"format_hint":"json"}}}}
JSON
"#,
        plugin_id = plugin_id,
        command_name = command_name,
    );
    write_executable_script(&plugin_path, &script);
}

#[test]
fn app_host_surfaces_native_commands_in_help_and_dispatch() {
    let app = App::builder()
        .with_native_commands(native_probe_registry())
        .build();
    let help_args = ["osp", "--defaults-only", "--help"];

    let mut help_sink = BufferedUiSink::default();
    let exit = app
        .run_with_sink(help_args, &mut help_sink)
        .expect("help should render");
    assert_eq!(exit, 0);
    assert!(help_sink.stdout.contains("native-probe"));
    assert!(help_sink.stdout.contains("Inspect resolved host config"));

    let dispatch_args = ["osp", "--defaults-only", "--json", "native-probe"];
    let mut dispatch_sink = BufferedUiSink::default();
    let exit = app
        .run_with_sink(dispatch_args, &mut dispatch_sink)
        .expect("native command should dispatch");
    assert_eq!(exit, 0);

    let payload = parse_json_output(
        "app_host_surfaces_native_commands_in_help_and_dispatch/native",
        &dispatch_args,
        &dispatch_sink.stdout,
        &dispatch_sink.stderr,
    );
    let rows = payload
        .as_array()
        .expect("native command output should be row array");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["active_profile"], "default");
}

#[test]
fn app_host_merges_active_profile_into_product_policy_context() {
    with_config_path(
        r#"
[default]
profile.default = "uio"

[profile.uio]

[profile.tsd]
"#,
        || {
            let mut policy = CommandPolicyRegistry::new();
            policy.register(
                CommandPolicy::new(CommandPath::new(["config"]))
                    .visibility(VisibilityMode::Authenticated)
                    .allow_profiles(["tsd"]),
            );
            let app = App::builder()
                .with_builtin_policy(policy)
                .with_policy_context(CommandPolicyContext::default().authenticated(true))
                .build();

            let mut default_sink = BufferedUiSink::default();
            app.run_with_sink(["osp", "config", "get", "theme.name"], &mut default_sink)
                .expect_err("default profile should be outside the policy allowlist");

            let mut selected_sink = BufferedUiSink::default();
            let selected_exit = app
                .run_with_sink(
                    ["osp", "--profile", "tsd", "config", "get", "theme.name"],
                    &mut selected_sink,
                )
                .expect("selected-profile access should be evaluated");
            assert_eq!(selected_exit, 0, "{}", selected_sink.stderr);
            assert!(selected_sink.stdout.contains("theme.name"));
        },
    );
}

#[test]
fn app_value_api_layers_product_defaults_and_process_style_failures() {
    let mut product_defaults = ConfigLayer::default();
    product_defaults.set("extensions.site.enabled", true);
    product_defaults.set_for_terminal("cli", "extensions.site.banner", "cli-wrapper");

    let app = App::new()
        .with_native_commands(site_status_registry())
        .with_product_defaults(product_defaults);

    let help_args = ["osp", "--defaults-only", "--help"];
    let mut help_sink = BufferedUiSink::default();
    let help_exit = app.run_process_with_sink(help_args, &mut help_sink);
    assert_eq!(help_exit, 0);

    let status_args = ["osp", "--json", "--defaults-only", "site-status"];
    let mut status_sink = BufferedUiSink::default();
    let status_exit = app.run_process_with_sink(status_args, &mut status_sink);
    assert_eq!(status_exit, 0);
    let payload = parse_json_output(
        "app_value_api_layers_product_defaults_and_process_style_failures/status",
        &status_args,
        &status_sink.stdout,
        &status_sink.stderr,
    );
    let rows = payload
        .as_array()
        .expect("site status output should be a row array");
    assert_eq!(rows[0]["enabled"], true);
    assert_eq!(rows[0]["banner"], "cli-wrapper");

    let mut invalid_sink = BufferedUiSink::default();
    let invalid_args = [
        "osp",
        "--defaults-only",
        "--quiet",
        "--definitely-not-a-flag",
    ];
    let invalid_exit = app.run_process_with_sink(invalid_args, &mut invalid_sink);
    assert_ne!(invalid_exit, 0);
    assert!(invalid_sink.stdout.is_empty());
    assert!(!invalid_sink.stderr.is_empty());
    assert!(
        invalid_sink.stderr.contains("definitely-not-a-flag")
            || invalid_sink.stderr.contains("unexpected argument")
    );
}

#[test]
fn app_host_passes_default_and_selected_profiles_into_native_context() {
    with_config_path(
        r#"[default]
profile.default = "uio"
theme.name = "nord"

[profile.tsd]
theme.name = "dracula"
"#,
        || {
            let app = App::builder()
                .with_native_commands(native_probe_registry())
                .build();

            let default_args = ["osp", "--no-env", "--json", "native-probe"];
            let mut default_sink = BufferedUiSink::default();
            let exit = app
                .run_with_sink(default_args, &mut default_sink)
                .expect("default profile command should run");
            assert_eq!(exit, 0);
            let payload = parse_json_output(
                "app_host_passes_default_and_selected_profiles_into_native_context/default",
                &default_args,
                &default_sink.stdout,
                &default_sink.stderr,
            );
            let rows = payload
                .as_array()
                .expect("default output should be row array");
            assert_eq!(rows[0]["active_profile"], "uio");
            assert_eq!(rows[0]["theme"], "nord");

            let explicit_args = [
                "osp",
                "--no-env",
                "--profile",
                "tsd",
                "--json",
                "native-probe",
            ];
            let mut explicit_sink = BufferedUiSink::default();
            let exit = app
                .run_with_sink(explicit_args, &mut explicit_sink)
                .expect("explicit profile command should run");
            assert_eq!(exit, 0);
            let payload = parse_json_output(
                "app_host_passes_default_and_selected_profiles_into_native_context/explicit",
                &explicit_args,
                &explicit_sink.stdout,
                &explicit_sink.stderr,
            );
            let rows = payload
                .as_array()
                .expect("explicit output should be row array");
            assert_eq!(rows[0]["active_profile"], "tsd");
            assert_eq!(rows[0]["theme"], "dracula");

            let positional_args = ["osp", "--no-env", "--profile=tsd", "--json", "native-probe"];
            let mut positional_sink = BufferedUiSink::default();
            let exit = app
                .run_with_sink(positional_args, &mut positional_sink)
                .expect("attached profile flag command should run");
            assert_eq!(exit, 0);
            let payload = parse_json_output(
                "app_host_passes_default_and_selected_profiles_into_native_context/positional",
                &positional_args,
                &positional_sink.stdout,
                &positional_sink.stderr,
            );
            let rows = payload
                .as_array()
                .expect("positional output should be row array");
            assert_eq!(rows[0]["active_profile"], "tsd");
            assert_eq!(rows[0]["theme"], "dracula");
        },
    );
}

#[test]
fn app_host_projects_native_commands_into_repl_completion_surface() {
    let app = App::builder()
        .with_native_commands(native_probe_registry())
        .build();

    let args = [
        "osp",
        "--json",
        "--defaults-only",
        "repl",
        "debug-complete",
        "--line",
        "native-",
    ];
    let mut sink = BufferedUiSink::default();
    let exit = app
        .run_with_sink(args, &mut sink)
        .expect("debug-complete should run");
    assert_eq!(exit, 0);
    assert!(sink.stderr.is_empty());

    let payload = parse_json_output(
        "app_host_projects_native_commands_into_repl_completion_surface",
        &args,
        &sink.stdout,
        &sink.stderr,
    );
    let matches = payload["matches"]
        .as_array()
        .expect("matches should render as an array");
    assert!(matches.iter().any(|item| item["label"] == "native-probe"));
}

#[cfg(unix)]
#[test]
fn app_host_routes_attached_and_separate_profile_flags_the_same_for_external_commands() {
    with_config_path(
        r#"[default]
profile.default = "uio"

[profile.tsd]
theme.name = "dracula"
"#,
        || {
            let dir = make_temp_dir("osp-cli-app-host-route-plugin");
            write_route_probe_plugin(dir.path(), "route-probe", "route-probe");
            let plugin_dir = dir.to_str().expect("plugin dir should be utf-8");
            let app = App::builder().build();

            let explicit_args = [
                "osp",
                "--json",
                "--no-env",
                "--plugin-dir",
                plugin_dir,
                "--profile",
                "tsd",
                "route-probe",
                "hello",
            ];
            let mut explicit_sink = BufferedUiSink::default();
            let exit = app
                .run_with_sink(explicit_args, &mut explicit_sink)
                .expect("explicit profile command should run");
            assert_eq!(exit, 0);

            let positional_args = [
                "osp",
                "--json",
                "--no-env",
                "--plugin-dir",
                plugin_dir,
                "--profile=tsd",
                "route-probe",
                "hello",
            ];
            let mut positional_sink = BufferedUiSink::default();
            let exit = app
                .run_with_sink(positional_args, &mut positional_sink)
                .expect("attached profile flag command should run");
            assert_eq!(exit, 0);

            let explicit = parse_json_output(
                "app_host_routes_attached_and_separate_profile_flags_the_same_for_external_commands/explicit",
                &explicit_args,
                &explicit_sink.stdout,
                &explicit_sink.stderr,
            );
            let positional = parse_json_output(
                "app_host_routes_attached_and_separate_profile_flags_the_same_for_external_commands/positional",
                &positional_args,
                &positional_sink.stdout,
                &positional_sink.stderr,
            );
            let explicit_row = explicit
                .as_array()
                .expect("explicit payload should be a row array")
                .first()
                .expect("explicit payload should contain one row");
            let positional_row = positional
                .as_array()
                .expect("positional payload should be a row array")
                .first()
                .expect("positional payload should contain one row");
            assert_eq!(explicit_row["profile"], "tsd");
            assert_eq!(positional_row["profile"], "tsd");
            assert_eq!(explicit_row["selected_command"], "route-probe");
            assert_eq!(positional_row["selected_command"], "route-probe");
            assert_eq!(explicit_row["arg0"], positional_row["arg0"]);
            assert_eq!(explicit_row["arg1"], positional_row["arg1"]);
        },
    );
}

#[cfg(unix)]
#[test]
fn app_host_keeps_unknown_leading_token_as_command_instead_of_profile() {
    with_config_path(
        r#"[default]
profile.default = "uio"

[profile.tsd]
theme.name = "dracula"
"#,
        || {
            let dir = make_temp_dir("osp-cli-app-host-unknown-profile-token");
            write_route_probe_plugin(dir.path(), "prod", "prod");
            let plugin_dir = dir.to_str().expect("plugin dir should be utf-8");
            let app = App::builder().build();

            let mut sink = BufferedUiSink::default();
            let args = [
                "osp",
                "--json",
                "--no-env",
                "--plugin-dir",
                plugin_dir,
                "prod",
            ];
            let exit = app
                .run_with_sink(args, &mut sink)
                .expect("unknown leading token command should run");
            assert_eq!(exit, 0);

            let payload = parse_json_output(
                "app_host_keeps_unknown_leading_token_as_command_instead_of_profile",
                &args,
                &sink.stdout,
                &sink.stderr,
            );
            let row = payload
                .as_array()
                .expect("payload should be a row array")
                .first()
                .expect("payload should contain one row");
            assert_eq!(row["selected_command"], "prod");
            assert_eq!(row["profile"], "uio");
        },
    );
}
