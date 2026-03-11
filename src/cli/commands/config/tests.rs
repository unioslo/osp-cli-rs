use super::{
    ConfigCommandContext, ConfigReadContext, ConfigScopeTarget, ConfigStore, ConfigStoreTarget,
    ConfigWriteTarget, config_diagnostics_rows, config_get_rows, config_store_name,
    resolve_config_scopes, resolve_config_store, resolve_scope_target, resolve_store_target,
    resolve_terminal_selector, run_config_get, run_config_set, run_config_unset,
    secrets_permissions_diagnostic, session_scoped_value, validate_write_scopes,
};
use crate::app::ReplCommandOutput;
use crate::app::{RuntimeContext, TerminalKind, UiState};
use crate::cli::{ConfigSetArgs, ConfigUnsetArgs};
use crate::config::{
    ConfigLayer, ConfigResolver, ResolveOptions, ResolvedConfig, RuntimeLoadOptions, Scope,
};
use crate::core::output::OutputFormat;
use crate::ui::RenderSettings;
use crate::ui::messages::MessageBuffer;
use crate::ui::messages::MessageLevel;
use crate::ui::theme_loader::ThemeCatalog;
use std::path::PathBuf;
use std::sync::Mutex;

fn build_resolved_config(defaults: ConfigLayer, terminal: TerminalKind) -> &'static ResolvedConfig {
    let mut resolver = ConfigResolver::default();
    resolver.set_defaults(defaults);
    Box::leak(Box::new(
        resolver
            .resolve(ResolveOptions::default().with_terminal(terminal.as_config_terminal()))
            .expect("test config should resolve"),
    ))
}

fn test_ui_state(format: OutputFormat) -> UiState {
    UiState::builder(RenderSettings::test_plain(format))
        .with_message_verbosity(MessageLevel::Success)
        .with_debug_verbosity(0)
        .build()
}

fn read_context(terminal: TerminalKind) -> ConfigReadContext<'static> {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "ops");
    let resolved = build_resolved_config(defaults, terminal);
    let context = Box::leak(Box::new(RuntimeContext::new(None, terminal, None)));
    let ui = Box::leak(Box::new(test_ui_state(OutputFormat::Table)));
    let themes = Box::leak(Box::new(ThemeCatalog::default()));
    let config_overrides = Box::leak(Box::new(ConfigLayer::default()));

    ConfigReadContext {
        context,
        config: resolved,
        ui,
        themes,
        config_overrides,
        runtime_load: RuntimeLoadOptions::default(),
    }
}

fn write_target(scope: ConfigScopeTarget) -> ConfigWriteTarget {
    ConfigWriteTarget {
        scope,
        terminal: None,
        store: ConfigStoreTarget::Default,
    }
}

fn read_context_with_defaults(
    terminal: TerminalKind,
    defaults: ConfigLayer,
) -> ConfigReadContext<'static> {
    let resolved = build_resolved_config(defaults, terminal);
    let context = Box::leak(Box::new(RuntimeContext::new(None, terminal, None)));
    let ui = Box::leak(Box::new(test_ui_state(OutputFormat::Table)));
    let themes = Box::leak(Box::new(ThemeCatalog::default()));
    let config_overrides = Box::leak(Box::new(ConfigLayer::default()));

    ConfigReadContext {
        context,
        config: resolved,
        ui,
        themes,
        config_overrides,
        runtime_load: RuntimeLoadOptions::default(),
    }
}

fn command_context(terminal: TerminalKind) -> ConfigCommandContext<'static> {
    command_context_with_format(terminal, OutputFormat::Table)
}

fn command_context_with_format(
    terminal: TerminalKind,
    format: OutputFormat,
) -> ConfigCommandContext<'static> {
    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "ops");
    let resolved = build_resolved_config(defaults, terminal);
    let context = Box::leak(Box::new(RuntimeContext::new(None, terminal, None)));
    let ui = Box::leak(Box::new(test_ui_state(format)));
    let themes = Box::leak(Box::new(ThemeCatalog::default()));
    let config_overrides = Box::leak(Box::new(ConfigLayer::default()));

    ConfigCommandContext {
        context,
        config: resolved,
        ui,
        themes,
        config_overrides,
        runtime_load: RuntimeLoadOptions::default(),
    }
}

