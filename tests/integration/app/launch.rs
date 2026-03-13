use crate::temp_support::make_temp_dir;
use anyhow::Result;
use clap::Command;
use osp_cli::app::{AppSession, AppStateBuilder, LaunchContext, RuntimeContext, TerminalKind};
use osp_cli::config::{ConfigLayer, ConfigResolver, ResolveOptions};
use osp_cli::core::command_policy::{CommandPath, VisibilityMode};
use osp_cli::{NativeCommand, NativeCommandContext, NativeCommandOutcome, NativeCommandRegistry};

use super::support::{with_path_prefix, write_executable_script};

fn resolved_config(entries: &[(&str, &str)]) -> osp_cli::config::ResolvedConfig {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "default");
    for (key, value) in entries {
        defaults.set(*key, (*value).to_string());
    }

    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    resolver
        .resolve(ResolveOptions::default().with_terminal("cli"))
        .expect("config should resolve")
}

#[cfg(unix)]
fn write_named_plugin(dir: &std::path::Path, name: &str) {
    let plugin_path = dir.join(format!("osp-{name}"));
    let script = format!(
        r#"#!/bin/sh
PATH=/usr/bin:/bin:$PATH
if [ "$1" = "--describe" ]; then
  cat <<'JSON'
{{"protocol_version":1,"plugin_id":"{name}","plugin_version":"0.1.0","min_osp_version":"0.1.0","commands":[{{"name":"{name}","about":"{name} plugin","args":[],"flags":{{}},"subcommands":[]}}]}}
JSON
  exit 0
fi

cat <<'JSON'
{{"protocol_version":1,"ok":true,"data":{{"message":"ok"}},"error":null,"meta":{{"format_hint":"table","columns":["message"]}}}}
JSON
"#,
        name = name,
    );
    write_executable_script(&plugin_path, &script);
}

struct NativeLaunchProbe;

impl NativeCommand for NativeLaunchProbe {
    fn command(&self) -> Command {
        Command::new("launch-native").about("Launch-aware native command")
    }

    fn auth(&self) -> Option<osp_cli::core::plugin::DescribeCommandAuthV1> {
        Some(osp_cli::core::plugin::DescribeCommandAuthV1 {
            visibility: Some(osp_cli::core::plugin::DescribeVisibilityModeV1::Authenticated),
            required_capabilities: Vec::new(),
            feature_flags: vec!["launch".to_string()],
        })
    }

    fn execute(
        &self,
        _args: &[String],
        _context: &NativeCommandContext<'_>,
    ) -> Result<NativeCommandOutcome> {
        Ok(NativeCommandOutcome::Exit(0))
    }
}

fn launch_native_registry() -> NativeCommandRegistry {
    NativeCommandRegistry::new().with_command(NativeLaunchProbe)
}

#[cfg(unix)]
#[test]
fn app_state_builder_uses_launch_context_for_plugin_roots_and_path_discovery() {
    let explicit_dir = make_temp_dir("osp-cli-launch-explicit");
    let path_dir = make_temp_dir("osp-cli-launch-path");
    let config_root = make_temp_dir("osp-cli-launch-config-root");
    let cache_root = make_temp_dir("osp-cli-launch-cache-root");
    write_named_plugin(explicit_dir.path(), "explicit-probe");
    write_named_plugin(path_dir.path(), "path-probe");

    let config = resolved_config(&[("extensions.plugins.discovery.path", "true")]);
    let launch = LaunchContext::default()
        .with_plugin_dir(explicit_dir.path().to_path_buf())
        .with_config_root(Some(config_root.path().to_path_buf()))
        .with_cache_root(Some(cache_root.path().to_path_buf()));

    with_path_prefix(path_dir.path(), || {
        let state = AppStateBuilder::from_resolved_config(
            RuntimeContext::new(None, TerminalKind::Cli, None),
            config.clone(),
        )
        .expect("app state builder should derive host inputs")
        .with_launch(launch.clone())
        .build();

        assert_eq!(
            state.runtime.launch.plugin_dirs,
            vec![explicit_dir.path().to_path_buf()]
        );
        assert_eq!(
            state.clients.plugins().config_root(),
            Some(config_root.path())
        );
        assert_eq!(
            state.clients.plugins().cache_root(),
            Some(cache_root.path())
        );

        let plugins = state.clients.plugins().list_plugins();
        assert!(
            plugins
                .iter()
                .any(|plugin| plugin.plugin_id == "explicit-probe")
        );
        assert!(
            plugins
                .iter()
                .any(|plugin| plugin.plugin_id == "path-probe")
        );
    });
}

#[test]
fn app_state_builder_projects_native_registry_into_external_policy() {
    let config = resolved_config(&[]);
    let state = AppStateBuilder::from_resolved_config(
        RuntimeContext::new(None, TerminalKind::Cli, None),
        config,
    )
    .expect("app state builder should derive host inputs")
    .with_native_commands(launch_native_registry())
    .build();

    assert!(
        state
            .clients
            .native_commands()
            .command("launch-native")
            .is_some()
    );
    assert!(
        state
            .runtime
            .auth
            .external_policy()
            .contains(&CommandPath::new(["launch-native"]))
    );
    let policy = state
        .runtime
        .auth
        .external_policy()
        .resolved_policy(&CommandPath::new(["launch-native"]))
        .expect("native policy should resolve");
    assert_eq!(policy.visibility, VisibilityMode::Authenticated);
    assert!(policy.feature_flags.contains("launch"));
}

#[test]
fn app_state_builder_tracks_session_public_helpers_from_resolved_config() {
    let config = resolved_config(&[]);
    let mut state = AppStateBuilder::from_resolved_config(
        RuntimeContext::new(None, TerminalKind::Repl, None),
        config,
    )
    .expect("app state builder should derive host inputs")
    .with_session(
        AppSession::builder()
            .with_prompt_prefix("demo")
            .with_history_enabled(false)
            .build(),
    )
    .build();

    assert_eq!(state.prompt_prefix(), "demo");

    let mut row = serde_json::Map::new();
    row.insert("uid".to_string(), serde_json::json!("alice"));
    state.record_repl_rows("ldap user alice", vec![row.clone()]);
    assert_eq!(state.repl_cache_size(), 1);
    let cached = state
        .cached_repl_rows("ldap user alice")
        .expect("cached rows should be available");
    assert_eq!(cached[0]["uid"], "alice");

    state.record_repl_failure("ldap user missing", "boom", "boom detail");
    let failure = state
        .last_repl_failure()
        .expect("last failure should be recorded");
    assert_eq!(failure.command_line, "ldap user missing");
    assert_eq!(failure.summary, "boom");
}

#[test]
fn app_state_builder_surfaces_invalid_theme_selection_errors() {
    let config = resolved_config(&[("theme.name", "definitely-not-a-theme")]);
    let err = AppStateBuilder::from_resolved_config(
        RuntimeContext::new(None, TerminalKind::Cli, None),
        config,
    )
    .err()
    .expect("invalid theme should fail during host-input derivation");

    let text = err.to_string();
    assert!(text.contains("definitely-not-a-theme"));
    assert!(text.contains("theme"));
}
