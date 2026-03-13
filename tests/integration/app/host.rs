use crate::temp_support::make_temp_dir;
use anyhow::Result;
use clap::Command;
use osp_cli::App;
use osp_cli::app::BufferedUiSink;
use osp_cli::config::ConfigLayer;
use osp_cli::core::plugin::{PLUGIN_PROTOCOL_V1, ResponseMetaV1, ResponseV1};
use osp_cli::{NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry};
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
                column_align: Vec::new(),
            },
        })))
    }
}

fn native_probe_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(NativeProbeCommand)
}

struct SiteStatusCommand;

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
                column_align: Vec::new(),
            },
        })))
    }
}

fn site_status_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(SiteStatusCommand)
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

            let positional_args = ["osp", "--no-env", "tsd", "--json", "native-probe"];
            let mut positional_sink = BufferedUiSink::default();
            let exit = app
                .run_with_sink(positional_args, &mut positional_sink)
                .expect("positional profile command should run");
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
fn app_host_routes_explicit_and_positional_profiles_the_same_for_external_commands() {
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
                "tsd",
                "route-probe",
                "hello",
            ];
            let mut positional_sink = BufferedUiSink::default();
            let exit = app
                .run_with_sink(positional_args, &mut positional_sink)
                .expect("positional profile command should run");
            assert_eq!(exit, 0);

            let explicit = parse_json_output(
                "app_host_routes_explicit_and_positional_profiles_the_same_for_external_commands/explicit",
                &explicit_args,
                &explicit_sink.stdout,
                &explicit_sink.stderr,
            );
            let positional = parse_json_output(
                "app_host_routes_explicit_and_positional_profiles_the_same_for_external_commands/positional",
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