fn config_set_args(key: &str, value: &str) -> ConfigSetArgs {
    ConfigSetArgs {
        key: key.to_string(),
        value: value.to_string(),
        global: false,
        profile: None,
        profile_all: false,
        terminal: None,
        session: false,
        config_store: false,
        secrets: false,
        save: false,
        yes: false,
        explain: false,
        dry_run: false,
    }
}

fn config_unset_args(key: &str) -> ConfigUnsetArgs {
    ConfigUnsetArgs {
        key: key.to_string(),
        global: false,
        profile: None,
        profile_all: false,
        terminal: None,
        session: false,
        config_store: false,
        secrets: false,
        save: false,
        dry_run: false,
    }
}

fn env_lock() -> &'static Mutex<()> {
    crate::tests::env_lock()
}

fn with_temp_config_paths<T>(callback: impl FnOnce(PathBuf, PathBuf) -> T) -> T {
    let _guard = env_lock().lock().expect("env lock should not be poisoned");
    let root = crate::tests::make_temp_dir("osp-cli-config-tests");
    let config_path = root.join("config.toml");
    let secrets_path = root.join("secrets.toml");
    let previous_config = std::env::var_os("OSP_CONFIG_FILE");
    let previous_secrets = std::env::var_os("OSP_SECRETS_FILE");
    unsafe {
        std::env::set_var("OSP_CONFIG_FILE", &config_path);
        std::env::set_var("OSP_SECRETS_FILE", &secrets_path);
    }

    let result = callback(config_path.clone(), secrets_path.clone());

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

#[test]
fn resolve_config_store_and_names_cover_defaults_and_explicit_targets_unit() {
    let args = write_target(ConfigScopeTarget::ActiveProfile);

    assert!(matches!(
        resolve_config_store(read_context(TerminalKind::Repl), &args),
        ConfigStore::Session
    ));
    assert!(matches!(
        resolve_config_store(read_context(TerminalKind::Cli), &args),
        ConfigStore::Config
    ));

    let repl = read_context(TerminalKind::Repl);
    assert!(matches!(
        resolve_config_store(
            repl,
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::ActiveProfile,
                terminal: None,
                store: ConfigStoreTarget::Session,
            }
        ),
        ConfigStore::Session
    ));
    assert!(matches!(
        resolve_config_store(
            repl,
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::ActiveProfile,
                terminal: None,
                store: ConfigStoreTarget::Secrets,
            }
        ),
        ConfigStore::Secrets
    ));
    assert!(matches!(
        resolve_config_store(
            repl,
            &ConfigWriteTarget {
                scope: ConfigScopeTarget::ActiveProfile,
                terminal: None,
                store: ConfigStoreTarget::Config,
            }
        ),
        ConfigStore::Config
    ));
    assert_eq!(config_store_name(ConfigStore::Session), "session");
    assert_eq!(config_store_name(ConfigStore::Config), "config");
    assert_eq!(config_store_name(ConfigStore::Secrets), "secrets");
}

#[test]
fn resolve_scope_target_store_target_and_terminal_selector_cover_precedence_helpers_unit() {
    let repl = read_context(TerminalKind::Repl);
    assert_eq!(
        resolve_terminal_selector(repl, Some(crate::app::CURRENT_TERMINAL_SENTINEL)),
        Some("repl".to_string())
    );
    assert_eq!(resolve_terminal_selector(repl, Some("  ")), None);
    assert_eq!(
        resolve_terminal_selector(repl, Some("CLI")),
        Some("cli".to_string())
    );

    assert!(matches!(
        resolve_scope_target(false, None, false),
        ConfigScopeTarget::ActiveProfile
    ));
    assert!(matches!(
        resolve_scope_target(true, Some("ops".to_string()), false),
        ConfigScopeTarget::Global
    ));
    assert!(matches!(
        resolve_scope_target(false, Some("ops".to_string()), false),
        ConfigScopeTarget::Profile(profile) if profile == "ops"
    ));
    assert!(matches!(
        resolve_scope_target(false, Some("ops".to_string()), true),
        ConfigScopeTarget::AllProfiles
    ));

    assert_eq!(
        resolve_store_target(true, true, true, true),
        ConfigStoreTarget::Session
    );
    assert_eq!(
        resolve_store_target(false, true, true, true),
        ConfigStoreTarget::Config
    );
    assert_eq!(
        resolve_store_target(false, false, true, false),
        ConfigStoreTarget::Secrets
    );
    assert_eq!(
        resolve_store_target(false, false, false, true),
        ConfigStoreTarget::Config
    );
    assert_eq!(
        resolve_store_target(false, false, false, false),
        ConfigStoreTarget::Default
    );
}

