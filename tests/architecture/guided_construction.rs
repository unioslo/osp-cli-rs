#![allow(missing_docs)]

use osp_cli::App;
use osp_cli::app::{AppClients, AppSession, LaunchContext, UiState};
use osp_cli::config::{ResolveOptions, RuntimeLoadOptions};
use osp_cli::core::command_policy::{CommandAvailability, CommandPolicyOverride, VisibilityMode};
use osp_cli::core::output::{ColorMode, OutputFormat, UnicodeMode};
use osp_cli::core::runtime::{RuntimeHints, RuntimeTerminalKind, UiVerbosity};
use osp_cli::plugin::PluginDispatchContext;
use osp_cli::repl::{HistoryConfig, ReplAppearance, ReplInputMode, ReplPrompt, ReplRunConfig};
use osp_cli::ui::messages::MessageLevel;
use osp_cli::ui::{RenderRuntime, RenderSettings};

fn fixture_history_config() -> HistoryConfig {
    HistoryConfig::builder()
        .with_max_entries(16)
        .with_profile(Some("dev".to_string()))
        .build()
}

#[test]
fn canonical_guided_construction_entrypoints_remain_public_unit() {
    let _: osp_cli::ui::RenderRuntimeBuilder = RenderRuntime::builder();
    let _: osp_cli::ui::RenderSettingsBuilder = RenderSettings::builder();
    let _: osp_cli::app::AppBuilder = App::builder();
    let _ = AppSession::builder().with_prompt_prefix("demo").build();
    let _: osp_cli::repl::HistoryConfigBuilder = HistoryConfig::builder();
    let _: osp_cli::repl::ReplAppearanceBuilder = ReplAppearance::builder();
    let _: osp_cli::repl::ReplRunConfigBuilder =
        ReplRunConfig::builder(ReplPrompt::simple("osp> "), fixture_history_config());
}

#[test]
fn canonical_guided_construction_paths_build_coherent_values_unit() {
    let runtime = RenderRuntime::builder().with_stdout_is_tty(true).build();
    let render_settings = RenderSettings::builder()
        .with_runtime(runtime)
        .with_format(OutputFormat::Json)
        .build();
    let ui = UiState::new(render_settings.clone(), MessageLevel::Success, 0);
    let launch =
        LaunchContext::default().with_runtime_load(RuntimeLoadOptions::new().with_env(false));
    let clients = AppClients::default();
    let session = AppSession::builder()
        .with_prompt_prefix("demo")
        .with_history_enabled(false)
        .build();
    let history = HistoryConfig::builder()
        .with_profile(Some(" Dev ".to_string()))
        .with_terminal(Some(" REPL ".to_string()))
        .build();
    let appearance = ReplAppearance::builder().with_history_menu_rows(8).build();
    let repl = ReplRunConfig::builder(ReplPrompt::simple("osp> "), history.clone())
        .with_appearance(appearance)
        .with_input_mode(ReplInputMode::Basic)
        .build();
    let hints = RuntimeHints::new(
        UiVerbosity::Info,
        9,
        OutputFormat::Json,
        ColorMode::Always,
        UnicodeMode::Never,
    )
    .with_profile(Some(" dev ".to_string()))
    .with_terminal(Some(" xterm-256color ".to_string()))
    .with_terminal_kind(RuntimeTerminalKind::Cli);
    let dispatch = PluginDispatchContext::new(hints.clone())
        .with_shared_env([("OSP_FORMAT", "json")])
        .with_provider_override(Some("ldap".to_string()));
    let options = ResolveOptions::new()
        .with_profile(" Dev ")
        .with_terminal(" REPL ");
    let override_policy = CommandPolicyOverride::new()
        .with_visibility(Some(VisibilityMode::CapabilityGated))
        .with_availability(Some(CommandAvailability::Disabled))
        .with_required_capabilities([" Orch.Read "]);

    assert!(ui.render_settings.runtime.stdout_is_tty);
    assert!(!launch.runtime_load.include_env);
    assert!(clients.plugins().explicit_dirs().is_empty());
    assert_eq!(session.prompt_prefix, "demo");
    assert!(!session.history_enabled);
    assert_eq!(history.profile.as_deref(), Some("dev"));
    assert_eq!(history.terminal.as_deref(), Some("repl"));
    assert_eq!(repl.input_mode, ReplInputMode::Basic);
    assert_eq!(hints.debug_level, 3);
    assert_eq!(hints.profile.as_deref(), Some("dev"));
    assert_eq!(dispatch.provider_override.as_deref(), Some("ldap"));
    assert_eq!(options.profile_override.as_deref(), Some("dev"));
    assert_eq!(options.terminal.as_deref(), Some("repl"));
    assert!(override_policy.required_capabilities.contains("orch.read"));
}