#[test]
fn resolve_config_scopes_cover_global_profile_known_profiles_and_terminal_variants_unit() {
    let cli = read_context(TerminalKind::Cli);

    let global_scopes = resolve_config_scopes(
        cli,
        &ConfigWriteTarget {
            scope: ConfigScopeTarget::Global,
            terminal: Some("cli".to_string()),
            store: ConfigStoreTarget::Default,
        },
    )
    .expect("global scopes should resolve");
    assert_eq!(global_scopes, vec![Scope::terminal("cli")]);

    let all_profile_scopes = resolve_config_scopes(
        cli,
        &ConfigWriteTarget {
            scope: ConfigScopeTarget::AllProfiles,
            terminal: Some("cli".to_string()),
            store: ConfigStoreTarget::Default,
        },
    )
    .expect("profile-all scopes should resolve");
    assert_eq!(
        all_profile_scopes,
        vec![Scope::profile_terminal("ops", "cli")]
    );

    let scopes = resolve_config_scopes(
        read_context(TerminalKind::Cli),
        &ConfigWriteTarget {
            scope: ConfigScopeTarget::Profile("Work".to_string()),
            terminal: None,
            store: ConfigStoreTarget::Default,
        },
    )
    .expect("profile scope should resolve");
    assert_eq!(scopes, vec![Scope::profile("Work")]);

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "ops");
    defaults.insert(
        "ui.format".to_string(),
        crate::config::ConfigValue::from("json"),
        Scope::profile("ops"),
    );
    defaults.insert(
        "ui.format".to_string(),
        crate::config::ConfigValue::from("table"),
        Scope::profile("dev"),
    );
    let context = read_context_with_defaults(TerminalKind::Cli, defaults);

    let all_profiles = resolve_config_scopes(
        context,
        &ConfigWriteTarget {
            scope: ConfigScopeTarget::AllProfiles,
            terminal: None,
            store: ConfigStoreTarget::Default,
        },
    )
    .expect("all known profile scopes should resolve");
    assert_eq!(
        all_profiles,
        vec![Scope::profile("dev"), Scope::profile("ops")]
    );

    let active_profile_terminal = resolve_config_scopes(
        context,
        &ConfigWriteTarget {
            scope: ConfigScopeTarget::ActiveProfile,
            terminal: Some("cli".to_string()),
            store: ConfigStoreTarget::Default,
        },
    )
    .expect("active profile terminal scope should resolve");
    assert_eq!(
        active_profile_terminal,
        vec![Scope::profile_terminal("ops", "cli")]
    );
}

#[test]
fn config_get_rows_and_run_config_get_cover_bootstrap_alias_and_missing_paths_unit() {
    let mut messages = MessageBuffer::default();
    let rows = config_get_rows(
        read_context(TerminalKind::Cli),
        &crate::cli::ConfigGetArgs {
            key: "profile.default".to_string(),
            sources: true,
            raw: false,
        },
        &mut messages,
    )
    .expect("bootstrap-only get should resolve")
    .expect("bootstrap-only key should produce a row");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("key").and_then(|value| value.as_str()),
        Some("profile.default")
    );
    assert_eq!(
        rows[0].get("source").and_then(|value| value.as_str()),
        Some("defaults")
    );
    assert!(messages.is_empty());

    let mut defaults = ConfigLayer::default();
    defaults.set("profile.default", "ops");
    defaults.set("alias.lookup", "ldap user");
    let context = read_context_with_defaults(TerminalKind::Cli, defaults);

    let alias_result = run_config_get(
        context,
        crate::cli::ConfigGetArgs {
            key: "lookup".to_string(),
            sources: false,
            raw: false,
        },
    )
    .expect("alias lookup should succeed");
    assert_eq!(alias_result.exit_code, 0);
    assert!(matches!(
        alias_result.output,
        Some(ReplCommandOutput::Output { .. })
    ));

    let missing_result = run_config_get(
        context,
        crate::cli::ConfigGetArgs {
            key: "missing.key".to_string(),
            sources: false,
            raw: false,
        },
    )
    .expect("missing key should return a structured miss");
    assert_eq!(missing_result.exit_code, 1);
    assert!(missing_result.output.is_none());
    assert!(
        missing_result
            .messages
            .render_grouped(MessageLevel::Error)
            .contains("config key not found: missing.key")
    );
}

#[test]
fn validate_write_scopes_and_session_lookup_cover_invalid_and_present_paths_unit() {
    let mut layer = ConfigLayer::default();
    layer.insert(
        "ui.format".to_string(),
        crate::config::ConfigValue::from("json"),
        Scope::profile("ops"),
    );
    assert_eq!(
        session_scoped_value(&layer, "ui.format", &Scope::profile("ops")),
        Some(crate::config::ConfigValue::from("json"))
    );
    assert_eq!(
        session_scoped_value(&layer, "ui.format", &Scope::profile("dev")),
        None
    );

    assert!(
        validate_write_scopes("profile.default", &[Scope::profile("ops")]).is_err(),
        "bootstrap-only key should reject profile scope"
    );
}

#[test]
fn run_config_set_and_unset_cover_session_paths_and_explain_output_unit() {
    let set_result = run_config_set(
        command_context(TerminalKind::Repl),
        config_set_args("ui.format", "json"),
    )
    .expect("session config set should succeed");

    assert_eq!(set_result.exit_code, 0);
    assert!(matches!(
        set_result.output,
        Some(ReplCommandOutput::Output { .. })
    ));
    assert!(
        set_result
            .messages
            .render_grouped(MessageLevel::Success)
            .contains("set value for ui.format")
    );

    let unset_context = command_context(TerminalKind::Repl);
    let active_profile = unset_context.config.active_profile().to_string();
    unset_context.config_overrides.insert(
        "ui.format".to_string(),
        crate::config::ConfigValue::from("json"),
        Scope::profile(&active_profile),
    );
    let unset_result = run_config_unset(unset_context, config_unset_args("ui.format"))
        .expect("session config unset should succeed");

    assert!(matches!(
        unset_result.output,
        Some(ReplCommandOutput::Output { .. })
    ));
    assert!(
        unset_result
            .messages
            .render_grouped(MessageLevel::Success)
            .contains("unset value for ui.format")
    );

    let mut explain_args = config_set_args("ui.format", "json");
    explain_args.explain = true;
    let result = run_config_set(
        command_context_with_format(TerminalKind::Repl, OutputFormat::Json),
        explain_args,
    )
    .expect("session config set explain should succeed");
    assert!(matches!(
        result.output,
        Some(ReplCommandOutput::Document(_))
    ));
}

#[test]
fn run_config_set_and_unset_reject_derived_profile_active_unit() {
    let mut set_args = config_set_args("profile.active", "ops");
    set_args.global = true;
    let set_err = run_config_set(command_context(TerminalKind::Cli), set_args)
        .expect_err("profile.active set should be rejected");
    assert!(set_err.to_string().contains("read-only"));

    let mut unset_args = config_unset_args("profile.active");
    unset_args.global = true;
    let unset_err = run_config_unset(command_context(TerminalKind::Cli), unset_args)
        .expect_err("profile.active unset should be rejected");
    assert!(unset_err.to_string().contains("read-only"));
}

#[test]
fn run_config_set_and_unset_cover_persistent_paths_and_warning_unit() {
    with_temp_config_paths(|config_path, secrets_path| {
        let mut config_args = config_set_args("ui.format", "json");
        config_args.config_store = true;
        let config_set = run_config_set(command_context(TerminalKind::Cli), config_args)
            .expect("persistent config set should succeed");
        assert!(config_path.exists());
        let config_payload =
            std::fs::read_to_string(&config_path).expect("config file should be readable");
        let config_root: toml::Value = config_payload
            .parse()
            .expect("config file should stay valid TOML");
        assert_eq!(
            config_root
                .get("default")
                .and_then(|value| value.get("ui"))
                .and_then(|value| value.get("format"))
                .and_then(toml::Value::as_str),
            None
        );
        assert_eq!(
            config_root
                .get("profile")
                .and_then(|value| value.get("ops"))
                .and_then(|value| value.get("ui"))
                .and_then(|value| value.get("format"))
                .and_then(toml::Value::as_str),
            Some("json")
        );
        assert!(
            config_set
                .messages
                .render_grouped(MessageLevel::Success)
                .contains("set value for ui.format")
        );

        let mut secrets_args = config_set_args("ui.format", "table");
        secrets_args.secrets = true;
        let secrets_set = run_config_set(command_context(TerminalKind::Cli), secrets_args)
            .expect("persistent secrets set should succeed");
        assert!(secrets_path.exists());
        let secrets_payload =
            std::fs::read_to_string(&secrets_path).expect("secrets file should be readable");
        let secrets_root: toml::Value = secrets_payload
            .parse()
            .expect("secrets file should stay valid TOML");
        assert_eq!(
            secrets_root
                .get("default")
                .and_then(|value| value.get("ui"))
                .and_then(|value| value.get("format"))
                .and_then(toml::Value::as_str),
            None
        );
        assert_eq!(
            secrets_root
                .get("profile")
                .and_then(|value| value.get("ops"))
                .and_then(|value| value.get("ui"))
                .and_then(|value| value.get("format"))
                .and_then(toml::Value::as_str),
            Some("table")
        );
        assert!(
            secrets_set
                .messages
                .render_grouped(MessageLevel::Success)
                .contains("set value for ui.format")
        );

        let mut unset_args = config_unset_args("ui.format");
        unset_args.secrets = true;
        let secrets_unset = run_config_unset(command_context(TerminalKind::Cli), unset_args)
            .expect("persistent secrets unset should succeed");
        assert!(
            secrets_unset
                .messages
                .render_grouped(MessageLevel::Success)
                .contains("unset value for ui.format")
        );
        let secrets_payload =
            std::fs::read_to_string(&secrets_path).expect("secrets file should still be readable");
        let secrets_root: toml::Value = secrets_payload
            .parse()
            .expect("secrets file should stay valid TOML");
        assert!(
            secrets_root
                .get("profile")
                .and_then(|value| value.get("ops"))
                .and_then(|value| value.get("ui"))
                .is_none(),
            "profile.ops.ui table should be pruned after unset: {secrets_payload}"
        );

        let mut missing_args = config_unset_args("ui.margin");
        missing_args.config_store = true;
        let missing_unset = run_config_unset(command_context(TerminalKind::Cli), missing_args)
            .expect("missing persistent unset should still succeed");
        assert!(
            missing_unset
                .messages
                .render_grouped(MessageLevel::Warning)
                .contains("no matching value for ui.margin")
        );
    });
}

#[cfg(unix)]
#[test]
fn secrets_permissions_diagnostic_covers_missing_ok_warning_and_issue_unit() {
    use std::os::unix::fs::PermissionsExt;

    let missing = secrets_permissions_diagnostic(None);
    assert_eq!(missing.status, "unavailable");

    let dir = crate::tests::make_temp_dir("osp-cli-config-secrets-diagnostic");

    let absent_path = dir.join("missing.toml");
    let absent = secrets_permissions_diagnostic(Some(absent_path));
    assert_eq!(absent.status, "missing");

    let ok_path = dir.join("ok.toml");
    std::fs::write(&ok_path, "token = 'secret'\n").expect("fixture should be written");
    std::fs::set_permissions(&ok_path, std::fs::Permissions::from_mode(0o600))
        .expect("permissions should be set");
    let ok = secrets_permissions_diagnostic(Some(ok_path.clone()));
    assert_eq!(ok.status, "ok");
    assert_eq!(ok.mode, serde_json::Value::String("600".to_string()));

    let warning_path = dir.join("warning.toml");
    std::fs::write(&warning_path, "token = 'secret'\n").expect("fixture should be written");
    std::fs::set_permissions(&warning_path, std::fs::Permissions::from_mode(0o400))
        .expect("permissions should be set");
    let warning = secrets_permissions_diagnostic(Some(warning_path));
    assert_eq!(warning.status, "warning");
    assert_eq!(warning.mode, serde_json::Value::String("400".to_string()));
    assert!(warning.message.contains("0600 is recommended"));

    let issue_path = dir.join("issue.toml");
    std::fs::write(&issue_path, "token = 'secret'\n").expect("fixture should be written");
    std::fs::set_permissions(&issue_path, std::fs::Permissions::from_mode(0o644))
        .expect("permissions should be set");
    let issue = secrets_permissions_diagnostic(Some(issue_path));
    assert_eq!(issue.status, "issue");
    assert!(
        issue
            .message
            .contains("owner-only permissions are required")
    );
}

#[test]
fn config_diagnostics_rows_include_secrets_status_unit() {
    let rows = config_diagnostics_rows(read_context(TerminalKind::Cli));
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains_key("secrets_permissions_status"));
    assert!(rows[0].contains_key("theme_issue_count"));
}
